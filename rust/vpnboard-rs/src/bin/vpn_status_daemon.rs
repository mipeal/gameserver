//! Port of `vpnboard/vpn_status_daemon.py` — periodically collect VPN status
//! (wireguard + pings) and route it to DB, VpnBoard files, logs and metrics.
//! The ping ThreadPool(24) is a tokio semaphore; the redis watcher is a task.

use chrono::Utc;
use controlserver_rs::models::Team;
use futures_util::StreamExt;
use ctf_commons_rs::config::Config;
use ctf_commons_rs::metrics::{metrics_record_many, MetricValue};
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{RwLock, Semaphore};
use vpnboard_rs::board::{ts_datetime, ts_secs, VpnBoard};
use vpnboard_rs::checks;
use vpnboard_rs::wg_status::get_wireguard_status;
use vpnboard_rs::VpnStatus;

const PING_CONCURRENCY: usize = 24;
const TEAM_QUERY: &str = "SELECT id, name, affiliation, website, logo, vpn_connected, vpn2_connected, vpn_connection_count, vpn_last_connect, vpn_last_disconnect, wg_vulnbox_connected, wg_boxes_connected FROM teams ORDER BY id";

#[derive(Clone)]
struct Shared {
    vulnbox_connection_available: Arc<RwLock<bool>>,
    banned_teams: Arc<RwLock<HashSet<i32>>>,
    initialized: Arc<AtomicBool>,
}

/// `VpnStatusThread.run` — watch redis network:state/ban/unban. Up to 18
/// connection attempts, then `sys.exit(1)`.
async fn watch_redis(shared: Shared, client: redis::Client) {
    for attempt in 0..18 {
        match watch_once(shared.clone(), &client).await {
            Ok(()) => return, // pubsub stream ended (should not happen)
            Err(e) => {
                eprintln!("{e}"); // traceback.print_exc()
                if attempt + 1 == 18 {
                    break;
                }
                tracing::info!("waiting for reconnect ...");
                tokio::time::sleep(Duration::from_secs(10)).await;
                tracing::info!("reconnecting ...");
            }
        }
    }
    println!("Too many connection attempts");
    std::process::exit(1);
}

async fn watch_once(shared: Shared, client: &redis::Client) -> Result<(), String> {
    let mut conn = redis::aio::ConnectionManager::new(client.clone())
        .await
        .map_err(|e| e.to_string())?;
    let state: Option<String> = redis::cmd("GET")
        .arg("network:state")
        .query_async(&mut conn)
        .await
        .map_err(|e| e.to_string())?;
    match state {
        None => {
            let _: Result<String, _> = redis::cmd("SET")
                .arg("network:state")
                .arg("off")
                .query_async(&mut conn)
                .await;
            *shared.vulnbox_connection_available.write().await = false;
        }
        Some(s) => {
            *shared.vulnbox_connection_available.write().await = s == "on" || s == "team";
        }
    }
    let banned: Vec<String> = redis::cmd("SMEMBERS")
        .arg("network:banned")
        .query_async(&mut conn)
        .await
        .map_err(|e| e.to_string())?;
    {
        let mut set = shared.banned_teams.write().await;
        for m in banned {
            if let Ok(id) = m.parse::<i32>() {
                set.insert(id);
            }
        }
    }
    shared.initialized.store(true, Ordering::Relaxed);

    let mut pubsub = client.get_async_pubsub().await.map_err(|e| e.to_string())?;
    for channel in ["network:state", "network:ban", "network:unban"] {
        pubsub.subscribe(channel).await.map_err(|e| e.to_string())?;
    }
    let mut stream = pubsub.into_on_message();
    while let Some(msg) = stream.next().await {
        let channel = msg.get_channel_name().to_string();
        let data: String = match msg.get_payload() {
            Ok(p) => p,
            Err(_) => continue,
        };
        match channel.as_str() {
            "network:state" => {
                *shared.vulnbox_connection_available.write().await = data == "on" || data == "team";
            }
            "network:ban" => {
                if let Ok(id) = data.parse::<i32>() {
                    shared.banned_teams.write().await.insert(id);
                }
            }
            "network:unban" => {
                if let Ok(id) = data.parse::<i32>() {
                    shared.banned_teams.write().await.remove(&id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `VpnStatusDaemon.update_team_wireguard_status` — fetch wireguard status and
/// write only the changed DB columns (SQLAlchemy dirty tracking equivalent).
async fn update_team_wireguard_status(
    db: &PgPool,
    cfg: &Config,
    states: &mut [VpnStatus],
    complete: bool,
) -> Result<(), sqlx::Error> {
    for state in states.iter_mut() {
        let old = TeamCols::of(&state.team);
        let team = &mut state.team;
        let wg = match get_wireguard_status(team.id).await {
            Ok(wg) => wg,
            Err(e) => {
                tracing::warn!("Team {} has no readable interface ({e})", team.id);
                if complete && (team.wg_boxes_connected || team.wg_vulnbox_connected) {
                    team.wg_boxes_connected = false;
                    team.wg_vulnbox_connected = false;
                    team.vpn_connection_count = 0;
                    write_team_changes(db, &old, team).await?;
                }
                continue;
            }
        };
        state.wg = Some(wg.clone());

        // check vulnbox connection status
        let vb_ip = cfg.network.team_id_to_vulnbox_ip(team.id as i64);
        let vulnbox_peer = wg.get_peer_for(&vb_ip).cloned();
        match vulnbox_peer {
            Some(vp) => {
                if !team.wg_vulnbox_connected && vp.connected {
                    // new connection
                    team.wg_vulnbox_connected = true;
                    team.vpn_last_connect = vp.last_handshake;
                } else if team.wg_vulnbox_connected && !vp.connected {
                    // disconnect
                    team.wg_vulnbox_connected = false;
                    team.vpn_last_disconnect = match vp.last_handshake {
                        Some(h) => Some(h + chrono::Duration::minutes(2)),
                        None => Some(Utc::now()),
                    };
                }
            }
            None if team.wg_vulnbox_connected => {
                // peer disappeared, no disconnect timestamp
                team.wg_vulnbox_connected = false;
            }
            None => {}
        }
        // check router/testbox connection status
        let gw_ip = cfg.network.team_id_to_gateway_ip(team.id as i64);
        let tb_ip = cfg.network.team_id_to_testbox_ip(team.id as i64);
        let boxes_connected = [&vb_ip, &gw_ip, &tb_ip]
            .iter()
            .any(|ip| wg.get_peer_for(ip).map(|p| p.connected).unwrap_or(false));
        if boxes_connected != team.wg_boxes_connected {
            team.wg_boxes_connected = boxes_connected;
        }
        // connection count
        let cc = wg.connection_count();
        if cc != team.vpn_connection_count {
            team.vpn_connection_count = cc;
        }
        write_team_changes(db, &old, team).await?;
    }
    Ok(())
}

/// The DB columns `update_team_wireguard_status` may change.
#[derive(Clone, Copy)]
struct TeamCols {
    wg_vulnbox_connected: bool,
    wg_boxes_connected: bool,
    vpn_connection_count: i32,
    vpn_last_connect: Option<chrono::DateTime<Utc>>,
    vpn_last_disconnect: Option<chrono::DateTime<Utc>>,
}

impl TeamCols {
    fn of(team: &Team) -> Self {
        TeamCols {
            wg_vulnbox_connected: team.wg_vulnbox_connected,
            wg_boxes_connected: team.wg_boxes_connected,
            vpn_connection_count: team.vpn_connection_count,
            vpn_last_connect: team.vpn_last_connect,
            vpn_last_disconnect: team.vpn_last_disconnect,
        }
    }
}

/// None when no tracked column changed. (QueryBuilders `separated(", ")`
/// emits the separator BEFORE each element, so the dynamic SET is built with
/// explicit commas instead.)
fn build_team_update_query(
    old: &TeamCols,
    team: &Team,
) -> Option<sqlx::QueryBuilder<'static, sqlx::Postgres>> {
    let mut q = sqlx::QueryBuilder::new("UPDATE teams SET ");
    let mut any = false;
    macro_rules! set_col {
        ($field:ident) => {
            if team.$field != old.$field {
                if any {
                    q.push(", ");
                }
                q.push(concat!(stringify!($field), " = "))
                    .push_bind(team.$field.clone());
                any = true;
            }
        };
    }
    set_col!(wg_vulnbox_connected);
    set_col!(wg_boxes_connected);
    set_col!(vpn_connection_count);
    set_col!(vpn_last_connect);
    set_col!(vpn_last_disconnect);
    if !any {
        return None;
    }
    q.push(" WHERE id = ").push_bind(team.id);
    Some(q)
}

async fn write_team_changes(db: &PgPool, old: &TeamCols, team: &Team) -> Result<(), sqlx::Error> {
    match build_team_update_query(old, team) {
        Some(mut q) => q.build().execute(db).await.map(|_| ()),
        None => Ok(()),
    }
}

/// `ping_teams_threadpool` — in-process "threadpool" with a 24-permit semaphore.
async fn ping_teams(
    cfg: &Config,
    states: &mut [VpnStatus],
    use_nping: bool,
    check_vulnboxes: bool,
) {
    let connected: Vec<usize> = states
        .iter()
        .enumerate()
        .filter(|(_, s)| s.connected())
        .map(|(i, _)| i)
        .collect();
    if connected.is_empty() {
        return;
    }
    let cfg = Arc::new(cfg.clone());
    let sem = Arc::new(Semaphore::new(PING_CONCURRENCY));
    let mut handles = Vec::new();
    for i in connected {
        let (team_id, vpn2) = (states[i].team.id, states[i].team.vpn2_connected);
        let (cfg, sem) = (cfg.clone(), sem.clone());
        handles.push((
            i,
            tokio::spawn(async move {
                let _permit = sem.acquire().await;
                // nping to gateway seems heavily rate-limited. We use normal ping instead.
                let router_ip = cfg.network.team_id_to_gateway_ip(team_id as i64);
                let testbox_ip = cfg.network.team_id_to_testbox_ip(team_id as i64);
                let vuln_ip = cfg.network.team_id_to_vulnbox_ip(team_id as i64);
                let router = if vpn2 || !use_nping {
                    checks::test_ping(&router_ip).await
                } else {
                    checks::test_nping(&router_ip).await
                };
                let testbox = if use_nping {
                    checks::test_nping(&testbox_ip).await
                } else {
                    checks::test_ping(&testbox_ip).await
                };
                let web = checks::test_web(&testbox_ip).await;
                let vuln = if check_vulnboxes {
                    if use_nping {
                        checks::test_nping(&vuln_ip).await
                    } else {
                        checks::test_ping(&vuln_ip).await
                    }
                } else {
                    None
                };
                (router, testbox, web, vuln)
            }),
        ));
    }
    for (i, h) in handles {
        let (router, testbox, web, vuln) = match h.await {
            Ok(v) => v,
            Err(e) => (None, None, format!("error: {e}"), None),
        };
        let state = &mut states[i];
        state.router_ping_ms = router;
        state.testbox_ping_ms = testbox;
        state.vulnbox_ping_ms = vuln;
        state.testbox_ok = web == "OK";
        state.testbox_err = Some(web);
    }
}

/// `update_status` (retry_on_sql_error attempts=40, sleeptime=1.5).
async fn update_status(
    db: &PgPool,
    cfg: &Config,
    check_wg: bool,
    complete: bool,
    check_vulnboxes: bool,
    use_nping: bool,
) -> Result<Vec<VpnStatus>, sqlx::Error> {
    let teams = {
        let mut teams = None;
        let mut last_err = None;
        for _ in 0..40 {
            match sqlx::query_as::<_, Team>(TEAM_QUERY).fetch_all(db).await {
                Ok(t) => {
                    teams = Some(t);
                    break;
                }
                Err(e) => {
                    tracing::error!("update_status query failed: {e}");
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
            }
        }
        match teams {
            Some(t) => t,
            None => return Err(last_err.expect("INFALLIBLE: 40 attempts set an error")),
        }
    };
    let mut states: Vec<VpnStatus> = teams.into_iter().map(VpnStatus::new).collect();
    if check_wg {
        update_team_wireguard_status(db, cfg, &mut states, complete).await?;
    }
    if complete {
        ping_teams(cfg, &mut states, use_nping, check_vulnboxes).await;
    }
    Ok(states)
}

/// `WireguardPeerLogger.update_wireguard`.
async fn wireguard_peer_logger_update_wireguard(states: &[VpnStatus]) {
    for state in states {
        if let Some(wg) = &state.wg {
            for peer in &wg.peers {
                if peer.connected {
                    tracing::info!(
                        team_id = state.team.id,
                        address = %peer.address,
                        last_handshake = ?peer.last_handshake,
                        remote_addr = ?peer.remote_addr,
                        remote_port = ?peer.remote_port,
                        "connected peer for {}", peer.address
                    );
                }
            }
        }
    }
}

/// `MetricStatusHandler.update_all` (the Python-side up-counters were dead locals).
fn metric_status_update_all(states: &[VpnStatus], check_vulnboxes: bool, ts: f64) {
    for state in states {
        metrics_record_many(
            "vpn_connection",
            vec![(
                "connected".into(),
                MetricValue::Int(state.connected() as i64),
            )],
            Some(ts),
            vec![("team_id".into(), MetricValue::Int(state.team.id as i64))],
        );
        if state.connected() {
            let mut metrics: Vec<(String, MetricValue)> = vec![
                (
                    "router_up".into(),
                    MetricValue::Int(if state.router_ping_ms.is_none() { 0 } else { 1 }),
                ),
                (
                    "testbox_up".into(),
                    MetricValue::Int(if state.testbox_ping_ms.is_none() {
                        0
                    } else {
                        1
                    }),
                ),
                (
                    "testbox_ok".into(),
                    MetricValue::Int(if state.testbox_ok { 0 } else { 1 }),
                ),
            ];
            if let Some(v) = state.router_ping_ms {
                metrics.push(("router_ping_ms".into(), MetricValue::Float(v)));
            }
            if let Some(v) = state.testbox_ping_ms {
                metrics.push(("testbox_ping_ms".into(), MetricValue::Float(v)));
            }
            if check_vulnboxes {
                metrics.push((
                    "vulnbox_up".into(),
                    MetricValue::Int(if state.vulnbox_ping_ms.is_none() {
                        0
                    } else {
                        1
                    }),
                ));
                if let Some(v) = state.vulnbox_ping_ms {
                    if v != 0.0 {
                        metrics.push(("vulnbox_ping_ms".into(), MetricValue::Float(v)));
                    }
                }
            }
            metrics_record_many(
                "vpn_board",
                metrics,
                Some(ts),
                vec![("team_id".into(), MetricValue::Int(state.team.id as i64))],
            );
        }
    }
}

/// `VpnStatusDaemon.run_once`.
async fn run_once(
    db: &PgPool,
    cfg: &Config,
    shared: &Shared,
    complete: bool,
    check_wg: bool,
    use_nping: bool,
) -> Result<(), sqlx::Error> {
    let start = Instant::now();
    let start_ts = ts_secs(SystemTime::now());
    let vulnbox_available = *shared.vulnbox_connection_available.read().await;
    let check_vulnboxes = vulnbox_available;
    let states = update_status(db, cfg, check_wg, complete, check_vulnboxes, use_nping).await?;

    if check_wg {
        wireguard_peer_logger_update_wireguard(&states).await;
    }
    if complete {
        let banned = shared.banned_teams.read().await.clone();
        if let Err(e) = VpnBoard::build_vpn_board(
            cfg,
            std::path::Path::new(&cfg.vpnboard_path),
            &states,
            &banned,
            check_vulnboxes,
            ts_datetime(SystemTime::now()),
        ) {
            tracing::error!("could not write vpn board files: {e}");
        }
        metric_status_update_all(&states, check_vulnboxes, start_ts);
    }

    let duration = start.elapsed().as_secs_f64();
    tracing::info!(
        "Created VPN board, took {duration:.3} seconds ({} vulnboxes).",
        if vulnbox_available { "with" } else { "without" }
    );
    Ok(())
}

/// `run_loop` — sleep = min(60, max(5, round((60 if duration>=25 else 30) - duration))).
fn sleeptime(duration: f64) -> u64 {
    let target = if duration >= 25.0 { 60.0 } else { 30.0 };
    ((target - duration).round().clamp(5.0, 60.0)) as u64
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (mut daemon, mut complete, mut check_wg, mut system_ping) = (false, false, false, false);
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--daemon" => daemon = true,
            "--complete" => complete = true,
            "--check-wg" => check_wg = true,
            "--system-ping" => system_ping = true,
            "--help" => {
                println!("VPN Status Daemon. Modes:\n    --check-wg:            Pull wireguard status info from this host\n    --complete:            Ping & write VPN board with infos from database\n    --check-wg --complete: Pull wireguard status, ping, and create vpnboard (single router only)\n    --daemon:              Run as daemon (vs: run once)\n    --system-ping:         Use system default ping utility instead of nping");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let (cfg, db, _redis_conn) = ctf_scripts_rs::init("vpn-status-daemon").await?;
    ctf_commons_rs::metrics::setup_default_metrics();

    let client = redis::Client::open(
        ctf_commons_rs::redis::redis_url(&cfg.redis).map_err(|e| format!("redis url: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let shared = Shared {
        vulnbox_connection_available: Arc::new(RwLock::new(false)),
        banned_teams: Arc::new(RwLock::new(HashSet::new())),
        initialized: Arc::new(AtomicBool::new(false)),
    };
    {
        let (shared, client) = (shared.clone(), client.clone());
        tokio::spawn(async move { watch_redis(shared, client).await });
    }
    // give redis time to connect (max 12 seconds)
    for _ in 0..12 {
        if shared.initialized.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // VpnBoard::new ensures the output dir and static files exist
    VpnBoard::new(&cfg)?;
    let use_nping = !system_ping;

    if daemon {
        tracing::info!("VPN Board daemon started.");
        loop {
            let start = Instant::now();
            match run_once(&db, &cfg, &shared, complete, check_wg, use_nping).await {
                Ok(()) => {}
                Err(e) => {
                    // SQLAlchemyError → sys.exit(1)
                    tracing::error!("Could not create VPN board: {e}");
                    std::process::exit(1);
                }
            }
            let duration = start.elapsed().as_secs_f64();
            tokio::time::sleep(Duration::from_secs(sleeptime(duration))).await;
        }
    } else {
        run_once(&db, &cfg, &shared, complete, check_wg, use_nping)
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(cols: TeamCols) -> Team {
        Team {
            id: 2,
            name: "t".into(),
            affiliation: None,
            website: None,
            logo: None,
            vpn_connected: false,
            vpn2_connected: false,
            vpn_connection_count: cols.vpn_connection_count,
            vpn_last_connect: cols.vpn_last_connect,
            vpn_last_disconnect: cols.vpn_last_disconnect,
            wg_vulnbox_connected: cols.wg_vulnbox_connected,
            wg_boxes_connected: cols.wg_boxes_connected,
        }
    }

    #[test]
    fn team_update_sql_has_no_leading_comma() {
        // regression: QueryBuilder::separated emits ", " BEFORE the first
        // field, producing `SET , col = $1` (syntax error, daemon exit-loop)
        let old = TeamCols {
            wg_vulnbox_connected: false,
            wg_boxes_connected: false,
            vpn_connection_count: 0,
            vpn_last_connect: None,
            vpn_last_disconnect: None,
        };
        let mut t = team(old);
        t.wg_vulnbox_connected = true;
        t.vpn_last_connect = Some(Utc::now());
        let q = build_team_update_query(&old, &t).expect("has changes");
        assert!(
            q.sql()
                .starts_with("UPDATE teams SET wg_vulnbox_connected = "),
            "{}",
            q.sql()
        );
        assert!(!q.sql().contains("SET ,"), "{}", q.sql());
        assert!(q.sql().contains(", vpn_last_connect = "), "{}", q.sql());
        assert!(q.sql().contains("WHERE id = "), "{}", q.sql());

        let same = build_team_update_query(&old, &team(old));
        assert!(same.is_none(), "no changes -> no query");
    }
}
