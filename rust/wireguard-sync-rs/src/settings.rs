//! `wireguard_sync/settings.py` — env defaults, overridden by the ctf
//! config's `wireguard_sync` section (keystore then lands next to the config
//! file, like `config.basedir / "keystore.json"`).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Settings {
    pub api_server: String,
    pub api_base: String,
    pub api_token: Option<String>,
    pub api_concurrency: i64,
    pub keystore_path: PathBuf,
    /// `INTERFACE_UP_HOOKS` — absolute paths resolved from the Python tree
    /// layout (BASE_DIR/../../vpn/...); the Rust binary takes them from the
    /// config's basedir instead.
    pub interface_up_hooks: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            api_server: "http://ctf.localhost".into(),
            api_base: "/api/router/".into(),
            api_token: None,
            api_concurrency: 1,
            keystore_path: PathBuf::from("./keystore.json"),
            interface_up_hooks: vec![],
        }
    }
}

impl Settings {
    /// env vars, then the ctf config section (Python order: env first,
    /// ctf settings win if present).
    pub fn load() -> Result<Settings, String> {
        let mut s = Settings::default();
        if let Ok(v) = std::env::var("API_SERVER") {
            s.api_server = v;
        }
        if let Ok(v) = std::env::var("API_BASE") {
            s.api_base = v;
        }
        if let Ok(v) = std::env::var("API_TOKEN") {
            s.api_token = Some(v);
        }
        if let Ok(v) = std::env::var("API_CONCURRENCY") {
            s.api_concurrency = v.parse().map_err(|_| "ValueError: API_CONCURRENCY")?;
        }
        if let Ok(v) = std::env::var("KEYSTORE_PATH") {
            s.keystore_path = PathBuf::from(v);
        }

        let cfg = match super::load_config() {
            Ok(c) => c,
            Err(_) => return Ok(s), // "No ctf settings available"
        };
        match &cfg.wireguard_sync {
            None => tracing::info!("Wireguard sync not configured in ctf settings"),
            Some(wg) => {
                println!("Imported ctf settings");
                s.api_server = wg.api_server.clone();
                s.api_base = wg.api_base.clone();
                s.api_token = Some(wg.api_token.clone());
                s.api_concurrency = wg.api_concurrency.as_i64().unwrap_or(1);
                s.keystore_path = cfg.basedir.join("keystore.json");
            }
        }
        // hooks relative to the config file, like the Python tree layout
        s.interface_up_hooks = vec![
            cfg.vpn_base_dir.join("bpf").join("install.sh"),
            cfg.vpn_base_dir.join("ratelimit").join("install.sh"),
        ];
        Ok(s)
    }
}
