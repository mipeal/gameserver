//! Port of `vpn/manage-hetzner-firewall.py` — opens/closes the Hetzner Cloud
//! firewall on game startup. The `hcloud` package is replaced by raw
//! reqwest calls against api.hetzner.cloud.

use futures_util::StreamExt;
use saarctf_commons_rs::config::Config;
use serde_json::{json, Value};
use std::time::Duration;

const HCLOUD_FIREWALL_NAME: &str = "Allow SSH/VPN Only";
const SOURCE_IPS: [&str; 2] = ["0.0.0.0/0", "::/0"];

fn token() -> Result<String, String> {
    std::env::var("HCLOUD_TOKEN").map_err(|_| "KeyError: 'HCLOUD_TOKEN'".to_string())
}

/// Wait for all actions to finish (`wait_until_finished(10)`).
async fn assert_actions(
    client: &reqwest::Client,
    token: &str,
    actions: &[Value],
) -> Result<(), String> {
    for action in actions {
        let id = action
            .get("id")
            .and_then(Value::as_i64)
            .ok_or("no action id")?;
        loop {
            let resp = client
                .get(format!("https://api.hetzner.cloud/v1/actions/{id}"))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let body: Value = resp.json().await.map_err(|e| e.to_string())?;
            let status = body["action"]["status"].as_str().unwrap_or("running");
            if status == "success" {
                break;
            }
            if status == "error" {
                return Err(format!("Hetzner action {id} failed: {body}"));
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    Ok(())
}

async fn update_state(client: &reqwest::Client, token: &str, state: &str) -> Result<(), String> {
    let allow_ssh = state == "on" || state == "team";
    let resp = client
        .get("https://api.hetzner.cloud/v1/firewalls")
        .bearer_auth(token)
        .query(&[("name", HCLOUD_FIREWALL_NAME)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let firewall = body["firewalls"]
        .as_array()
        .and_then(|f| f.first())
        .ok_or_else(|| format!("KeyError: Firewall {HCLOUD_FIREWALL_NAME:?} not found"))?;
    let rules = firewall["rules"]
        .as_array()
        .ok_or("ValueError: Firewall has no rules list")?;
    let ssh_rule = rules.iter().position(|rule| {
        rule["protocol"] == "tcp"
            && rule["direction"] == "in"
            && rule["port"] == "22"
            && rule["source_ips"]
                .as_array()
                .map(|ips| ips.iter().filter_map(Value::as_str).collect::<Vec<_>>() == SOURCE_IPS)
                .unwrap_or(false)
    });
    if allow_ssh == ssh_rule.is_some() {
        tracing::info!(
            "[-]   No change necessary, SSH is {}",
            if ssh_rule.is_some() {
                "allowed"
            } else {
                "blocked"
            }
        );
    } else if allow_ssh && ssh_rule.is_none() {
        let mut new_rules = rules.clone();
        new_rules.push(json!({
            "direction": "in",
            "protocol": "tcp",
            "source_ips": SOURCE_IPS,
            "port": "22",
            "description": "Open SSH access"
        }));
        let resp = client
            .post(format!(
                "https://api.hetzner.cloud/v1/firewalls/{}/actions/set_rules",
                firewall["id"].as_i64().unwrap_or(0)
            ))
            .bearer_auth(token)
            .json(&json!({ "rules": new_rules }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        assert_actions(client, token, body["actions"].as_array().unwrap_or(&vec![])).await?;
        tracing::info!("[OK]  SSH has been allowed");
    } else if !allow_ssh && ssh_rule.is_some() {
        let new_rules: Vec<Value> = rules
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != ssh_rule)
            .map(|(_, r)| r.clone())
            .collect();
        let resp = client
            .post(format!(
                "https://api.hetzner.cloud/v1/firewalls/{}/actions/set_rules",
                firewall["id"].as_i64().unwrap_or(0)
            ))
            .bearer_auth(token)
            .json(&json!({ "rules": new_rules }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        assert_actions(client, token, body["actions"].as_array().unwrap_or(&vec![])).await?;
        tracing::info!("[OK]  SSH has been blocked");
    }
    Ok(())
}

async fn run(
    _cfg: &Config,
    conn: &mut redis::aio::ConnectionManager,
    client_redis: redis::Client,
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let token = token()?;

    let state: Option<String> = redis::cmd("GET")
        .arg("network:state")
        .query_async(conn)
        .await
        .map_err(|e| e.to_string())?;
    let state = match state {
        Some(s) => s,
        None => {
            let _: Result<String, _> = redis::cmd("SET")
                .arg("network:state")
                .arg("off")
                .query_async(conn)
                .await;
            "off".to_string()
        }
    };
    update_state(&http, &token, &state).await?;

    let mut pubsub = client_redis
        .get_async_pubsub()
        .await
        .map_err(|e| e.to_string())?;
    pubsub
        .subscribe("network:state")
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = pubsub.into_on_message();
    while let Some(msg) = stream.next().await {
        if msg.get_channel_name() == "network:state" {
            let data: String = match msg.get_payload() {
                Ok(p) => p,
                Err(_) => continue,
            };
            tracing::info!("[debug] message \"network:state\", data {data:?}");
            update_state(&http, &token, &data).await?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, _db, mut redis_conn) = saarctf_scripts_rs::init("manage-hetzner-firewall").await?;
    let client_redis = redis::Client::open(
        saarctf_commons_rs::redis::redis_url(&cfg.redis).map_err(|e| format!("redis url: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    tracing::info!("      Connecting...");
    run(&cfg, &mut redis_conn, client_redis).await
}
