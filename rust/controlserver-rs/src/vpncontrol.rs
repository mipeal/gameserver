//! Port of `controlserver/vpncontrol.py` — VPN/network state in redis,
//! driving manage-iptables (network:state/banned/permissions + pubsub).

use crate::logger;
use crate::models::{LOG_IMPORTANT, LOG_INFO};
use ctf_commons_rs::config::Config;
use sqlx::PgPool;

/// `VpnStatus` StrEnum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnStatus {
    Off,
    On,
    TeamsOnly,
    TeamsOnlyNoVulnbox,
}

impl VpnStatus {
    pub fn value(&self) -> &'static str {
        match self {
            VpnStatus::Off => "off",
            VpnStatus::On => "on",
            VpnStatus::TeamsOnly => "team",
            VpnStatus::TeamsOnlyNoVulnbox => "team-no-vulnbox",
        }
    }

    pub fn from_value(s: &str) -> Option<VpnStatus> {
        match s {
            "off" => Some(VpnStatus::Off),
            "on" => Some(VpnStatus::On),
            "team" => Some(VpnStatus::TeamsOnly),
            "team-no-vulnbox" => Some(VpnStatus::TeamsOnlyNoVulnbox),
            _ => None,
        }
    }
}

/// (banned team ID, tick where the ban gets lifted).
pub type BannedTeamId = (i32, Option<i64>);

pub struct VpnControl {
    pub cfg: Config,
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,
}

impl VpnControl {
    pub async fn new(cfg: &Config, db: PgPool) -> Result<VpnControl, String> {
        Ok(VpnControl {
            cfg: cfg.clone(),
            db,
            redis: ctf_commons_rs::redis::get_redis_connection(&cfg.redis)
                .await
                .map_err(|e| e.to_string())?,
        })
    }

    pub async fn get_state(&mut self) -> Result<VpnStatus, String> {
        let b: Option<String> = redis::cmd("GET")
            .arg("network:state")
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        Ok(match b.as_deref() {
            Some("on") => VpnStatus::On,
            Some("team") => VpnStatus::TeamsOnly,
            Some("team-no-vulnbox") => VpnStatus::TeamsOnlyNoVulnbox,
            _ => VpnStatus::Off,
        })
    }

    pub async fn set_state(&mut self, state: VpnStatus) -> Result<(), String> {
        let old_state = self.get_state().await?;
        let onoff = state.value();
        let _: () = redis::cmd("SET")
            .arg("network:state")
            .arg(onoff)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("PUBLISH")
            .arg("network:state")
            .arg(onoff)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        if old_state != state {
            let msg = match state {
                VpnStatus::On => "Network open",
                VpnStatus::TeamsOnly => "Network open within teams only",
                VpnStatus::TeamsOnlyNoVulnbox => {
                    "Network open within teams only, excluding vulnboxes"
                }
                VpnStatus::Off => "Network closed",
            };
            if let Err(e) = logger::log(&self.db, "vpn", msg, "", LOG_IMPORTANT).await {
                tracing::error!("log: {e}");
            }
            // delay further CTF events (e.g. checker script dispatcher) until
            // the firewall is really open
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Ok(())
    }

    /// (banned team ID, tick where the ban gets lifted), sorted by id.
    pub async fn get_banned_team_ids(&mut self) -> Result<Vec<BannedTeamId>, String> {
        let ids: Vec<String> = redis::cmd("SMEMBERS")
            .arg("network:banned")
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let mut ids: Vec<i32> = ids.iter().filter_map(|s| s.parse().ok()).collect();
        ids.sort();
        let mut result = Vec::new();
        for team_id in ids {
            let tick: Option<String> = redis::cmd("GET")
                .arg(format!("network:bannedteam:{team_id}"))
                .query_async(&mut self.redis)
                .await
                .map_err(|e| e.to_string())?;
            result.push((team_id, tick.and_then(|t| t.parse().ok())));
        }
        Ok(result)
    }

    pub async fn ban_team(&mut self, team_id: i32, until_tick: Option<i64>) -> Result<(), String> {
        if team_id == 0 {
            return Ok(());
        }
        let until_tick = until_tick.filter(|t| *t != 0);
        let _: () = redis::cmd("SADD")
            .arg("network:banned")
            .arg(team_id)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("SET")
            .arg(format!("network:bannedteam:{team_id}"))
            .arg(match until_tick {
                Some(t) => t.to_string(),
                None => String::new(),
            })
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("PUBLISH")
            .arg("network:ban")
            .arg(team_id.to_string())
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let msg = match until_tick {
            Some(t) => format!("Banned team #{team_id} until tick {t}"),
            None => format!("Banned team #{team_id}"),
        };
        if let Err(e) = logger::log(&self.db, "vpn", &msg, "", LOG_INFO).await {
            tracing::error!("log: {e}");
        }
        Ok(())
    }

    pub async fn unban_team(&mut self, team_id: i32) -> Result<(), String> {
        let _: () = redis::cmd("SREM")
            .arg("network:banned")
            .arg(team_id)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("DEL")
            .arg(format!("network:bannedteam:{team_id}"))
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("PUBLISH")
            .arg("network:unban")
            .arg(team_id.to_string())
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let msg = format!("Removed ban from team #{team_id}");
        if let Err(e) = logger::log(&self.db, "vpn", &msg, "", LOG_INFO).await {
            tracing::error!("log: {e}");
        }
        Ok(())
    }

    pub async fn unban_for_tick(&mut self, tick: i64) -> Result<(), String> {
        let ids: Vec<String> = redis::cmd("SMEMBERS")
            .arg("network:banned")
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        for id in ids {
            let id: i32 = match id.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let until: Option<String> = redis::cmd("GET")
                .arg(format!("network:bannedteam:{id}"))
                .query_async(&mut self.redis)
                .await
                .map_err(|e| e.to_string())?;
            if until.as_deref() == Some(&tick.to_string()) {
                self.unban_team(id).await?;
            }
        }
        Ok(())
    }

    pub async fn get_open_teams(&mut self) -> Result<Vec<crate::models::Team>, String> {
        let ids: Vec<String> = redis::cmd("SMEMBERS")
            .arg("network:permissions")
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let mut ids: Vec<i32> = ids.iter().filter_map(|s| s.parse().ok()).collect();
        ids.sort();
        sqlx::query_as::<_, crate::models::Team>(
            "SELECT * FROM teams WHERE id = ANY($1) ORDER BY id",
        )
        .bind(&ids)
        .fetch_all(&self.db)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn add_permission_team(&mut self, team_id: i32) -> Result<(), String> {
        if team_id == 0 {
            return Ok(());
        }
        let _: () = redis::cmd("SADD")
            .arg("network:permissions")
            .arg(team_id)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("PUBLISH")
            .arg("network:add_permission")
            .arg(team_id.to_string())
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let msg = format!("Open network for team #{team_id}");
        if let Err(e) = logger::log(&self.db, "vpn", &msg, "", LOG_INFO).await {
            tracing::error!("log: {e}");
        }
        Ok(())
    }

    pub async fn remove_permission_team(&mut self, team_id: i32) -> Result<(), String> {
        let _: () = redis::cmd("SREM")
            .arg("network:permissions")
            .arg(team_id)
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let _: () = redis::cmd("PUBLISH")
            .arg("network:remove_permission")
            .arg(team_id.to_string())
            .query_async(&mut self.redis)
            .await
            .map_err(|e| e.to_string())?;
        let msg = format!("Close network for team #{team_id}");
        if let Err(e) = logger::log(&self.db, "vpn", &msg, "", LOG_INFO).await {
            tracing::error!("log: {e}");
        }
        Ok(())
    }
}
