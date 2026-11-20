//! Port of `checker_runner/runner.py` + `runners/saarctf.py` execution path.
//! Every checker runs as a Python subprocess through the existing
//! `checker_runner/runners/saarctf.py` entry point (ADR-013: checkers stay
//! Python), so subprocess semantics are the ones that matter: timeout at the
//! soft limit → TIMEOUT, non-zero exit → CRASHED, result after SEPARATOR.

use crate::dispatcher::TaskMessage;
use serde_json::Value;
use sqlx::Row;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// `PackageLoader.ensure_package_exists` + `DBFilesystem.load_package_to_folder`
/// (the checker subprocess is disk-only now, ADR-017): materialize a package
/// from `checker_filesystem`/`checker_files` into `<packages_path>/<package>`,
/// large files (>500 KB) symlinked from the LFS dir. Returns true if it was
/// freshly loaded, false if the folder already existed.
pub async fn ensure_package_exists(
    db: &sqlx::PgPool,
    package: &str,
    packages_path: &str,
    lfs_path: Option<&str>,
) -> Result<bool, String> {
    use std::path::PathBuf;
    let folder = Path::new(packages_path).join(package);
    if folder.exists() {
        return Ok(false);
    }
    let base = PathBuf::from(format!("{}.tmp", folder.display()));
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    if let Some(lfs) = lfs_path {
        std::fs::create_dir_all(lfs).map_err(|e| e.to_string())?;
    }
    let rows = sqlx::query(
        "SELECT path, file_hash FROM checker_filesystem WHERE package = $1 ORDER BY path",
    )
    .bind(package)
    .fetch_all(db)
    .await
    .map_err(|e| e.to_string())?;
    for row in rows {
        let path: String = row.try_get("path").map_err(|e| e.to_string())?;
        let file_hash: Option<String> = row.try_get("file_hash").map_err(|e| e.to_string())?;
        let res: Result<(), String> = match file_hash {
            Some(hash) => {
                let lfs_file = lfs_path.map(|l| Path::new(l).join(&hash));
                if lfs_file.as_deref().is_some_and(|p| p.exists()) {
                    #[cfg(unix)]
                    return_link(lfs_file.as_deref().unwrap(), &base, &path);
                    Ok(())
                } else {
                    let content: Vec<u8> =
                        sqlx::query("SELECT content FROM checker_files WHERE file_hash = $1")
                            .bind(&hash)
                            .fetch_one(db)
                            .await
                            .map_err(|e| e.to_string())?
                            .try_get("content")
                            .map_err(|e| e.to_string())?;
                    match lfs_path {
                        Some(lfs) if content.len() > 500_000 => {
                            let lfs_file = Path::new(lfs).join(&hash);
                            if !lfs_file.exists() {
                                std::fs::write(&lfs_file, &content).map_err(|e| e.to_string())?;
                                #[cfg(unix)]
                                std::fs::set_permissions(
                                    &lfs_file,
                                    std::os::unix::fs::PermissionsExt::from_mode(0o400),
                                )
                                .map_err(|e| e.to_string())?;
                            }
                            #[cfg(unix)]
                            return_link(&lfs_file, &base, &path);
                            #[cfg(not(unix))]
                            std::fs::write(base.join(&path), &content)
                                .map_err(|e| e.to_string())?;
                            Ok(())
                        }
                        _ => std::fs::write(base.join(&path), &content).map_err(|e| e.to_string()),
                    }
                }
            }
            None => std::fs::create_dir_all(base.join(&path)).map_err(|e| e.to_string()),
        };
        if let Err(e) = res {
            // a concurrent worker renamed the .tmp dir away mid-write
            return if folder.exists() { Ok(false) } else { Err(e) };
        }
    }
    // os.rename(base, folder); a concurrent worker may have won the race (the
    // original held a FileLock here — second locker returns False untouched)
    match std::fs::rename(&base, &folder) {
        Ok(()) => Ok(true),
        Err(_) if folder.exists() => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(unix)]
fn return_link(target: &Path, base: &Path, rel: &str) {
    let _ = std::os::unix::fs::symlink(target, base.join(rel));
}

/// `CheckerRunOutput` (checker_execution.py).
#[derive(Debug, Clone)]
pub struct CheckerRunOutput {
    pub status: String,
    pub message: Option<String>,
    pub output: Option<String>,
    pub data: Option<Value>,
}

const SEPARATOR: &str =
    "\n\n------------------------------------------------------------------------";

/// `run_checkerscript`: debug specials + the subprocess run with the soft limit
/// as timeout (celery soft_time_limit, ADR-015). (`eno:*` runner specs are
/// dispatched natively in worker.rs — checker_runner/runners/eno.py port.)
pub async fn run_checker(task: &TaskMessage, python_dir: &std::path::Path) -> CheckerRunOutput {
    let mut script = task.checker_script.clone();
    if script == "crashtest" {
        // Python: raise Exception("Invalid script!") → celery FAILURE → CRASHED row
        return CheckerRunOutput {
            status: "CRASHED".into(),
            message: None,
            output: Some("<class 'Exception'> Invalid script!".into()),
            data: None,
        };
    }
    if script == "sleeptest" {
        // "hard sleeper": ignores the soft timeout, killed by the hard limit
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        return CheckerRunOutput {
            status: "SUCCESS".into(),
            message: None,
            output: Some("Wakeup".into()),
            data: None,
        };
    }
    if script == "pendingtest" {
        script = "checker_runner.demo_checker:TimeoutService".to_string();
    }
    run_saarctf_subprocess(task, &script, python_dir).await
}

/// `SaarctfServiceRunner.execute_checker_subprocess` — now spawns the Rust
/// `checker-exec` binary (embedded CPython shim, ADR-017) instead of
/// `python3 checker_runner/runners/saarctf.py`. Service config comes via argv.
async fn run_saarctf_subprocess(
    task: &TaskMessage,
    script: &str,
    python_dir: &std::path::Path,
) -> CheckerRunOutput {
    let mut cmd = Command::new("checker-exec");
    cmd.arg(task.package.as_deref().unwrap_or(""))
        .arg(script)
        .arg(task.service_id.to_string())
        .arg(task.team_id.to_string())
        .arg(task.tick.to_string())
        .arg(&task.service_name)
        .arg(&task.flag_ids)
        // shim imports gamelib/saarctf_commons from here (as the original did)
        .env("PYTHONPATH", python_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // set_limits(): RLIMIT_AS 2GB soft / 3GB hard, unless SAARCTF_NO_RLIMIT is
    // set (that one is inherited from our own environment).
    #[cfg(unix)]
    if std::env::var_os("SAARCTF_NO_RLIMIT").is_none() {
        // SAFETY: runs setrlimit in the forked child before exec; no shared state touched.
        unsafe {
            cmd.pre_exec(|| {
                let soft: u64 = 2048 * 1_000_000;
                let hard: u64 = 3072 * 1_000_000;
                // fallback: libc is already in the tree via sqlx/redis deps
                set_rlimit_as(soft, hard);
                Ok(())
            });
        }
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CheckerRunOutput {
                status: "CRASHED".into(),
                message: None,
                output: Some(format!("SubprocessError: {e}")),
                data: None,
            }
        }
    };
    let run = async { child.wait_with_output().await };
    match tokio::time::timeout(
        std::time::Duration::from_secs(task.soft_limit.max(1) as u64),
        run,
    )
    .await
    {
        Err(_) => {
            // TimeoutExpired (kill_on_drop SIGKILLs the child on drop)
            CheckerRunOutput {
                status: "TIMEOUT".into(),
                message: Some("Timeout, service too slow".into()),
                output: None,
                data: None,
            }
        }
        Ok(Err(e)) => CheckerRunOutput {
            status: "CRASHED".into(),
            message: None,
            output: Some(format!("SubprocessError: {e}")),
            data: None,
        },
        Ok(Ok(out)) => {
            let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
            output.push_str(&String::from_utf8_lossy(&out.stderr));
            let output = output.replace('\x00', "<0x00>");
            if !out.status.success() {
                // CalledProcessError → CRASHED with the captured output
                return CheckerRunOutput {
                    status: "CRASHED".into(),
                    message: None,
                    output: Some(output),
                    data: None,
                };
            }
            match output.rfind(SEPARATOR) {
                Some(p) => {
                    let tail = &output[p + SEPARATOR.len() + 1..];
                    let (status, message) = tail.split_once('|').unwrap_or((tail, ""));
                    CheckerRunOutput {
                        status: status.trim().to_string(),
                        message: if message.trim().is_empty() {
                            None
                        } else {
                            Some(message.trim().to_string())
                        },
                        output: Some(output),
                        data: None,
                    }
                }
                None => CheckerRunOutput {
                    // Python: rfind raises → task FAILURE → CRASHED
                    status: "CRASHED".into(),
                    message: None,
                    output: Some(output),
                    data: None,
                },
            }
        }
    }
}

#[cfg(unix)]
fn set_rlimit_as(soft: u64, hard: u64) {
    unsafe {
        let lim = libc::rlimit {
            rlim_cur: soft,
            rlim_max: hard,
        };
        let _ = libc::setrlimit(libc::RLIMIT_AS, &lim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn crashtest_special_returns_crashed() {
        let task = TaskMessage {
            task_id: "t".into(),
            runner_spec: "saarctf:SaarctfServiceRunner".into(),
            package: None,
            checker_script: "crashtest".into(),
            service_id: 1,
            team_id: 1,
            tick: 1,
            runner_config: None,
            hard_limit: 35,
            soft_limit: 30,
            deliver_at: None,
            kind: None,
            args: None,
            service_name: "demoservice".into(),
            flag_ids: String::new(),
        };
        let out = run_checker(&task, std::path::Path::new(".")).await;
        assert_eq!(out.status, "CRASHED");
        assert!(out.output.unwrap().contains("Invalid script!"));
    }
}
