//! Simulation boot steps (plan task 3): when `SAARCTF_SIMULATION=1`, apply the
//! sim seed, import the sim checker sources from `services_path`, enable them,
//! and auto-start the CTF. With the env unset nothing here runs — the real
//! pipeline is byte-identical (ADR-006).

use sqlx::PgPool;
use saarctf_commons_rs::config::Config;

use crate::service_mgr::ServiceRepoManager;

/// Injectable core: env value -> simulation mode on?
pub fn simulation_enabled_with(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Reads `SAARCTF_SIMULATION` from the real environment.
pub fn simulation_enabled() -> bool {
    simulation_enabled_with(std::env::var("SAARCTF_SIMULATION").ok().as_deref())
}

/// Seed + import + enable, in that order; idempotent (seed uses ON CONFLICT
/// DO NOTHING, import skips existing names, UPDATE is a no-op when already set).
pub async fn run(cfg: &Config, db: &PgPool, seed_path: Option<&str>) -> Result<(), String> {
    if let Some(path) = seed_path {
        let sql = std::fs::read_to_string(path)
            .map_err(|e| format!("sim seed unreadable at {path}: {e}"))?;
        sqlx::raw_sql(&sql)
            .execute(db)
            .await
            .map_err(|e| format!("sim seed failed: {e}"))?;
        tracing::info!("simulation mode: seed applied from {path}");
    }
    ServiceRepoManager::new()
        .import_services(cfg, db)
        .await?;
    sqlx::query("UPDATE services SET checker_enabled = TRUE")
        .execute(db)
        .await
        .map_err(|e| format!("sim enable checkers failed: {e}"))?;
    Ok(())
}
