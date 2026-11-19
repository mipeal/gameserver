//! Port of `controlserver/scoring/scoreboard.py` (+ filtered_scoring's
//! ctftime filter). Writes the JSON API files the Angular scoreboard reads.

use crate::scoring::{RankingEntry, ScoringCalculation};
use crate::timer::CtfState;
use crate::{logger, models};
use chrono::Utc;
use redis::AsyncCommands;
use saarctf_commons_rs::config::Config;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn is_frozen(cfg: &Config, tick: i64, public: bool) -> bool {
    public
        && cfg
            .scoreboard_freeze
            .as_ref()
            .and_then(|v| v.as_i64())
            .is_some_and(|freeze| tick > freeze)
}

/// `TickInformation`.
pub type CheckerResults = HashMap<(i16, i16), (String, Option<String>)>;

pub struct TickInformation {
    pub ticknumber: i64,
    pub ranking: Vec<RankingEntry>,
    pub team_points: HashMap<(i16, i16), models::TeamPointsLite>,
    pub checker_results: CheckerResults,
}

impl TickInformation {
    /// `get_attacker_victim_count`: per service, the teams that gained since
    /// the previous tick.
    pub fn get_attacker_victim_count(
        &self,
        previous_info: &TickInformation,
    ) -> (HashMap<i16, i64>, HashMap<i16, i64>) {
        let mut attacker_count: HashMap<i16, i64> = HashMap::new();
        let mut victim_count: HashMap<i16, i64> = HashMap::new();
        for (key, pts) in &self.team_points {
            let Some(prev_pts) = previous_info.team_points.get(key) else {
                continue;
            };
            if pts.flag_captured_count > prev_pts.flag_captured_count {
                *attacker_count.entry(key.1).or_default() += 1;
            }
            if pts.flag_stolen_count > prev_pts.flag_stolen_count {
                *victim_count.entry(key.1).or_default() += 1;
            }
        }
        (attacker_count, victim_count)
    }
}

fn read_json(path: &Path, default: Value) -> Value {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(default),
        Err(_) => default,
    }
}

fn write_json(path: &Path, data: &Value) -> Result<(), String> {
    std::fs::write(path, serde_json::to_string(data).unwrap_or_default()).map_err(|e| e.to_string())
}

/// FileLock(self.output / ".lock"): an exclusive flock held for the scope.
pub struct ScoreboardLock(std::fs::File);

impl ScoreboardLock {
    pub fn acquire(path: &Path) -> ScoreboardLock {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .unwrap_or_else(|e| panic!("scoreboard lock {path:?}: {e}"));
        #[cfg(unix)]
        {
            // SAFETY: flock on our own freshly opened file descriptor.
            unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&f), libc::LOCK_EX) };
        }
        ScoreboardLock(f)
    }
}

impl Drop for ScoreboardLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: same descriptor we locked in acquire.
            unsafe {
                libc::flock(
                    std::os::unix::io::AsRawFd::as_raw_fd(&self.0),
                    libc::LOCK_UN,
                )
            };
        }
    }
}

/// `Scoreboard`.
pub struct Scoreboard {
    pub cfg: Config,
    pub db: PgPool,
    pub calculation: Arc<ScoringCalculation>,
    pub angular_build_path: PathBuf,
    pub firstblood_level: i16,
    prepared_static_files: std::sync::atomic::AtomicBool,
    pub teams: Vec<models::Team>,
    pub services: Vec<models::Service>,
    should_publish: bool,
    pub public: bool,
    pub output: PathBuf,
}

impl Scoreboard {
    pub fn new(
        cfg: &Config,
        db: PgPool,
        calculation: Arc<ScoringCalculation>,
        output: PathBuf,
        publish: bool,
        public: bool,
    ) -> Scoreboard {
        Scoreboard {
            cfg: cfg.clone(),
            db,
            firstblood_level: calculation.first_blood.get_required_level(),
            calculation,
            angular_build_path: saarctf_commons_rs::config::python_dir(cfg)
                .join("scoreboard")
                .join("dist")
                .join("scoreboard"),
            prepared_static_files: std::sync::atomic::AtomicBool::new(false),
            teams: vec![],
            services: vec![],
            should_publish: publish,
            public,
            output,
        }
    }

    async fn publish(&self, scoreboard_tick: i64) {
        if let Ok(mut conn) = saarctf_commons_rs::redis::get_redis_connection(&self.cfg.redis).await
        {
            let _: Result<(), _> = conn
                .set("timing:scoreboard_tick", scoreboard_tick.to_string())
                .await;
            let _: Result<(), _> = conn
                .publish::<_, _, ()>("timing:scoreboard_tick", scoreboard_tick.to_string())
                .await;
        }
    }

    pub fn exists(&self, ticknumber: i64, has_started: bool) -> bool {
        let ticknumber = if ticknumber == 0 && !has_started {
            -1
        } else {
            ticknumber
        };
        self.output
            .join("api")
            .join(format!("scoreboard_round_{ticknumber}.json"))
            .exists()
    }

    async fn update_team_service_list(&mut self) -> Result<(), String> {
        self.teams = self
            .calculation
            .get_considered_teams(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        self.services = self
            .calculation
            .get_considered_services(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `create_scoreboard`: write the scoreboard as it is AFTER a given tick.
    pub async fn create_scoreboard(
        &mut self,
        ticknumber: i64,
        has_started: bool,
        is_live: bool,
    ) -> Result<(), String> {
        self.update_team_service_list().await?;
        let ticknumber = if ticknumber == 0 && !has_started {
            -1
        } else {
            ticknumber
        };
        if self.should_publish {
            self.publish(ticknumber).await;
        }
        let scoreboard_is_frozen = is_frozen(&self.cfg, ticknumber, self.public);
        let frozen_tick = if scoreboard_is_frozen {
            self.cfg.scoreboard_freeze.as_ref().and_then(|v| v.as_i64())
        } else {
            None
        };

        let info = self.fetch_data(ticknumber).await?;
        let previous_info = self.fetch_data(ticknumber - 1).await?;
        let frozen_previous_info = match frozen_tick {
            Some(t) => Some(self.fetch_data(t).await?),
            None => None,
        };
        let last_checker_results = vec![
            self.calculation
                .get_checker_results_lite(&self.db, ticknumber - 2)
                .await
                .map_err(|e| e.to_string())?,
            self.calculation
                .get_checker_results_lite(&self.db, ticknumber - 3)
                .await
                .map_err(|e| e.to_string())?,
        ];

        self.check_scoreboard_prepared(false)?;
        self.create_logos().await?;
        self.create_team_json()?;
        let prev_info = frozen_previous_info.as_ref().unwrap_or(&previous_info);
        self.create_json_for_tick(
            &info,
            prev_info,
            &last_checker_results,
            scoreboard_is_frozen,
        )
        .await?;
        self.create_json_for_teams(&info, prev_info, scoreboard_is_frozen)
            .await?;
        self.create_service_stat_json(&info, prev_info, scoreboard_is_frozen)
            .await?;

        if is_live {
            self.update_tick_info(Some(info.ticknumber)).await?;
        }
        Ok(())
    }

    /// `__create_tick_info_json`.
    pub async fn update_tick_info(&self, scoreboard_tick: Option<i64>) -> Result<i64, String> {
        self.check_scoreboard_prepared(false)?;
        let _lock = ScoreboardLock::acquire(&self.output.join(".lock"));
        let scoreboard_tick = match scoreboard_tick {
            Some(t) => t,
            None => {
                let old = read_json(
                    &self.output.join("api").join("scoreboard_current.json"),
                    json!({"scoreboard_tick": -1}),
                );
                old.get("scoreboard_tick")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1)
            }
        };
        let mut data = json!({
            "validity_period": self.calculation.cfg.flags_rounds_valid,
            "scoreboard_tick": scoreboard_tick,
        });
        // Timer state comes from redis (this process may not own the timer)
        if let Ok(state) = crate::timer::read_timing_state(&self.cfg).await {
            data["current_tick"] = json!(state.current_tick);
            data["state"] = json!(state.state.name());
            data["current_tick_start"] = json!(state.tick_start);
            data["current_tick_until"] = json!(state.tick_end);
        }
        if let Ok(mut conn) = saarctf_commons_rs::redis::get_redis_connection(&self.cfg.redis).await
        {
            let banned: Vec<String> = conn.smembers("network:banned").await.unwrap_or_default();
            data["banned_teams"] = json!(banned);
        }
        if is_frozen(&self.cfg, scoreboard_tick, self.public) {
            data["frozen"] = json!(true);
        }
        write_json(
            &self.output.join("api").join("scoreboard_current.json"),
            &data,
        )?;
        if self.should_publish {
            self.publish(scoreboard_tick).await;
        }
        Ok(scoreboard_tick)
    }

    /// `__fetch_data`.
    async fn fetch_data(&self, ticknumber: i64) -> Result<TickInformation, String> {
        let team_ids: Vec<i16> = self.teams.iter().map(|t| t.id as i16).collect();
        let mut team_points = self
            .calculation
            .get_results_for_tick_lite(&self.db, ticknumber, Some(&team_ids), Some(&self.services))
            .await
            .map_err(|e| e.to_string())?;
        // default zero entries for teams/services the query did not return
        for t in &team_ids {
            for s in &self.services {
                team_points
                    .entry((*t, s.id as i16))
                    .or_insert_with(|| models::TeamPointsLite {
                        team_id: *t,
                        service_id: s.id as i16,
                        tick: ticknumber as i32,
                        ..Default::default()
                    });
            }
        }
        let checker_results = self
            .calculation
            .get_checker_results_lite(&self.db, ticknumber)
            .await
            .map_err(|e| e.to_string())?;
        let ranking = self
            .calculation
            .get_ranking_for_tick(&self.db, ticknumber)
            .await
            .map_err(|e| e.to_string())?;
        Ok(TickInformation {
            ticknumber,
            ranking,
            team_points,
            checker_results,
        })
    }

    /// `__create_json_for_tick`.
    async fn create_json_for_tick(
        &self,
        info: &TickInformation,
        previous_info: &TickInformation,
        last_checker_results: &[CheckerResults],
        frozen: bool,
    ) -> Result<(), String> {
        let mut scoreboard = Vec::new();
        for ranking in &info.ranking {
            let (mut off_points, mut def_points, mut sla_points) = (0.0, 0.0, 0.0);
            let (mut prev_off, mut prev_def, mut prev_sla) = (0.0, 0.0, 0.0);
            let mut services = Vec::new();
            for service in &self.services {
                let sid = service.id as i16;
                let check = info.checker_results.get(&(ranking.team_id, sid));
                let prev_pts = previous_info
                    .team_points
                    .get(&(ranking.team_id, sid))
                    .cloned()
                    .unwrap_or_default();
                let pts = info
                    .team_points
                    .get(&(ranking.team_id, sid))
                    .cloned()
                    .unwrap_or_default();
                off_points += pts.off_points;
                def_points += pts.def_points;
                sla_points += pts.sla_points;
                prev_off += prev_pts.off_points;
                prev_def += prev_pts.def_points;
                prev_sla += prev_pts.sla_points;
                let entry = if frozen {
                    json!({
                        "o": prev_pts.off_points, "d": prev_pts.def_points, "s": prev_pts.sla_points,
                        "do": 0, "dd": 0, "ds": 0,
                        "st": prev_pts.flag_stolen_count, "cap": prev_pts.flag_captured_count,
                        "dst": 0, "dcap": 0,
                        "c": check.map(|(st, _)| st.clone()).unwrap_or_else(|| "REVOKED".into()),
                        "m": check.and_then(|(_, m)| m.clone()),
                        "dc": last_checker_results
                            .iter()
                            .map(|results| results.get(&(ranking.team_id, sid)).map(|(st, _)| st.clone()).unwrap_or_else(|| "REVOKED".into()))
                            .collect::<Vec<_>>(),
                    })
                } else {
                    json!({
                        "o": pts.off_points, "d": pts.def_points, "s": pts.sla_points,
                        "do": pts.off_points - prev_pts.off_points,
                        "dd": pts.def_points - prev_pts.def_points,
                        "ds": pts.sla_points - prev_pts.sla_points,
                        "st": pts.flag_stolen_count, "cap": pts.flag_captured_count,
                        "dst": pts.flag_stolen_count - prev_pts.flag_stolen_count,
                        "dcap": pts.flag_captured_count - prev_pts.flag_captured_count,
                        "c": check.map(|(st, _)| st.clone()).unwrap_or_else(|| "REVOKED".into()),
                        "m": check.and_then(|(_, m)| m.clone()),
                        "dc": last_checker_results
                            .iter()
                            .map(|results| results.get(&(ranking.team_id, sid)).map(|(st, _)| st.clone()).unwrap_or_else(|| "REVOKED".into()))
                            .collect::<Vec<_>>(),
                    })
                };
                services.push(entry);
            }
            // frozen: rank/points from the previous tick's ranking
            let entry = if frozen {
                let prev_rank = previous_info
                    .ranking
                    .iter()
                    .find(|r| r.team_id == ranking.team_id);
                json!({
                    "team_id": ranking.team_id,
                    "rank": prev_rank.map(|r| r.rank).unwrap_or(ranking.rank),
                    "points": prev_rank.map(|r| r.points).unwrap_or(ranking.points),
                    "services": services,
                    "o": prev_off, "d": prev_def, "s": prev_sla,
                    "do": 0, "dd": 0, "ds": 0,
                })
            } else {
                json!({
                    "team_id": ranking.team_id,
                    "rank": ranking.rank,
                    "points": ranking.points,
                    "services": services,
                    "o": off_points, "d": def_points, "s": sla_points,
                    "do": off_points - prev_off,
                    "dd": def_points - prev_def,
                    "ds": sla_points - prev_sla,
                })
            };
            scoreboard.push(entry);
        }
        if frozen {
            // do not leak the original order
            scoreboard.sort_by(|a, b| {
                b.get("points")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .partial_cmp(&a.get("points").and_then(Value::as_f64).unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let fb_tick = if frozen {
            previous_info.ticknumber
        } else {
            info.ticknumber
        };
        let first_blood_info = self.get_first_blood_info(fb_tick).await?;
        let counts = if frozen {
            (HashMap::new(), HashMap::new())
        } else {
            info.get_attacker_victim_count(previous_info)
        };
        let services_json: Vec<Value> = self
            .services
            .iter()
            .map(|service| {
                let sid = service.id as i16;
                let (name, attackers, victims) = if frozen {
                    ("?".to_string(), json!("?"), json!("?"))
                } else {
                    (
                        if info.ticknumber >= 0 {
                            service.name.clone()
                        } else {
                            "???".to_string()
                        },
                        json!(counts.0.get(&sid).copied().unwrap_or(0)),
                        json!(counts.1.get(&sid).copied().unwrap_or(0)),
                    )
                };
                let fb = first_blood_info
                    .get(&sid)
                    .cloned()
                    .unwrap_or((vec![], HashSet::new()));
                json!({
                    "name": name,
                    "attackers": attackers,
                    "victims": victims,
                    "first_blood": fb.0,
                    "flag_stores": if service.num_payloads > 1 { service.num_payloads } else { 1 },
                    "flag_stores_exploited": if service.num_payloads > 1 {
                        fb.1.len()
                    } else if !fb.1.is_empty() {
                        1
                    } else {
                        0
                    },
                })
            })
            .collect();
        let data =
            json!({"tick": info.ticknumber, "scoreboard": scoreboard, "services": services_json});
        write_json(
            &self
                .output
                .join("api")
                .join(format!("scoreboard_round_{}.json", info.ticknumber)),
            &data,
        )
    }

    /// `__create_json_for_teams`.
    async fn create_json_for_teams(
        &self,
        info: &TickInformation,
        previous_info: &TickInformation,
        scoreboard_freeze: bool,
    ) -> Result<(), String> {
        let _lock = ScoreboardLock::acquire(&self.output.join(".lock"));
        for team in &self.teams {
            let filename = self
                .output
                .join(format!("api/scoreboard_team_{}.json", team.id));
            team_stat_data(
                &self.db,
                &filename,
                team.id as i16,
                &self.services,
                info,
                previous_info,
                scoreboard_freeze,
                &self.cfg,
            )
            .await?;
        }
        Ok(())
    }

    /// `__create_service_stat_json`.
    async fn create_service_stat_json(
        &self,
        info: &TickInformation,
        previous_info: &TickInformation,
        scoreboard_freeze: bool,
    ) -> Result<(), String> {
        let _lock = ScoreboardLock::acquire(&self.output.join(".lock"));
        service_stat_data(
            &self.db,
            &self
                .output
                .join("api")
                .join("scoreboard_service_stats.json"),
            &self.services,
            info,
            previous_info,
            scoreboard_freeze,
            &self.cfg,
        )
        .await
    }

    /// `__create_team_json`.
    fn create_team_json(&self) -> Result<(), String> {
        let _lock = ScoreboardLock::acquire(&self.output.join(".lock"));
        let mut data = serde_json::Map::new();
        for team in &self.teams {
            data.insert(
                team.id.to_string(),
                json!({
                    "name": team.name,
                    "vulnbox": self.cfg.network.team_id_to_vulnbox_ip(i64::from(team.id)),
                    "aff": team.affiliation.clone().unwrap_or_default(),
                    "web": team.website.clone().unwrap_or_default(),
                    "logo": team.logo.clone().map(|l| format!("{l}.png")).unwrap_or_else(|| "false".into()),
                }),
            );
        }
        write_json(
            &self.output.join("api").join("scoreboard_teams.json"),
            &Value::Object(data),
        )
    }

    /// `__create_logos`: write missing team logos from the database.
    async fn create_logos(&self) -> Result<(), String> {
        let path = self.output.join("logos");
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let existing: HashSet<String> = std::fs::read_dir(&path)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        for team in &self.teams {
            let Some(logo) = &team.logo else { continue };
            let fname = format!("{logo}.png");
            if existing.contains(&fname) {
                continue;
            }
            let img: Option<(Vec<u8>,)> =
                sqlx::query_as("SELECT content FROM team_logos WHERE hash=$1")
                    .bind(logo)
                    .fetch_optional(&self.db)
                    .await
                    .map_err(|e| e.to_string())?;
            match img {
                Some((content,)) => {
                    std::fs::write(path.join(&fname), &content).map_err(|e| e.to_string())?
                }
                None => return Err(format!("Exception: Image missing: {logo}")),
            }
        }
        Ok(())
    }

    /// `create_ctftime_json` (FilteredScoringCalculation: excludes the NOP team).
    pub async fn create_ctftime_json(&self, ticknumber: i64) -> Result<String, String> {
        let rankings = self
            .calculation
            .get_ranking_for_tick(&self.db, ticknumber)
            .await
            .map_err(|e| e.to_string())?;
        let nop = self.calculation.cfg.nop_team_id.as_i64().unwrap_or(-1);
        let standings: Vec<Value> = rankings
            .iter()
            .filter(|r| r.points > 0.0 && i64::from(r.team_id) != nop)
            .map(|r| json!({"pos": r.rank, "team": r.team_name, "score": (r.points * 10000.0).round() / 10000.0}))
            .collect();
        Ok(serde_json::to_string_pretty(&json!({"standings": standings})).unwrap_or_default())
    }

    /// `__get_first_blood_info`: service id → (first-blood entries, payloads).
    async fn get_first_blood_info(
        &self,
        ticknumber: i64,
    ) -> Result<HashMap<i16, (Vec<Value>, HashSet<i32>)>, String> {
        #[derive(sqlx::FromRow)]
        struct Fb {
            service_id: i16,
            payload: i32,
            is_firstblood: i16,
            ts: chrono::DateTime<Utc>,
            name: String,
        }
        let flags: Vec<Fb> = sqlx::query_as(
            "SELECT sf.service_id, sf.payload, sf.is_firstblood, sf.ts, t.name \
             FROM submitted_flags sf JOIN teams t ON t.id = sf.submitted_by \
             WHERE sf.tick_submitted <= $1 AND sf.is_firstblood > 0 AND sf.is_firstblood <= $2 \
             ORDER BY sf.is_firstblood DESC, sf.ts",
        )
        .bind(ticknumber as i16)
        .bind(self.firstblood_level)
        .fetch_all(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        let mut result: HashMap<i16, (Vec<Value>, HashSet<i32>)> = HashMap::new();
        for flag in flags {
            let entry = result
                .entry(flag.service_id)
                .or_insert_with(|| (vec![], HashSet::new()));
            if !entry.1.insert(flag.payload) {
                continue;
            }
            let fp = json!({
                "name": flag.name,
                "ts": flag.ts.timestamp(),
                "confirmed": flag.is_firstblood == self.firstblood_level,
                "level": flag.is_firstblood,
            });
            // join entries if one team scores multiple firstbloods within 5 min
            if let Some(last) = entry.0.last() {
                let same = last.get("name").and_then(Value::as_str) == Some(flag.name.as_str())
                    && last.get("confirmed").and_then(Value::as_bool)
                        == Some(flag.is_firstblood == self.firstblood_level)
                    && (last.get("ts").and_then(Value::as_i64).unwrap_or(0) - flag.ts.timestamp())
                        .abs()
                        < 300;
                if same {
                    continue;
                }
            }
            entry.0.push(fp);
        }
        Ok(result)
    }

    /// `check_scoreboard_prepared`: copy the static files + angular build.
    pub fn check_scoreboard_prepared(&self, force_recreate: bool) -> Result<(), String> {
        if self
            .prepared_static_files
            .load(std::sync::atomic::Ordering::Relaxed)
            && self.output.join("api").exists()
            && !force_recreate
        {
            return Ok(());
        }
        let api_path = self.output.join("api");
        std::fs::create_dir_all(&api_path).map_err(|e| e.to_string())?;
        let base = saarctf_commons_rs::config::python_dir(&self.cfg);
        let static_dir = base.join("controlserver").join("static");
        if static_dir.exists() {
            std::fs::copy(
                static_dir.join("css").join("index.css"),
                self.output.join("index.css"),
            )
            .map_err(|e| e.to_string())?;
        }
        copy_tree(&self.angular_build_path, &self.output)?;
        if !self.public {
            let index = self.output.join("index.html");
            if let Ok(html) = std::fs::read_to_string(&index) {
                let _ = std::fs::write(
                    &index,
                    html.replace("<html lang", "<html class=\"unredacted\" lang"),
                );
            }
        }
        self.prepared_static_files
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// `update_team_info`.
    pub async fn update_team_info(&mut self) -> Result<(), String> {
        self.update_team_service_list().await?;
        self.create_logos().await?;
        self.create_team_json()
    }

    /// `reset_to_tick`.
    pub async fn reset_to_tick(&mut self, tick: i64) -> Result<(), String> {
        let info = self.fetch_data(tick).await?;
        let previous_info = self.fetch_data(tick - 1).await?;
        let frozen = is_frozen(&self.cfg, tick, self.public);
        self.create_json_for_teams(&info, &previous_info, frozen)
            .await?;
        self.create_service_stat_json(&info, &previous_info, frozen)
            .await?;
        self.update_tick_info(Some(tick)).await?;
        Ok(())
    }
}

/// shutil.copytree/copyfile over the angular build.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Ok(()); // no angular build present (e.g. dev) — nothing to copy
    }
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            if target.exists() {
                std::fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// `TeamStatisticJsonGenerator.update_file`: per-service total points of one
/// team. Fast path updates the current tick in place; slow path rebuilds from
/// the DB (Python's get_all_tick_info ignores freeze for the rebuild).
#[allow(clippy::too_many_arguments)]
async fn team_stat_data(
    db: &PgPool,
    path: &Path,
    team_id: i16,
    services: &[models::Service],
    info: &TickInformation,
    previous_info: &TickInformation,
    scoreboard_freeze: bool,
    cfg: &Config,
) -> Result<(), String> {
    let servicenames: Vec<String> = services
        .iter()
        .map(|s| {
            if info.ticknumber >= 0 {
                s.name.clone()
            } else {
                "???".to_string()
            }
        })
        .collect();
    let mut data = read_json(
        path,
        json!({"services": servicenames, "points": vec![Value::Array(vec![]); services.len()]}),
    );
    if data["points"].as_array().is_none_or(|a| a.is_empty()) {
        data["services"] = json!(servicenames);
    }
    if info.ticknumber >= 0 {
        let rows_ok = data["services"] == json!(servicenames)
            && data["points"]
                .as_array()
                .is_some_and(|r| r.len() == services.len())
            && data["points"].as_array().unwrap().iter().all(|row| {
                row.as_array()
                    .is_some_and(|r| r.len() >= info.ticknumber.max(0) as usize)
            });
        if rows_ok {
            // fast path: update this tick only
            for (i, service) in services.iter().enumerate() {
                let sid = service.id as i16;
                let pts = if scoreboard_freeze && is_frozen(cfg, info.ticknumber, true) {
                    previous_info.team_points.get(&(team_id, sid))
                } else {
                    info.team_points.get(&(team_id, sid))
                }
                .cloned()
                .unwrap_or_default();
                let value = json!(pts.off_points + pts.def_points + pts.sla_points);
                let row = &mut data["points"][i];
                let row_arr = row.as_array_mut().expect("checked above");
                let tick_idx = info.ticknumber as usize;
                if row_arr.len() > tick_idx {
                    row_arr[tick_idx] = value;
                } else {
                    row_arr.push(value);
                }
            }
        } else {
            // slow path: recreate up to the current tick
            let rows = data["points"].as_array().cloned().unwrap_or_default();
            let needs_rebuild = rows.len() != services.len()
                || rows
                    .iter()
                    .filter_map(|r| r.as_array().map(|a| a.len()))
                    .min()
                    .unwrap_or(0)
                    <= info.ticknumber as usize;
            if needs_rebuild {
                data["points"] = json!(vec![
                    Value::Array(vec![
                        json!(0.0);
                        (info.ticknumber + 1) as usize
                    ]);
                    services.len()
                ]);
            }
            if info.ticknumber > 0 {
                data["services"] = json!(servicenames);
            }
            let points: Vec<(i32, i16, f64, f64, f64)> = sqlx::query_as(
                "SELECT tick, service_id, off_points, def_points, sla_points FROM team_points \
                 WHERE team_id=$1 AND tick >= 0 AND tick <= $2",
            )
            .bind(i32::from(team_id))
            .bind(info.ticknumber as i32)
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
            for (tick, service_id, o, d, s) in points {
                let idx = services.iter().position(|sv| sv.id as i16 == service_id);
                let Some(idx) = idx else { continue };
                data["points"][idx][tick as usize] = json!(o + d + s);
            }
        }
    }
    write_json(path, &data)
}

/// `ServiceStatisticJsonGenerator.update_file`: attacker/victim counts per
/// service and tick. (Python's rebuild compares each row against itself, so the
/// rebuilt counts are always 0 — ported exactly, ADR-004.)
#[allow(clippy::too_many_arguments)]
async fn service_stat_data(
    _db: &PgPool,
    path: &Path,
    services: &[models::Service],
    info: &TickInformation,
    previous_info: &TickInformation,
    scoreboard_freeze: bool,
    cfg: &Config,
) -> Result<(), String> {
    let servicenames: Vec<String> = services
        .iter()
        .map(|s| {
            if info.ticknumber >= 0 {
                s.name.clone()
            } else {
                "???".to_string()
            }
        })
        .collect();
    let empty_row = || json!({"a": 0, "v": 0});
    let mut data = read_json(
        path,
        json!({"services": servicenames, "stats": vec![Value::Array(vec![]); services.len()]}),
    );
    if data["stats"].as_array().is_none_or(|a| a.is_empty()) {
        data["services"] = json!(servicenames);
    }
    if info.ticknumber >= 0 {
        let rows_ok = data["services"] == json!(servicenames)
            && data["stats"]
                .as_array()
                .is_some_and(|r| r.len() == services.len())
            && data["stats"].as_array().unwrap().iter().all(|row| {
                row.as_array()
                    .is_some_and(|r| r.len() >= info.ticknumber.max(0) as usize)
            });
        if rows_ok {
            let frozen = scoreboard_freeze && is_frozen(cfg, info.ticknumber, true);
            let counts = if frozen {
                (HashMap::new(), HashMap::new())
            } else {
                info.get_attacker_victim_count(previous_info)
            };
            for (i, service) in services.iter().enumerate() {
                let sid = service.id as i16;
                let value = if frozen {
                    json!({"a": "?", "v": "?"})
                } else {
                    json!({
                        "a": counts.0.get(&sid).copied().unwrap_or(0),
                        "v": counts.1.get(&sid).copied().unwrap_or(0),
                    })
                };
                let row_arr = data["stats"][i].as_array_mut().expect("checked above");
                let tick_idx = info.ticknumber as usize;
                if row_arr.len() > tick_idx {
                    row_arr[tick_idx] = value;
                } else {
                    row_arr.push(value);
                }
            }
        } else {
            let rows = data["stats"].as_array().cloned().unwrap_or_default();
            let needs_rebuild = rows.len() != services.len()
                || rows
                    .iter()
                    .filter_map(|r| r.as_array().map(|a| a.len()))
                    .min()
                    .unwrap_or(0)
                    <= info.ticknumber as usize;
            if needs_rebuild {
                data["stats"] = json!(vec![
                    Value::Array(vec![
                        empty_row();
                        (info.ticknumber + 1) as usize
                    ]);
                    services.len()
                ]);
            }
            if info.ticknumber > 0 {
                data["services"] = json!(servicenames);
            }
            // same-key lookup in Python means every comparison is a no-op:
            // all counts stay 0.
            for (i, _service) in services.iter().enumerate() {
                let row_arr = data["stats"][i].as_array_mut().expect("checked above");
                for tick in 0..=info.ticknumber {
                    let value = json!({"a": 0, "v": 0});
                    let tick_idx = tick as usize;
                    if row_arr.len() > tick_idx {
                        row_arr[tick_idx] = value;
                    } else {
                        row_arr.push(value);
                    }
                }
            }
        }
    }
    write_json(path, &data)
}

/// `default_scoreboards`.
pub fn default_scoreboards(
    cfg: &Config,
    db: PgPool,
    calculation: Arc<ScoringCalculation>,
    publish: bool,
) -> Vec<Scoreboard> {
    let mut scoreboards = Vec::new();
    if !cfg.scoreboard_path.is_empty() {
        scoreboards.push(Scoreboard::new(
            cfg,
            db.clone(),
            calculation.clone(),
            PathBuf::from(&cfg.scoreboard_path),
            publish && cfg.scoreboard_path_internal.is_none(),
            true,
        ));
    }
    if let Some(internal) = &cfg.scoreboard_path_internal {
        scoreboards.push(Scoreboard::new(
            cfg,
            db,
            calculation,
            PathBuf::from(internal),
            publish,
            false,
        ));
    }
    scoreboards
}

/// `run_scoreboard_generator`: prepare history, then follow
/// `timing:scoreboard_tick` publications (its own daemon binary).
pub async fn run_scoreboard_generator(cfg: &Config, db: PgPool) -> Result<(), String> {
    let scoring = Arc::new(
        ScoringCalculation::new(&db, &cfg.scoring)
            .await
            .map_err(|e| format!("ValueError: {e}"))?,
    );
    let mut scoreboards = default_scoreboards(cfg, db.clone(), scoring, false);
    let timer = crate::timer::spawn_timer(cfg, false, vec![])
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("Preparing old scoreboard data ...");
    {
        let state = timer.lock().await;
        let has_started = state.state != CtfState::Stopped;
        let prepare_until = if state.state == CtfState::Running {
            state.current_tick - 1
        } else {
            state.current_tick
        };
        let total = scoreboards.len();
        for (i, scoreboard) in scoreboards.iter_mut().enumerate() {
            scoreboard.check_scoreboard_prepared(false)?;
            scoreboard.update_team_info().await?;
            let mut current: i64 = -1;
            if !scoreboard.exists(0, false) {
                scoreboard
                    .create_scoreboard(0, false, prepare_until == 0)
                    .await?;
            }
            while current <= prepare_until {
                if !scoreboard.exists(current, has_started) {
                    scoreboard
                        .create_scoreboard(current, has_started, current == prepare_until)
                        .await?;
                    tracing::info!("- Prepared scoreboard for tick {current}");
                }
                current += 1;
            }
            tracing::info!(
                "Scoreboard {}/{} prepared up to tick {}",
                i + 1,
                total,
                current - 1
            );
        }
    }

    // listen for future scoreboard publications
    tracing::info!("Waiting for new ticks ...");
    let redis_url = saarctf_commons_rs::redis::redis_url(&cfg.redis)?;
    let client = redis::Client::open(redis_url.as_str()).map_err(|e| e.to_string())?;
    let mut current: i64 = -1;
    loop {
        let mut pubsub = match client.get_async_pubsub().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("scoreboard pubsub: {e}; retrying");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        if pubsub.subscribe("timing:scoreboard_tick").await.is_err() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }
        let mut stream = pubsub.into_on_message();
        while let Some(msg) = futures_util::StreamExt::next(&mut stream).await {
            if msg.get_channel_name() != "timing:scoreboard_tick" {
                continue;
            }
            let prepare_until: i64 = match msg
                .get_payload::<String>()
                .ok()
                .and_then(|s| s.parse().ok())
            {
                Some(v) => v,
                None => continue,
            };
            let has_started = {
                let state = timer.lock().await;
                state.state != CtfState::Stopped
            };
            while current <= prepare_until {
                tracing::info!("- Create scoreboard for tick {current} ...");
                for scoreboard in scoreboards.iter_mut() {
                    if let Err(e) = scoreboard
                        .create_scoreboard(current, has_started, true)
                        .await
                    {
                        let _ = logger::log(
                            &scoreboard.db,
                            "scoring",
                            "Scoreboard failed",
                            &e,
                            models::LOG_ERROR,
                        )
                        .await;
                        tracing::error!("Scoreboard failed: {e} (daemon)");
                    }
                }
                current += 1;
            }
        }
    }
}
