//! Port of `backend.service.ts` + `ui.service.ts` — data polling, caches and
//! the derived notification events, as a leptos context.

use crate::models::*;
use gloo_timers::callback::Timeout;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use std::collections::{HashMap, HashSet};

/// ("firstblood", service, team) or ("final", "", "")
pub type Event = (&'static str, String, String);

#[derive(Clone, Copy)]
pub struct Store {
    pub teams: RwSignal<HashMap<String, Team>>,
    pub state: RwSignal<CurrentState>,
    /// newestScoreboardTick BehaviorSubject
    pub newest_scoreboard_tick: RwSignal<i64>,
    /// (client time) - (server time) in seconds, from the Date header
    pub delta_client_to_server: RwSignal<i64>,
    /// eventNotifications Subject, drained by the overlay
    pub events: RwSignal<Vec<Event>>,
    /// set when the game switches RUNNING → STOPPED
    pub final_scoreboard_tick: RwSignal<Option<i64>>,
    rankings: RwSignal<HashMap<i64, RoundInformation>>,
    ranking_inflight: RwSignal<HashSet<i64>>,
    team_point_history: RwSignal<HashMap<i64, Vec<Vec<f64>>>>,
    history_inflight: RwSignal<HashSet<i64>>,
}

async fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    gloo_net::http::Request::get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GET {url}: bad JSON: {e}"))
}

/// retryWithBackoff: `max_retry` attempts, `delay_ms + n * backoff_ms` apart.
async fn fetch_retry_json(
    url: &str,
    delay_ms: u32,
    max_retry: u32,
    backoff_ms: u32,
) -> Result<serde_json::Value, String> {
    let mut attempt = 0u32;
    loop {
        match fetch_json(url).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                attempt += 1;
                if attempt > max_retry {
                    return Err(e);
                }
                TimeoutFuture::new(delay_ms + (attempt - 1) * backoff_ms).await;
            }
        }
    }
}

impl Store {
    pub fn new() -> Store {
        let store = Store {
            teams: RwSignal::new(HashMap::new()),
            state: RwSignal::new(CurrentState::default()),
            newest_scoreboard_tick: RwSignal::new(-1),
            delta_client_to_server: RwSignal::new(0),
            events: RwSignal::new(vec![]),
            final_scoreboard_tick: RwSignal::new(None),
            rankings: RwSignal::new(HashMap::new()),
            ranking_inflight: RwSignal::new(HashSet::new()),
            team_point_history: RwSignal::new(HashMap::new()),
            history_inflight: RwSignal::new(HashSet::new()),
        };
        leptos::task::spawn_local(store.load_teams_loop());
        leptos::task::spawn_local(store.update_current_state_loop());
        store
    }

    async fn load_teams_loop(self) {
        // retryWithBackoff(2000, 20); on final failure, retry every 5s forever
        loop {
            match fetch_retry_json("api/scoreboard_teams.json", 2000, 20, 500).await {
                Ok(v) => {
                    let mut teams = HashMap::new();
                    if let Some(obj) = v.as_object() {
                        for (id, t) in obj {
                            let mut team: Team =
                                serde_json::from_value(t.clone()).unwrap_or_default();
                            team.id = id.parse().unwrap_or(0);
                            teams.insert(id.clone(), team);
                        }
                    }
                    self.teams.set(teams);
                    return;
                }
                Err(_) => TimeoutFuture::new(5000).await,
            }
        }
    }

    /// `updateCurrentState`: adaptive poll of `scoreboard_current.json`.
    async fn update_current_state_loop(self) {
        let mut last_date_header: Option<String> = None;
        let mut old_state = CurrentState::default();
        loop {
            let fetched = async {
                let resp = gloo_net::http::Request::get("api/scoreboard_current.json")
                    .send()
                    .await
                    .map_err(|e| format!("current: {e}"))?;
                let date = resp.headers().get("Date");
                let body: CurrentState = resp
                    .json()
                    .await
                    .map_err(|e| format!("current: bad JSON: {e}"))?;
                Ok::<_, String>((body, date))
            };
            match fetched.await {
                Ok((new, date_header)) => {
                    // Date-header delta (only when not served from browser cache)
                    if let Some(dh) = &date_header {
                        if Some(dh.as_str()) != last_date_header.as_deref() {
                            let now_ms = js_sys::Date::now() as i64;
                            let server_ms = js_sys::Date::parse(dh) as i64;
                            if server_ms > 0 {
                                self.delta_client_to_server.set((now_ms - server_ms) / 1000);
                            }
                        }
                    }
                    last_date_header = date_header;

                    // next wait time (backend.service.ts logic)
                    let now = (js_sys::Date::now() / 1000.0) as i64;
                    let mut wait_time = new.current_tick_until - now;
                    if wait_time < 2 || new.scoreboard_tick < new.current_tick - 1 {
                        wait_time = if new.state == STATE_RUNNING {
                            1
                        } else if new.state == STATE_SUSPENDED {
                            5
                        } else {
                            10
                        };
                    } else if wait_time > 10 {
                        wait_time = 10;
                    }

                    // trigger events
                    if new.state == STATE_STOPPED && old_state.state == STATE_RUNNING {
                        self.final_scoreboard_tick.set(Some(new.current_tick));
                    }
                    if new.scoreboard_tick != old_state.scoreboard_tick {
                        self.newest_scoreboard_tick.set(new.scoreboard_tick);
                    }
                    old_state = new.clone();
                    self.state.set(new);

                    TimeoutFuture::new((wait_time * 1000) as u32).await;
                }
                Err(_) => TimeoutFuture::new(5000).await,
            }
        }
    }

    /// `getRanking` as a tracked read: cached per tick, otherwise fetches.
    /// New current-tick data emits firstblood / final events and appends to
    /// the team history caches.
    pub fn ranking(&self, tick: i64) -> Option<RoundInformation> {
        let map = self.rankings.get(); // tracked
        if let Some(info) = map.get(&tick) {
            return Some(info.clone());
        }
        if !self.ranking_inflight.get_untracked().contains(&tick) {
            self.ranking_inflight.update(|s| { s.insert(tick); });
            let this = *self;
            leptos::task::spawn_local(async move {
                // retryWithBackoff(1500, 10)
                let url = format!("api/scoreboard_round_{tick}.json");
                match fetch_retry_json(&url, 1500, 10, 1500).await {
                    Ok(v) => match serde_json::from_value::<RoundInformation>(v) {
                        Ok(info) => this.store_ranking(info),
                        Err(_) => this.ranking_inflight.update(|s| { s.remove(&tick); }),
                    },
                    Err(_) => this.ranking_inflight.update(|s| { s.remove(&tick); }),
                }
            });
        }
        None
    }

    fn store_ranking(self, info: RoundInformation) {
        let tick = info.tick;
        let st = self.state.get_untracked();
        let mut rankings = self.rankings.get_untracked();

        if tick == st.scoreboard_tick {
            if let Some(old) = rankings.get(&(tick - 1)) {
                // firstbloods: confirmed count per service grew?
                for (i, service) in info.services.iter().enumerate() {
                    let old_count = old
                        .services
                        .get(i)
                        .map(|s| s.first_blood.iter().filter(|fb| fb.confirmed).count())
                        .unwrap_or(0);
                    for fb in service.first_blood.iter().skip(old_count) {
                        if fb.confirmed {
                            self.events
                                .update(|e| e.push(("firstblood", service.name.clone(), fb.name.clone())));
                        }
                    }
                }
                // final greeting
                if self.final_scoreboard_tick.get_untracked() == Some(tick)
                    && st.state == STATE_STOPPED
                {
                    self.events.update(|e| e.push(("final", String::new(), String::new())));
                }
                // periodic reload (upstream quirk, kept)
                if tick % 60 == 7 {
                    let _ = Timeout::new(
                        5000 + (js_sys::Math::random() * 50_000.0) as u32,
                        || {
                            if let Some(w) = web_sys::window() {
                                let _ = w.location().reload();
                            }
                        },
                    );
                }
                // evict caches if the services changed
                let old_names: Vec<&str> = old.services.iter().map(|s| s.name.as_str()).collect();
                let new_names: Vec<&str> = info.services.iter().map(|s| s.name.as_str()).collect();
                if old_names != new_names {
                    self.team_point_history.set(HashMap::new());
                }
                // append to team history caches
                let mut hist = self.team_point_history.get_untracked();
                for (team_id, points) in hist.iter_mut() {
                    if points.first().map(|p| p.len()) == Some(tick as usize) {
                        if let Some(rank) = info.scoreboard.iter().find(|r| r.team_id == *team_id) {
                            for (i, svc) in rank.services.iter().enumerate() {
                                if let Some(p) = points.get_mut(i) {
                                    p.push(svc.o + svc.d + svc.s);
                                }
                            }
                        }
                    }
                }
                self.team_point_history.set(hist);
            }
        }
        rankings.insert(tick, info);
        self.rankings.set(rankings);
    }

    /// `getTeamPointHistory`: [service][tick] point sums of one team, as a
    /// tracked read that fetches on miss. Like the original, the completeness
    /// check only skips the fetch — incomplete data is served as-is.
    pub fn team_history(&self, team_id: i64) -> Option<Vec<Vec<f64>>> {
        let hist = self.team_point_history.get(); // tracked
        let cached = hist.get(&team_id).cloned();
        // the history file holds one point per finished tick (0..scoreboard_tick),
        // i.e. exactly `current_tick` entries — comparing to current_tick + 1
        // never passes and made every reactive re-read refetch forever
        let complete = cached
            .as_ref()
            .and_then(|p| p.first())
            .map(|p| p.len())
            .unwrap_or(0)
            >= self.state.get_untracked().current_tick.max(0) as usize;
        if !complete && !self.history_inflight.get_untracked().contains(&team_id) {
            self.history_inflight.update(|s| { s.insert(team_id); });
            let this = *self;
            leptos::task::spawn_local(async move {
                // retryWithBackoff(1500, 3)
                let url = format!("api/scoreboard_team_{team_id}.json");
                match fetch_retry_json(&url, 1500, 3, 1500).await {
                    Ok(v) => {
                        if let Ok(info) = serde_json::from_value::<TeamHistoryInformation>(v) {
                            if !info.points.is_empty() {
                                this.team_point_history
                                    .update(|h| { h.insert(team_id, info.points); });
                                this.history_inflight.update(|s| { s.remove(&team_id); });
                                return;
                            }
                        }
                        this.history_inflight.update(|s| { s.remove(&team_id); });
                    }
                    Err(_) => this.history_inflight.update(|s| { s.remove(&team_id); }),
                }
            });
        }
        cached
    }

    /// `getTeamPointHistorySimple`: [tick] total points of one team.
    pub fn team_history_simple(&self, team_id: i64) -> Vec<f64> {
        let Some(points) = self.team_history(team_id) else {
            return vec![];
        };
        let Some(first) = points.first() else { return vec![] };
        let mut result = vec![0.0; first.len()];
        for series in &points {
            for (i, p) in series.iter().enumerate() {
                result[i] += p;
            }
        }
        result
    }

    /// `getServiceStatHistory`.
    pub async fn service_stat_history(&self) -> Option<ServiceStatsInformation> {
        let v = fetch_retry_json("api/scoreboard_service_stats.json", 1500, 3, 1500)
            .await
            .ok()?;
        serde_json::from_value(v).ok()
    }

    pub fn banned(&self, team_id: i64) -> bool {
        self.state.get_untracked().banned_teams.contains(&team_id)
    }

    pub fn team(&self, team_id: i64) -> Option<Team> {
        self.teams
            .get_untracked()
            .values()
            .find(|t| t.id == team_id)
            .cloned()
    }
}
