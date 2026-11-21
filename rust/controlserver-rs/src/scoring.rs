//! Port of `controlserver/scoring/scoring.py` + `scoring/algorithms/*`.
//! Two steps per tick: results (team_points) and ranking (team_rankings).
//! The Python ImportFactory names map onto enum variants (ADR-004: same
//! selection via config strings).

use crate::logger;
use crate::models::{SubmittedFlag, TeamPointsLite};
use ctf_commons_rs::config::ScoringConfig;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};

pub const DEFAULT_ALGORITHM: &str = "ctf:CtfScoreAlgorithm";

/// `StolenFlag` (algorithm.py): a flag submitted this tick plus how often that
/// flag (same victim/service/tick/payload) was already submitted before.
#[derive(Debug, Clone)]
pub struct StolenFlag {
    pub flag: SubmittedFlag,
    pub num_previous_submissions: i64,
    pub num_submissions: i64,
    pub previous_submitter_ids: Vec<i16>,
}

/// `FlagSet`: dedup by (service, victim, tick_issued, payload) — submitter ignored.
#[derive(Default)]
pub struct FlagSet(HashSet<(i16, i16, i16, i32)>);

impl FlagSet {
    pub fn is_new(&mut self, flag: &SubmittedFlag) -> bool {
        self.0.insert((
            flag.service_id,
            flag.team_id,
            flag.tick_issued,
            flag.payload,
        ))
    }
}

/// `ScoreTickAlgorithm` variants: scoring algorithm + happy-hour flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Algo {
    Ctf,
    CtfHappyHour,
    Playground,
    PlaygroundHappyHour,
}

impl Algo {
    pub fn from_config(name: &str) -> Result<Algo, String> {
        match name {
            "ctf:CtfScoreAlgorithm" => Ok(Algo::Ctf),
            "ctf:CtfScoreAlgorithmWithHappyHour" => Ok(Algo::CtfHappyHour),
            "playground:PlaygroundScoreAlgorithm" => Ok(Algo::Playground),
            "playground:PlaygroundScoreAlgorithmWithHappyHour" => Ok(Algo::PlaygroundHappyHour),
            other => Err(format!("ValueError: unknown scoring algorithm {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FirstBloodAlgo {
    Default,
    Multi,
}

impl FirstBloodAlgo {
    pub fn from_config(name: &str) -> Result<FirstBloodAlgo, String> {
        match name {
            "firstblood:DefaultFirstBloodAlgorithm" => Ok(FirstBloodAlgo::Default),
            "firstblood:MultiFirstBloodAlgorithm" => Ok(FirstBloodAlgo::Multi),
            other => Err(format!(
                "ValueError: unknown firstblood algorithm {other:?}"
            )),
        }
    }
}

fn flag_points_happy_hour(cfg: &ScoringConfig, tick_issued: i32) -> f64 {
    // HappyHourFlagPoints.factor: 2.0 if tick >= data['happy_hour_tick'] else 1.0
    let threshold = cfg
        .data
        .get("happy_hour_tick")
        .and_then(|v| v.as_i64())
        .unwrap_or(0xff_ffff);
    if i64::from(tick_issued) >= threshold {
        2.0
    } else {
        1.0
    }
}

/// `FlagPointsCtfDefault` / `FlagPointsFixedWeightDefault`.
fn off_points(
    algo: Algo,
    cfg: &ScoringConfig,
    attacking_teams_count: &HashMap<(i16, i32), i64>,
    flag: &StolenFlag,
    victim_rank: i64,
) -> f64 {
    let base = match algo {
        Algo::Ctf | Algo::CtfHappyHour => {
            // OFF = 1 + sqrt(1 / num_submissions) + sqrt(1 / victim_rank)
            1.0 + ((flag.num_previous_submissions + flag.num_submissions) as f64)
                .recip()
                .sqrt()
                + (victim_rank as f64).recip().sqrt()
        }
        Algo::Playground | Algo::PlaygroundHappyHour => {
            // OFF = 1 + sqrt(1 / attacking_teams_this_tick) + sqrt(1 / victim_rank)
            let n = attacking_teams_count
                .get(&(flag.flag.service_id, flag.flag.payload))
                .copied()
                .unwrap_or(1);
            1.0 + (n as f64).recip().sqrt() + (victim_rank as f64).recip().sqrt()
        }
    };
    match algo {
        Algo::CtfHappyHour | Algo::PlaygroundHappyHour => {
            base * flag_points_happy_hour(cfg, i32::from(flag.flag.tick_issued))
        }
        _ => base,
    }
}

/// `off_points_previous`: None means past results are never corrected (playground).
fn off_points_previous(
    algo: Algo,
    cfg: &ScoringConfig,
    flag: &StolenFlag,
    victim_rank: i64,
) -> Option<f64> {
    match algo {
        Algo::Ctf => Some(
            1.0 + (flag.num_previous_submissions as f64).recip().sqrt()
                + (victim_rank as f64).recip().sqrt(),
        ),
        Algo::CtfHappyHour => Some(
            (1.0 + (flag.num_previous_submissions as f64).recip().sqrt()
                + (victim_rank as f64).recip().sqrt())
                * flag_points_happy_hour(cfg, i32::from(flag.flag.tick_issued)),
        ),
        Algo::Playground | Algo::PlaygroundHappyHour => None,
    }
}

/// `DefensiveCtfDefault`: DEF = (submissions / num_active_teams)^0.3 * SLA.
fn def_points(num_submissions: i64, num_active_teams: i64, victim_sla_when_issued: f64) -> f64 {
    (num_submissions as f64 / num_active_teams as f64).powf(0.3) * victim_sla_when_issued
}

/// `SlaCtfDefault.sla_points`.
fn sla_points(status: &str) -> f64 {
    match status {
        "SUCCESS" => 1.0,
        "RECOVERING" => 0.5,
        _ => 0.0,
    }
}

const ACTIVE_STATUSES: [&str; 3] = ["SUCCESS", "FLAGMISSING", "RECOVERING"];

/// `FirstBloodAlgorithm` variants.
pub enum FirstBlood {
    Default {
        cache: HashMap<i16, HashSet<i32>>, // service_id => {payload}
    },
    Multi {
        limit: i16,
        scoreboard_limit: i16,
        cache_max: HashMap<(i16, i32), i16>, // (service, payload or 0) => level
        cache_victims: HashMap<(i16, i32, i16), HashSet<i16>>, // (service, payload or 0, attacker)
    },
}

impl FirstBlood {
    pub fn new(cfg: &ScoringConfig) -> FirstBlood {
        match FirstBloodAlgo::from_config(&cfg.firstblood_algorithm) {
            Ok(FirstBloodAlgo::Multi) => {
                let limit = cfg
                    .data
                    .get("firstblood_limit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(3) as i16;
                let scoreboard_limit = cfg
                    .data
                    .get("firstblood_limit_scoreboard")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(i64::from(limit)) as i16;
                FirstBlood::Multi {
                    limit,
                    scoreboard_limit,
                    cache_max: HashMap::new(),
                    cache_victims: HashMap::new(),
                }
            }
            _ => FirstBlood::Default {
                cache: HashMap::new(),
            },
        }
    }

    /// `get_required_level`: the "final" firstblood level for the scoreboard.
    pub fn get_required_level(&self) -> i16 {
        match self {
            FirstBlood::Default { .. } => 1,
            FirstBlood::Multi {
                scoreboard_limit, ..
            } => *scoreboard_limit,
        }
    }
}

/// `ScoringCalculation`.
pub struct ScoringCalculation {
    pub cfg: ScoringConfig,
    pub algo: Algo,
    services: Vec<crate::models::Service>,
    pub first_blood: FirstBlood,
    /// `FilteredScoringCalculation` options: empty/false = unfiltered (the
    /// Python subclass overrode these three methods; folded in here).
    pub exclude_team_ids: HashSet<i16>,
    pub subtract_nop_points: bool,
}

impl ScoringCalculation {
    pub async fn new(db: &PgPool, cfg: &ScoringConfig) -> Result<ScoringCalculation, String> {
        let services: Vec<crate::models::Service> = sqlx::query_as("SELECT * FROM services")
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ScoringCalculation {
            cfg: cfg.clone(),
            algo: Algo::from_config(&cfg.algorithm)?,
            services,
            first_blood: FirstBlood::new(cfg),
            exclude_team_ids: HashSet::new(),
            subtract_nop_points: false,
        })
    }

    /// `FilteredScoringCalculation` — a ScoringCalculation that acts like some
    /// teams don't exist (optionally offsetting every point sum by the NOP
    /// team's points so anyone worse than NOP gets 0).
    pub async fn filtered(
        db: &PgPool,
        cfg: &ScoringConfig,
        exclude_team_ids: HashSet<i16>,
        subtract_nop_points: bool,
    ) -> Result<ScoringCalculation, String> {
        let mut calc = Self::new(db, cfg).await?;
        calc.exclude_team_ids = exclude_team_ids;
        calc.subtract_nop_points = subtract_nop_points;
        Ok(calc)
    }

    fn flags_rounds_valid(&self) -> i64 {
        self.cfg.flags_rounds_valid.as_i64().unwrap_or(10)
    }

    /// `scoring_and_ranking`.
    pub async fn scoring_and_ranking(&self, db: &PgPool, tick: i64) -> Result<(), String> {
        self.calculate_scoring_for_tick(db, tick).await?;
        self.calculate_ranking_per_tick(db, tick).await?;
        Ok(())
    }

    /// `_get_results_for_tick_lite` — reads (and calculates if incomplete) the
    /// results of a tick; tick <= 0 gets all-zero defaults.
    pub async fn get_results_for_tick_lite(
        &self,
        db: &PgPool,
        tick: i64,
        team_ids: Option<&[i16]>,
        services: Option<&[crate::models::Service]>,
    ) -> Result<HashMap<(i16, i16), TeamPointsLite>, sqlx::Error> {
        let teams: Vec<i16> = match team_ids {
            Some(t) => t.to_vec(),
            None => sqlx::query_as::<_, (i16,)>("SELECT id FROM teams")
                .fetch_all(db)
                .await?
                .into_iter()
                .map(|(id,)| id)
                .collect(),
        };
        let services = services.map(|s| s.to_vec());
        if tick <= 0 {
            let mut results = HashMap::new();
            for id in teams {
                for service in services.as_deref().unwrap_or(&self.services) {
                    results.insert(
                        (id, service.id as i16),
                        TeamPointsLite {
                            team_id: id,
                            service_id: service.id as i16,
                            tick: tick as i32,
                            flag_captured_count: 0,
                            flag_stolen_count: 0,
                            off_points: 0.0,
                            def_points: 0.0,
                            sla_points: 0.0,
                            sla_delta: 0.0,
                        },
                    );
                }
            }
            results.retain(|(team_id, _), _| !self.exclude_team_ids.contains(team_id));
            return Ok(results);
        }
        let mut team_points: Vec<TeamPointsLite> = sqlx::query_as(
            "SELECT team_id, service_id, tick, flag_captured_count, flag_stolen_count, \
             off_points, def_points, sla_points, sla_delta FROM team_points WHERE tick=$1",
        )
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        let expected = teams.len() * services.as_deref().unwrap_or(&self.services).len();
        if team_points.len() < expected {
            Box::pin(self.calculate_scoring_for_tick(db, tick))
                .await
                .map_err(|e| sqlx::Error::Configuration(e.into()))?;
            team_points = sqlx::query_as(
                "SELECT team_id, service_id, tick, flag_captured_count, flag_stolen_count, \
                 off_points, def_points, sla_points, sla_delta FROM team_points WHERE tick=$1",
            )
            .bind(tick as i32)
            .fetch_all(db)
            .await?;
        }
        let mut results: HashMap<(i16, i16), TeamPointsLite> = team_points
            .into_iter()
            .map(|tp| ((tp.team_id, tp.service_id), tp))
            .collect();
        results.retain(|(team_id, _), _| !self.exclude_team_ids.contains(team_id));
        Ok(results)
    }

    /// `_get_submitted_flags` — this tick's submissions plus per-flag counts of
    /// previous submissions (within `flags_rounds_valid + 2` ticks).
    async fn get_submitted_flags(
        &self,
        db: &PgPool,
        tick: i64,
    ) -> Result<Vec<StolenFlag>, sqlx::Error> {
        let flags: Vec<SubmittedFlag> =
            sqlx::query_as("SELECT * FROM submitted_flags WHERE tick_submitted=$1 ORDER BY ts, id")
                .bind(tick as i16)
                .fetch_all(db)
                .await?;
        if flags.is_empty() {
            return Ok(vec![]);
        }
        // count + distinct previous submitters per flag key
        #[derive(sqlx::FromRow)]
        struct Agg {
            team_id: i16,
            service_id: i16,
            tick_issued: i16,
            payload: i32,
            cnt: i64,
            submitters: Vec<Option<i16>>,
        }
        let window_start = tick - self.flags_rounds_valid() - 2;
        let prev: Vec<Agg> = sqlx::query_as(
            "SELECT team_id, service_id, tick_issued, payload, COUNT(DISTINCT id) AS cnt, \
             array_agg(DISTINCT submitted_by) AS submitters FROM submitted_flags \
             WHERE tick_submitted < $1 AND tick_submitted >= $2 \
             GROUP BY team_id, service_id, tick_issued, payload",
        )
        .bind(tick as i16)
        .bind(window_start as i16)
        .fetch_all(db)
        .await?;
        let this: Vec<Agg> = sqlx::query_as(
            "SELECT team_id, service_id, tick_issued, payload, COUNT(DISTINCT id) AS cnt, \
             array_agg(DISTINCT submitted_by) AS submitters FROM submitted_flags \
             WHERE tick_submitted = $1 GROUP BY team_id, service_id, tick_issued, payload",
        )
        .bind(tick as i16)
        .fetch_all(db)
        .await?;
        let key = |f: &SubmittedFlag| (f.team_id, f.service_id, f.tick_issued, f.payload);
        let agg_key = |a: Agg| {
            let k = (a.team_id, a.service_id, a.tick_issued, a.payload);
            (k, a)
        };
        let prev: HashMap<(i16, i16, i16, i32), Agg> = prev.into_iter().map(agg_key).collect();
        let this: HashMap<(i16, i16, i16, i32), Agg> = this.into_iter().map(agg_key).collect();
        Ok(flags
            .into_iter()
            .map(|flag| {
                let k = key(&flag);
                StolenFlag {
                    num_previous_submissions: prev.get(&k).map(|a| a.cnt).unwrap_or(0),
                    num_submissions: this.get(&k).map(|a| a.cnt).unwrap_or(1),
                    previous_submitter_ids: prev
                        .get(&k)
                        .map(|a| a.submitters.iter().flatten().copied().collect())
                        .unwrap_or_default(),
                    flag,
                }
            })
            .collect())
    }

    /// `_ranking_for_last_ticks`.
    async fn ranking_for_last_ticks(
        &self,
        db: &PgPool,
        tick: i64,
    ) -> Result<HashMap<(i32, i16), i32>, sqlx::Error> {
        let rows: Vec<(i32, i16, i32)> = sqlx::query_as(
            "SELECT tick, team_id, rank FROM team_rankings WHERE tick >= $1 AND tick < $2",
        )
        .bind(tick as i32 - self.flags_rounds_valid() as i32 - 1)
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(t, team, r)| ((t, team), r))
            .collect())
    }

    /// `_sla_delta_for`.
    async fn sla_delta_for(
        &self,
        db: &PgPool,
        service_id: i16,
        tick: i64,
    ) -> Result<HashMap<i16, f64>, sqlx::Error> {
        let rows: Vec<(i16, f64)> = sqlx::query_as(
            "SELECT team_id, sla_delta FROM team_points WHERE service_id=$1 AND tick=$2",
        )
        .bind(service_id)
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// `_calculate_scoring_for_tick` (+ firstblood incremental).
    pub async fn calculate_scoring_for_tick(&self, db: &PgPool, tick: i64) -> Result<(), String> {
        let teams: Vec<i16> = sqlx::query_as::<_, (i32,)>("SELECT id FROM teams")
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id,)| id as i16)
            .collect();
        let services: Vec<crate::models::Service> = sqlx::query_as("SELECT * FROM services")
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
        let services_by_id: HashMap<i16, &crate::models::Service> =
            services.iter().map(|s| (s.id as i16, s)).collect();

        let last_tick_points = self
            .get_results_for_tick_lite(db, tick - 1, Some(&teams), Some(&services))
            .await
            .map_err(|e| e.to_string())?;
        let team_rank_in_tick = self
            .ranking_for_last_ticks(db, tick)
            .await
            .map_err(|e| e.to_string())?;
        let checker_results = self
            .get_checker_results_lite(db, tick)
            .await
            .map_err(|e| e.to_string())?;
        let mut flags = self
            .get_submitted_flags(db, tick)
            .await
            .map_err(|e| e.to_string())?;
        let mut sla_delta_cache: HashMap<(i16, i64), HashMap<i16, f64>> = HashMap::new();
        for flag in &flags {
            let key = (flag.flag.service_id, i64::from(flag.flag.tick_issued));
            if let std::collections::hash_map::Entry::Vacant(e) = sla_delta_cache.entry(key) {
                let v = self
                    .sla_delta_for(db, flag.flag.service_id, i64::from(flag.flag.tick_issued))
                    .await
                    .map_err(|e| e.to_string())?;
                e.insert(v);
            }
        }

        // first blood marking (writes DB + logs)
        self.compute_firstblood_incremental(db, &mut flags)
            .await
            .map_err(|e| e.to_string())?;

        // per (service,payload): distinct submitters this tick (playground)
        let mut attacking_teams: HashMap<(i16, i32), HashSet<i16>> = HashMap::new();
        for flag in &flags {
            attacking_teams
                .entry((flag.flag.service_id, flag.flag.payload))
                .or_default()
                .insert(flag.flag.submitted_by);
        }
        let attacking_teams_count: HashMap<(i16, i32), i64> = attacking_teams
            .into_iter()
            .map(|(k, v)| (k, v.len() as i64))
            .collect();

        let off_factor = self.cfg.off_factor.as_f64().unwrap_or(1.0);
        let def_factor = self.cfg.def_factor.as_f64().unwrap_or(1.0);
        let sla_factor = self.cfg.sla_factor.as_f64().unwrap_or(1.0);

        // 1. spaces for results (this tick's deltas only, until the last step)
        let mut new_tick_points: HashMap<(i16, i16), TeamPointsLite> = HashMap::new();
        for team_id in &teams {
            for service in &services {
                new_tick_points
                    .entry((*team_id, service.id as i16))
                    .or_insert_with(|| TeamPointsLite {
                        team_id: *team_id,
                        service_id: service.id as i16,
                        tick: tick as i32,
                        ..Default::default()
                    });
            }
        }

        // 2. SLA + number of active teams
        let mut num_active_teams: HashSet<i16> = HashSet::new();
        for ((team_id, service_id), tp) in new_tick_points.iter_mut() {
            let status = checker_results
                .get(&(*team_id, *service_id))
                .map(|(st, _)| st.as_str())
                .unwrap_or("REVOKED");
            let sla = sla_points(status);
            tp.sla_delta = sla * sla_factor;
            if sla > 0.0 || ACTIVE_STATUSES.contains(&status) {
                num_active_teams.insert(*team_id);
            }
        }
        let num_active_teams = num_active_teams.len().max(1) as i64;
        for tp in new_tick_points.values_mut() {
            tp.sla_delta *= (num_active_teams as f64).sqrt();
        }
        for ((team_id, service_id), tp) in &new_tick_points {
            sla_delta_cache
                .entry((*service_id, tick))
                .or_default()
                .insert(*team_id, tp.sla_delta);
        }

        // 3. distribute points for the flags submitted this tick
        let mut stolen_flags = FlagSet::default();
        for flag in &flags {
            let Some(service) = services_by_id.get(&flag.flag.service_id) else {
                let msg = format!(
                    "Flag submitted for invalid team/service: flag #{} ({}, {})",
                    flag.flag.id, flag.flag.team_id, flag.flag.service_id
                );
                tracing::warn!("{msg}");
                let _ = logger::log(
                    db,
                    "scoring",
                    "Flag submitted for invalid team/service",
                    &msg,
                    crate::models::LOG_WARNING,
                )
                .await;
                continue;
            };
            let service_flags_per_tick = service.flags_per_tick;
            let victim_rank = team_rank_in_tick
                .get(&(i32::from(flag.flag.tick_issued) - 1, flag.flag.team_id))
                .copied()
                .unwrap_or(teams.len() as i32) as i64;

            // attacker points
            let attacker_key = (flag.flag.submitted_by, flag.flag.service_id);
            if let Some(attacker) = new_tick_points.get_mut(&attacker_key) {
                attacker.flag_captured_count += 1;
                attacker.off_points += off_points(
                    self.algo,
                    &self.cfg,
                    &attacking_teams_count,
                    flag,
                    victim_rank,
                ) / service_flags_per_tick
                    * off_factor;
            }
            if stolen_flags.is_new(&flag.flag) {
                // victim deduction, once per stolen flag
                let victim_key = (flag.flag.team_id, flag.flag.service_id);
                let victim_sla_when_issued = sla_delta_cache
                    .get(&(flag.flag.service_id, i64::from(flag.flag.tick_issued)))
                    .and_then(|m| m.get(&flag.flag.team_id))
                    .copied()
                    .unwrap_or(0.0);
                let prev_damage = def_points(
                    flag.num_previous_submissions,
                    num_active_teams,
                    victim_sla_when_issued,
                );
                let new_damage = def_points(
                    flag.num_previous_submissions + flag.num_submissions,
                    num_active_teams,
                    victim_sla_when_issued,
                );
                if let Some(victim) = new_tick_points.get_mut(&victim_key) {
                    victim.def_points -=
                        (new_damage - prev_damage) / service_flags_per_tick * def_factor;
                    if flag.num_previous_submissions == 0 {
                        victim.flag_stolen_count += 1;
                    }
                }
                // previous attackers' value correction, once per stolen flag
                if flag.num_previous_submissions > 0 {
                    if let Some(previous_flagpoints) =
                        off_points_previous(self.algo, &self.cfg, flag, victim_rank)
                    {
                        let new_flagpoints = off_points(
                            self.algo,
                            &self.cfg,
                            &attacking_teams_count,
                            flag,
                            victim_rank,
                        );
                        for ps in &flag.previous_submitter_ids {
                            if let Some(prev_attacker) =
                                new_tick_points.get_mut(&(*ps, flag.flag.service_id))
                            {
                                prev_attacker.off_points += (new_flagpoints - previous_flagpoints)
                                    / service_flags_per_tick
                                    * off_factor;
                            }
                        }
                    }
                }
            }
        }

        // 4. add the points from the previous tick
        for ((team_id, service_id), tp) in new_tick_points.iter_mut() {
            if let Some(lr) = last_tick_points.get(&(*team_id, *service_id)) {
                tp.off_points += lr.off_points;
                tp.def_points += lr.def_points;
                tp.sla_points = lr.sla_points + tp.sla_delta;
                tp.flag_captured_count += lr.flag_captured_count;
                tp.flag_stolen_count += lr.flag_stolen_count;
            }
        }

        // 5. save (delete + bulk insert, `TeamPoints.efficient_insert`)
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM team_points WHERE tick=$1")
            .bind(tick as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let items: Vec<&TeamPointsLite> = new_tick_points.values().collect();
        save_teampoints(&mut tx, tick as i32, &items)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `_get_checker_results` → (team, service) => (status, message).
    pub async fn get_checker_results_lite(
        &self,
        db: &PgPool,
        tick: i64,
    ) -> Result<HashMap<(i16, i16), (String, Option<String>)>, sqlx::Error> {
        if tick <= 0 {
            return Ok(HashMap::new());
        }
        let rows: Vec<(i16, i16, String, Option<String>)> = sqlx::query_as(
            "SELECT team_id, service_id, status, message FROM checker_results WHERE tick=$1",
        )
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(t, s, st, m)| ((t, s), (st, m)))
            .collect())
    }

    /// `compute_firstblood_incremental`.
    pub async fn compute_firstblood_incremental(
        &self,
        db: &PgPool,
        flags: &mut [StolenFlag],
    ) -> Result<(), sqlx::Error> {
        let firstbloods = self.get_firstbloods(db, flags).await?;
        for (flag, level) in firstbloods {
            let service = self
                .services
                .iter()
                .find(|s| s.id as i16 == flag.service_id);
            self.record_first_blood(db, &flag, service, level, true)
                .await?;
        }
        Ok(())
    }

    /// FirstBloodAlgorithm.get_firstbloods.
    async fn get_firstbloods(
        &self,
        db: &PgPool,
        flags: &[StolenFlag],
    ) -> Result<Vec<(SubmittedFlag, i16)>, sqlx::Error> {
        let mut result = Vec::new();
        let mut candidates = FlagSet::default();
        match &self.first_blood {
            FirstBlood::Default { cache } => {
                let mut cache = cache.clone();
                for sf in flags {
                    let flag = &sf.flag;
                    if !self.services.iter().any(|s| s.id as i16 == flag.service_id) {
                        continue;
                    }
                    if !candidates.is_new(flag) {
                        continue;
                    }
                    let service = self
                        .services
                        .iter()
                        .find(|s| s.id as i16 == flag.service_id)
                        .unwrap();
                    if is_first_blood(db, &mut cache, service, flag).await? {
                        result.push((flag.clone(), 1));
                        cache
                            .entry(flag.service_id)
                            .or_default()
                            .insert(flag.payload);
                    }
                }
            }
            FirstBlood::Multi {
                limit,
                cache_max,
                cache_victims,
                ..
            } => {
                let mut cache_max = cache_max.clone();
                let mut cache_victims = cache_victims.clone();
                for sf in flags {
                    let flag = &sf.flag;
                    let Some(service) = self
                        .services
                        .iter()
                        .find(|s| s.id as i16 == flag.service_id)
                    else {
                        continue;
                    };
                    let use_payload = service.num_payloads > 0;
                    let key1 = if use_payload {
                        (flag.service_id, flag.payload)
                    } else {
                        (flag.service_id, 0)
                    };
                    let key2 = if use_payload {
                        (flag.service_id, flag.payload, flag.submitted_by)
                    } else {
                        (flag.service_id, 0, flag.submitted_by)
                    };
                    if cache_max.get(&key1).copied().unwrap_or(0) >= *limit {
                        continue;
                    }
                    if cache_victims
                        .get(&key2)
                        .is_some_and(|v| v.contains(&flag.team_id))
                    {
                        continue;
                    }
                    let current = cache_max
                        .get(&key1)
                        .copied()
                        .unwrap_or(0)
                        .max(max_firstblood_level(db, service, flag).await?);
                    cache_max.insert(key1, current);
                    if current >= *limit {
                        continue;
                    }
                    let previous = previous_victims(db, service, flag).await?;
                    cache_victims.entry(key2).or_default().extend(previous);
                    if cache_victims
                        .get(&key2)
                        .is_some_and(|v| v.contains(&flag.team_id))
                    {
                        continue;
                    }
                    cache_victims.entry(key2).or_default().insert(flag.team_id);
                    let level = cache_victims
                        .get(&key2)
                        .map(|v| v.len() as i16)
                        .unwrap_or(1);
                    if level > current {
                        result.push((flag.clone(), level));
                        cache_max.insert(key1, level);
                    }
                }
            }
        }
        Ok(result)
    }

    /// `recompute_first_blood_flags` — drop all first-blood markings and
    /// recompute from scratch (per service). Restart other scoring
    /// calculators afterwards (to clear their cache).
    pub async fn recompute_first_blood_flags(&self, db: &PgPool) -> Result<(), String> {
        sqlx::query("UPDATE submitted_flags SET is_firstblood=0 WHERE is_firstblood>0")
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
        match &self.first_blood {
            FirstBlood::Default { .. } => {
                // after the reset, the earliest submission per (service, payload)
                // — or per service when num_payloads == 0 — is the first blood
                for service in &self.services {
                    println!("recompute firstbloods for {}...", service.name);
                    if service.num_payloads == 0 {
                        sqlx::query(
                            "UPDATE submitted_flags SET is_firstblood=1 WHERE id = (\
                             SELECT id FROM submitted_flags WHERE service_id=$1 ORDER BY ts, id LIMIT 1)",
                        )
                        .bind(service.id)
                        .execute(db)
                        .await
                        .map_err(|e| e.to_string())?;
                    } else {
                        sqlx::query(
                            "UPDATE submitted_flags SET is_firstblood=1 WHERE id IN (\
                             SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY payload, service_id \
                             ORDER BY ts, tick_submitted, id) AS rk FROM submitted_flags \
                             WHERE service_id=$1) s WHERE rk=1)",
                        )
                        .bind(service.id)
                        .execute(db)
                        .await
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
            FirstBlood::Multi { limit, .. } => {
                // get_firstbloods per (service, tick) with fresh caches
                let mut cache_max: HashMap<(i16, i32), i16> = HashMap::new();
                let mut cache_victims: HashMap<(i16, i32, i16), HashSet<i16>> = HashMap::new();
                for service in &self.services {
                    println!("recompute firstbloods for {}...", service.name);
                    let (min_tick, max_tick): (Option<i16>, Option<i16>) = sqlx::query_as(
                        "SELECT MIN(tick_submitted), MAX(tick_submitted) FROM submitted_flags WHERE service_id=$1",
                    )
                    .bind(service.id)
                    .fetch_one(db)
                    .await
                    .map_err(|e| e.to_string())?;
                    let (Some(min_tick), Some(max_tick)) = (min_tick, max_tick) else {
                        continue;
                    };
                    for tick in min_tick..=max_tick {
                        let flags: Vec<SubmittedFlag> = sqlx::query_as(
                            "SELECT * FROM submitted_flags WHERE tick_submitted=$1 AND service_id=$2 ORDER BY ts, id",
                        )
                        .bind(tick)
                        .bind(service.id)
                        .fetch_all(db)
                        .await
                        .map_err(|e| e.to_string())?;
                        let mut new_firstbloods: Vec<(SubmittedFlag, i16)> = Vec::new();
                        for flag in &flags {
                            let use_payload = service.num_payloads > 0;
                            let key1 = if use_payload {
                                (service.id as i16, flag.payload)
                            } else {
                                (service.id as i16, 0)
                            };
                            let key2 = if use_payload {
                                (service.id as i16, flag.payload, flag.submitted_by)
                            } else {
                                (service.id as i16, 0, flag.submitted_by)
                            };
                            if cache_max.get(&key1).copied().unwrap_or(0) >= *limit {
                                continue;
                            }
                            if cache_victims
                                .get(&key2)
                                .is_some_and(|v| v.contains(&flag.team_id))
                            {
                                continue;
                            }
                            let current = cache_max.get(&key1).copied().unwrap_or(0).max(
                                max_firstblood_level(db, service, flag)
                                    .await
                                    .map_err(|e| e.to_string())?,
                            );
                            cache_max.insert(key1, current);
                            if current >= *limit {
                                continue;
                            }
                            let previous = previous_victims(db, service, flag)
                                .await
                                .map_err(|e| e.to_string())?;
                            cache_victims.entry(key2).or_default().extend(previous);
                            if cache_victims
                                .get(&key2)
                                .is_some_and(|v| v.contains(&flag.team_id))
                            {
                                continue;
                            }
                            cache_victims.entry(key2).or_default().insert(flag.team_id);
                            let level = cache_victims
                                .get(&key2)
                                .map(|v| v.len() as i16)
                                .unwrap_or(1);
                            if level > current {
                                new_firstbloods.push((flag.clone(), level));
                                cache_max.insert(key1, level);
                            }
                        }
                        let done = !new_firstbloods.is_empty()
                            && (0..std::cmp::max(1, service.num_payloads)).all(|pl| {
                                cache_max
                                    .get(&(service.id as i16, pl))
                                    .copied()
                                    .unwrap_or(0)
                                    >= *limit
                            });
                        for (flag, level) in new_firstbloods {
                            let svc = self
                                .services
                                .iter()
                                .find(|s| s.id as i16 == flag.service_id);
                            self.record_first_blood(db, &flag, svc, level, false)
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                        if done {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `_record_first_blood`.
    async fn record_first_blood(
        &self,
        db: &PgPool,
        flag: &SubmittedFlag,
        service: Option<&crate::models::Service>,
        level: i16,
        write_log: bool,
    ) -> Result<(), sqlx::Error> {
        if write_log {
            let submitted_by: Option<(String,)> =
                sqlx::query_as("SELECT name FROM teams WHERE id=$1")
                    .bind(i32::from(flag.submitted_by))
                    .fetch_optional(db)
                    .await?;
            let victim: Option<(String,)> = sqlx::query_as("SELECT name FROM teams WHERE id=$1")
                .bind(i32::from(flag.team_id))
                .fetch_optional(db)
                .await?;
            let service_name = service.map(|s| s.name.as_str()).unwrap_or("?");
            let submitter_name = submitted_by
                .map(|n| n.0)
                .unwrap_or_else(|| flag.submitted_by.to_string());
            let text = format!(
                "Time: {}\nStolen from: {}\nSubmitted by: {}\nFlag #{}, payload {}, issued in tick {}.",
                flag.ts.format("%H:%M:%S"),
                victim.map(|n| n.0).unwrap_or_else(|| flag.team_id.to_string()),
                submitter_name,
                flag.id,
                flag.payload,
                flag.tick_issued
            );
            let _ = logger::log(
                db,
                "scoring",
                &format!(
                    "First Blood: \"{submitter_name}\" on \"{service_name}\" (flag {})",
                    flag.payload
                ),
                &text,
                crate::models::LOG_NOTIFICATION,
            )
            .await;
        }
        sqlx::query("UPDATE submitted_flags SET is_firstblood=$1 WHERE id=$2")
            .bind(level)
            .bind(flag.id)
            .execute(db)
            .await?;
        Ok(())
    }

    // ----- Ranking ---

    /// `calculate_ranking_per_tick`.
    pub async fn calculate_ranking_per_tick(&self, db: &PgPool, tick: i64) -> Result<(), String> {
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM team_rankings WHERE tick=$1")
            .bind(tick as i32)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;

        let teams: Vec<i16> = sqlx::query_as::<_, (i32,)>("SELECT id FROM teams")
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id,)| id as i16)
            .collect();
        let results = self
            .get_results_for_tick_lite(db, tick, Some(&teams), None)
            .await
            .map_err(|e| e.to_string())?;
        let mut ranking: Vec<(i16, f64)> = teams.iter().map(|id| (*id, 0.0)).collect();
        let mut by_team: HashMap<i16, f64> = HashMap::new();
        for tp in results.values() {
            *by_team.entry(tp.team_id).or_default() +=
                tp.off_points + tp.def_points + tp.sla_points;
        }
        for (team_id, points) in ranking.iter_mut() {
            *points = by_team.get(team_id).copied().unwrap_or(0.0);
        }
        let ranks = order_by_points(ranking);
        // bulk insert
        let team_ids: Vec<i16> = ranks.iter().map(|r| r.0).collect();
        let rank_values: Vec<i32> = ranks.iter().map(|r| r.1).collect();
        let points: Vec<f64> = ranks.iter().map(|r| r.2).collect();
        sqlx::query(
            "INSERT INTO team_rankings (tick, team_id, rank, points) \
             SELECT $1, unnest($2::int2[]), unnest($3::int4[]), unnest($4::float8[])",
        )
        .bind(tick as i32)
        .bind(&team_ids)
        .bind(&rank_values)
        .bind(&points)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `get_considered_teams`.
    pub async fn get_considered_teams(
        &self,
        db: &PgPool,
    ) -> Result<Vec<crate::models::Team>, sqlx::Error> {
        let teams: Vec<crate::models::Team> = sqlx::query_as("SELECT * FROM teams ORDER BY id")
            .fetch_all(db)
            .await?;
        Ok(teams
            .into_iter()
            .filter(|t| !self.exclude_team_ids.contains(&(t.id as i16)))
            .collect())
    }

    /// `get_considered_services`.
    pub async fn get_considered_services(
        &self,
        db: &PgPool,
    ) -> Result<Vec<crate::models::Service>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM services ORDER BY id")
            .fetch_all(db)
            .await
    }

    /// `get_ranking_for_tick`: the ordered ranking rows of a tick
    /// (tick <= 0: all teams with rank 1 and no points).
    pub async fn get_ranking_for_tick(
        &self,
        db: &PgPool,
        tick: i64,
    ) -> Result<Vec<RankingEntry>, sqlx::Error> {
        if tick <= 0 {
            let teams: Vec<(i32, String)> =
                sqlx::query_as("SELECT id, name FROM teams ORDER BY id")
                    .fetch_all(db)
                    .await?;
            let entries: Vec<RankingEntry> = teams
                .into_iter()
                .map(|(id, name)| RankingEntry {
                    team_id: id as i16,
                    team_name: name,
                    tick: 0,
                    rank: 1,
                    points: 0.0,
                })
                .collect();
            return Ok(self.filter_rankings(entries));
        }
        let rows: Vec<(i16, i32, i32, f64)> = sqlx::query_as(
            "SELECT r.team_id, r.tick, r.rank, r.points FROM team_rankings r \
             WHERE r.tick=$1 ORDER BY r.rank, r.team_id",
        )
        .bind(tick as i32)
        .fetch_all(db)
        .await?;
        let names: HashMap<i16, String> =
            sqlx::query_as::<_, (i32, String)>("SELECT id, name FROM teams")
                .fetch_all(db)
                .await?
                .into_iter()
                .map(|(id, name)| (id as i16, name))
                .collect();
        Ok(rows
            .into_iter()
            .map(|(team_id, tick, rank, points)| RankingEntry {
                team_name: names.get(&team_id).cloned().unwrap_or_default(),
                team_id,
                tick,
                rank,
                points,
            })
            .collect::<Vec<_>>())
        .map(|rankings| self.filter_rankings(rankings))
    }

    /// `FilteredScoringCalculation.get_ranking_for_tick`: drop excluded teams
    /// (compensating ranks) and optionally offset points by the NOP team's sum.
    fn filter_rankings(&self, mut rankings: Vec<RankingEntry>) -> Vec<RankingEntry> {
        if self.exclude_team_ids.is_empty() && !self.subtract_nop_points {
            return rankings;
        }
        rankings.sort_by_key(|r| r.rank);
        let nop = self.cfg.nop_team_id.as_i64().unwrap_or(-1);
        let point_offset = if self.subtract_nop_points {
            rankings
                .iter()
                .find(|r| r.team_id as i64 == nop)
                .map(|r| r.points)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let mut removed = 0;
        rankings
            .into_iter()
            .filter_map(|mut r| {
                if self.exclude_team_ids.contains(&r.team_id) {
                    removed += 1;
                    None
                } else {
                    r.rank -= removed;
                    r.points = (r.points - point_offset).max(0.0);
                    Some(r)
                }
            })
            .collect()
    }
}

/// TeamRanking row plus the team's name (Python carried the ORM relation).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RankingEntry {
    pub team_id: i16,
    #[sqlx(default)]
    pub team_name: String,
    pub tick: i32,
    pub rank: i32,
    pub points: f64,
}

/// `TeamPoints.efficient_insert` (unnest bulk insert).
async fn save_teampoints(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tick: i32,
    items: &[&TeamPointsLite],
) -> Result<(), sqlx::Error> {
    if items.is_empty() {
        return Ok(());
    }
    let teams: Vec<i16> = items.iter().map(|x| x.team_id).collect();
    let services: Vec<i16> = items.iter().map(|x| x.service_id).collect();
    let captured: Vec<i32> = items.iter().map(|x| x.flag_captured_count).collect();
    let stolen: Vec<i32> = items.iter().map(|x| x.flag_stolen_count).collect();
    let off: Vec<f64> = items.iter().map(|x| x.off_points).collect();
    let def: Vec<f64> = items.iter().map(|x| x.def_points).collect();
    let sla: Vec<f64> = items.iter().map(|x| x.sla_points).collect();
    let sla_delta: Vec<f64> = items.iter().map(|x| x.sla_delta).collect();
    sqlx::query(
        "INSERT INTO team_points (team_id, service_id, tick, flag_captured_count, flag_stolen_count, \
         off_points, def_points, sla_points, sla_delta) \
         SELECT unnest($1::int2[]), unnest($2::int2[]), $3, unnest($4::int4[]), unnest($5::int4[]), \
         unnest($6::float8[]), unnest($7::float8[]), unnest($8::float8[]), unnest($9::float8[])",
    )
    .bind(&teams)
    .bind(&services)
    .bind(tick)
    .bind(&captured)
    .bind(&stolen)
    .bind(&off)
    .bind(&def)
    .bind(&sla)
    .bind(&sla_delta)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// `_order_by_points`: stable sort by points desc, ties keep the previous rank,
/// `i` advances only on positive points.
fn order_by_points(ranking: Vec<(i16, f64)>) -> Vec<(i16, i32, f64)> {
    let mut sorted: Vec<(i16, f64)> = ranking;
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::with_capacity(sorted.len());
    let mut i: i32 = 1;
    let mut prev: Option<(i32, f64)> = None; // (rank, points)
    for (team_id, points) in sorted {
        let rank = match prev {
            Some((pr, pp)) if pp == points => pr,
            _ => i,
        };
        prev = Some((rank, points));
        if points > 0.0 {
            i += 1;
        }
        out.push((team_id, rank, points));
    }
    out
}

/// `DefaultFirstBloodAlgorithm._is_first_blood`.
async fn is_first_blood(
    db: &PgPool,
    cache: &mut HashMap<i16, HashSet<i32>>,
    service: &crate::models::Service,
    flag: &SubmittedFlag,
) -> Result<bool, sqlx::Error> {
    if service.num_payloads > 0
        && cache
            .get(&(service.id as i16))
            .is_some_and(|s| s.contains(&flag.payload))
    {
        return Ok(false);
    }
    if service.num_payloads == 0 && cache.contains_key(&(service.id as i16)) {
        return Ok(false);
    }
    let cnt: i64 = if service.num_payloads > 0 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM submitted_flags WHERE service_id=$1 AND is_firstblood>0 AND payload=$2 AND ts <= $3",
        )
        .bind(flag.service_id)
        .bind(flag.payload)
        .bind(flag.ts)
        .fetch_one(db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM submitted_flags WHERE service_id=$1 AND is_firstblood>0 AND ts <= $2",
        )
        .bind(flag.service_id)
        .bind(flag.ts)
        .fetch_one(db)
        .await?
    };
    Ok(cnt == 0)
}

/// `MultiFirstBloodAlgorithm._get_max_firstblood_level`.
async fn max_firstblood_level(
    db: &PgPool,
    service: &crate::models::Service,
    flag: &SubmittedFlag,
) -> Result<i16, sqlx::Error> {
    let max: Option<i16> = if service.num_payloads > 0 {
        sqlx::query_scalar(
            "SELECT MAX(is_firstblood) FROM submitted_flags \
             WHERE is_firstblood>0 AND service_id=$1 AND payload=$2 AND ts < $3",
        )
        .bind(flag.service_id)
        .bind(flag.payload)
        .bind(flag.ts)
        .fetch_one(db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT MAX(is_firstblood) FROM submitted_flags \
             WHERE is_firstblood>0 AND service_id=$1 AND ts < $2",
        )
        .bind(flag.service_id)
        .bind(flag.ts)
        .fetch_one(db)
        .await?
    };
    Ok(max.unwrap_or(0))
}

/// `MultiFirstBloodAlgorithm._get_previous_victims`.
async fn previous_victims(
    db: &PgPool,
    service: &crate::models::Service,
    flag: &SubmittedFlag,
) -> Result<Vec<i16>, sqlx::Error> {
    let rows: Vec<(i16,)> = if service.num_payloads > 0 {
        sqlx::query_as(
            "SELECT team_id FROM submitted_flags \
             WHERE submitted_by=$1 AND service_id=$2 AND payload=$3 AND ts < $4",
        )
        .bind(flag.submitted_by)
        .bind(flag.service_id)
        .bind(flag.payload)
        .bind(flag.ts)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as(
            "SELECT team_id FROM submitted_flags WHERE submitted_by=$1 AND service_id=$2 AND ts < $3",
        )
        .bind(flag.submitted_by)
        .bind(flag.service_id)
        .bind(flag.ts)
        .fetch_all(db)
        .await?
    };
    Ok(rows.into_iter().map(|(t,)| t).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_by_points_matches_python() {
        // stable desc sort; ties keep the previous rank; i only advances on points > 0
        let ranking = vec![(1, 10.0), (2, 5.0), (3, 5.0), (4, 0.0), (5, 0.0), (6, 1.0)];
        let out = order_by_points(ranking);
        assert_eq!(
            out,
            vec![
                (1, 1, 10.0),
                (2, 2, 5.0),
                (3, 2, 5.0),
                (6, 4, 1.0),
                (4, 5, 0.0),
                (5, 5, 0.0)
            ]
        );
        // all zero: everyone keeps rank 1
        let out = order_by_points(vec![(7, 0.0), (8, 0.0)]);
        assert_eq!(out, vec![(7, 1, 0.0), (8, 1, 0.0)]);
    }

    #[test]
    fn algorithm_config_strings_map() {
        assert_eq!(
            Algo::from_config("ctf:CtfScoreAlgorithm"),
            Ok(Algo::Ctf)
        );
        assert_eq!(
            Algo::from_config("ctf:CtfScoreAlgorithmWithHappyHour"),
            Ok(Algo::CtfHappyHour)
        );
        assert_eq!(
            Algo::from_config("playground:PlaygroundScoreAlgorithm"),
            Ok(Algo::Playground)
        );
        assert!(Algo::from_config("nope:Nope").is_err());
        assert_eq!(
            FirstBloodAlgo::from_config("firstblood:MultiFirstBloodAlgorithm"),
            Ok(FirstBloodAlgo::Multi)
        );
    }

    #[test]
    fn flag_set_dedups_on_flag_key_ignoring_submitter() {
        let mut fs = FlagSet::default();
        let mk = |team: i16, submitted_by: i16| SubmittedFlag {
            id: 1,
            submitted_by,
            team_id: team,
            service_id: 2,
            tick_issued: 3,
            payload: 4,
            tick_submitted: 5,
            is_firstblood: 0,
            ts: chrono::Utc::now(),
        };
        assert!(fs.is_new(&mk(9, 1)));
        assert!(!fs.is_new(&mk(9, 2))); // same flag, different attacker
        assert!(fs.is_new(&mk(10, 1))); // different victim
    }
}
