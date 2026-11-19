//! Golden-file equivalence tests (ADR-014): replay `tests/goldens/goldens.json`
//! — captured from the Python original by `generate_goldens.py` — against the
//! Rust port. Byte-exact where Python is byte-exact.

use saarctf_commons_rs::config::Config;
use saarctf_commons_rs::metrics::{MetricValue, format_line};
use serde_json::Value;

fn goldens() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/goldens/goldens.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("goldens.json present"))
        .expect("goldens.json valid")
}

fn env_fn<'a>(overrides: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |key: &str| {
        overrides
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    }
}

const ENV_OVERRIDE: &[(&str, &str)] = &[
    ("POSTGRES_SERVER", "override-db.example"),
    ("REDIS_PASSWORD", "override-pw"),
    ("REDIS_DB", "7"),
    ("METRICS_PORT", "1234"),
    ("SECRET_FLAG_KEY", "deadbeefdeadbeefdeadbeefdeadbeef"),
    ("RABBITMQ_HOST", "rabbit.example"),
];

fn check_fixture(g: &Value, name: &str) {
    let expected = &g["fixtures"][name];
    let base = name.trim_end_matches("+env");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_string() + base;
    let path = std::path::Path::new(&file);

    let env = if name.ends_with("+env") {
        ENV_OVERRIDE
    } else {
        &[]
    };
    let cfg = Config::load_from_file_env(path, true, &env_fn(env))
        .unwrap_or_else(|e| panic!("{name}: load failed: {e}"));

    let actual = cfg_to_json(&cfg);
    assert_eq!(
        actual, expected["to_dict"],
        "{name}: to_dict mismatch\nactual:   {}\nexpected: {}",
        actual, expected["to_dict"]
    );

    let urls = &expected["urls"];
    assert_eq!(
        Value::String(cfg.postgres_sqlalchemy()),
        urls["postgres_sqlalchemy"],
        "{name} sqlalchemy"
    );
    assert_eq!(
        Value::String(cfg.postgres_psycopg2()),
        urls["postgres_psycopg2"],
        "{name} psycopg2"
    );
}

// helper: build a serde_json::Value with the same shape the Python to_dict has
fn cfg_to_json(cfg: &Config) -> Value {
    let mut d = cfg.config.clone();
    let obj = d.as_object_mut().unwrap();
    let dbs = obj.get_mut("databases").unwrap().as_object_mut().unwrap();
    dbs.insert("postgres".into(), cfg.postgres.clone());
    dbs.insert("redis".into(), cfg.redis.clone());
    dbs.insert(
        "rabbitmq".into(),
        cfg.rabbitmq.clone().unwrap_or(Value::Null),
    );
    dbs.insert("metrics".into(), cfg.metrics.clone());
    let map = serde_json::json!({
        "tick_duration_default": cfg.tick_duration_default,
        "vpnboard_path": cfg.vpnboard_path.clone(),
        "checker_packages_path": cfg.checker_packages_path.clone(),
        "services_path": cfg.services_path.clone(),
        "patches_path": cfg.patches_path.clone(),
        "patches_public_path": cfg.patches_public_path.clone(),
        "flower_url": cfg.flower_url,
        "flower_internal_url": cfg.flower_internal_url,
        "flower_ajax_url": cfg.flower_ajax_url,
        "coder_url": cfg.coder_url,
        "scoreboard_url": cfg.scoreboard_url,
        "grafana_url": cfg.grafana_url,
        "patches_url": cfg.patches_url,
        "secret_flags": hex(&cfg.secret_flag_key),
        "flag_prefix": cfg.flag_prefix,
        "scoring": scoring_to_json(cfg),
        "service_remotes": cfg.service_remotes,
        "dispatcher_check_vpn_status": cfg.dispatcher_check_vpn_status,
        "external_timer": cfg.external_timer,
        "network": cfg.network.to_dict(),
        "wireguard_sync": cfg.wireguard_sync.as_ref().map(|w| w.to_dict()),
        "runner": cfg.runner.to_dict(),
        "scoreboard_freeze": cfg.scoreboard_freeze.clone(),
        "scoreboard_path": cfg.scoreboard_path.clone(),
        "scoreboard_path_internal": cfg.scoreboard_path_internal.clone(),
        "ctfroute_namespace": cfg.ctfroute_namespace,
    });
    for (k, v) in map.as_object().unwrap() {
        obj.insert(k.clone(), v.clone());
    }
    d
}

fn scoring_to_json(cfg: &Config) -> Value {
    let s = &cfg.scoring;
    let mut d = serde_json::json!({
        "algorithm": s.algorithm,
        "firstblood_algorithm": s.firstblood_algorithm,
        "flags_rounds_valid": s.flags_rounds_valid,
        "nop_team_id": s.nop_team_id,
        "off_factor": s.off_factor,
        "def_factor": s.def_factor,
        "sla_factor": s.sla_factor,
    });
    for (k, v) in &s.data {
        d.as_object_mut().unwrap().insert(k.clone(), v.clone());
    }
    d
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn fixtures_match_goldens() {
    let g = goldens();
    for name in g["fixtures"].as_object().unwrap().keys() {
        check_fixture(&g, name);
    }
}

#[test]
fn network_math_matches_goldens() {
    let g = goldens();
    for (name, fx) in g["fixtures"].as_object().unwrap() {
        if !name.ends_with(".yaml") && !name.ends_with(".json") {
            continue;
        }
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_string() + name;
        let cfg = Config::load_from_file_env(std::path::Path::new(&file), true, &|_| None).unwrap();
        let net = &cfg.network;
        for (tid, expected) in fx["network"].as_object().unwrap() {
            if tid == "ip_to_id" {
                continue;
            }
            let id: i64 = tid.parse().unwrap();
            assert_eq!(
                net.team_id_to_vulnbox_ip(id),
                expected["vulnbox"].as_str().unwrap(),
                "{name} vulnbox {id}"
            );
            assert_eq!(
                net.team_id_to_gateway_ip(id),
                expected["gateway"].as_str().unwrap(),
                "{name} gateway {id}"
            );
            assert_eq!(
                net.team_id_to_testbox_ip(id),
                expected["testbox"].as_str().unwrap(),
                "{name} testbox {id}"
            );
            assert_eq!(
                net.team_id_to_network_range(id),
                expected["network_range"].as_str().unwrap(),
                "{name} range {id}"
            );
            let peers = expected["vpn_peers"].as_array().unwrap();
            assert_eq!(
                net.team_id_to_vpn_peers(id).0,
                peers[0].as_str().unwrap(),
                "{name} vpn0 {id}"
            );
            assert_eq!(
                net.team_id_to_vpn_peers(id).1,
                peers[1].as_str().unwrap(),
                "{name} vpn1 {id}"
            );
        }
        for (probe, expected) in fx["network"]["ip_to_id"].as_object().unwrap() {
            // "vulnbox(tid)" probes are re-derived; plain keys are literal IPs
            let result = if let Some(tid) = probe
                .strip_prefix("vulnbox(")
                .and_then(|s| s.strip_suffix(')'))
                .and_then(|s| s.parse::<i64>().ok())
            {
                net.network_ip_to_id(&net.team_id_to_vulnbox_ip(tid))
            } else {
                net.network_ip_to_id(probe)
            };
            assert_eq!(result, expected.as_i64(), "{name} ip_to_id {probe}");
        }
    }
}

#[test]
fn validate_matches_goldens() {
    let g = goldens();
    for (name, fx) in g["fixtures"].as_object().unwrap() {
        if !name.ends_with(".yaml") && !name.ends_with(".json") {
            continue;
        }
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_string() + name;
        let cfg = Config::load_from_file_env(std::path::Path::new(&file), true, &|_| None).unwrap();
        match cfg.validate() {
            Ok(()) => assert_eq!(fx["validate"].as_str(), Some("ok"), "{name}"),
            Err(e) => {
                assert_eq!(Value::String(e), fx["validate"], "{name} validate error");
            }
        }
    }
}

// --- Influx line protocol: same cases as INFLUX_CASES in generate_goldens.py ---

fn mv(k: &str, v: MetricValue) -> (String, MetricValue) {
    (k.into(), v)
}

#[test]
fn influx_lines_match_goldens() {
    let g = goldens();
    let lines = g["influx"]["lines"].as_array().unwrap();

    type Case = (
        &'static str,
        Vec<(String, MetricValue)>,
        Vec<(String, MetricValue)>,
        f64,
    );
    let cases: Vec<Case> = vec![
        (
            "test_metric",
            vec![mv("x", MetricValue::Int(42))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("x", MetricValue::Int(-7))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("b", MetricValue::Bool(true))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("b", MetricValue::Bool(false))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("s", MetricValue::Str("hello \"world\"\n\tü微😀".into()))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("s", MetricValue::Str("plain".into()))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(1.5))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(15.0))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(1e16))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(1.5e16))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(1e-5))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(-0.0))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(0.1))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(1.0 / 3.0))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(f64::NAN))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(f64::INFINITY))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("f", MetricValue::Float(f64::NEG_INFINITY))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![
                ("f".into(), MetricValue::Float(3.0)),
                ("i".into(), MetricValue::Int(3)),
            ],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![
                ("team".into(), MetricValue::Int(5)),
                ("service".into(), MetricValue::Str("name with space".into())),
            ],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Float(2.5))],
            vec![("ok".into(), MetricValue::Bool(true))],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![],
            f64::from_bits(4744917124794279607), // exactly 1700000000.123456789
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![],
            0.0,
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![],
            -5.0,
        ),
        // datetime.fromtimestamp(1700000000, tz=utc).timestamp() / .5
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![],
            1700000000.0,
        ),
        (
            "test_metric",
            vec![mv("v", MetricValue::Int(1))],
            vec![],
            1700000000.5,
        ),
    ];

    for (i, (metric, values, attrs, ts)) in cases.iter().enumerate() {
        let actual = format_line(metric, values, attrs, *ts);
        assert_eq!(
            actual,
            lines[i].as_str().unwrap(),
            "influx case {i} mismatch\nactual:   {actual:?}\nexpected: {:?}",
            lines[i].as_str()
        );
    }
}
