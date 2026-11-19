//! `scripts/import_eno_services.py` — import EnoCheckerRunner services from a
//! file of service urls. ARGUMENTS: <file.json> [-r|--reset]

use serde_json::Value;
use std::collections::BTreeMap;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reset = args.iter().any(|a| a == "-r" || a == "--reset");
    let Some(fname) = args.iter().skip(1).find(|a| !a.starts_with('-')) else {
        saarctf_scripts_rs::die("usage: import-eno-services <file.json> [-r|--reset]");
    };

    let (cfg, db, _redis) = saarctf_scripts_rs::init("import_eno_services.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));

    if reset {
        sqlx::query("DELETE FROM services")
            .execute(&db)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
    }

    let file_content = std::fs::read_to_string(fname)
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("fs: {e}")));
    let service_urls: BTreeMap<String, String> = serde_json::from_str(&file_content)
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("json: {e}")));

    let client = reqwest::Client::new();
    for (idx, (_name, host)) in service_urls.iter().enumerate() {
        let resp = client
            .get(format!("{host}/service"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("http: {e}")));
        let data: Value = resp
            .json()
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("http: {e}")));
        let service_name = data
            .get("serviceName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let flag_variants = data
            .get("flagVariants")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32;
        let runner_config = serde_json::json!({ "url": host });
        let flag_ids: Vec<&str> = vec!["custom"; flag_variants.max(0) as usize];
        println!("Fetched {}", service_name);
        sqlx::query(
            "INSERT INTO services (id, name, checker_timeout, checker_runner, checker_script, runner_config, num_payloads, flag_ids, flags_per_tick) \
             VALUES ($1, $2, $3, 'eno:EnoCheckerRunner', 'nono', $4, $5, $6, $7)",
        )
        .bind((idx + 1) as i32)
        .bind(&service_name)
        .bind(cfg.tick_duration_default as i32)
        .bind(&runner_config)
        .bind(flag_variants)
        .bind(flag_ids.join(","))
        .bind(flag_variants as f64)
        .execute(&db)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
        println!("Imported {}", service_name);
    }
}
