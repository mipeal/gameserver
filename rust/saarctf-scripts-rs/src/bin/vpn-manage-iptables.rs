//! Port of `vpn/manage-iptables.py` — keeps running on the VPN server,
//! dynamically adding iptables rules to open/close the network and ban/unban
//! teams, driven by redis pubsub. The NewTeamWatcher thread is a tokio task.

use futures_util::StreamExt;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::collections::HashSet;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

const CHAIN: &str = "vpn-blocking";
const RULE_NETCLOSED: [&str; 6] = [
    "-j",
    "REJECT",
    "-m",
    "comment",
    "--comment",
    "Network is closed",
];
const RULE_NETTEAMONLY: [&str; 10] = [
    "-m",
    "mark",
    "--mark",
    "0x0800/0x0c00",
    "-j",
    "RETURN",
    "-m",
    "comment",
    "--comment",
    "Network is open for teams only",
];

fn iptables_exists(chain: &str, rule: &[&str]) -> bool {
    Command::new("iptables")
        .arg("-C")
        .arg(chain)
        .args(rule)
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn iptables_add(chain: &str, rule: &[&str]) -> Result<(), String> {
    if !iptables_exists(chain, rule) {
        let status = Command::new("iptables")
            .arg("-A")
            .arg(chain)
            .args(rule)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("CalledProcessError: iptables -A".to_string());
        }
    }
    Ok(())
}

fn iptables_insert(chain: &str, rule: &[&str]) -> Result<(), String> {
    if !iptables_exists(chain, rule) {
        let status = Command::new("iptables")
            .arg("-I")
            .arg(chain)
            .arg("1")
            .args(rule)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("CalledProcessError: iptables -I".to_string());
        }
    }
    Ok(())
}

fn iptables_remove(chain: &str, rule: &[&str]) {
    while Command::new("iptables")
        .arg("-D")
        .arg(chain)
        .args(rule)
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {}
}

fn iptables_flush(chain: &str) -> Result<(), String> {
    let status = Command::new("iptables")
        .arg("-F")
        .arg(chain)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("CalledProcessError: iptables -F".to_string());
    }
    Ok(())
}

struct IpTablesManager {
    chain: String,
    state: String,
    banned: HashSet<String>,
    allowed: HashSet<String>,
    cfg: Config,
    db: PgPool,
}

impl IpTablesManager {
    async fn init_firewall(
        &mut self,
        state: &str,
        banned: Vec<String>,
        allowed: Vec<String>,
    ) -> Result<(), String> {
        iptables_flush(&self.chain)?;
        self.banned = banned.into_iter().collect();
        self.allowed = allowed.into_iter().collect();
        self.state = state.to_string();
        self.rewrite_chain().await?;
        tracing::info!("[OK]   Updated firewall");
        Ok(())
    }

    async fn update_state(&mut self, state: &str) -> Result<(), String> {
        self.state = state.to_string();
        self.rewrite_chain().await
    }

    async fn rewrite_chain(&mut self) -> Result<(), String> {
        match self.state.as_str() {
            "on" => {
                iptables_flush(&self.chain)?;
                tracing::info!("[UP]   Network open");
            }
            "team" => {
                iptables_flush(&self.chain)?;
                iptables_add(&self.chain, &RULE_NETTEAMONLY)?;
                iptables_add(&self.chain, &RULE_NETCLOSED)?;
                tracing::info!("[TEAM] Network open within teams only");
            }
            "team-no-vulnbox" => {
                iptables_flush(&self.chain)?;
                // first deny vulnboxes, then allow team-internal traffic, then deny other traffic. build backwards
                iptables_add(&self.chain, &RULE_NETCLOSED)?;
                iptables_insert(&self.chain, &RULE_NETTEAMONLY)?;
                self.insert_no_vulnbox_rules().await?;
                tracing::info!("[TEAM] Network open within teams only, excluding vulnboxes");
            }
            _ => {
                iptables_flush(&self.chain)?;
                iptables_add(&self.chain, &RULE_NETCLOSED)?;
                tracing::info!("[DOWN] Network closed");
            }
        }
        for team in self.banned.clone() {
            self.ban_rules(&team)?;
        }
        for team in self.allowed.clone() {
            self.allow_rules(&team)?;
        }
        Ok(())
    }

    async fn insert_no_vulnbox_rules(&self) -> Result<(), String> {
        let max_id: Option<i32> = sqlx::query_scalar("SELECT max(id) FROM teams")
            .fetch_one(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        let max_id = max_id.unwrap_or(0);
        for team_id in 1..(max_id + 2) {
            let net = self.cfg.network.team_id_to_vulnbox_ip(team_id as i64);
            iptables_insert(&self.chain, &["-s", &net, "-j", "REJECT"])?;
            iptables_insert(&self.chain, &["-d", &net, "-j", "REJECT"])?;
        }
        Ok(())
    }

    fn ban_rules(&self, team: &str) -> Result<String, String> {
        let team_id: i32 = team.parse().map_err(|e| format!("ValueError: {e}"))?;
        let iprange = self.cfg.network.team_id_to_network_range(team_id as i64);
        for base in [0, 1000, 2000] {
            let tun = format!("tun{}", base + team_id);
            iptables_add(
                &self.chain,
                &[
                    "-i",
                    &tun,
                    "-j",
                    "DROP",
                    "-m",
                    "comment",
                    "--comment",
                    &format!("Ban team {team} (incoming)"),
                ],
            )?;
            iptables_add(
                &self.chain,
                &[
                    "-o",
                    &tun,
                    "-j",
                    "DROP",
                    "-m",
                    "comment",
                    "--comment",
                    &format!("Ban team {team} (outgoing)"),
                ],
            )?;
        }
        Ok(iprange)
    }

    async fn ban_team(&mut self, team: &str) -> Result<(), String> {
        self.banned.insert(team.to_string());
        let iprange = self.ban_rules(team)?;
        tracing::info!("[+B]   Ban team {team} ({iprange})");
        Ok(())
    }

    fn unban_rules(&self, team: &str) -> Result<String, String> {
        let team_id: i32 = team.parse().map_err(|e| format!("ValueError: {e}"))?;
        let iprange = self.cfg.network.team_id_to_network_range(team_id as i64);
        for base in [0, 1000, 2000] {
            let tun = format!("tun{}", base + team_id);
            iptables_remove(
                &self.chain,
                &[
                    "-i",
                    &tun,
                    "-j",
                    "DROP",
                    "-m",
                    "comment",
                    "--comment",
                    &format!("Ban team {team} (incoming)"),
                ],
            );
            iptables_remove(
                &self.chain,
                &[
                    "-o",
                    &tun,
                    "-j",
                    "DROP",
                    "-m",
                    "comment",
                    "--comment",
                    &format!("Ban team {team} (outgoing)"),
                ],
            );
        }
        Ok(iprange)
    }

    async fn unban_team(&mut self, team: &str) -> Result<(), String> {
        self.banned.remove(team);
        let iprange = self.unban_rules(team)?;
        tracing::info!("[-B]   Remove ban from team {team} ({iprange})");
        Ok(())
    }

    fn allow_rules(&self, team: &str) -> Result<(String, String), String> {
        let team_id: i32 = team.parse().map_err(|e| format!("ValueError: {e}"))?;
        let iprange1 = self.cfg.network.team_id_to_network_range(team_id as i64);
        let iprange2 = self
            .cfg
            .config
            .get("network")
            .and_then(|n| n.get("gameserver_range"))
            .and_then(|v| v.as_str())
            .ok_or("config network.gameserver_range missing")?
            .to_string();
        let comment = format!("Open network for team {team}");
        iptables_insert(
            &self.chain,
            &[
                "--src",
                &iprange1,
                "--dst",
                &iprange2,
                "-j",
                "RETURN",
                "-m",
                "comment",
                "--comment",
                &comment,
            ],
        )?;
        iptables_insert(
            &self.chain,
            &[
                "--src",
                &iprange2,
                "--dst",
                &iprange1,
                "-j",
                "RETURN",
                "-m",
                "comment",
                "--comment",
                &comment,
            ],
        )?;
        Ok((iprange1, iprange2))
    }

    async fn add_network_team(&mut self, team: &str) -> Result<(), String> {
        self.allowed.insert(team.to_string());
        let (iprange1, iprange2) = self.allow_rules(team)?;
        tracing::info!("[+A]   Add network for team {team} ({iprange1} <-> {iprange2})");
        Ok(())
    }

    fn disallow_rules(&self, team: &str) -> Result<(String, String), String> {
        let team_id: i32 = team.parse().map_err(|e| format!("ValueError: {e}"))?;
        let iprange1 = self.cfg.network.team_id_to_network_range(team_id as i64);
        let iprange2 = self
            .cfg
            .config
            .get("network")
            .and_then(|n| n.get("gameserver_range"))
            .and_then(|v| v.as_str())
            .ok_or("config network.gameserver_range missing")?
            .to_string();
        let comment = format!("Open network for team {team}");
        iptables_remove(
            &self.chain,
            &[
                "--src",
                &iprange1,
                "--dst",
                &iprange2,
                "-j",
                "RETURN",
                "-m",
                "comment",
                "--comment",
                &comment,
            ],
        );
        iptables_remove(
            &self.chain,
            &[
                "--src",
                &iprange2,
                "--dst",
                &iprange1,
                "-j",
                "RETURN",
                "-m",
                "comment",
                "--comment",
                &comment,
            ],
        );
        Ok((iprange1, iprange2))
    }

    async fn remove_network_team(&mut self, team: &str) -> Result<(), String> {
        self.allowed.remove(team);
        let (iprange1, iprange2) = self.disallow_rules(team)?;
        tracing::info!("[-A]   Remove network for team {team} ({iprange1} <-> {iprange2})");
        Ok(())
    }
}

/// `NewTeamWatcher.run` — rewrite rules when the number of teams changes.
async fn team_watcher(manager: Arc<Mutex<IpTablesManager>>, db: PgPool) {
    let mut num_teams: i64 = sqlx::query_scalar("SELECT count(*) FROM teams")
        .fetch_one(&db)
        .await
        .unwrap_or(0);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        match sqlx::query_scalar::<_, i64>("SELECT count(*) FROM teams")
            .fetch_one(&db)
            .await
        {
            Ok(current) => {
                if num_teams != current {
                    tracing::info!(
                        "[team] Number of teams changed from {num_teams} to {current}, re-writing rules..."
                    );
                    let mut m = manager.lock().await;
                    if let Err(e) = m.rewrite_chain().await {
                        tracing::error!("team number check failed: {e}");
                    }
                    num_teams = current;
                }
            }
            Err(e) => tracing::error!("team number check failed: {e}"),
        }
    }
}

async fn manage_iptables(
    cfg: Config,
    db: PgPool,
    conn: &mut redis::aio::ConnectionManager,
    client: redis::Client,
) -> Result<(), String> {
    let state: Option<String> = redis::cmd("GET")
        .arg("network:state")
        .query_async(conn)
        .await
        .map_err(|e| e.to_string())?;
    let state = match state {
        Some(s) => s,
        None => {
            let _: Result<String, _> = redis::cmd("SET")
                .arg("network:state")
                .arg("off")
                .query_async(conn)
                .await;
            "off".to_string()
        }
    };
    let banned: Vec<String> = redis::cmd("SMEMBERS")
        .arg("network:banned")
        .query_async(conn)
        .await
        .map_err(|e| e.to_string())?;
    let allowed: Vec<String> = redis::cmd("SMEMBERS")
        .arg("network:permissions")
        .query_async(conn)
        .await
        .map_err(|e| e.to_string())?;

    let mut pubsub = client.get_async_pubsub().await.map_err(|e| e.to_string())?;
    for channel in [
        "network:state",
        "network:ban",
        "network:unban",
        "network:add_permission",
        "network:remove_permission",
    ] {
        pubsub.subscribe(channel).await.map_err(|e| e.to_string())?;
    }

    let manager = Arc::new(Mutex::new(IpTablesManager {
        chain: CHAIN.to_string(),
        state: "off".to_string(),
        banned: HashSet::new(),
        allowed: HashSet::new(),
        cfg,
        db: db.clone(),
    }));
    {
        let mut m = manager.lock().await;
        m.init_firewall(&state, banned, allowed).await?;
    }

    let watcher = tokio::spawn(team_watcher(manager.clone(), db));
    let mut stream = pubsub.into_on_message();
    while let Some(msg) = stream.next().await {
        let channel = msg.get_channel_name().to_string();
        let data: String = match msg.get_payload() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let mut m = manager.lock().await;
        let r = match channel.as_str() {
            "network:state" => m.update_state(&data).await,
            "network:ban" => m.ban_team(&data).await,
            "network:unban" => m.unban_team(&data).await,
            "network:add_permission" => m.add_network_team(&data).await,
            "network:remove_permission" => m.remove_network_team(&data).await,
            other => {
                tracing::info!("[debug] unknown message {other:?}, data {data:?}");
                Ok(())
            }
        };
        if let Err(e) = r {
            tracing::error!("{e}");
        }
    }
    watcher.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, db, mut redis_conn) = saarctf_scripts_rs::init("manage-iptables").await?;
    let client = redis::Client::open(
        saarctf_commons_rs::redis::redis_url(&cfg.redis).map_err(|e| format!("redis url: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    for attempt in 0..18 {
        // wait at most 3 minutes if redis connections fail, then abort
        tracing::info!("       Connecting to redis...");
        match manage_iptables(cfg.clone(), db.clone(), &mut redis_conn, client.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("{e}");
                if attempt + 1 == 18 {
                    break;
                }
                tracing::info!("       waiting for reconnect ...");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }
    Err("Too many connection retries".to_string())
}
