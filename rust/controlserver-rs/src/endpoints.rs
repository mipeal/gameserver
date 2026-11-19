//! Port of `controlserver/endpoints/` — the JSON API surface of the Flask app
//! (api, metrics, overview, log view, checker-result view, services, scripts).
//!
//! The server-rendered HTML admin pages are NOT ported here: the GUI is being
//! replaced by the Leptos SPA (ADR-002), which will consume these JSON routes.
//! Routes needing later-wave components are deferred:
//! - `/overview/vpn`, `/overview/set_vpn`, `/api/ctfroute/*` → VPNControl (wave 2)
//! - `/packages/*` → ServiceRepoManager/DBFilesystem + worker broadcasts (next slice)
//! - HTML list pages (`/teams/`, `/log_messages/`, `/checker_results/`, …) → SPA wave

use crate::logger;
use crate::models::{self, LOG_WARNING};
use crate::scoring::ScoringCalculation;
use crate::scoreboard;
use crate::timer::{CtfState, SharedTimer};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use saarctf_commons_rs::config::Config;
use saarctf_commons_rs::metrics::{metrics_record_many, MetricValue};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;

pub struct AppState {
    pub cfg: Config,
    pub db: PgPool,
    pub timer: SharedTimer,
}

/// Bind address (Flask ran via `flask run --host=0.0.0.0`, i.e. :5000).
fn bind_addr() -> String {
    std::env::var("CONTROL_SERVER_BIND").unwrap_or_else(|_| "0.0.0.0:5000".into())
}

pub async fn run_http_server(state: AppState) -> Result<(), String> {
    let app = router(state);
    let addr = bind_addr();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    tracing::info!("Controlserver HTTP API listening on {addr}");
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

fn router(state: AppState) -> Router {
    let shared = Arc::new(state);
    Router::new()
        .route("/api/teams", get(api_teams))
        .route("/api/services", get(api_services))
        .route("/api/grafana_warning", post(api_grafana_warning))
        .route("/overview/timing", get(overview_timing))
        .route("/overview/set_timing", post(overview_set_timing))
        .route("/overview/logs", get(overview_logs))
        .route("/overview/logs/{min_id}", get(overview_logs))
        .route("/checker_results/view/{id}", get(checker_results_view))
        .route("/services/checker_status", post(services_set_checker_status))
        .route("/scripts/recreate_scoreboard", post(scripts_recreate_scoreboard))
        .route("/metrics/write", post(metrics_write))
        .with_state(shared)
}

// ---- /api/teams, /api/services -----------------------------------------------

type TeamRow = (i32, String, Option<String>, Option<String>, Option<String>, Option<String>);

async fn api_teams(State(state): State<Arc<AppState>>) -> Json<Value> {
    let rows: Vec<TeamRow> =
        sqlx::query_as(
            "SELECT id, name, website, affiliation, vulnbox_ip, logo FROM teams ORDER BY id",
        )
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    Json(Value::Array(
        rows.iter()
            .map(|(id, name, website, affiliation, ip, logo)| {
                json!({
                    "id": id, "name": name, "website": website,
                    "affiliation": affiliation, "ip": ip, "logo": logo,
                })
            })
            .collect(),
    ))
}

async fn api_services(State(state): State<Arc<AppState>>) -> Json<Value> {
    let rows: Vec<(i32, i16, String)> =
        sqlx::query_as("SELECT id, num_payloads, name FROM services ORDER BY id")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    Json(Value::Array(
        rows.iter()
            .map(|(id, num_payloads, name)| {
                json!({"id": id, "flagstores": num_payloads, "name": name})
            })
            .collect(),
    ))
}

// ---- /api/grafana_warning ------------------------------------------------------

async fn api_grafana_warning(
    State(state): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let Json(data) = body.ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Invalid"))?;
    if !data.is_object() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Invalid"));
    }
    if data.get("state").and_then(Value::as_str) != Some("alerting") {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "We only want alerts"));
    }
    if let Some(rule_name) = data.get("ruleName").and_then(Value::as_str) {
        let text = format!(
            "{}\nRule: {}\n{}",
            data["message"].as_str().unwrap_or(""),
            rule_name,
            data["ruleUrl"].as_str().unwrap_or("")
        );
        log_grafana(&state, data["title"].as_str().unwrap_or(""), &text).await;
    } else if let Some(alerts) = data.get("alerts").and_then(Value::as_array) {
        for alert in alerts {
            if alert["status"].as_str() != Some("firing") {
                continue;
            }
            let mut title = format!(
                "[Grafana] {}",
                alert["labels"]["alertname"].as_str().unwrap_or("")
            );
            if let Some(rulename) = alert["labels"]["rulename"].as_str() {
                title += " / ";
                title += rulename;
            }
            let mut text = Vec::new();
            if let Some(u) = alert["dashboardURL"].as_str().filter(|s| !s.is_empty()) {
                text.push(format!("Dashboard: {u}"));
            }
            if let Some(u) = alert["panelURL"].as_str().filter(|s| !s.is_empty()) {
                text.push(format!("Panel: {u}"));
            }
            log_grafana(&state, &title, &text.join("\n")).await;
        }
    } else {
        let title = data["title"].as_str().unwrap_or("");
        let text = format!(
            "{}\n{}",
            title,
            data["message"].as_str().unwrap_or("")
        );
        log_grafana(&state, title, &text).await;
    }
    Ok("OK")
}

async fn log_grafana(state: &AppState, title: &str, text: &str) {
    let _ = logger::log(&state.db, "Grafana", title, text, LOG_WARNING).await;
}

// ---- /overview/timing, /overview/set_timing ------------------------------------

fn ctf_state_from_int(v: i64) -> Option<CtfState> {
    match v {
        1 => Some(CtfState::Stopped),
        2 => Some(CtfState::Suspended),
        3 => Some(CtfState::Running),
        _ => None,
    }
}

fn opt_i64(v: &Value) -> Option<i64> {
    v.as_i64().filter(|x| *x != 0)
}

async fn overview_timing(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let t = state.timer.lock().await;
    let master_timers = {
        let mut redis = saarctf_commons_rs::redis::get_redis_connection(&state.cfg.redis)
            .await
            .map_err(internal)?;
        let sub: (String, i64) = redis::cmd("PUBSUB")
            .arg("NUMSUB")
            .arg("timing:master")
            .query_async(&mut redis)
            .await
            .map_err(internal)?;
        sub.1
    };
    Ok(Json(json!({
        "state": t.t.state.name(),
        "desiredState": t.t.desired_state.name(),
        "currentRound": t.t.current_tick,
        "roundStart": t.t.tick_start,
        "roundEnd": t.t.tick_end,
        "roundTime": t.t.tick_time,
        "lastRound": t.t.stop_after_tick,
        "startAt": t.t.start_at,
        "serverTime": crate::timer::now_secs(),
        "masterTimers": master_timers,
    })))
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn overview_set_timing(
    State(state): State<Arc<AppState>>,
    body: Option<Json<Value>>,
) -> Result<&'static str, (StatusCode, String)> {
    let Json(body) = body.ok_or_else(|| internal("no body"))?;
    let mut t = state.timer.lock().await;
    if let Some(v) = body.get("state").and_then(Value::as_i64).and_then(ctf_state_from_int) {
        match v {
            CtfState::Running => t.start_ctf().await.map_err(internal)?,
            CtfState::Suspended => t.suspend_ctf_after_tick().await.map_err(internal)?,
            CtfState::Stopped => t.stop_ctf_after_tick().await.map_err(internal)?,
        }
    }
    if let Some(roundtime) = body.get("roundtime").and_then(Value::as_i64) {
        if (5..1000).contains(&roundtime) {
            t.set_tick_time(roundtime).await.map_err(internal)?;
        }
    }
    if let Some(v) = body.get("lastround") {
        let lastround = opt_i64(v);
        match lastround {
            None => t.set_stop_after_tick(None).await.map_err(internal)?,
            Some(lr) if lr > t.t.current_tick => t.set_stop_after_tick(Some(lr)).await.map_err(internal)?,
            Some(lr) if lr == t.t.current_tick => t.stop_ctf_after_tick().await.map_err(internal)?,
            _ => {}
        }
    }
    let now = crate::timer::now_secs();
    if let Some(v) = body.get("startAt") {
        let start_at = opt_i64(v);
        if start_at.is_none() || start_at >= Some(now) {
            t.set_start_at(start_at).await.map_err(internal)?;
        }
    }
    if let Some(v) = body.get("openVulnboxAccessAt") {
        let open_at = opt_i64(v);
        if open_at.is_none() || open_at >= Some(now) {
            t.set_open_vulnbox_access_at(open_at).await.map_err(internal)?;
        }
    }
    Ok("OK")
}

// ---- /overview/logs -------------------------------------------------------------

async fn overview_logs(
    State(state): State<Arc<AppState>>,
    Path(min_id): Path<Option<i32>>,
) -> Json<Value> {
    let limit: i64 = if min_id.is_some() { 100 } else { 30 };
    let logs = match min_id {
        Some(min_id) => {
            sqlx::query_as::<_, models::LogMessage>(
                "SELECT id, created, component, level, title, text FROM logmessages \
                 WHERE level >= $1 AND id > $2 ORDER BY id DESC LIMIT $3",
            )
            .bind(models::LOG_INFO)
            .bind(min_id)
            .bind(limit)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
        }
        None => {
            sqlx::query_as::<_, models::LogMessage>(
                "SELECT id, created, component, level, title, text FROM logmessages \
                 WHERE level >= $1 ORDER BY id DESC LIMIT $2",
            )
            .bind(models::LOG_INFO)
            .bind(limit)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
        }
    };
    Json(Value::Array(
        logs.iter()
            .map(|l| {
                json!({
                    "id": l.id,
                    "created": l.created.timestamp(),
                    "component": l.component,
                    "level": l.level,
                    "title": l.title,
                    "text": l.text,
                })
            })
            .collect(),
    ))
}

// ---- /checker_results/view/<id> -------------------------------------------------

async fn checker_results_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let is_json = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/json"))
        .unwrap_or(false);
    if !is_json {
        // HTML rendering is the admin GUI — replaced by the Leptos SPA (ADR-002).
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "HTML GUI not ported (Leptos SPA pending)".to_string(),
        ));
    }
    type CheckerResultRow = (
        i32, i32, i16, i16, String, Option<String>, Option<f64>, Option<String>, Option<chrono::DateTime<chrono::Utc>>,
    );
    let result: Option<CheckerResultRow> =
        sqlx::query_as(
            "SELECT id, tick, team_id, service_id, status, message, time, output, finished \
             FROM checker_results WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match result {
        Some((id, tick, team_id, service_id, status, message, time, output, finished)) => Ok(Json(json!({
            "id": id, "tick": tick, "team_id": team_id, "service_id": service_id,
            "status": status, "message": message, "time": time, "output": output,
            "finished": finished,
        }))),
        None => Err((StatusCode::NOT_FOUND, "Not found".to_string())),
    }
}

// ---- /services/checker_status ---------------------------------------------------

#[derive(Deserialize)]
struct CheckerStatusForm {
    id: String,
    status: String,
}

async fn services_set_checker_status(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CheckerStatusForm>,
) -> Result<&'static str, (StatusCode, &'static str)> {
    let id: i32 = form
        .id
        .parse()
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Expected a value"))?;
    let res = sqlx::query("UPDATE services SET checker_enabled = $1 WHERE id = $2")
        .bind(form.status == "1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?;
    if res.rows_affected() == 0 {
        // Python `expect(...)` raises when the service doesn't exist.
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Expected a value"));
    }
    Ok("OK")
}

// ---- /scripts/recreate_scoreboard ------------------------------------------------

async fn scripts_recreate_scoreboard(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let db = state.db.clone();
    let cfg = state.cfg.clone();
    let timer = state.timer.clone();
    tokio::spawn(async move {
        let t0 = std::time::Instant::now();
        let result = recreate_scoreboard_inner(&cfg, db.clone(), timer).await;
        match result {
            Ok(()) => {
                let _ = logger::log(
                    &db,
                    "scoring",
                    &format!("Scoreboard generated manually, took {:.1} sec", t0.elapsed().as_secs_f64()),
                    "",
                    models::LOG_INFO,
                )
                .await;
            }
            Err((msg, trace)) => {
                let _ = logger::log(&db, "scoring", &format!("Scoreboard failed (manually): {msg}"), &trace, models::LOG_ERROR).await;
            }
        }
    });
    Json(json!({"ok": true, "message": "Recalculation started"}))
}

/// `recreate_scoreboard_inner` — rebuild every tick's scoreboard from scratch.
async fn recreate_scoreboard_inner(
    cfg: &Config,
    db: PgPool,
    timer: SharedTimer,
) -> Result<(), (String, String)> {
    let scoring = Arc::new(
        ScoringCalculation::new(&db, &cfg.scoring)
            .await
            .map_err(|e| (format!("ValueError: {e}"), String::new()))?,
    );
    let mut scoreboards = scoreboard::default_scoreboards(cfg, db, scoring, false);
    for sb in scoreboards.iter_mut() {
        sb.create_scoreboard(0, false, false)
            .await
            .map_err(|e| (e.clone(), String::new()))?;
        let mut rn: i64 = 0;
        let mut end_round = current_end_round(&timer).await;
        while rn <= end_round {
            let has_started = {
                let t = timer.lock().await;
                t.t.state != CtfState::Stopped || end_round > 0
            };
            sb.create_scoreboard(rn, has_started, rn == end_round)
                .await
                .map_err(|e| (e.clone(), String::new()))?;
            rn += 1;
            end_round = current_end_round(&timer).await;
        }
    }
    Ok(())
}

async fn current_end_round(timer: &SharedTimer) -> i64 {
    let t = timer.lock().await;
    if t.t.state == CtfState::Running {
        t.t.current_tick - 1
    } else {
        t.t.current_tick
    }
}

// ---- /metrics/write ---------------------------------------------------------------

#[derive(Deserialize)]
struct MetricRecord {
    metric: String,
    values: std::collections::HashMap<String, Value>,
    #[serde(default)]
    ts: Option<f64>,
    #[serde(default)]
    attributes: std::collections::HashMap<String, Value>,
}

fn metric_value(v: &Value) -> Option<MetricValue> {
    match v {
        Value::Bool(b) => Some(MetricValue::Bool(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(MetricValue::Int)
            .or_else(|| n.as_f64().map(MetricValue::Float)),
        Value::String(s) => Some(MetricValue::Str(s.clone())),
        _ => None,
    }
}

fn metric_pairs(map: &std::collections::HashMap<String, Value>) -> Vec<(String, MetricValue)> {
    map.iter()
        .filter_map(|(k, v)| metric_value(v).map(|v| (k.clone(), v)))
        .collect()
}

async fn metrics_write(
    Json(records): Json<Vec<MetricRecord>>,
) -> impl IntoResponse {
    for record in records {
        metrics_record_many(
            &record.metric,
            metric_pairs(&record.values),
            record.ts,
            metric_pairs(&record.attributes),
        );
    }
    "OK"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctf_state_int_matches_python_enum() {
        // timer.py: STOPPED=1 SUSPENDED=2 RUNNING=3
        assert_eq!(ctf_state_from_int(1), Some(CtfState::Stopped));
        assert_eq!(ctf_state_from_int(2), Some(CtfState::Suspended));
        assert_eq!(ctf_state_from_int(3), Some(CtfState::Running));
        assert_eq!(ctf_state_from_int(0), None);
    }

    #[test]
    fn metric_value_json_rules() {
        assert!(matches!(metric_value(&json!(5)), Some(MetricValue::Int(5))));
        assert!(matches!(metric_value(&json!(5.5)), Some(MetricValue::Float(_))));
        assert!(matches!(metric_value(&json!(true)), Some(MetricValue::Bool(true))));
        assert!(matches!(metric_value(&json!("x")), Some(MetricValue::Str(_))));
        assert!(metric_value(&json!([1])).is_none());
    }

    #[test]
    fn opt_i64_zero_becomes_none_like_python_or() {
        assert_eq!(opt_i64(&json!(0)), None);
        assert_eq!(opt_i64(&json!(7)), Some(7));
        assert_eq!(opt_i64(&Value::Null), None);
    }
}
