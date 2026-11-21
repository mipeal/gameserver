//! Port of `controlserver/flag_id_file.py` — writes `attack.json` flag-ID maps
//! for the scoreboard and the checkers.

use crate::models::{Service, Team};
use crate::timer::TimingState;
use redis::aio::ConnectionManager;
use ctf_commons_rs::config::Config;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub const FLAG_ID_KEY: &str = "flag_ids";

/// `FlagIDFileGenerator.generate_and_save`.
pub async fn generate_and_save(
    cfg: &Config,
    redis: &mut ConnectionManager,
    teams: &[Team],
    services: &[Service],
    timing: &TimingState,
    tick: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = generate(cfg, redis, teams, services, timing, tick).await?;
    let text = serde_json::to_string(&data)?;
    for path in [
        (!cfg.scoreboard_path.is_empty()).then_some(cfg.scoreboard_path.as_str()),
        cfg.scoreboard_path_internal.as_deref(),
    ] {
        let Some(path) = path.map(Path::new) else {
            continue;
        };
        let file_path = path.join("api").join("attack.json");
        let alternative_path = path.join("attack.json");
        if std::fs::create_dir_all(file_path.parent().unwrap_or(path)).is_err() {
            continue;
        }
        if std::fs::write(&file_path, &text).is_err() {
            continue;
        }
        if !alternative_path.exists() {
            // Python: symlink to file_path.relative_to(alternative_path.parent)
            if let Some(rel) = file_path
                .strip_prefix(alternative_path.parent().unwrap_or(path))
                .ok()
                .and_then(|p| p.to_str())
            {
                let _ = std::os::unix::fs::symlink(rel, &alternative_path);
            }
        }
    }
    Ok(())
}

/// `FlagIDFileGenerator.generate`.
pub async fn generate(
    cfg: &Config,
    redis: &mut ConnectionManager,
    teams: &[Team],
    services: &[Service],
    timing: &TimingState,
    tick: i64,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut flag_ids_map = serde_json::Map::new();
    for service in services {
        if service.flag_ids.as_deref().is_some_and(|s| !s.is_empty()) {
            flag_ids_map.insert(
                service.name.clone(),
                get_service_flag_ids(cfg, redis, service, teams, tick).await?,
            );
        }
    }
    Ok(json!({
        "flag_regex": gamelib_rs::port::get_flag_regex_pattern(),
        "teams": teams.iter().map(|team| json!({
            "id": team.id,
            "name": team.name,
            "affiliation": team.affiliation,
            "logo": team.logo,
            "ip": cfg.network.team_id_to_vulnbox_ip(team.id as i64),
            "online": team.vpn_connected || team.vpn2_connected || team.wg_vulnbox_connected,
        })).collect::<Vec<_>>(),
        FLAG_ID_KEY: Value::Object(flag_ids_map),
        "current_tick": timing.current_tick,
        "current_tick_start": timing.tick_start,
        "current_tick_until": timing.tick_end,
    }))
}

/// `FlagIDFileGenerator.get_service_flag_ids` — flag IDs for the ticks
/// [min_tick, latest_tick) per team vulnbox IP.
pub async fn get_service_flag_ids(
    cfg: &Config,
    redis: &mut ConnectionManager,
    service: &Service,
    teams: &[Team],
    latest_tick: i64,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut data = serde_json::Map::new();
    let flag_id_types: Vec<&str> = service
        .flag_ids
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').collect())
        .unwrap_or_default();
    let flags_rounds_valid = cfg.scoring.flags_rounds_valid.as_i64().unwrap_or(10);
    let min_tick = std::cmp::max(1, latest_tick - flags_rounds_valid);

    let custom_flag_ids = if flag_id_types.contains(&"custom") {
        load_custom_flag_ids(redis, service.id as i64, min_tick, latest_tick).await?
    } else {
        HashMap::new()
    };

    for team in teams {
        let mut per_team = serde_json::Map::new();
        for tick in min_tick..latest_tick {
            let ids: Vec<Option<String>> = flag_id_types
                .iter()
                .enumerate()
                .map(|(i, flag_id_type)| {
                    if *flag_id_type == "custom" {
                        Ok(custom_flag_ids
                            .get(&(tick, team.id as i64, i as i64))
                            .cloned()
                            .flatten())
                    } else {
                        Ok(Some(gamelib_rs::port::generate_flag_id(
                            (*flag_id_type).to_string(),
                            service.id as i64,
                            team.id as i64,
                            tick,
                            i as i64,
                        )?))
                    }
                })
                .collect::<Result<_, String>>()?;
            let value = if ids.len() == 1 {
                match &ids[0] {
                    Some(s) => json!(s),
                    None => Value::Null,
                }
            } else {
                json!(ids)
            };
            per_team.insert(tick.to_string(), value);
        }
        data.insert(
            cfg.network.team_id_to_vulnbox_ip(team.id as i64),
            Value::Object(per_team),
        );
    }
    Ok(Value::Object(data))
}

/// `_load_custom_flag_ids` — custom flag ids live under
/// `custom_flag_ids:{service}:{tick}:{team}:{idx}`.
async fn load_custom_flag_ids(
    redis: &mut ConnectionManager,
    service_id: i64,
    min_tick: i64,
    latest_tick: i64,
) -> Result<HashMap<(i64, i64, i64), Option<String>>, redis::RedisError> {
    let mut result = HashMap::new();
    for tick in min_tick..latest_tick {
        // excluding latest = current_tick
        let pattern = format!("custom_flag_ids:{service_id}:{tick}:*");
        let keys: Vec<String> = redis::cmd("KEYS").arg(&pattern).query_async(redis).await?;
        if keys.is_empty() {
            continue;
        }
        let values: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(redis).await?;
        for (k, v) in keys.into_iter().zip(values) {
            // custom_flag_ids:{service}:{tick}:{team}:{idx}
            let parts: Vec<&str> = k.split(':').collect();
            if parts.len() == 5 {
                if let (Ok(tck), Ok(teamid), Ok(idx)) = (
                    parts[2].parse::<i64>(),
                    parts[3].parse::<i64>(),
                    parts[4].parse::<i64>(),
                ) {
                    result.insert((tck, teamid, idx), v);
                }
            }
        }
    }
    Ok(result)
}
