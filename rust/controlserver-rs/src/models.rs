//! Port of `controlserver/models.py` — row structs for the baseline schema
//! (migrations/0001_baseline.sql, ADR-005). Queries are written at the call
//! sites that need them; the upserts live here because both binaries share them.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;

pub const CHECKER_STATES: [&str; 8] = [
    "SUCCESS",
    "FLAGMISSING",
    "MUMBLE",
    "OFFLINE",
    "TIMEOUT",
    "RECOVERING",
    "REVOKED",
    "CRASHED",
];

// LogMessage levels
pub const LOG_DEBUG: i16 = 1;
pub const LOG_INFO: i16 = 5;
pub const LOG_IMPORTANT: i16 = 10;
pub const LOG_NOTIFICATION: i16 = 15;
pub const LOG_WARNING: i16 = 20;
pub const LOG_ERROR: i16 = 30;

#[derive(Debug, Clone, FromRow)]
pub struct Team {
    pub id: i32,
    pub name: String,
    pub affiliation: Option<String>,
    pub website: Option<String>,
    pub logo: Option<String>,
    pub vpn_connected: bool,
    pub vpn2_connected: bool,
    pub vpn_connection_count: i32,
    pub vpn_last_connect: Option<DateTime<Utc>>,
    pub vpn_last_disconnect: Option<DateTime<Utc>>,
    pub wg_vulnbox_connected: bool,
    pub wg_boxes_connected: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct Service {
    pub id: i32,
    pub name: String,
    pub checker_timeout: i32,
    pub checker_enabled: bool,
    pub checker_runner: String,
    pub checker_script_dir: Option<String>,
    pub checker_script: String,
    pub package: Option<String>,
    pub setup_package: Option<String>,
    pub checker_subprocess: bool,
    pub checker_route: Option<String>,
    pub runner_config: Option<Value>,
    pub num_payloads: i32,
    pub flag_ids: Option<String>,
    pub flags_per_tick: f64,
    pub ports: String,
}

impl Service {
    /// `parse_ports`: "tcp:123,udp:125" -> [("tcp", 123), ("udp", 125)]
    pub fn parse_ports(&self) -> Result<Vec<(String, i32)>, String> {
        parse_ports_str(&self.ports)
    }
}

/// `Service.parse_ports` on a bare string (for queries that fetch only ports).
pub fn parse_ports_str(ports: &str) -> Result<Vec<(String, i32)>, String> {
    ports
        .split(',')
            .filter_map(|x| {
                let x = x.trim();
                if x.is_empty() {
                    return None;
                }
                Some(x)
            })
            .map(|x| {
                let (proto, port) = x
                    .split_once(':')
                    .ok_or_else(|| format!("ValueError: bad port spec {x:?}"))?;
                Ok((
                    proto.to_string(),
                    port.parse()
                        .map_err(|_| format!("ValueError: bad port {port:?}"))?,
                ))
            })
            .collect()
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckerResult {
    pub id: i32,
    pub tick: i32,
    pub team_id: i16,
    pub service_id: i16,
    pub status: String,
    pub message: Option<String>,
    pub time: Option<f64>,
    pub celery_id: String,
    pub output: Option<String>,
    pub finished: Option<DateTime<Utc>>,
    pub data: Option<Value>,
    pub run_over_time: bool,
}

/// Payload for `CheckerResult.upsert` — the fields `props_dict` carries.
#[derive(Debug, Clone)]
pub struct CheckerResultUpsert {
    pub tick: i32,
    pub team_id: i16,
    pub service_id: i16,
    pub status: String,
    pub message: Option<String>,
    pub time: Option<f64>,
    pub celery_id: String,
    pub output: Option<String>,
    pub run_over_time: Option<bool>,
    pub finished: Option<DateTime<Utc>>,
    pub data: Option<Value>,
}

impl CheckerResultUpsert {
    /// `CheckerResult.upsert(entry)`: on conflict set status/message/time/celery_id/
    /// output/data always; run_over_time and finished only when the entry carries them.
    pub async fn execute(&self, db: &sqlx::PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO checker_results (tick, team_id, service_id, status, message, \"time\", celery_id, output, run_over_time, finished, data) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
             ON CONFLICT (tick, team_id, service_id) DO UPDATE SET status=EXCLUDED.status, message=EXCLUDED.message, \
               \"time\"=EXCLUDED.\"time\", celery_id=EXCLUDED.celery_id, output=EXCLUDED.output, data=EXCLUDED.data \
               , run_over_time=EXCLUDED.run_over_time, finished=EXCLUDED.finished",
        )
        .bind(self.tick)
        .bind(self.team_id)
        .bind(self.service_id)
        .bind(&self.status)
        .bind(&self.message)
        .bind(self.time)
        .bind(&self.celery_id)
        .bind(&self.output)
        .bind(self.run_over_time)
        .bind(self.finished)
        .bind(&self.data)
        .execute(db)
        .await?;
        Ok(())
    }
}

/// `CheckerResultLite.efficient_insert` — bulk insert with empty celery_id.
/// (Postgres-side unnest; one statement for N rows.)
pub async fn checker_results_lite_insert(
    db: &sqlx::PgPool,
    items: &[CheckerResultLite],
) -> Result<(), sqlx::Error> {
    if items.is_empty() {
        return Ok(());
    }
    let team_ids: Vec<i16> = items.iter().map(|x| x.team_id).collect();
    let service_ids: Vec<i16> = items.iter().map(|x| x.service_id).collect();
    let ticks: Vec<i32> = items.iter().map(|x| x.tick).collect();
    let statuses: Vec<String> = items.iter().map(|x| x.status.clone()).collect();
    let run_over: Vec<bool> = items.iter().map(|x| x.run_over_time).collect();
    let messages: Vec<String> = items.iter().map(|x| x.message.clone()).collect();
    sqlx::query(
        "INSERT INTO checker_results (team_id, service_id, tick, status, run_over_time, message, celery_id) \
         SELECT * FROM unnest($1::int2[], $2::int2[], $3::int[], $4::varchar[], $5::bool[], $6::text[], '{}')",
    )
    .bind(&team_ids)
    .bind(&service_ids)
    .bind(&ticks)
    .bind(&statuses)
    .bind(&run_over)
    .bind(&messages)
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CheckerResultLite {
    pub team_id: i16,
    pub service_id: i16,
    pub tick: i32,
    pub status: String,
    pub run_over_time: bool,
    pub message: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct TeamPoints {
    pub id: i32,
    pub tick: i32,
    pub team_id: i16,
    pub service_id: i16,
    pub flag_captured_count: i32,
    pub flag_stolen_count: i32,
    pub off_points: f64,
    pub def_points: f64,
    pub sla_points: f64,
    pub sla_delta: f64,
}

/// `TeamPointsLite` — the projection scoring reads and writes.
#[derive(Debug, Clone, Default, FromRow)]
pub struct TeamPointsLite {
    pub team_id: i16,
    pub service_id: i16,
    pub tick: i32,
    pub flag_captured_count: i32,
    pub flag_stolen_count: i32,
    pub off_points: f64,
    pub def_points: f64,
    pub sla_points: f64,
    pub sla_delta: f64,
}

impl TeamPointsLite {
    pub async fn upsert(&self, db: &sqlx::PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO team_points (tick, team_id, service_id, flag_captured_count, flag_stolen_count, off_points, def_points, sla_points, sla_delta) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (tick, team_id, service_id) DO UPDATE SET \
               flag_captured_count=EXCLUDED.flag_captured_count, flag_stolen_count=EXCLUDED.flag_stolen_count, \
               off_points=EXCLUDED.off_points, def_points=EXCLUDED.def_points, sla_points=EXCLUDED.sla_points, \
               sla_delta=EXCLUDED.sla_delta",
        )
        .bind(self.tick)
        .bind(self.team_id)
        .bind(self.service_id)
        .bind(self.flag_captured_count)
        .bind(self.flag_stolen_count)
        .bind(self.off_points)
        .bind(self.def_points)
        .bind(self.sla_points)
        .bind(self.sla_delta)
        .execute(db)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct TeamRanking {
    pub id: i32,
    pub tick: i32,
    pub team_id: i16,
    pub rank: i32,
    pub points: f64,
}

#[derive(Debug, Clone, FromRow)]
pub struct SubmittedFlag {
    pub id: i32,
    pub submitted_by: i16,
    pub team_id: i16,
    pub service_id: i16,
    pub tick_issued: i16,
    pub payload: i32,
    pub tick_submitted: i16,
    pub is_firstblood: i16,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct LogMessage {
    pub id: i32,
    pub created: DateTime<Utc>,
    pub component: String,
    pub level: Option<i16>,
    pub title: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckerFilesystem {
    pub id: i32,
    pub package: String,
    pub path: String,
    pub file_hash: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckerFile {
    pub id: i32,
    pub file_hash: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Tick {
    pub tick: i32,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl Tick {
    /// `Tick.set_start` / `Tick.set_end`.
    pub async fn set_field(
        db: &sqlx::PgPool,
        field: &str,
        tick: i32,
        ts: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        assert!(field == "start" || field == "end");
        let sql = format!(
            "INSERT INTO ticks (tick, \"{field}\") VALUES ($1, $2) ON CONFLICT (tick) DO UPDATE SET \"{field}\" = $2"
        );
        sqlx::query(&sql).bind(tick).bind(ts).execute(db).await?;
        Ok(())
    }
}
