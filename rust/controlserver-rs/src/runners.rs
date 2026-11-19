//! Port of `checker_runner/runner.py` + `runners/saarctf.py` execution path.
//! Every checker runs as a Python subprocess through the existing
//! `checker_runner/runners/saarctf.py` entry point (ADR-013: checkers stay
//! Python), so subprocess semantics are the ones that matter: timeout at the
//! soft limit → TIMEOUT, non-zero exit → CRASHED, result after SEPARATOR.

use crate::dispatcher::TaskMessage;
use serde_json::Value;
use std::process::Stdio;
use tokio::process::Command;

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

/// `SaarctfServiceRunner.execute_checker_subprocess`.
async fn run_saarctf_subprocess(
    task: &TaskMessage,
    script: &str,
    python_dir: &std::path::Path,
) -> CheckerRunOutput {
    let script_path = python_dir
        .join("checker_runner")
        .join("runners")
        .join("saarctf.py");
    let mut cmd = Command::new("python3");
    cmd.arg(script_path)
        .arg(task.package.as_deref().unwrap_or(""))
        .arg(script)
        .arg(task.service_id.to_string())
        .arg(task.team_id.to_string())
        .arg(task.tick.to_string())
        .current_dir(python_dir)
        // set at saarctf.py import for pwntools
        .env("TERM", "xterm")
        .env("PWNLIB_NOTERM", "1")
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
        };
        let out = run_checker(&task, std::path::Path::new(".")).await;
        assert_eq!(out.status, "CRASHED");
        assert!(out.output.unwrap().contains("Invalid script!"));
    }
}
