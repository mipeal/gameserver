//! Port of `controlserver/events_impl.py`: LogCTFEvents, DeferredCTFEvents
//! (dispatcher/scoring/scoreboards at tick boundaries) and DatabaseTickRecording.
//! VPNCTFEvents stays in Python until the wave-2 vpn port (HANDOFF.md).

use crate::dispatcher::Dispatcher;
use crate::logger::{self, log_result_of_execution};
use crate::models::{self, LOG_IMPORTANT};
use crate::scoreboard::Scoreboard;
use crate::scoring::ScoringCalculation;
use crate::timer::CtfEvents;
use chrono::Utc;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::sync::Arc;

/// `LogCTFEvents`: log the game-state events.
pub struct LogCtfEvents(pub PgPool);

#[async_trait::async_trait]
impl CtfEvents for LogCtfEvents {
    async fn on_start_tick(&self, tick: i64, _ts: chrono::DateTime<Utc>) {
        let _ = logger::log(
            &self.0,
            "timer",
            &format!("New tick: {tick}"),
            "",
            LOG_IMPORTANT,
        )
        .await;
    }
    async fn on_start_ctf(&self) {
        let _ = logger::log(&self.0, "timer", "CTF starts", "", LOG_IMPORTANT).await;
    }
    async fn on_suspend_ctf(&self) {
        let _ = logger::log(&self.0, "timer", "CTF suspended", "", LOG_IMPORTANT).await;
    }
    async fn on_end_ctf(&self) {
        let _ = logger::log(&self.0, "timer", "CTF stopped", "", LOG_IMPORTANT).await;
    }
    async fn on_open_vulnbox_access(&self) {
        let _ = logger::log(&self.0, "timer", "Vulnbox access opened", "", LOG_IMPORTANT).await;
    }
}

/// `DatabaseTickRecording`.
pub struct DatabaseTickRecording(pub PgPool);

#[async_trait::async_trait]
impl CtfEvents for DatabaseTickRecording {
    async fn on_start_tick(&self, tick: i64, ts: chrono::DateTime<Utc>) {
        let _ = log_result_of_execution::<(), sqlx::Error, _, _>(
            &self.0,
            "timer",
            || models::Tick::set_field(&self.0, "start", tick as i32, ts),
            None,
            Some("Couldn't set tick start: {}"),
        )
        .await;
    }
    async fn on_end_tick(&self, tick: i64, ts: chrono::DateTime<Utc>) {
        let _ = log_result_of_execution::<(), sqlx::Error, _, _>(
            &self.0,
            "timer",
            || models::Tick::set_field(&self.0, "end", tick as i32, ts),
            None,
            Some("Couldn't set tick end: {}"),
        )
        .await;
    }
}

/// `DeferredCTFEvents`: dispatcher at tick start; revoke + collect + scoring +
/// scoreboards at tick end. Python ran each in its own thread — here each is a
/// tokio task (tick boundaries are minutes apart, so clone-and-spawn is fine).
pub struct DeferredCtfEvents {
    pub cfg: Config,
    pub db: PgPool,
    pub dispatcher: Arc<Dispatcher>,
    pub scoring: Arc<ScoringCalculation>,
    pub scoreboards: Vec<Arc<tokio::sync::Mutex<Scoreboard>>>,
}

impl DeferredCtfEvents {
    /// `__init__`: default_scoreboards(scoring, publish=True).
    pub async fn new(cfg: &Config, db: PgPool) -> Result<DeferredCtfEvents, String> {
        let scoring = Arc::new(
            ScoringCalculation::new(&db, &cfg.scoring)
                .await
                .map_err(|e| e.to_string())?,
        );
        let mut scoreboards = Vec::new();
        if !cfg.scoreboard_path.is_empty() {
            scoreboards.push(Arc::new(tokio::sync::Mutex::new(Scoreboard::new(
                cfg,
                db.clone(),
                scoring.clone(),
                std::path::PathBuf::from(&cfg.scoreboard_path),
                cfg.scoreboard_path_internal.is_none(),
                true,
            ))));
        }
        if let Some(internal) = &cfg.scoreboard_path_internal {
            scoreboards.push(Arc::new(tokio::sync::Mutex::new(Scoreboard::new(
                cfg,
                db.clone(),
                scoring.clone(),
                std::path::PathBuf::from(internal),
                true,
                false,
            ))));
        }
        Ok(DeferredCtfEvents {
            cfg: cfg.clone(),
            db,
            dispatcher: Arc::new(Dispatcher::from_config(cfg)),
            scoring,
            scoreboards,
        })
    }

    async fn create_scoreboards(&self, tick: i64, has_started: bool, is_live: bool) {
        for (i, sb) in self.scoreboards.iter().enumerate() {
            let sb = sb.clone();
            let n = i + 1;
            tokio::spawn(async move {
                let mut sb = sb.lock().await;
                if let Err(e) = sb.create_scoreboard(tick, has_started, is_live).await {
                    tracing::error!("Scoreboard {n} failed: {e}");
                }
            });
        }
    }
}

#[async_trait::async_trait]
impl CtfEvents for DeferredCtfEvents {
    async fn on_start_tick(&self, tick: i64, _ts: chrono::DateTime<Utc>) {
        // dispatcher
        let dispatcher = self.dispatcher.clone();
        let db = self.db.clone();
        let cfg = self.cfg.clone();
        tokio::spawn(async move {
            let mut redis = match saarctf_commons_rs::redis::get_redis_connection(&cfg.redis).await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Couldn't start checker scripts: redis {e}");
                    return;
                }
            };
            let timing = match crate::timer::read_timing_state(&cfg).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("Couldn't start checker scripts: timing {e}");
                    return;
                }
            };
            let t = std::time::Instant::now();
            match dispatcher
                .dispatch_checker_scripts(&db, &mut redis, &timing, tick)
                .await
            {
                Ok(()) => {
                    let _ = logger::log(
                        &db,
                        "dispatcher",
                        &format!(
                            "Checker scripts dispatched, took {:.3} sec",
                            t.elapsed().as_secs_f64()
                        ),
                        "",
                        models::LOG_INFO,
                    )
                    .await;
                }
                Err(e) => {
                    let _ = logger::log(
                        &db,
                        "dispatcher",
                        &format!("Couldn't start checker scripts: {e}"),
                        &format!("{e:?}"),
                        models::LOG_ERROR,
                    )
                    .await;
                }
            }
        });
        // tick == 1: initial scoreboard
        if tick == 1 {
            self.create_scoreboards(0, true, true).await;
        }
    }

    async fn on_end_tick(&self, tick: i64, _ts: chrono::DateTime<Utc>) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        // revoke
        {
            let dispatcher = self.dispatcher.clone();
            let cfg = self.cfg.clone();
            tokio::spawn(async move {
                let Ok(mut redis) =
                    saarctf_commons_rs::redis::get_redis_connection(&cfg.redis).await
                else {
                    return;
                };
                if let Err(e) = dispatcher.revoke_checker_scripts(&mut redis, tick).await {
                    tracing::error!("Couldn't revoke checker scripts: {e}");
                }
            });
        }
        // collect
        {
            let dispatcher = self.dispatcher.clone();
            let db = self.db.clone();
            let cfg = self.cfg.clone();
            let _ = tokio::spawn(async move {
                let Ok(mut redis) =
                    saarctf_commons_rs::redis::get_redis_connection(&cfg.redis).await
                else {
                    return;
                };
                let t = std::time::Instant::now();
                match dispatcher
                    .collect_checker_results(&db, &mut redis, tick)
                    .await
                {
                    Ok(()) => {
                        let _ = logger::log(
                            &db,
                            "dispatcher",
                            &format!(
                                "Collected checker script results, took {:.3} sec",
                                t.elapsed().as_secs_f64()
                            ),
                            "",
                            models::LOG_INFO,
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = logger::log(
                            &db,
                            "dispatcher",
                            &format!("Couldn't collect checker script results: {e}"),
                            &format!("{e:?}"),
                            models::LOG_ERROR,
                        )
                        .await;
                    }
                }
            })
            .await;
        }
        // scoring
        {
            let scoring = self.scoring.clone();
            let db = self.db.clone();
            let t = std::time::Instant::now();
            match scoring.scoring_and_ranking(&db, tick).await {
                Ok(()) => {
                    let _ = logger::log(
                        &self.db,
                        "scoring",
                        &format!(
                            "Ranking calculated, took {:.3} sec",
                            t.elapsed().as_secs_f64()
                        ),
                        "",
                        models::LOG_INFO,
                    )
                    .await;
                }
                Err(e) => {
                    let _ = logger::log(
                        &self.db,
                        "scoring",
                        &format!("Ranking calculation failed: {e}"),
                        &e,
                        models::LOG_ERROR,
                    )
                    .await;
                }
            }
        }
        // scoreboards for this tick ...
        self.create_scoreboards(tick, true, true).await;
        // ... and backfill the previous one if missing
        for (i, sb) in self.scoreboards.iter().enumerate() {
            let (sb, n) = (sb.clone(), i + 1);
            tokio::spawn(async move {
                let mut sb = sb.lock().await;
                if tick > 0 && !sb.exists(tick - 1, true) {
                    if let Err(e) = sb.create_scoreboard(tick - 1, true, false).await {
                        tracing::error!("Scoreboard {n} failed: {e}");
                    }
                }
            });
        }
    }

    async fn on_start_ctf(&self) {
        for sb in &self.scoreboards {
            let sb = sb.clone();
            tokio::spawn(async move {
                if let Err(e) = sb.lock().await.update_tick_info(None).await {
                    tracing::error!("Couldn't create initial scoreboard: {e}");
                }
            });
        }
    }

    async fn on_update_times(&self) {
        for sb in &self.scoreboards {
            let sb = sb.clone();
            tokio::spawn(async move {
                if let Err(e) = sb.lock().await.update_tick_info(None).await {
                    tracing::error!("Couldn't create updated scoreboard: {e}");
                }
            });
        }
    }
}
