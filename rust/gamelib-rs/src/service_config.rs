//! Port of `gamelib.ServiceConfig` (gamelib.py:69) — the per-service
//! `checkers/config.toml`. Only what the controlserver needs (ADR-013).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub interface_file: String,
    pub interface_class: String,
    #[serde(default)]
    pub flag_ids: Vec<String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default = "one")]
    pub num_payloads: i64,
    #[serde(default = "one_f")]
    pub flags_per_tick: f64,
    #[serde(default = "one")]
    pub service_id: i64,
}

fn one() -> i64 {
    1
}
fn one_f() -> f64 {
    1.0
}

impl ServiceConfig {
    /// `ServiceConfig.from_file` — TOML. Missing optional fields get the
    /// dataclass defaults; missing required fields are a ValueError.
    pub fn from_file(filename: impl AsRef<Path>) -> Result<ServiceConfig, String> {
        let text = std::fs::read_to_string(filename.as_ref())
            .map_err(|e| format!("FileNotFoundError: {e}"))?;
        let cfg: ServiceConfig = toml::from_str(&text).map_err(|e| format!("ValueError: {e}"))?;
        // __post_init__: flag ids must not contain commas (the DB column is ,-joined)
        if cfg.flag_ids.iter().any(|id| id.contains(',')) {
            return Err("ValueError: Flag IDs with , are not supported".into());
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_toml_with_defaults() {
        let dir = std::env::temp_dir().join(format!("svc-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        std::fs::write(&f, "name = 'demo'\ninterface_file = 'checker.py'\ninterface_class = 'Checker'\nports = ['tcp:1234']\n").unwrap();
        let cfg = ServiceConfig::from_file(&f).unwrap();
        assert_eq!(cfg.name, "demo");
        assert_eq!(cfg.num_payloads, 1);
        assert_eq!(cfg.flags_per_tick, 1.0);
        assert!(cfg.flag_ids.is_empty());
    }

    #[test]
    fn rejects_comma_flag_ids_like_post_init() {
        let dir = std::env::temp_dir().join(format!("svc-cfg2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.toml");
        std::fs::write(&f, "name = 'demo'\ninterface_file = 'c.py'\ninterface_class = 'C'\nflag_ids = ['a,b']\n").unwrap();
        assert!(ServiceConfig::from_file(&f).is_err());
    }
}
