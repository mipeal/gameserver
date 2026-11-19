//! Port of `controlserver/service_mgr.py` — git checkout/pull of service
//! repos, import into the `services` table, and uploading checker scripts
//! into DB packages via `db_filesystem`.

use crate::db_filesystem;
use gamelib_rs::service_config::ServiceConfig;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::path::{Path, PathBuf};

pub struct ServiceRepoManager {
    pub messages: Vec<String>,
}

fn _getuser() -> u32 {
    #[cfg(unix)]
    return unsafe { libc::getuid() };
    #[cfg(not(unix))]
    return 0;
}

impl Default for ServiceRepoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRepoManager {
    pub fn new() -> ServiceRepoManager {
        ServiceRepoManager {
            messages: Vec::new(),
        }
    }

    fn report(&mut self, msg: String) {
        println!("{msg}");
        self.messages.push(msg);
    }

    /// `_directory` — SERVICES_PATH / <basename without .git>.
    fn directory(&self, cfg: &Config, remote_url: &str) -> PathBuf {
        let mut name = if let Some(pos) = remote_url.rfind('/') {
            &remote_url[pos + 1..]
        } else {
            remote_url
        };
        if let Some(stripped) = name.strip_suffix(".git") {
            name = stripped;
        }
        Path::new(&cfg.services_path).join(name)
    }

    /// `is_owner` — SERVICES_PATH owner == current uid (Unix).
    pub fn is_owner(&self, cfg: &Config) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(&cfg.services_path) {
                Ok(md) => md.uid() == _getuser(),
                Err(_) => false,
            }
        }
        #[cfg(not(unix))]
        {
            let _ = cfg;
            true
        }
    }

    /// `git_checkout_service` — clone unless it already exists. Returns true if cloned.
    pub fn git_checkout_service(&mut self, cfg: &Config, remote_url: &str) -> Result<bool, String> {
        let dir = self.directory(cfg, remote_url);
        if dir.exists() && dir.join(".git").exists() {
            return Ok(false);
        }
        self.report(format!("Checking out {remote_url} ..."));
        let out = std::process::Command::new("git")
            .args(["clone", remote_url, &dir.to_string_lossy()])
            .current_dir(dir.parent().unwrap_or(Path::new(".")))
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "git clone failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        self.report(format!("Populated: {}", dir.to_string_lossy()));
        Ok(true)
    }

    /// `git_pull_service` — stash/pull --rebase/stash-apply when dirty.
    pub fn git_pull_service(&mut self, cfg: &Config, remote_url: &str) -> Result<(), String> {
        let dir = self.directory(cfg, remote_url);
        let run = |args: &[&str], cwd: &Path| -> Result<std::process::Output, String> {
            std::process::Command::new(args[0])
                .args(&args[1..])
                .current_dir(cwd)
                .output()
                .map_err(|e| e.to_string())
        };
        let is_unclean = run(&["git", "diff", "--quiet"], &dir)
            .map(|o| !o.status.success())
            .unwrap_or(false);
        if is_unclean {
            run(&["git", "stash"], dir.parent().unwrap_or(Path::new(".")))
                .map_err(|e| e.to_string())?;
        }
        let result = (|| -> Result<(), String> {
            self.report(format!("Pulling {}", dir.to_string_lossy()));
            let out = run(&["git", "pull", "--rebase"], &dir)?;
            if !out.status.success() {
                return Err(format!(
                    "git pull failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            Ok(())
        })();
        if is_unclean {
            run(
                &["git", "stash", "apply"],
                dir.parent().unwrap_or(Path::new(".")),
            )
            .map_err(|e| e.to_string())?;
        }
        result?;
        self.report(format!("Pulled {}", dir.to_string_lossy()));
        Ok(())
    }

    /// `find_services` — every SERVICES_PATH subdir with a checkers/config.toml.
    pub fn find_services(&mut self, cfg: &Config) -> Vec<(PathBuf, ServiceConfig)> {
        let mut results = Vec::new();
        let Ok(entries) = std::fs::read_dir(&cfg.services_path) else {
            return results;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            match ServiceConfig::from_file(dir.join("checkers").join("config.toml")) {
                Ok(c) => results.push((dir, c)),
                Err(_) => self.report(format!(
                    "Not a service (or no config): {}",
                    dir.to_string_lossy()
                )),
            }
        }
        results
    }

    /// `upload_checker_scripts` — main package + optional setup package
    /// (dependencies.sh / checker-requirements.txt next to the checker dir).
    pub async fn upload_checker_scripts(
        &mut self,
        db: &PgPool,
        checker_script_dir: &str,
    ) -> Result<(String, Option<String>, bool), String> {
        let (package, is_new) =
            db_filesystem::move_folder_to_package(db, Path::new(checker_script_dir)).await?;
        let base = Path::new(checker_script_dir)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let fnames: Vec<PathBuf> = [
            base.join("dependencies.sh"),
            base.join("checker-requirements.txt"),
        ]
        .into_iter()
        .filter(|f| f.exists())
        .collect();
        let setup_package = if fnames.is_empty() {
            None
        } else {
            let (sp, _) = db_filesystem::move_single_files_to_package(db, &fnames).await?;
            Some(sp)
        };
        Ok((package, setup_package, is_new))
    }

    /// `_cfg_to_db` — a new services row from a config.toml.
    async fn cfg_to_db(
        &mut self,
        db: &PgPool,
        dir: &Path,
        cfg: &ServiceConfig,
    ) -> Result<(), String> {
        let checker_dir = dir.join("checkers").to_string_lossy().into_owned();
        let (package, setup_package, _) = self.upload_checker_scripts(db, &checker_dir).await?;
        sqlx::query(
            "INSERT INTO services (name, checker_script_dir, checker_script, num_payloads, flag_ids, flags_per_tick, ports, package, setup_package) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&cfg.name)
        .bind(&checker_dir)
        .bind(format!("{}:{}", cfg.interface_file, cfg.interface_class))
        .bind(cfg.num_payloads as i32)
        .bind(cfg.flag_ids.join(","))
        .bind(cfg.flags_per_tick)
        .bind(cfg.ports.join(","))
        .bind(&package)
        .bind(setup_package)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
        self.report(format!("Added \"{}\" to DB", cfg.name));
        Ok(())
    }

    /// `_cfg_equals_db` — DB row still matches an on-disk config (untouched manually).
    fn cfg_equals_db(dir: &Path, cfg: &ServiceConfig, row: &ServiceRow) -> bool {
        row.name == cfg.name
            && row.checker_script_dir.as_deref() == Some(&dir.join("checkers").to_string_lossy())
            && row.checker_script == format!("{}:{}", cfg.interface_file, cfg.interface_class)
            && row.num_payloads as i64 == cfg.num_payloads
            && row.flag_ids.as_deref() == Some(&cfg.flag_ids.join(","))
            && row.flags_per_tick == cfg.flags_per_tick
            && row.ports == cfg.ports.join(",")
    }

    /// `import_services` — add disk services missing from the DB.
    pub async fn import_services(&mut self, cfg: &Config, db: &PgPool) -> Result<(), String> {
        let disk_services = self.find_services(cfg);
        for (dir, cfg) in disk_services {
            let exists: Option<i32> = sqlx::query_scalar("SELECT id FROM services WHERE name = $1")
                .bind(&cfg.name)
                .fetch_optional(db)
                .await
                .map_err(|e| e.to_string())?;
            if exists.is_none() {
                self.cfg_to_db(db, &dir, &cfg).await?;
            }
        }
        Ok(())
    }

    /// `update_service` — checkout or pull one remote and sync config changes
    /// into the DB (only if the row was not altered manually).
    pub async fn update_service(
        &mut self,
        cfg: &Config,
        db: &PgPool,
        remote_url: &str,
    ) -> Result<(), String> {
        let dir = self.directory(cfg, remote_url);
        if !dir.exists() {
            self.git_checkout_service(cfg, remote_url)?;
            let cfg = ServiceConfig::from_file(dir.join("checkers").join("config.toml"))?;
            self.cfg_to_db(db, &dir, &cfg).await?;
            return Ok(());
        }
        let cfg_before = ServiceConfig::from_file(dir.join("checkers").join("config.toml"))?;
        self.git_pull_service(cfg, remote_url)?;
        let cfg_after = ServiceConfig::from_file(dir.join("checkers").join("config.toml"))?;
        let rows = sqlx::query_as::<_, ServiceRow>(
            "SELECT id, name, checker_script_dir, checker_script, num_payloads, flag_ids, flags_per_tick, ports FROM services",
        )
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(row) = rows.iter().find(|r| r.name == cfg_after.name) {
            let changed = to_toml_value(&cfg_before) != to_toml_value(&cfg_after);
            if changed && Self::cfg_equals_db(&dir, &cfg_before, row) {
                sqlx::query(
                    "UPDATE services SET name=$1, checker_script_dir=$2, checker_script=$3, num_payloads=$4, flag_ids=$5, flags_per_tick=$6, ports=$7 WHERE id=$8",
                )
                .bind(&cfg_after.name)
                .bind(dir.join("checkers").to_string_lossy().as_ref())
                .bind(format!("{}:{}", cfg_after.interface_file, cfg_after.interface_class))
                .bind(cfg_after.num_payloads as i32)
                .bind(cfg_after.flag_ids.join(","))
                .bind(cfg_after.flags_per_tick)
                .bind(cfg_after.ports.join(","))
                .bind(row.id)
                .execute(db)
                .await
                .map_err(|e| e.to_string())?;
                self.report(format!(
                    "Updating service {}: {} because config changed",
                    row.id, row.name
                ));
            }
        }
        Ok(())
    }

    /// `update_all_services` — owner check, then every SERVICE_REMOTES entry.
    pub async fn update_all_services(&mut self, cfg: &Config, db: &PgPool) -> Result<(), String> {
        std::fs::create_dir_all(&cfg.services_path).map_err(|e| e.to_string())?;
        if !self.is_owner(cfg) {
            return Err(format!(
                "PermissionError: You are not the owner of {}. Please do not mess up permissions.",
                cfg.services_path
            ));
        }
        for remote in cfg.service_remotes.clone() {
            self.update_service(cfg, db, &remote).await?;
        }
        Ok(())
    }
}

/// Minimal row projection for the config-vs-DB comparison.
#[derive(sqlx::FromRow, Debug, Clone)]
struct ServiceRow {
    id: i32,
    name: String,
    checker_script_dir: Option<String>,
    checker_script: String,
    num_payloads: i32,
    flag_ids: Option<String>,
    flags_per_tick: f64,
    ports: String,
}

/// Serialize a ServiceConfig so two configs compare structurally
/// (Python compared the dataclasses with `!=`).
fn to_toml_value(cfg: &ServiceConfig) -> String {
    format!(
        "{}|{}|{}|{:?}|{:?}|{}|{}|{}",
        cfg.name,
        cfg.interface_file,
        cfg.interface_class,
        cfg.flag_ids,
        cfg.ports,
        cfg.num_payloads,
        cfg.flags_per_tick,
        cfg.service_id
    )
}
