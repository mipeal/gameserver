//! Port of `controlserver/dispatcher.py` — dispatches checker tasks, revokes
//! them at tick end, collects incomplete results. Redis Streams replace Celery
//! (ADR-015): the worker saves result rows itself; collect fills in what is
//! missing (REVOKED = never started, TIMEOUT = still running).

use crate::models::{CheckerResultUpsert, Service, Team};
use crate::timer::{redis_set_and_publish, TimingState};
use rand::seq::SliceRandom;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use saarctf_commons_rs::config::Config;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;

/// One checker task. Stream message payload (JSON in the `msg` field).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    pub task_id: String,
    pub runner_spec: String,
    pub package: Option<String>,
    pub checker_script: String,
    pub service_id: i64,
    pub team_id: i64,
    pub tick: i64,
    pub runner_config: Option<serde_json::Value>,
    pub hard_limit: i64,
    pub soft_limit: i64,
    /// epoch seconds, for the delaying dispatcher (ADR-015: worker-side delay,
    /// like celery's broker-held ETA)
    #[serde(default)]
    pub deliver_at: Option<i64>,
}

/// `dispatcher:ref:{tick}` value — per-stream segment covering this tick's tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRef {
    pub streams: Vec<StreamSeg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamSeg {
    pub stream: String,
    pub first: String,
    pub last: String,
    pub count: usize,
}

fn stream_for_route(route: &str) -> String {
    format!("checker:{route}")
}

/// The dispatcher (GenericDispatcher + CeleryDispatcher + DelayingCeleryDispatcher).
pub struct Dispatcher {
    pub cfg: Config,
    pub delaying: bool, // config.RUNNER.dispatcher == "DelayingCeleryDispatcher"
}

impl Dispatcher {
    pub fn from_config(cfg: &Config) -> Dispatcher {
        Dispatcher {
            cfg: cfg.clone(),
            delaying: cfg.runner.dispatcher == "DelayingCeleryDispatcher",
        }
    }

    fn build_task(
        &self,
        service: &Service,
        team_id: i64,
        tick: i64,
        package_override: Option<String>,
        route_override: Option<&str>,
        deliver_at: Option<i64>,
    ) -> (TaskMessage, String) {
        // celery: subprocess tasks get timeout+5 as the hard limit, else timeout
        let (hard, soft) = if service.checker_subprocess {
            (
                service.checker_timeout as i64 + 5 + 5,
                service.checker_timeout as i64 + 5,
            )
        } else {
            (
                service.checker_timeout as i64 + 5,
                service.checker_timeout as i64,
            )
        };
        let msg = TaskMessage {
            task_id: uuid::Uuid::new_v4().to_string(),
            runner_spec: service.checker_runner.clone(),
            package: Some(
                package_override.unwrap_or_else(|| service.package.clone().unwrap_or_default()),
            )
            .filter(|p| !p.is_empty()),
            checker_script: service.checker_script.clone(),
            service_id: service.id as i64,
            team_id,
            tick,
            runner_config: service.runner_config.clone(),
            hard_limit: hard,
            soft_limit: soft,
            deliver_at,
        };
        // celery: route override, else service route, else default queue
        let route = route_override
            .map(str::to_owned)
            .or_else(|| service.checker_route.clone())
            .unwrap_or_else(|| "celery".to_string());
        (msg, stream_for_route(&route))
    }

    async fn xadd(
        redis: &mut ConnectionManager,
        stream: &str,
        msg: &TaskMessage,
    ) -> Result<String, redis::RedisError> {
        let payload = serde_json::to_string(msg).expect("TaskMessage serializes");
        redis::cmd("XADD")
            .arg(stream)
            .arg("*")
            .arg("task_id")
            .arg(&msg.task_id)
            .arg("msg")
            .arg(payload)
            .query_async(redis)
            .await
    }

    /// `dispatch_checker_scripts`.
    pub async fn dispatch_checker_scripts(
        &self,
        db: &PgPool,
        redis: &mut ConnectionManager,
        timing: &TimingState,
        tick: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let teams: Vec<Team> = if self.cfg.dispatcher_check_vpn_status {
            sqlx::query_as(
                "SELECT * FROM teams WHERE vpn_connected OR vpn2_connected OR wg_vulnbox_connected",
            )
            .fetch_all(db)
            .await?
        } else {
            sqlx::query_as("SELECT * FROM teams").fetch_all(db).await?
        };
        let services: Vec<Service> =
            sqlx::query_as("SELECT * FROM services WHERE checker_enabled ORDER BY id")
                .fetch_all(db)
                .await?;

        let mut combinations: Vec<(&Team, &Service)> = teams
            .iter()
            .flat_map(|t| services.iter().map(move |s| (t, s)))
            .collect();
        if combinations.is_empty() {
            return Ok(());
        }
        combinations.shuffle(&mut rand::thread_rng());

        // delaying dispatcher: spread tasks across the tick (only within the current tick)
        let now = crate::timer::now_secs();
        let spread = self.delaying
            && Some(tick) == Some(timing.current_tick)
            && timing.tick_start.is_some()
            && timing.tick_end.is_some()
            && timing.tick_start.unwrap() <= now
            && now < timing.tick_end.unwrap()
            && timing.tick_end.unwrap() - now < 900;

        // per-service counts/factors for the spreading
        let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut factors: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for (_, service) in &combinations {
            *counts.entry(service.id as i64).or_default() += 1;
            factors.entry(service.id as i64).or_insert_with(|| {
                timing.tick_end.unwrap_or(0)
                    - timing.tick_start.unwrap_or(0)
                    - service.checker_timeout as i64
                    - if service.checker_subprocess { 15 } else { 10 }
            });
        }
        let mut seen: std::collections::HashMap<i64, i64> =
            counts.keys().map(|k| (*k, 0)).collect();

        // per-combination stream index; order:{tick} rows are [team, service, tick, stream_idx]
        let mut streams: Vec<StreamSeg> = Vec::new();
        let mut order_rows: Vec<[i64; 4]> = Vec::new();
        for (team, service) in &combinations {
            let (msg, stream) = if spread {
                let sid = service.id as i64;
                let mut delay = factors[&sid] * seen[&sid] / counts[&sid];
                if delay < 3 {
                    delay = 0;
                }
                *seen.entry(sid).or_default() += 1;
                self.build_task(
                    service,
                    team.id as i64,
                    tick,
                    None,
                    None,
                    Some(timing.tick_start.unwrap_or(now) + delay),
                )
            } else {
                self.build_task(service, team.id as i64, tick, None, None, None)
            };
            let id = Self::xadd(redis, &stream, &msg).await?;
            let idx = match streams.last_mut() {
                Some(seg) if seg.stream == stream => {
                    seg.last = id.clone();
                    seg.count += 1;
                    streams.len() - 1
                }
                _ => {
                    streams.push(StreamSeg {
                        stream,
                        first: id,
                        last: String::new(),
                        count: 1,
                    });
                    streams.len() - 1
                }
            };
            let seg = streams.last_mut().unwrap();
            if seg.last.is_empty() {
                seg.last = seg.first.clone();
            }
            order_rows.push([team.id as i64, service.id as i64, tick, idx as i64]);
        }
        let _: () = redis
            .set(
                format!("dispatcher:order:{tick}"),
                serde_json::to_string(&order_rows).unwrap(),
            )
            .await?;
        let _: () = redis
            .set(
                format!("dispatcher:ref:{tick}"),
                serde_json::to_string(&DispatchRef { streams }).unwrap(),
            )
            .await?;

        crate::flag_id_file::generate_and_save(&self.cfg, redis, &teams, &services, timing, tick)
            .await?;
        Ok(())
    }

    /// `dispatch_test_script` — single (team, service) PENDING test run.
    /// Returns (task_id, celery_id-of-row).
    pub async fn dispatch_test_script(
        &self,
        db: &PgPool,
        redis: &mut ConnectionManager,
        team_id: i64,
        service: &Service,
        tick: i64,
        package: Option<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        sqlx::query("DELETE FROM checker_results WHERE team_id=$1 AND service_id=$2 AND tick=$3")
            .bind(team_id as i16)
            .bind(service.id as i16)
            .bind(tick as i32)
            .execute(db)
            .await?;
        sqlx::query(
            "INSERT INTO checker_results (tick, team_id, service_id, status, celery_id) VALUES ($1,$2,$3,'PENDING','')",
        )
        .bind(tick as i32)
        .bind(team_id as i16)
        .bind(service.id as i16)
        .execute(db)
        .await?;
        let (msg, stream) = self.build_task(service, team_id, tick, package, Some("tests"), None);
        Self::xadd(redis, &stream, &msg).await?;
        sqlx::query("UPDATE checker_results SET celery_id=$1 WHERE tick=$2 AND team_id=$3 AND service_id=$4")
            .bind(&msg.task_id)
            .bind(tick as i32)
            .bind(team_id as i16)
            .bind(service.id as i16)
            .execute(db)
            .await?;
        Ok(msg.task_id)
    }

    /// `revoke_checker_scripts` — set the per-tick revoke flag; workers check it
    /// before starting (celery revoke-on-not-started).
    pub async fn revoke_checker_scripts(
        &self,
        redis: &mut ConnectionManager,
        tick: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ref_json: Option<String> = redis.get(format!("dispatcher:ref:{tick}")).await?;
        if ref_json.is_some() {
            redis_set_and_publish(redis, &format!("dispatcher:revoke:{tick}"), Some("1")).await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Ok(())
    }

    /// `collect_checker_results` — fill in REVOKED / TIMEOUT rows for the tick.
    pub async fn collect_checker_results(
        &self,
        db: &PgPool,
        redis: &mut ConnectionManager,
        tick: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if tick <= 0 {
            return Ok(());
        }
        let collect_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let ref_json: Option<String> = redis.get(format!("dispatcher:ref:{tick}")).await?;
        let order_json: Option<String> = redis.get(format!("dispatcher:order:{tick}")).await?;
        let combinations: Vec<(i64, i64, i64)> = match order_json {
            Some(json) => serde_json::from_str(&json)?,
            None => vec![],
        };

        let mut stats = Stats::default();
        if let Some(ref_json) = &ref_json {
            if !combinations.is_empty() {
                let reference: DispatchRef = serde_json::from_str(ref_json)?;
                // task id per (team, service) from the stream segments, aligned
                // with order:{tick}'s stream_idx (ADR-015 formats)
                let mut task_of: std::collections::HashMap<(i64, i64), String> =
                    std::collections::HashMap::new();
                for seg in &reference.streams {
                    let entries: Vec<(String, std::collections::HashMap<String, String>)> =
                        redis::cmd("XRANGE")
                            .arg(&seg.stream)
                            .arg(&seg.first)
                            .arg(&seg.last)
                            .query_async(redis)
                            .await?;
                    for (_, fields) in entries {
                        if let (Some(t), Some(s), Some(id)) = (
                            fields.get("team_id").and_then(|v| v.parse::<i64>().ok()),
                            fields.get("service_id").and_then(|v| v.parse::<i64>().ok()),
                            fields.get("task_id"),
                        ) {
                            task_of.insert((t, s), id.clone());
                        }
                    }
                }
                let running: HashSet<String> = redis
                    .smembers::<_, Vec<String>>(format!("checker:running:{tick}"))
                    .await?
                    .into_iter()
                    .collect();

                for (team_id, service_id, tick_i) in &combinations {
                    let task_id = task_of
                        .get(&(*team_id, *service_id))
                        .cloned()
                        .unwrap_or_default();
                    let exists: Option<(String,)> = sqlx::query_as(
                        "SELECT status FROM checker_results WHERE tick=$1 AND team_id=$2 AND service_id=$3",
                    )
                    .bind(*tick_i as i32)
                    .bind(*team_id as i16)
                    .bind(*service_id as i16)
                    .fetch_optional(db)
                    .await?;
                    match exists {
                        Some(_) => stats.success += 1,
                        None if running.contains(&task_id) => {
                            self.write_collect_row(
                                db,
                                *team_id,
                                *service_id,
                                *tick_i,
                                &task_id,
                                true,
                            )
                            .await?;
                            stats.started += 1;
                        }
                        None => {
                            self.write_collect_row(
                                db,
                                *team_id,
                                *service_id,
                                *tick_i,
                                &task_id,
                                false,
                            )
                            .await?;
                            stats.revoked += 1;
                        }
                    }
                }
                if combinations[0].2 >= 0 {
                    self.issue_warnings(db, &stats, tick, collect_time).await?;
                }
            }
        }

        // Log checker script errors
        let crashed: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT service_id, COUNT(*) FROM checker_results WHERE tick=$1 AND status='CRASHED' GROUP BY service_id",
        )
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        for (service_id, count) in crashed {
            let name: Option<(String,)> = sqlx::query_as("SELECT name FROM services WHERE id=$1")
                .bind(service_id as i16)
                .fetch_optional(db)
                .await?;
            let msg = format!(
                "Checker scripts for {} produced {count} errors in tick {tick}",
                name.map(|n| n.0).unwrap_or_else(|| service_id.to_string())
            );
            let _ = crate::logger::log(db, "dispatcher", &msg, "", crate::models::LOG_ERROR).await;
        }
        Ok(())
    }

    async fn write_collect_row(
        &self,
        db: &PgPool,
        team_id: i64,
        service_id: i64,
        tick: i64,
        task_id: &str,
        started: bool,
    ) -> Result<(), sqlx::Error> {
        let entry = if started {
            CheckerResultUpsert {
                tick: tick as i32,
                team_id: team_id as i16,
                service_id: service_id as i16,
                status: "TIMEOUT".into(),
                message: Some("Service not checked completely".into()),
                time: None,
                celery_id: task_id.into(),
                output: Some("Still running after tick end...".into()),
                run_over_time: Some(true),
                finished: None,
                data: None,
            }
        } else {
            CheckerResultUpsert {
                tick: tick as i32,
                team_id: team_id as i16,
                service_id: service_id as i16,
                status: "REVOKED".into(),
                message: Some("Service not checked".into()),
                time: None,
                celery_id: task_id.into(),
                output: Some("Not started before the tick ended".into()),
                run_over_time: None,
                finished: None,
                data: None,
            }
        };
        entry.execute(db).await
    }

    /// `_issue_warnings`.
    async fn issue_warnings(
        &self,
        db: &PgPool,
        stats: &Stats,
        tick: i64,
        collect_time: f64,
    ) -> Result<(), sqlx::Error> {
        let total = stats.success + stats.started + stats.revoked;
        if stats.revoked > 0 {
            let _ = crate::logger::log(
                db,
                "dispatcher",
                &format!(
                    "Not all checker scripts have been executed. {} / {} revoked, {} / {} still active",
                    stats.revoked, total, stats.started, total
                ),
                "",
                crate::models::LOG_WARNING,
            )
            .await;
        } else if stats.started > 0 {
            let _ = crate::logger::log(
                db,
                "dispatcher",
                &format!(
                    "Not all checker scripts finished in time: {} / {} still active",
                    stats.started, total
                ),
                "",
                crate::models::LOG_WARNING,
            )
            .await;
        } else {
            let last_finished: Option<(chrono::DateTime<chrono::Utc>,)> =
                sqlx::query_as("SELECT MAX(finished) FROM checker_results WHERE tick=$1")
                    .bind(tick as i32)
                    .fetch_optional(db)
                    .await?;
            if let Some((ts,)) = last_finished {
                let delta = collect_time - ts.timestamp() as f64;
                if delta <= 3.5 {
                    let _ = crate::logger::log(
                        db,
                        "dispatcher",
                        &format!(
                            "Worker close to overload: Last checker script finished {delta:.1} sec before deadline"
                        ),
                        "",
                        crate::models::LOG_WARNING,
                    )
                    .await;
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Stats {
    success: usize,
    started: usize,
    revoked: usize,
}
