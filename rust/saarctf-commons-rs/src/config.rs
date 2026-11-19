//! Port of `saarctf_commons/config.py` — config file loading with the exact
//! merge/default semantics of the Python original (ADR-014: verified by goldens,
//! see `tests/golden.rs`).

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::network::NetworkConfig;

// ---------------------------------------------------------------------------
// helpers mirroring the small Python utilities
// ---------------------------------------------------------------------------

/// `opt_path`: `Path(s) if s else None`.
pub fn opt_path(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Python tree root for subprocesses and generated scripts: `SAARCTF_PYTHON_DIR`
/// env, defaulting to the config file's directory.
pub fn python_dir(cfg: &Config) -> PathBuf {
    std::env::var("SAARCTF_PYTHON_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            cfg.config_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        })
}

fn as_object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("'{what}' must be a dict"))
}

fn get_str<'a>(d: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    d.get(key).and_then(Value::as_str)
}

/// `initial_config.get(key, default)` for string-ish values.
fn get_str_or<'a>(d: &'a Map<String, Value>, key: &str, default: &'a str) -> &'a str {
    d.get(key).and_then(Value::as_str).unwrap_or(default)
}

/// `initial_config.get(key, False) or None` — falsy (None/""/False) becomes `None`.
fn url_or_none(d: &Map<String, Value>, key: &str) -> Option<String> {
    match d.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// `binascii.unhexlify` — even length, hex digits only.
fn unhexlify(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("binascii.Error: Odd-length string".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| {
                format!(
                    "binascii.Error: Non-hexadecimal digit found '{}'",
                    &s[i..i + 2]
                )
            })
        })
        .collect()
}

fn hexlify(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `_clean_comments`: drop `__*` keys, recursively.
fn clean_comments(d: &mut Value) {
    if let Value::Object(map) = d {
        let drop: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with("__"))
            .cloned()
            .collect();
        for k in drop {
            map.remove(&k);
        }
        for v in map.values_mut() {
            clean_comments(v);
        }
    }
}

/// `Path(a) / b` string join (absolute paths only in this codebase's configs).
fn path_join(base: &str, next: &str) -> String {
    if next.is_empty() {
        return base.to_string();
    }
    format!("{}/{next}", base.trim_end_matches('/'))
}

// ---------------------------------------------------------------------------
// config sections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ScoringConfig {
    pub flags_rounds_valid: Value,
    pub nop_team_id: Value,
    pub off_factor: Value,
    pub def_factor: Value,
    pub sla_factor: Value,
    pub algorithm: String,
    pub firstblood_algorithm: String,
    /// `data`: every key the original does not know about, passed through.
    pub data: Map<String, Value>,
}

const SCORING_ALGORITHM: &str = "saarctf:SaarctfScoreAlgorithm";
const FIRSTBLOOD_ALGORITHM: &str = "firstblood:DefaultFirstBloodAlgorithm";

impl ScoringConfig {
    /// `ScoringConfig.from_dict`: known fields, everything else into `data`.
    fn from_dict(d: &Map<String, Value>) -> ScoringConfig {
        let mut s = ScoringConfig {
            flags_rounds_valid: 10.into(),
            nop_team_id: 1.into(),
            off_factor: 1.0.into(),
            def_factor: 1.0.into(),
            sla_factor: 1.0.into(),
            algorithm: SCORING_ALGORITHM.into(),
            firstblood_algorithm: FIRSTBLOOD_ALGORITHM.into(),
            data: Map::new(),
        };
        for (k, v) in d {
            match k.as_str() {
                "flags_rounds_valid" => s.flags_rounds_valid = v.clone(),
                "nop_team_id" => s.nop_team_id = v.clone(),
                "off_factor" => s.off_factor = v.clone(),
                "def_factor" => s.def_factor = v.clone(),
                "sla_factor" => s.sla_factor = v.clone(),
                "algorithm" => s.algorithm = v.as_str().unwrap_or_default().to_owned(),
                "firstblood_algorithm" => {
                    s.firstblood_algorithm = v.as_str().unwrap_or_default().to_owned()
                }
                _ => {
                    s.data.insert(k.clone(), v.clone());
                }
            }
        }
        s
    }

    pub fn to_dict(&self) -> Value {
        let mut d = Map::new();
        d.insert("algorithm".into(), Value::String(self.algorithm.clone()));
        d.insert(
            "firstblood_algorithm".into(),
            Value::String(self.firstblood_algorithm.clone()),
        );
        d.insert("flags_rounds_valid".into(), self.flags_rounds_valid.clone());
        d.insert("nop_team_id".into(), self.nop_team_id.clone());
        d.insert("off_factor".into(), self.off_factor.clone());
        d.insert("def_factor".into(), self.def_factor.clone());
        d.insert("sla_factor".into(), self.sla_factor.clone());
        for (k, v) in &self.data {
            d.insert(k.clone(), v.clone());
        }
        Value::Object(d)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnoRunnerConfig {
    pub check_past_ticks: Value,
    pub timeout: Value,
}

impl Default for EnoRunnerConfig {
    fn default() -> Self {
        // Python default is the int literal 15 (the field is typed float but never coerced)
        EnoRunnerConfig {
            check_past_ticks: 5.into(),
            timeout: 15.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunnerConfig {
    pub dispatcher: String,
    pub eno: EnoRunnerConfig,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        RunnerConfig {
            dispatcher: "dispatcher:CeleryDispatcher".into(),
            eno: EnoRunnerConfig::default(),
        }
    }
}

impl RunnerConfig {
    /// `ConfigSection.from_dict`: only known keys — an unknown key is a TypeError.
    fn from_dict(d: &Map<String, Value>) -> Result<RunnerConfig, String> {
        let mut r = RunnerConfig::default();
        for (k, v) in d {
            match k.as_str() {
                "dispatcher" => r.dispatcher = v.as_str().unwrap_or_default().to_owned(),
                "eno" => {
                    let e = as_object(v, "eno")?;
                    r.eno = EnoRunnerConfig {
                        check_past_ticks: e
                            .get("check_past_ticks")
                            .cloned()
                            .unwrap_or_else(|| 5.into()),
                        timeout: e.get("timeout").cloned().unwrap_or_else(|| 15.into()),
                    };
                }
                other => return Err(format!("TypeError: unexpected keyword argument '{other}'")),
            }
        }
        Ok(r)
    }

    pub fn to_dict(&self) -> Value {
        serde_json::json!({
            "dispatcher": self.dispatcher,
            "eno": {
                "check_past_ticks": self.eno.check_past_ticks,
                "timeout": self.eno.timeout,
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WireguardSyncConfig {
    pub api_server: String,
    pub api_token: String,
    pub api_base: String,
    pub api_concurrency: Value,
}

impl WireguardSyncConfig {
    fn from_dict(d: &Map<String, Value>) -> Result<WireguardSyncConfig, String> {
        // `cls(**d)`: api_server/api_token have no default — missing is a TypeError
        let mut w = WireguardSyncConfig {
            api_server: d
                .get("api_server")
                .ok_or("TypeError: missing required argument: 'api_server'")?
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            api_token: d
                .get("api_token")
                .ok_or("TypeError: missing required argument: 'api_token'")?
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            api_base: "/api/router/".into(),
            api_concurrency: 1.into(),
        };
        for (k, v) in d {
            match k.as_str() {
                "api_server" | "api_token" => {}
                "api_base" => w.api_base = v.as_str().unwrap_or_default().to_owned(),
                "api_concurrency" => w.api_concurrency = v.clone(),
                other => return Err(format!("TypeError: unexpected keyword argument '{other}'")),
            }
        }
        Ok(w)
    }

    pub fn to_dict(&self) -> Value {
        serde_json::json!({
            "api_server": self.api_server,
            "api_token": self.api_token,
            "api_base": self.api_base,
            "api_concurrency": self.api_concurrency,
        })
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub basedir: PathBuf,
    pub vpn_base_dir: PathBuf,
    pub cloudconfig_file: PathBuf,
    /// The cleaned raw config dict (`CONFIG`).
    pub config: Value,
    pub config_file: PathBuf,

    pub postgres: Value,
    pub postgres_use_socket: bool,
    pub redis: Value,
    pub rabbitmq: Option<Value>,
    pub metrics: Value,

    pub tick_duration_default: i64,
    pub scoreboard_path: String,
    pub scoreboard_path_internal: Option<String>,
    pub vpnboard_path: String,
    pub checker_packages_path: String,
    pub checker_packages_lfs: Option<String>,
    pub services_path: String,
    pub patches_path: String,
    pub patches_public_path: Option<String>,
    pub flower_url: String,
    pub flower_internal_url: String,
    pub flower_ajax_url: String,
    pub coder_url: Option<String>,
    pub scoreboard_url: Option<String>,
    pub grafana_url: Option<String>,
    pub patches_url: Option<String>,

    pub secret_flag_key: Vec<u8>,
    pub flag_prefix: String,
    pub dispatcher_check_vpn_status: bool,
    pub scoring: ScoringConfig,
    pub service_remotes: Vec<String>,

    pub external_timer: bool,

    pub network: NetworkConfig,
    pub wireguard_sync: Option<WireguardSyncConfig>,
    pub runner: RunnerConfig,

    pub ctfroute_namespace: String,
    pub scoreboard_freeze: Option<Value>,
}

/// `Path.absolute()`: no symlink resolution, cwd-relative made absolute.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

impl Config {
    pub fn load_from_file(filename: &Path, interpolate_env: bool) -> Result<Config, String> {
        Self::load_from_file_env(filename, interpolate_env, &|_| None)
    }

    /// `load_from_file` with an injectable environment (tests pass a fixed map;
    /// production passes `std::env::var`).
    pub fn load_from_file_env(
        filename: &Path,
        interpolate_env: bool,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Config, String> {
        let text = std::fs::read_to_string(filename)
            .map_err(|e| format!("reading {}: {e}", filename.display()))?;
        let mut d: Value = if filename.extension().is_some_and(|e| e == "json") {
            serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {e}"))?
        } else {
            serde_yaml::from_str(&text).map_err(|e| format!("invalid YAML: {e}"))?
        };
        if let Some(additional) = additional_overlay(filename, env) {
            merge_shallow(&mut d, additional);
        }
        Self::from_dict(filename, d, interpolate_env, env)
    }

    /// `Config.from_dict` — the exact default/merge semantics.
    pub fn from_dict(
        filename: &Path,
        mut initial: Value,
        interpolate_env: bool,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Config, String> {
        clean_comments(&mut initial);
        let cfg = as_object(&initial, "initial_config")?.clone();

        let databases = as_object(
            cfg.get("databases").ok_or("KeyError: 'databases'")?,
            "databases",
        )?;
        let empty = Map::new();
        let postgres: Value = databases.get("postgres").cloned().unwrap_or(Value::Null);
        let postgres = if postgres.is_null() {
            Value::Object(empty.clone())
        } else {
            postgres
        };
        let postgres = as_object(&postgres, "postgres")?.clone();

        let postgres_use_socket = env("SAARCTF_POSTGRES_USE_SOCKET")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let redis: Value = databases.get("redis").cloned().unwrap_or(Value::Null);
        let redis = if redis.is_null() {
            Value::Object(empty.clone())
        } else {
            redis
        };
        let redis = as_object(&redis, "redis")?.clone();

        let rabbitmq: Option<Value> = databases.get("rabbitmq").cloned().filter(|v| !v.is_null());

        // metrics = postgres | {database: db + "_metrics"} | metrics_config
        let mut metrics = Value::Object(postgres.clone());
        let db_name = metrics
            .get("database")
            .and_then(Value::as_str)
            .ok_or("KeyError: 'database'")?
            .to_string();
        metrics
            .as_object_mut()
            .unwrap()
            .insert("database".into(), format!("{db_name}_metrics").into());
        if let Some(Value::Object(m)) = databases.get("metrics") {
            let obj = metrics.as_object_mut().unwrap();
            for (k, v) in m {
                obj.insert(k.clone(), v.clone());
            }
        }

        let tick_duration_default = cfg
            .get("tick_duration_default")
            .and_then(Value::as_i64)
            .unwrap_or(120);
        let scoreboard_path = get_str_or(&cfg, "scoreboard_path", "").to_string();
        let scoreboard_path_internal = opt_path(get_str(&cfg, "scoreboard_path_internal"));
        let vpnboard_path = get_str(&cfg, "vpnboard_path")
            .map(str::to_owned)
            .unwrap_or_else(|| scoreboard_path.clone());
        let checker_packages_path = get_str(&cfg, "checker_packages_path")
            .ok_or("KeyError: 'checker_packages_path'")?
            .to_string();
        let checker_packages_lfs = Some(path_join(&checker_packages_path, "lfs"));
        let patches_path = get_str(&cfg, "patches_path")
            .map(str::to_owned)
            .unwrap_or_else(|| path_join(&checker_packages_path, "patches"));
        let patches_public_path = opt_path(get_str(&cfg, "patches_public_path"));
        let services_path = get_str(&cfg, "services_path")
            .map(str::to_owned)
            .unwrap_or_else(|| path_join(&checker_packages_path, "services"));

        let flower_url = get_str_or(&cfg, "flower_url", "").to_string();
        let flower_internal_url = get_str_or(&cfg, "flower_internal_url", &flower_url).to_string();
        let flower_ajax_url = get_str_or(&cfg, "flower_ajax_url", &flower_url).to_string();
        let coder_url = url_or_none(&cfg, "coder_url");
        let scoreboard_url = url_or_none(&cfg, "scoreboard_url");
        let grafana_url = url_or_none(&cfg, "grafana_url");
        // (patches_url or score_url + "/patches") if scoreboard_url else None
        let patches_url = match &scoreboard_url {
            None => None,
            Some(url) => match url_or_none(&cfg, "patches_url") {
                Some(p) => Some(p),
                None => Some(format!("{}/patches", url.trim_end_matches('/'))),
            },
        };

        let flag_prefix = get_str_or(&cfg, "flag_prefix", "SAAR").to_string();
        let secret_flag_key = unhexlify(get_str_or(&cfg, "secret_flags", ""))?;
        let dispatcher_check_vpn_status = cfg
            .get("dispatcher_check_vpn_status")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut scoring_obj = match cfg.get("scoring") {
            Some(Value::Object(m)) => m.clone(),
            Some(Value::Null) | None => Map::new(),
            Some(other) => return Err(format!("'scoring' must be a dict, got {other}")),
        };
        for top in ["nop_team_id", "flags_rounds_valid"] {
            if let (Some(v), None) = (cfg.get(top), scoring_obj.get(top)) {
                scoring_obj.insert(top.into(), v.clone());
            }
        }
        let scoring = ScoringConfig::from_dict(&scoring_obj);

        let service_remotes: Vec<String> = cfg
            .get("service_remotes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default();

        let external_timer = cfg
            .get("external_timer")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let network = NetworkConfig::from_dict(cfg.get("network").ok_or("KeyError: 'network'")?)?;
        let wireguard_sync = match cfg.get("wireguard_sync") {
            Some(Value::Null) | None => None,
            Some(Value::Object(m)) => Some(WireguardSyncConfig::from_dict(m)?),
            Some(other) => return Err(format!("'wireguard_sync' must be a dict, got {other}")),
        };
        let ctfroute_namespace = get_str_or(&cfg, "ctfroute_namespace", "").to_string();
        let runner = match cfg.get("runner") {
            Some(Value::Object(m)) => RunnerConfig::from_dict(m)?,
            Some(Value::Null) | None => RunnerConfig::default(),
            Some(other) => return Err(format!("'runner' must be a dict, got {other}")),
        };
        let scoreboard_freeze = cfg
            .get("scoreboard_freeze")
            .cloned()
            .filter(|v| !v.is_null());

        let basedir = absolute(filename)
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();

        let mut config = Config {
            basedir: basedir.clone(),
            vpn_base_dir: basedir.join("vpn"),
            cloudconfig_file: basedir.join("cloud-status.json"),
            config: initial,
            config_file: filename.to_path_buf(),
            postgres: Value::Object(postgres),
            postgres_use_socket,
            redis: Value::Object(redis),
            rabbitmq,
            metrics,
            tick_duration_default,
            scoreboard_path,
            scoreboard_path_internal,
            vpnboard_path,
            checker_packages_path,
            checker_packages_lfs,
            services_path,
            patches_path,
            patches_public_path,
            flower_url,
            flower_internal_url,
            flower_ajax_url,
            coder_url,
            scoreboard_url,
            grafana_url,
            patches_url,
            secret_flag_key,
            flag_prefix,
            dispatcher_check_vpn_status,
            scoring,
            service_remotes,
            external_timer,
            network,
            wireguard_sync,
            runner,
            ctfroute_namespace,
            scoreboard_freeze,
        };
        if interpolate_env {
            config.interpolate_env_with(env)?;
        }
        Ok(config)
    }

    /// `interpolate_env` with `std::env` (environment wins over file entries).
    pub fn interpolate_env(&mut self) -> Result<(), String> {
        self.interpolate_env_with(&|k| std::env::var(k).ok())
    }

    fn interpolate_env_with(&mut self, env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
        if env("RABBITMQ_HOST").is_some() && self.rabbitmq.is_none() {
            self.rabbitmq = Some(Value::Object(Map::new()));
        }
        // environment wins over file entries, per section
        let apply = |d: &mut Value, pairs: &[(&str, &str)]| -> Result<(), String> {
            let obj = as_object_mut(d, "dict")?;
            for (k, e) in pairs {
                if let Some(v) = env(e) {
                    obj.insert(k.to_string(), Value::String(v));
                }
            }
            Ok(())
        };
        apply(
            &mut self.postgres,
            &[
                ("server", "POSTGRES_SERVER"),
                ("port", "POSTGRES_PORT"),
                ("username", "POSTGRES_USERNAME"),
                ("password", "POSTGRES_PASSWORD"),
                ("database", "POSTGRES_DATABASE"),
            ],
        )?;
        apply(
            &mut self.redis,
            &[
                ("host", "REDIS_HOST"),
                ("port", "REDIS_PORT"),
                ("db", "REDIS_DB"),
                ("password", "REDIS_PASSWORD"),
            ],
        )?;
        if let Some(rmq) = self.rabbitmq.as_mut() {
            apply(
                rmq,
                &[
                    ("host", "RABBITMQ_HOST"),
                    ("vhost", "RABBITMQ_VHOST"),
                    ("port", "RABBITMQ_PORT"),
                    ("username", "RABBITMQ_USERNAME"),
                    ("password", "RABBITMQ_PASSWORD"),
                ],
            )?;
        }
        apply(
            &mut self.metrics,
            &[
                ("server", "METRICS_SERVER"),
                ("port", "METRICS_PORT"),
                ("username", "METRICS_USERNAME"),
                ("password", "METRICS_PASSWORD"),
                ("database", "METRICS_DATABASE"),
            ],
        )?;

        if let Some(db) = self
            .redis
            .get("db")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            let parsed: i64 = db.trim().parse().map_err(|_| {
                format!("ValueError: invalid literal for int() with base 10: '{db}'")
            })?;
            self.redis
                .as_object_mut()
                .unwrap()
                .insert("db".into(), parsed.into());
        }
        if let Some(key) = env("SECRET_FLAG_KEY") {
            self.secret_flag_key = unhexlify(&key)?;
        }
        if let Some(p) = env("SCOREBOARD_PATH") {
            self.scoreboard_path = p;
        }
        if let Some(p) = env("SCOREBOARD_PATH_INTERNAL") {
            self.scoreboard_path_internal = Some(p);
        }
        Ok(())
    }

    /// `to_dict` — byte-for-byte the Python dict shape.
    pub fn to_dict(&self) -> Value {
        let mut d = self.config.as_object().cloned().unwrap_or_default();
        if let Some(Value::Object(dbs)) = d.get_mut("databases") {
            for (k, v) in [
                ("postgres", self.postgres.clone()),
                ("redis", self.redis.clone()),
                ("rabbitmq", self.rabbitmq.clone().unwrap_or(Value::Null)),
                ("metrics", self.metrics.clone()),
            ] {
                dbs.insert(k.into(), v);
            }
        }
        let mut top = Map::new();
        for (k, v) in [
            (
                "tick_duration_default",
                Value::from(self.tick_duration_default),
            ),
            ("vpnboard_path", self.vpnboard_path.clone().into()),
            (
                "checker_packages_path",
                self.checker_packages_path.clone().into(),
            ),
            ("services_path", self.services_path.clone().into()),
            ("patches_path", self.patches_path.clone().into()),
            (
                "patches_public_path",
                match &self.patches_public_path {
                    Some(p) => Value::String(p.clone()),
                    None => Value::Null,
                },
            ),
            ("flower_url", self.flower_url.clone().into()),
            (
                "flower_internal_url",
                self.flower_internal_url.clone().into(),
            ),
            ("flower_ajax_url", self.flower_ajax_url.clone().into()),
            (
                "coder_url",
                self.coder_url
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "scoreboard_url",
                self.scoreboard_url
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "grafana_url",
                self.grafana_url
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "patches_url",
                self.patches_url
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            ("secret_flags", hexlify(&self.secret_flag_key).into()),
            ("flag_prefix", self.flag_prefix.clone().into()),
            ("scoring", self.scoring.to_dict()),
            (
                "service_remotes",
                Value::Array(
                    self.service_remotes
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            ),
            (
                "dispatcher_check_vpn_status",
                self.dispatcher_check_vpn_status.into(),
            ),
            ("external_timer", self.external_timer.into()),
            ("network", self.network.to_dict()),
            (
                "wireguard_sync",
                self.wireguard_sync
                    .as_ref()
                    .map(|w| w.to_dict())
                    .unwrap_or(Value::Null),
            ),
            ("runner", self.runner.to_dict()),
            (
                "scoreboard_freeze",
                self.scoreboard_freeze.clone().unwrap_or(Value::Null),
            ),
            ("scoreboard_path", self.scoreboard_path.clone().into()),
            (
                "scoreboard_path_internal",
                match &self.scoreboard_path_internal {
                    Some(p) => Value::String(p.clone()),
                    None => Value::Null,
                },
            ),
            ("ctfroute_namespace", self.ctfroute_namespace.clone().into()),
        ] {
            top.insert(k.into(), v);
        }
        for (k, v) in top {
            d.insert(k, v);
        }
        Value::Object(d)
    }

    /// `postgres_sqlalchemy()`.
    pub fn postgres_sqlalchemy(&self) -> String {
        let pg = self.postgres.as_object().unwrap();
        let mut conn = "postgresql+psycopg2://".to_string();
        let username = get_str(pg, "username").unwrap_or("");
        let password = get_str(pg, "password").unwrap_or("");
        if !username.is_empty() {
            conn += username;
            if !password.is_empty() {
                conn += ":";
                conn += password;
            }
            conn += "@";
        }
        let server = get_str(pg, "server").unwrap_or("");
        if !server.is_empty() && !self.postgres_use_socket {
            conn += &format!(
                "{}:{}",
                server,
                pg.get("port").map(|p| p.to_string()).unwrap_or_default()
            );
        }
        format!("{}/{}", conn, get_str(pg, "database").unwrap_or(""))
    }

    /// `postgres_psycopg2()`.
    pub fn postgres_psycopg2(&self) -> String {
        let pg = self.postgres.as_object().unwrap();
        let mut conn = format!(
            "host='{}' port={} dbname='{}'",
            get_str(pg, "server").unwrap_or(""),
            pg.get("port").map(|p| p.to_string()).unwrap_or_default(),
            get_str(pg, "database").unwrap_or("")
        );
        if !get_str(pg, "username").unwrap_or("").is_empty() {
            conn += &format!(" user='{}'", get_str(pg, "username").unwrap_or(""));
            if !get_str(pg, "password").unwrap_or("").is_empty() {
                conn += &format!(" password='{}'", get_str(pg, "password").unwrap_or(""));
            }
        }
        conn
    }

    /// `celery_redis_url()` — celery uses redis db + 1.
    pub fn celery_redis_url(&self) -> String {
        let r = self.redis.as_object().unwrap();
        let host = get_str(r, "host").unwrap_or("");
        let port = r.get("port").map(|p| p.to_string()).unwrap_or_default();
        let db = r.get("db").and_then(Value::as_i64).unwrap_or(0) + 1;
        match get_str(r, "password") {
            Some(pw) => format!("redis://:{pw}@{host}:{port}/{db}"),
            None => format!("redis://{host}:{port}/{db}"),
        }
    }

    /// `celery_rabbitmq_url()`.
    pub fn celery_rabbitmq_url(&self) -> Result<String, String> {
        let Some(Value::Object(rmq)) = &self.rabbitmq else {
            return Err("ValueError: RabbitMQ not configured".into());
        };
        Ok(format!(
            "amqp://{}:{}@{}:{}/{}",
            get_str(rmq, "username").unwrap_or(""),
            get_str(rmq, "password").unwrap_or(""),
            get_str(rmq, "host").unwrap_or(""),
            rmq.get("port").map(|p| p.to_string()).unwrap_or_default(),
            get_str(rmq, "vhost").unwrap_or("")
        ))
    }

    /// `celery_url()`.
    pub fn celery_url(&self) -> Result<String, String> {
        if self.rabbitmq.is_some() {
            self.celery_rabbitmq_url()
        } else {
            Ok(self.celery_redis_url())
        }
    }

    /// `validate()` — check the most severe fuckups.
    pub fn validate(&self) -> Result<(), String> {
        if self.scoreboard_path.is_empty() || Path::new(&self.scoreboard_path).file_name().is_none()
        {
            return Err("ValueError: SCOREBOARD_PATH not configured".into());
        }
        if self.secret_flag_key.len() < 16 {
            return Err("ValueError: SECRET_FLAG_KEY not configured or too short".into());
        }
        if self.flag_prefix.chars().count() != 4 {
            return Err("ValueError: FLAG_PREFIX must be exactly 4 characters long".into());
        }
        Ok(())
    }
}

fn as_object_mut<'a>(v: &'a mut Value, what: &str) -> Result<&'a mut Map<String, Value>, String> {
    v.as_object_mut()
        .ok_or_else(|| format!("'{what}' must be a dict"))
}

/// `load_default_config_file`'s `additional` overlay: file dict `|` additional.
/// (The original only reads `additional` from callers; production passes none.)
fn additional_overlay(_filename: &Path, _env: &dyn Fn(&str) -> Option<String>) -> Option<Value> {
    None
}

fn merge_shallow(base: &mut Value, overlay: Value) {
    if let (Some(b), Value::Object(o)) = (base.as_object_mut(), overlay) {
        for (k, v) in o {
            b.insert(k, v);
        }
    }
}
