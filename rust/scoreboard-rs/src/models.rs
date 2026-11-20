//! Data shapes of the generated `api/scoreboard_*.json` files (models.ts).

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[allow(dead_code)] // data-contract fields; not all are rendered
pub struct Team {
    #[serde(default)]
    pub id: i64, // filled in from the object key
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vulnbox: String,
    #[serde(default)]
    pub aff: String,
    #[serde(default)]
    pub web: String,
    #[serde(default)]
    pub logo: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FirstBlood {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ts: f64, // the live generator writes fractional seconds
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub level: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Service {
    #[serde(default)]
    pub name: String,
    pub attackers: Value, // number or "?" (frozen)
    pub victims: Value,
    #[serde(default)]
    pub first_blood: Vec<FirstBlood>,
    #[serde(default)]
    pub flag_stores: i64,
    #[serde(default)]
    pub flag_stores_exploited: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ServiceResult {
    #[serde(default)]
    pub o: f64,
    #[serde(default)]
    pub d: f64,
    #[serde(default)]
    pub s: f64,
    #[serde(default)]
    pub do_: f64, // "do" — delta off
    #[serde(default)]
    pub dd: f64,
    #[serde(default)]
    pub ds: f64,
    #[serde(default)]
    pub st: i64,
    #[serde(default)]
    pub cap: i64,
    #[serde(default)]
    pub dst: i64,
    #[serde(default)]
    pub dcap: i64,
    #[serde(default)]
    pub c: String,
    #[serde(default)]
    pub dc: Vec<String>,
    #[serde(default)]
    pub m: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Rank {
    #[serde(default)]
    pub team_id: i64,
    #[serde(default)]
    pub rank: i64,
    #[serde(default)]
    pub points: f64,
    #[serde(default)]
    pub o: f64,
    #[serde(default)]
    pub d: f64,
    #[serde(default)]
    pub s: f64,
    #[serde(default)]
    pub do_: f64,
    #[serde(default)]
    pub dd: f64,
    #[serde(default)]
    pub ds: f64,
    #[serde(default)]
    pub services: Vec<ServiceResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RoundInformation {
    #[serde(default)]
    pub tick: i64,
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub scoreboard: Vec<Rank>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TeamHistoryInformation {
    #[serde(default)]
    pub services: Vec<String>, // the generator writes service names here
    #[serde(default)]
    pub points: Vec<Vec<f64>>, // [service][tick]
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ServiceStat {
    pub a: Value, // number or "?"
    pub v: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ServiceStatsInformation {
    #[serde(default)]
    pub services: Vec<String>, // the generator writes service names here
    #[serde(default)]
    pub stats: Vec<Vec<ServiceStat>>, // [service][tick]
}

/// `CurrentStateJson` — `state` is an IntEnum (1/2/3) in the Python generator
/// but a string ("STOPPED"/"SUSPENDED"/"RUNNING") on the live deployment:
/// accept both.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CurrentState {
    #[serde(default)]
    pub current_tick: i64,
    #[serde(default = "default_state", deserialize_with = "deserialize_state")]
    pub state: String,
    #[serde(default)]
    pub current_tick_until: i64,
    #[serde(default)]
    pub scoreboard_tick: i64,
    #[serde(default)]
    pub banned_teams: Vec<i64>,
    #[serde(default)]
    pub frozen: bool,
}

fn default_state() -> String {
    "STOPPED".into()
}

fn deserialize_state<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        N(i64),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::S(s) => s,
        // controlserver.timer.TimerState
        Raw::N(2) => STATE_SUSPENDED.into(),
        Raw::N(3) => STATE_RUNNING.into(),
        Raw::N(_) => STATE_STOPPED.into(),
    })
}

impl Default for CurrentState {
    fn default() -> Self {
        CurrentState {
            current_tick: -1,
            state: "STOPPED".into(),
            current_tick_until: 0,
            scoreboard_tick: -1,
            banned_teams: vec![],
            frozen: false,
        }
    }
}

pub const STATE_STOPPED: &str = "STOPPED";
pub const STATE_SUSPENDED: &str = "SUSPENDED";
pub const STATE_RUNNING: &str = "RUNNING";

/// StatusUtils.
pub fn status_to_class(status: &str) -> &'static str {
    match status {
        "SUCCESS" => "label-success",
        "FLAGMISSING" | "MUMBLE" => "label-warning",
        "OFFLINE" | "TIMEOUT" | "CRASH" => "label-danger",
        "RECOVERING" => "label-info",
        _ => "label-default",
    }
}

pub fn status_to_tooltip(status: &str) -> &str {
    match status {
        "SUCCESS" => "Service online",
        "FLAGMISSING" => "No flag found",
        "MUMBLE" => "Mumble",
        "OFFLINE" | "TIMEOUT" => "Service unreachable",
        "RECOVERING" => "Service online, but flags from previous ticks not found",
        "CRASH" => "Checker crashed",
        "REVOKED" | "PENDING" => "Not checked",
        other => other,
    }
}

pub fn status_to_text(status: &str) -> &str {
    match status {
        "SUCCESS" => "up",
        "FLAGMISSING" => "flag",
        "MUMBLE" => "mumble",
        "OFFLINE" | "TIMEOUT" => "down",
        "RECOVERING" => "rec",
        _ => "-",
    }
}
