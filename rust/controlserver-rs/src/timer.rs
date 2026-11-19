//! Port of `controlserver/timer.py` — master/slave CTF clock over redis keys
//! `timing:*` + `round:{tick}:{start,end,time}`. One struct with a `master`
//! flag for the two modes (Python had CTFTimer / CTFTimerSlave).
//!
//! Concurrency: the redis pubsub listener and the once-per-second clock both
//! need the timer, so `spawn` hands out `Arc<tokio::sync::Mutex<Ctftimer>>`.

use chrono::{DateTime, Timelike, Utc};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use saarctf_commons_rs::config::Config;

/// CTFState IntEnum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtfState {
    Stopped,
    Suspended,
    Running,
}

impl CtfState {
    pub fn name(&self) -> &'static str {
        match self {
            CtfState::Stopped => "STOPPED",
            CtfState::Suspended => "SUSPENDED",
            CtfState::Running => "RUNNING",
        }
    }
    pub fn from_name(s: &str) -> Option<CtfState> {
        match s {
            "STOPPED" => Some(CtfState::Stopped),
            "SUSPENDED" => Some(CtfState::Suspended),
            "RUNNING" => Some(CtfState::Running),
            _ => None,
        }
    }
}

/// `to_int` — redis value (or the literal "None") -> Option<i64>.
pub fn to_int(x: Option<&str>) -> Option<i64> {
    match x {
        None | Some("") | Some("None") => None,
        Some(s) => s.parse().ok(),
    }
}

/// `redis_set_and_publish` — None becomes the literal "None".
pub async fn redis_set_and_publish(
    redis: &mut ConnectionManager,
    key: &str,
    value: Option<&str>,
) -> Result<(), redis::RedisError> {
    let value = value.unwrap_or("None");
    let _: () = redis::cmd("SET")
        .arg(key)
        .arg(value)
        .query_async(redis)
        .await?;
    let _: () = redis::cmd("PUBLISH")
        .arg(key)
        .arg(value)
        .query_async(redis)
        .await?;
    Ok(())
}

fn to_redis_val(v: Option<i64>) -> String {
    v.map(|i| i.to_string()).unwrap_or_else(|| "None".into())
}

pub const TIMING_KEYS: [&str; 9] = [
    "timing:state",
    "timing:desiredState",
    "timing:currentRound",
    "timing:roundStart",
    "timing:roundEnd",
    "timing:roundTime",
    "timing:stopAfterRound",
    "timing:startAt",
    "timing:openVulnboxAccessAt",
];

/// Listener interface (`CTFEvents` in events.py); all methods default no-op.
#[async_trait::async_trait]
pub trait CtfEvents: Send + Sync {
    async fn on_start_tick(&self, _tick: i64, _ts: DateTime<Utc>) {}
    async fn on_end_tick(&self, _tick: i64, _ts: DateTime<Utc>) {}
    async fn on_start_ctf(&self) {}
    async fn on_suspend_ctf(&self) {}
    async fn on_end_ctf(&self) {}
    async fn on_update_times(&self) {}
    async fn on_open_vulnbox_access(&self) {}
}

/// `ConsoleCTFEvents`.
pub struct ConsoleCtfEvents;
#[async_trait::async_trait]
impl CtfEvents for ConsoleCtfEvents {
    async fn on_start_tick(&self, tick: i64, ts: DateTime<Utc>) {
        println!(
            "{} | Start of tick {}",
            ts.format("%d.%m.%Y %H:%M:%S"),
            tick
        );
    }
    async fn on_end_tick(&self, tick: i64, ts: DateTime<Utc>) {
        println!("{} | End of tick {}", ts.format("%d.%m.%Y %H:%M:%S"), tick);
    }
    async fn on_start_ctf(&self) {
        println!(
            "{} | CTF initially started",
            Utc::now().format("%d.%m.%Y %H:%M:%S")
        );
    }
    async fn on_suspend_ctf(&self) {
        println!("{} | CTF suspended", Utc::now().format("%d.%m.%Y %H:%M:%S"));
    }
    async fn on_end_ctf(&self) {
        println!("{} | CTF is over!", Utc::now().format("%d.%m.%Y %H:%M:%S"));
    }
    async fn on_open_vulnbox_access(&self) {
        println!(
            "{} | Vulnbox access opened",
            Utc::now().format("%d.%m.%Y %H:%M:%S")
        );
    }
}

pub type SharedTimer = std::sync::Arc<tokio::sync::Mutex<Ctftimer>>;

/// The nine timing fields (CTFTimerBase's state).
#[derive(Debug, Clone)]
pub struct TimingState {
    pub state: CtfState,
    pub desired_state: CtfState,
    pub current_tick: i64,
    pub tick_start: Option<i64>,
    pub tick_end: Option<i64>,
    pub tick_time: i64,
    pub stop_after_tick: Option<i64>,
    pub start_at: Option<i64>,
    pub open_vulnbox_access_at: Option<i64>,
}

/// CTFTimerBase + CTFTimer + CTFTimerSlave merged.
pub struct Ctftimer {
    pub initialized: bool,
    pub master: bool,
    pub t: TimingState,
    pub listener: Vec<Box<dyn CtfEvents>>,
    redis: ConnectionManager,
    redis_url: String,
}

impl std::ops::Deref for Ctftimer {
    type Target = TimingState;
    fn deref(&self) -> &TimingState {
        &self.t
    }
}
impl std::ops::DerefMut for Ctftimer {
    fn deref_mut(&mut self) -> &mut TimingState {
        &mut self.t
    }
}

/// `init_cp_timer` + `bind_to_redis` + `run_master_timer` in one call:
/// builds the timer, restores state from redis, spawns the pubsub listener
/// and (master only) the once-per-second clock task.
pub async fn spawn_timer(
    cfg: &Config,
    master: bool,
    listener: Vec<Box<dyn CtfEvents>>,
) -> Result<SharedTimer, Box<dyn std::error::Error + Send + Sync>> {
    let mut timer = Ctftimer {
        initialized: false,
        master,
        t: TimingState {
            state: CtfState::Stopped,
            desired_state: CtfState::Stopped,
            current_tick: 0,
            tick_start: None,
            tick_end: None,
            tick_time: cfg.tick_duration_default,
            stop_after_tick: None,
            start_at: None,
            open_vulnbox_access_at: None,
        },
        listener,
        redis: saarctf_commons_rs::redis::get_redis_connection(&cfg.redis).await?,
        redis_url: saarctf_commons_rs::redis::redis_url(&cfg.redis)?,
    };
    timer.init_from_redis().await?;
    let redis_url = timer.redis_url.clone();
    let shared: SharedTimer = std::sync::Arc::new(tokio::sync::Mutex::new(timer));
    // pubsub listener (own connection; resubscribes on error, like redis-py)
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            loop {
                match listen_loop(&shared, &redis_url).await {
                    Ok(()) => return,
                    Err(e) => {
                        tracing::warn!("timer redis listener: {e}; reconnecting");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });
    }
    if master {
        let shared = shared.clone();
        tokio::spawn(async move {
            tracing::info!("Timer active...");
            loop {
                {
                    let mut timer = shared.lock().await;
                    if let Err(e) = timer.check_time().await {
                        tracing::error!("timer: check_time failed: {e}");
                    }
                }
                let now = now_secs() as f64;
                tokio::time::sleep(std::time::Duration::from_secs_f64(1.0 - (now % 1.0))).await;
            }
        });
    }
    Ok(shared)
}

async fn listen_loop(
    timer: &SharedTimer,
    redis_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = redis::Client::open(redis_url.to_string())?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(&TIMING_KEYS).await?;
    let mut stream = pubsub.into_on_message();
    while let Some(msg) = futures_util::StreamExt::next(&mut stream).await {
        let channel = msg.get_channel_name().to_string();
        let payload: Option<String> = msg.get_payload().ok();
        let mut t = timer.lock().await;
        t.apply_message(&channel, payload.as_deref());
    }
    Ok(())
}

impl Ctftimer {}

impl TimingState {
    /// `__listen_for_redis_events` — apply one pubsub message to local state.
    pub fn apply_message(&mut self, channel: &str, payload: Option<&str>) {
        match channel {
            "timing:state" => {
                if let Some(s) = payload.and_then(CtfState::from_name) {
                    self.state = s;
                }
            }
            "timing:desiredState" => {
                if let Some(s) = payload.and_then(CtfState::from_name) {
                    self.desired_state = s;
                }
            }
            "timing:currentRound" => {
                self.current_tick = payload
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(self.current_tick);
            }
            "timing:roundStart" => self.tick_start = to_int(payload),
            "timing:roundEnd" => self.tick_end = to_int(payload),
            "timing:roundTime" => {
                if let Some(t) = to_int(payload).filter(|t| *t != 0) {
                    self.tick_time = t;
                }
            }
            "timing:stopAfterRound" => self.stop_after_tick = to_int(payload),
            "timing:startAt" => self.start_at = to_int(payload),
            "timing:openVulnboxAccessAt" => self.open_vulnbox_access_at = to_int(payload),
            _ => {}
        }
    }
}

impl Ctftimer {
    async fn init_from_redis(&mut self) -> Result<(), redis::RedisError> {
        let mut pipe = redis::pipe();
        for k in TIMING_KEYS {
            pipe.get(k);
        }
        let keys: Vec<Option<String>> = pipe.query_async(&mut self.redis).await?;
        if keys[0].is_none() {
            return Ok(());
        }
        self.state = CtfState::from_name(keys[0].as_deref().unwrap_or("")).unwrap_or(self.state);
        self.desired_state =
            CtfState::from_name(keys[1].as_deref().unwrap_or("")).unwrap_or(self.desired_state);
        self.current_tick = to_int(keys[2].as_deref()).unwrap_or(0);
        self.tick_start = to_int(keys[3].as_deref());
        self.tick_end = to_int(keys[4].as_deref());
        if let Some(t) = to_int(keys[5].as_deref()) {
            if t != 0 {
                self.tick_time = t;
            }
        }
        self.stop_after_tick = to_int(keys[6].as_deref());
        self.start_at = to_int(keys[7].as_deref());
        self.open_vulnbox_access_at = to_int(keys[8].as_deref());
        Ok(())
    }

    // -- command API (works for master and slave) --

    pub async fn start_ctf(&mut self) -> Result<(), redis::RedisError> {
        if !self.master {
            self.desired_state = CtfState::Running;
            return redis_set_and_publish(&mut self.redis, "timing:desiredState", Some("RUNNING"))
                .await;
        }
        self.desired_state = CtfState::Running;
        if self.state != CtfState::Running {
            self.current_tick += 1;
            self.tick_start = Some(now_secs());
            self.tick_end = Some(self.tick_start.unwrap() + self.tick_time);
            let old_state = self.state;
            self.state = CtfState::Running;
            if old_state == CtfState::Stopped {
                self.on_start_ctf().await;
            }
            self.on_start_tick(self.current_tick).await;
            self.on_update_times().await?;
        }
        Ok(())
    }

    pub async fn suspend_ctf_after_tick(&mut self) -> Result<(), redis::RedisError> {
        self.desired_state = CtfState::Suspended;
        self.publish_update_times().await
    }

    pub async fn stop_ctf_after_tick(&mut self) -> Result<(), redis::RedisError> {
        self.desired_state = CtfState::Stopped;
        self.publish_update_times().await
    }

    /// setters: tick_time / stop_after_tick / start_at / open_vulnbox_access_at
    pub async fn set_tick_time(&mut self, t: i64) -> Result<(), redis::RedisError> {
        if self.tick_time == t {
            return Ok(());
        }
        self.tick_time = t;
        if let Some(start) = self.tick_start {
            self.tick_end = Some(start + t);
        }
        if self.master {
            self.on_update_times().await
        } else {
            if let Some(end) = self.tick_end {
                redis_set_and_publish(&mut self.redis, "timing:roundEnd", Some(&end.to_string()))
                    .await?;
            }
            redis_set_and_publish(&mut self.redis, "timing:roundTime", Some(&t.to_string())).await
        }
    }

    pub async fn set_stop_after_tick(&mut self, v: Option<i64>) -> Result<(), redis::RedisError> {
        if self.stop_after_tick == v {
            return Ok(());
        }
        self.stop_after_tick = v;
        if self.master {
            self.on_update_times().await
        } else {
            redis_set_and_publish(
                &mut self.redis,
                "timing:stopAfterRound",
                Some(&to_redis_val(v)),
            )
            .await
        }
    }

    pub async fn set_start_at(&mut self, v: Option<i64>) -> Result<(), redis::RedisError> {
        if self.start_at == v {
            return Ok(());
        }
        self.start_at = v;
        if self.master {
            self.on_update_times().await
        } else {
            redis_set_and_publish(&mut self.redis, "timing:startAt", Some(&to_redis_val(v))).await
        }
    }

    pub async fn set_open_vulnbox_access_at(
        &mut self,
        v: Option<i64>,
    ) -> Result<(), redis::RedisError> {
        if self.open_vulnbox_access_at == v {
            return Ok(());
        }
        self.open_vulnbox_access_at = v;
        if self.master {
            self.on_update_times().await
        } else {
            redis_set_and_publish(
                &mut self.redis,
                "timing:openVulnboxAccessAt",
                Some(&to_redis_val(v)),
            )
            .await
        }
    }

    // -- master clock --

    /// `check_time` — called once per second.
    pub async fn check_time(&mut self) -> Result<(), redis::RedisError> {
        if !self.master {
            return Ok(());
        }
        let t = now_secs();
        if self.state == CtfState::Running && self.tick_end.is_some() && t >= self.tick_end.unwrap()
        {
            self.on_end_tick(self.current_tick).await;
            if self.desired_state == CtfState::Running {
                self.current_tick += 1;
                self.tick_start = Some(if t > self.tick_end.unwrap() + 1 {
                    t
                } else {
                    self.tick_end.unwrap()
                });
                self.tick_end = Some(self.tick_start.unwrap() + self.tick_time);
                self.on_start_tick(self.current_tick).await;
            } else {
                self.state = self.desired_state;
                if self.state == CtfState::Suspended {
                    self.on_suspend_ctf().await;
                } else {
                    self.on_end_ctf().await;
                }
            }
            self.on_update_times().await
        } else if self.state != CtfState::Running && self.desired_state == CtfState::Running {
            self.start_ctf().await
        } else if self.state != CtfState::Running
            && self.start_at.is_some()
            && t >= self.start_at.unwrap()
            && t <= self.start_at.unwrap() + 4
        {
            self.start_at = None;
            self.start_ctf().await
        } else if self.state != CtfState::Running
            && self.open_vulnbox_access_at.is_some()
            && t >= self.open_vulnbox_access_at.unwrap()
            && t <= self.open_vulnbox_access_at.unwrap() + 4
        {
            self.open_vulnbox_access_at = None;
            self.on_open_vulnbox_access().await;
            self.on_update_times().await
        } else {
            Ok(())
        }
    }

    // -- event fanout --

    /// `on_start_tick` — also applies the stop-after-tick conversion
    /// (python does this inside on_start_tick).
    async fn on_start_tick(&mut self, tick: i64) {
        let dt = Utc::now().trunc_subsecs(0);
        if self.stop_after_tick == Some(tick) {
            self.desired_state = CtfState::Stopped;
            self.stop_after_tick = None;
        }
        for l in &self.listener {
            l.on_start_tick(tick, dt).await;
        }
    }

    async fn on_end_tick(&self, tick: i64) {
        let dt = Utc::now().trunc_subsecs(0);
        for l in &self.listener {
            l.on_end_tick(tick, dt).await;
        }
    }

    async fn on_start_ctf(&self) {
        for l in &self.listener {
            l.on_start_ctf().await;
        }
    }

    async fn on_suspend_ctf(&self) {
        for l in &self.listener {
            l.on_suspend_ctf().await;
        }
    }

    async fn on_end_ctf(&self) {
        for l in &self.listener {
            l.on_end_ctf().await;
        }
    }

    async fn on_open_vulnbox_access(&self) {
        for l in &self.listener {
            l.on_open_vulnbox_access().await;
        }
    }

    /// `on_update_times` (master): write + publish all nine keys, round:*,
    /// and fan out on_update_times to the listeners.
    pub async fn on_update_times(&mut self) -> Result<(), redis::RedisError> {
        self.publish_update_times().await?;
        self.update_tick_times().await?;
        for l in &self.listener {
            l.on_update_times().await;
        }
        Ok(())
    }

    /// The redis writes of on_update_times; master only (slave publishes its
    /// keys directly from the setters).
    async fn publish_update_times(&mut self) -> Result<(), redis::RedisError> {
        if !self.master {
            return Ok(());
        }
        let vals = [
            ("timing:state", self.t.state.name().to_string()),
            (
                "timing:desiredState",
                self.t.desired_state.name().to_string(),
            ),
            ("timing:currentRound", self.t.current_tick.to_string()),
            ("timing:roundStart", to_redis_val(self.t.tick_start)),
            ("timing:roundEnd", to_redis_val(self.t.tick_end)),
            ("timing:roundTime", self.t.tick_time.to_string()),
            (
                "timing:stopAfterRound",
                to_redis_val(self.t.stop_after_tick),
            ),
            ("timing:startAt", to_redis_val(self.t.start_at)),
            (
                "timing:openVulnboxAccessAt",
                to_redis_val(self.t.open_vulnbox_access_at),
            ),
        ];
        for (k, v) in &vals {
            redis_set_and_publish(&mut self.redis, k, Some(v)).await?;
        }
        Ok(())
    }

    /// `update_tick_times` — round:{tick}:start/end/time when running.
    pub async fn update_tick_times(&mut self) -> Result<(), redis::RedisError> {
        if self.state == CtfState::Running {
            let tick = self.current_tick;
            let _: () = self
                .redis
                .set(format!("round:{tick}:start"), to_redis_val(self.tick_start))
                .await?;
            let _: () = self
                .redis
                .set(format!("round:{tick}:end"), to_redis_val(self.tick_end))
                .await?;
            let _: () = self
                .redis
                .set(format!("round:{tick}:time"), self.tick_time.to_string())
                .await?;
        }
        Ok(())
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

trait TruncSubsecs {
    fn trunc_subsecs(self, digits: u32) -> DateTime<Utc>;
}
impl TruncSubsecs for DateTime<Utc> {
    fn trunc_subsecs(self, digits: u32) -> DateTime<Utc> {
        let step = 1_000_000_000u32 / 10u32.pow(digits);
        self.with_nanosecond(self.nanosecond() / step * step)
            .unwrap_or(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_int_matches_python() {
        assert_eq!(to_int(Some("5")), Some(5));
        assert_eq!(to_int(Some("None")), None);
        assert_eq!(to_int(Some("")), None);
        assert_eq!(to_int(None), None);
        assert_eq!(to_int(Some("-3")), Some(-3));
    }

    #[test]
    fn state_roundtrip() {
        for s in [CtfState::Stopped, CtfState::Suspended, CtfState::Running] {
            assert_eq!(CtfState::from_name(s.name()), Some(s));
        }
        assert_eq!(CtfState::from_name("other"), None);
    }

    #[test]
    fn apply_message_updates_state() {
        let mut t = TimingState {
            state: CtfState::Stopped,
            desired_state: CtfState::Stopped,
            current_tick: 0,
            tick_start: None,
            tick_end: None,
            tick_time: 60,
            stop_after_tick: None,
            start_at: None,
            open_vulnbox_access_at: None,
        };
        t.apply_message("timing:desiredState", Some("RUNNING"));
        assert_eq!(t.desired_state, CtfState::Running);
        t.apply_message("timing:currentRound", Some("7"));
        assert_eq!(t.current_tick, 7);
        t.apply_message("timing:roundTime", Some("120"));
        assert_eq!(t.tick_time, 120);
        t.apply_message("timing:roundTime", Some("0")); // 0 keeps old value
        assert_eq!(t.tick_time, 120);
        t.apply_message("timing:roundStart", Some("None"));
        assert_eq!(t.tick_start, None);
        t.apply_message("timing:unknown", Some("1")); // ignored
    }
}
