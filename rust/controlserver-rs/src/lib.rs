//! Wave-1 tangled core (ADR-009/012/015): controlserver + checker_runner as one
//! axum/tokio crate with two binaries. Schema: migrations/0001_baseline.sql (ADR-005).

pub mod db_filesystem;
pub mod dispatcher;
pub mod endpoints;
pub mod eno;
pub mod events;
pub mod flag_id_file;
pub mod logger;
pub mod models;
pub mod runners;
pub mod scoreboard;
pub mod scoring;
pub mod service_mgr;
pub mod timer;
pub mod worker;

use saarctf_commons_rs::config::Config;
use std::path::PathBuf;

/// `load_default_config()` as app.py does it: `CONFIG` env var or the default
/// config file, env interpolation on.
pub fn load_config() -> Result<Config, String> {
    let filename = match std::env::var("CONFIG") {
        Ok(f) => PathBuf::from(f),
        Err(_) => {
            // saarctf_commons.config.load_default_config_file uses DEFAULT_CONFIG_FILE
            PathBuf::from("../config/config.yaml")
        }
    };
    Config::load_from_file(&filename, true)
}

/// `postgres://` URL from the config's databases.postgres section.
/// (Socket mode has no server — sqlx needs a host, so that errors clearly.)
pub fn postgres_url(cfg: &Config) -> Result<String, String> {
    let pg = cfg
        .postgres
        .as_object()
        .ok_or_else(|| "ValueError: no postgres section".to_string())?;
    let get = |k: &str| pg.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let server = get("server");
    if server.is_empty() || cfg.postgres_use_socket {
        return Err("ValueError: unix-socket postgres not supported; set server".into());
    }
    let port = pg.get("port").and_then(|v| v.as_u64()).unwrap_or(5432);
    Ok(format!(
        "postgres://{}:{}@{}:{}/{}",
        get("username"),
        get("password"),
        server,
        port,
        get("database")
    ))
}

/// Engine setup (`init_database`): a sqlx pool where SQLAlchemy had one.
pub async fn connect_db(cfg: &Config) -> Result<sqlx::PgPool, sqlx::Error> {
    let url = postgres_url(cfg).map_err(|e| sqlx::Error::Configuration(e.into()))?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(50)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&url)
        .await
}
