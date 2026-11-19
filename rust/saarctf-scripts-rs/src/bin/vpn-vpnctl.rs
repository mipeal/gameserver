//! Port of `vpn/vpnctl.py` — VPN systemd service status control.

use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

static DRY: AtomicBool = AtomicBool::new(false);

mod colors {
    pub const HEADER: &str = "\x1b[95m";
    pub const OKGREEN: &str = "\x1b[92m";
    pub const WARNING: &str = "\x1b[93m";
    pub const FAIL: &str = "\x1b[91m";
    pub const ENDC: &str = "\x1b[0m";
}

fn service_vpn_selfhosted(team_id: i32) -> String {
    format!("vpn@team{team_id}")
}
fn service_vpn_cloudhosted(team_id: i32) -> String {
    format!("vpn@team{team_id}-vulnbox")
}
fn service_vpn_cloud_players(team_id: i32) -> String {
    format!("vpn2@team{team_id}-cloud")
}

fn execute(cmd: &[String], check: bool) -> Result<String, String> {
    let out = std::process::Command::new("systemctl")
        .args(cmd)
        .stdout(std::process::Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if check && !out.status.success() {
        return Err(format!(
            "CalledProcessError: systemctl {} exited {:?}",
            cmd.join(" "),
            out.status.code()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `SystemCtlUtils.is_active` — batches of 100, `check=False` because
/// `systemctl is-active` returns code 3 if ALL selected services are down.
fn is_active(service_names: &[String]) -> Result<HashMap<String, bool>, String> {
    let mut results: Vec<bool> = Vec::new();
    for offset in (0..service_names.len()).step_by(100) {
        let batch = &service_names[offset..(offset + 100).min(service_names.len())];
        let text = execute(&[&["is-active".to_string()][..], batch].concat(), false)?;
        for (i, line) in text.trim().split('\n').enumerate() {
            if line == "active" || line == "activating" {
                if line == "activating" {
                    println!(
                        "{}[WARN] Service {} is activating{}",
                        colors::WARNING,
                        batch[i],
                        colors::ENDC
                    );
                }
                results.push(true);
            } else if line == "inactive" || line == "deactivating" {
                results.push(false);
            } else {
                return Err(format!(
                    "Exception: Unexpected line in systemctl is-active output: {line:?}"
                ));
            }
        }
    }
    if results.len() != service_names.len() {
        return Err(format!(
            "Exception: Unexpected length difference in systemctl is-active outputs: {}, but expected {}",
            results.len(),
            service_names.len()
        ));
    }
    Ok(service_names.iter().cloned().zip(results).collect())
}

fn start(service_names: &[String]) -> Result<(), String> {
    if DRY.load(Ordering::Relaxed) {
        println!("WOULD EXECUTE: systemctl start {}", service_names.join(" "));
    } else if !service_names.is_empty() {
        execute(&[&["start".to_string()][..], service_names].concat(), true)?;
    }
    Ok(())
}

fn stop(service_names: &[String]) -> Result<(), String> {
    // NOTE: the Python original prints 'start' in dry-run mode (bug, kept)
    if DRY.load(Ordering::Relaxed) {
        println!("WOULD EXECUTE: systemctl start {}", service_names.join(" "));
    } else if !service_names.is_empty() {
        execute(&[&["stop".to_string()][..], service_names].concat(), true)?;
    }
    Ok(())
}

#[derive(Clone)]
struct VpnStatus {
    team_id: i32,
    self_connected: bool,
    cloud_connected: bool,
    cloud_player_conns: i32,
    self_active: bool,
    cloud_active: bool,
    cloud_players_active: bool,
}

#[derive(Default)]
struct VpnStatusCheck {
    team_id: i32,
    ok: bool,
    messages: Vec<String>,
    mode: Option<&'static str>,
    fix_start_service: Vec<String>,
    fix_stop_service: Vec<String>,
    fix_needs_reset: bool,
}

impl VpnStatusCheck {
    fn add_error(&mut self, message: &str) {
        self.ok = false;
        self.messages.push(message.to_string());
    }

    fn from_status(status: &VpnStatus) -> VpnStatusCheck {
        let mut r = VpnStatusCheck {
            team_id: status.team_id,
            ok: true,
            ..Default::default()
        };
        if status.cloud_connected {
            r.mode = Some("cloud");
            if status.self_connected {
                r.add_error("Both self-hosted and cloud-hosted VPN are connected.");
                r.fix_needs_reset = true;
            }
            if status.self_active {
                r.add_error("Self-hosted VPN is active although cloud-hosted VPN is connected.");
                r.fix_stop_service
                    .push(service_vpn_selfhosted(status.team_id));
            }
            if !status.cloud_active {
                r.add_error("Cloud-hosted VPN is connected but inactive (wtf?).");
                r.fix_start_service
                    .push(service_vpn_cloud_players(status.team_id));
            }
            if !status.cloud_players_active {
                r.add_error("Cloud players VPN is inactive.");
                r.fix_start_service
                    .push(service_vpn_cloud_players(status.team_id));
            }
        } else if status.self_connected {
            r.mode = Some("self");
            if status.cloud_player_conns > 0 {
                r.add_error("Both self-hosted and cloud player VPNs have connections.");
                if status.cloud_players_active {
                    r.fix_stop_service
                        .push(service_vpn_cloud_players(status.team_id));
                } else {
                    r.fix_needs_reset = true;
                }
            }
            if !status.self_active {
                r.add_error("Self-hosted VPN is connected but inactive (wtf?).");
                r.fix_needs_reset = true;
            }
            if !status.cloud_active {
                r.add_error("Cloud-hosted VPN is inactive.");
                r.fix_start_service
                    .push(service_vpn_cloudhosted(status.team_id));
            }
            if status.cloud_players_active {
                r.add_error("Cloud players VPN is active while self-hosted VPN is connected.");
                r.fix_stop_service
                    .push(service_vpn_cloud_players(status.team_id));
            }
        } else {
            if !status.self_active {
                r.add_error("Self-hosted VPN is inactive.");
                r.fix_start_service
                    .push(service_vpn_selfhosted(status.team_id));
            }
            if !status.cloud_active {
                r.add_error("Cloud-hosted VPN is inactive.");
                r.fix_start_service
                    .push(service_vpn_cloudhosted(status.team_id));
            }
            if !status.cloud_players_active {
                r.add_error("Cloud players VPN is inactive.");
                r.fix_start_service
                    .push(service_vpn_cloud_players(status.team_id));
            }
        }
        r
    }
}

struct TeamRow {
    id: i32,
    vpn_connected: bool,
    vpn2_connected: bool,
    vpn_connection_count: i32,
}

async fn fetch_teams(db: &PgPool, ids: Option<&[i32]>) -> Result<Vec<TeamRow>, String> {
    let rows: Vec<(i32, bool, bool, i32)> = match ids {
        Some(ids) => sqlx::query_as(
            "SELECT id, vpn_connected, vpn2_connected, vpn_connection_count FROM teams WHERE id = ANY($1) ORDER BY id",
        )
        .bind(ids)
        .fetch_all(db),
        None => sqlx::query_as(
            "SELECT id, vpn_connected, vpn2_connected, vpn_connection_count FROM teams ORDER BY id",
        )
        .fetch_all(db),
    }
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(
            |(id, vpn_connected, vpn2_connected, vpn_connection_count)| TeamRow {
                id,
                vpn_connected,
                vpn2_connected,
                vpn_connection_count,
            },
        )
        .collect())
}

async fn from_teams(_db: &PgPool, teams: Vec<TeamRow>) -> Result<Vec<VpnStatus>, String> {
    let mut services = Vec::new();
    for t in &teams {
        services.push(service_vpn_selfhosted(t.id));
        services.push(service_vpn_cloudhosted(t.id));
        services.push(service_vpn_cloud_players(t.id));
    }
    let status = is_active(&services)?;
    Ok(teams
        .into_iter()
        .map(|t| VpnStatus {
            team_id: t.id,
            self_connected: t.vpn_connected,
            cloud_connected: t.vpn2_connected,
            cloud_player_conns: t.vpn_connection_count,
            self_active: status[&service_vpn_selfhosted(t.id)],
            cloud_active: status[&service_vpn_cloudhosted(t.id)],
            cloud_players_active: status[&service_vpn_cloud_players(t.id)],
        })
        .collect())
}

/// VPNs for teams that do not yet exist in DB (not affected by fix/reset).
fn for_future_teams(team_ids: Vec<i32>, service_status: &HashMap<String, bool>) -> Vec<VpnStatus> {
    team_ids
        .into_iter()
        .map(|team_id| VpnStatus {
            team_id,
            self_connected: false,
            cloud_connected: false,
            cloud_player_conns: 0,
            self_active: service_status[&service_vpn_selfhosted(team_id)],
            cloud_active: service_status[&service_vpn_cloudhosted(team_id)],
            cloud_players_active: service_status[&service_vpn_cloud_players(team_id)],
        })
        .collect()
}

async fn fixer_reset(db: &PgPool, check_list: &[VpnStatusCheck]) -> Result<(), String> {
    let mut services = Vec::new();
    for check in check_list {
        services.push(service_vpn_selfhosted(check.team_id));
        services.push(service_vpn_cloudhosted(check.team_id));
        services.push(service_vpn_cloud_players(check.team_id));
    }
    println!("[reset] Stopping {} services ...", services.len());
    stop(&services)?;

    let ids: Vec<i32> = check_list.iter().map(|c| c.team_id).collect();
    let changes = if DRY.load(Ordering::Relaxed) {
        // Python executes the UPDATE but never commits (rolled back)
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        let n = sqlx::query(
            "UPDATE teams SET vpn_connected = FALSE, vpn2_connected = FALSE, vpn_connection_count = 0 WHERE id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
        drop(tx); // rollback
        n
    } else {
        sqlx::query(
            "UPDATE teams SET vpn_connected = FALSE, vpn2_connected = FALSE, vpn_connection_count = 0 WHERE id = ANY($1)",
        )
        .bind(&ids)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected()
    };
    if DRY.load(Ordering::Relaxed) {
        println!("WOULD UPDATE {changes} teams in the db");
    } else {
        println!("[reset] DB reset ({changes} teams)");
    }

    println!("[reset] Starting {} services ...", services.len());
    start(&services)?;
    Ok(())
}

async fn fixer_fix(db: &PgPool, check_list: Vec<VpnStatusCheck>) -> Result<(), String> {
    let mut reset_checks = Vec::new();
    let mut start_services = Vec::new();
    let mut stop_services = Vec::new();
    for check in check_list {
        if check.fix_needs_reset {
            reset_checks.push(check);
        } else {
            start_services.extend(check.fix_start_service);
            stop_services.extend(check.fix_stop_service);
        }
    }
    if !reset_checks.is_empty() {
        println!("Resetting {} VPNs ...", reset_checks.len());
        fixer_reset(db, &reset_checks).await?;
    }
    if !start_services.is_empty() {
        println!("Starting {} services ...", start_services.len());
        start(&start_services)?;
    }
    if !stop_services.is_empty() {
        println!("Stopping {} services ...", stop_services.len());
        stop(&stop_services)?;
    }
    Ok(())
}

async fn command_status(db: &PgPool, team_ids: Option<Vec<i32>>) -> Result<(), String> {
    let teams = fetch_teams(db, team_ids.as_deref()).await?;
    let team_status = from_teams(db, teams).await?;
    let mut errors = 0;

    println!("{}{}{}", colors::HEADER, "-".repeat(55), colors::ENDC);
    println!(
        "{}Team      | Mode  | Self VPN  | Cloud VPN | Player VPN{}",
        colors::HEADER,
        colors::ENDC
    );
    println!("{}{}{}", colors::HEADER, "-".repeat(55), colors::ENDC);
    for status in &team_status {
        let check = VpnStatusCheck::from_status(status);
        let state_self = if status.self_connected {
            format!("{}connected{}", colors::OKGREEN, colors::ENDC)
        } else if status.self_active {
            "on".to_string()
        } else {
            "off".to_string()
        };
        let state_cloud = if status.cloud_connected {
            format!("{}connected{}", colors::OKGREEN, colors::ENDC)
        } else if status.cloud_active {
            "on".to_string()
        } else {
            "off".to_string()
        };
        let state_players = if status.cloud_player_conns > 0 {
            format!(
                "{}{:>4} conns{}",
                colors::OKGREEN,
                status.cloud_player_conns,
                colors::ENDC
            )
        } else if status.cloud_players_active {
            "on".to_string()
        } else {
            "off".to_string()
        };
        let line = format!(
            "Team #{:<3} | {:<5} | {:>9} | {:>9} | {:>10}",
            status.team_id,
            check.mode.unwrap_or("-"),
            state_self,
            state_cloud,
            state_players
        );
        if check.ok {
            println!("{line}");
        } else {
            errors += 1;
            println!("{}{}{}", colors::FAIL, line, colors::ENDC);
            for msg in &check.messages {
                println!("  - {msg}");
            }
        }
    }
    println!();
    if errors > 0 {
        println!("=> {} VPNs, {errors} in invalid status", team_status.len());
        std::process::exit(1);
    } else {
        println!(
            "=> {} VPNs, {}all good{}",
            team_status.len(),
            colors::OKGREEN,
            colors::ENDC
        );
    }
    Ok(())
}

async fn command_fix(db: &PgPool, team_ids: Option<Vec<i32>>) -> Result<(), String> {
    let teams = match team_ids {
        Some(ids) => fetch_teams(db, Some(&ids)).await?,
        None => fetch_teams(db, None).await?,
    };
    let team_status = from_teams(db, teams).await?;
    fixer_fix(
        db,
        team_status
            .iter()
            .map(VpnStatusCheck::from_status)
            .collect(),
    )
    .await?;
    println!("Fix finished.");
    Ok(())
}

async fn command_reset(db: &PgPool, team_ids: Option<Vec<i32>>) -> Result<(), String> {
    let teams = match team_ids {
        Some(ids) => fetch_teams(db, Some(&ids)).await?,
        None => fetch_teams(db, None).await?,
    };
    let team_status = from_teams(db, teams).await?;
    let checks: Vec<VpnStatusCheck> = team_status
        .iter()
        .map(VpnStatusCheck::from_status)
        .collect();
    fixer_reset(db, &checks).await?;
    println!("Reset finished.");
    Ok(())
}

fn command_set(mode: &str, team_ids: &[i32]) -> Result<(), String> {
    let services_self: Vec<String> = team_ids
        .iter()
        .map(|&i| service_vpn_selfhosted(i))
        .collect();
    let services_cloud: Vec<String> = team_ids
        .iter()
        .map(|&i| service_vpn_cloudhosted(i))
        .collect();
    let services_players: Vec<String> = team_ids
        .iter()
        .map(|&i| service_vpn_cloud_players(i))
        .collect();
    match mode {
        "self" => {
            stop(&[services_players, services_cloud].concat())?;
            start(&services_self)?;
        }
        "cloud" => {
            stop(&services_self)?;
            start(&[services_players, services_cloud].concat())?;
        }
        "off" => {
            stop(&[services_self, services_cloud, services_players].concat())?;
        }
        _ => return Err("Invalid mode".to_string()),
    }
    Ok(())
}

async fn command_startup(
    db: &PgPool,
    cfg: &saarctf_commons_rs::config::Config,
) -> Result<(), String> {
    // get team IDs from existing configurations
    let mut team_ids: Vec<i32> = Vec::new();
    let re = regex::Regex::new(r"^team(\d+).conf$").map_err(|e| e.to_string())?;
    for entry in
        std::fs::read_dir(cfg.vpn_base_dir.join("config-server")).map_err(|e| e.to_string())?
    {
        let fname = entry.map_err(|e| e.to_string())?.file_name();
        let fname = fname.to_string_lossy();
        if let Some(caps) = re.captures(&fname) {
            if &caps[1] != "0" {
                team_ids.push(caps[1].parse().map_err(|e| format!("ValueError: {e}"))?);
            }
        }
    }
    // get status from all configured teams
    let teams = fetch_teams(db, None).await?;
    let mut status_list = from_teams(db, teams).await?;
    let existing: std::collections::HashSet<i32> = status_list.iter().map(|s| s.team_id).collect();
    let future: Vec<i32> = team_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<i32>>()
        .difference(&existing)
        .cloned()
        .collect();
    if !future.is_empty() {
        let mut services = Vec::new();
        for team_id in &future {
            services.push(service_vpn_selfhosted(*team_id));
            services.push(service_vpn_cloudhosted(*team_id));
            services.push(service_vpn_cloud_players(*team_id));
        }
        let service_status = is_active(&services)?;
        status_list.extend(for_future_teams(future, &service_status));
    }
    // collect services to start
    let mut services: Vec<String> = Vec::new();
    for status in &status_list {
        services.extend(VpnStatusCheck::from_status(status).fix_start_service);
    }
    if !services.is_empty() {
        println!("Starting {} services ...", services.len());
        println!("{}", services.join(" "));
        start(&services)?;
        println!("Done.");
    } else {
        println!("Nothing to start.");
    }
    Ok(())
}

fn print_help() {
    println!(
        "usage: vpnctl [--dry] {{status [TEAMID ...] | fix [TEAMID ...] [--all] | reset [TEAMID ...] [--all] | set {{self|cloud|off}} TEAMID [TEAMID ...] | startup}}\n\
         \n\
         VPN Systemd Service Status Control"
    );
}

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = match saarctf_scripts_rs::init("vpnctl").await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dry = false;
    let mut rest: Vec<String> = Vec::new();
    for a in &args {
        if a == "--dry" {
            dry = true;
        } else {
            rest.push(a.clone());
        }
    }
    if dry {
        DRY.store(true, Ordering::Relaxed);
    }

    let result = match rest.first().map(String::as_str) {
        Some("status") => {
            let ids = parse_team_ids(&rest[1..]);
            command_status(&db, ids).await
        }
        Some("fix") => {
            let (ids, all) = parse_team_ids_with_all(&rest[1..]);
            if !all && ids.is_none() {
                println!("Please specify team ids or use --all.");
                Ok(())
            } else {
                command_fix(&db, ids).await
            }
        }
        Some("reset") => {
            let (ids, all) = parse_team_ids_with_all(&rest[1..]);
            if !all && ids.is_none() {
                println!("Please specify team ids or use --all.");
                Ok(())
            } else {
                command_reset(&db, ids).await
            }
        }
        Some("set") => {
            let mode = rest.get(1).cloned().unwrap_or_default();
            let ids = match parse_team_ids(&rest[2..]) {
                Some(v) if !v.is_empty() => v,
                _ => {
                    eprintln!("argument teamid: expected at least one argument");
                    std::process::exit(2);
                }
            };
            command_set(&mode, &ids)
        }
        Some("startup") => command_startup(&db, &cfg).await,
        Some(_) => {
            println!("{}unknown command{}", colors::FAIL, colors::ENDC);
            print_help();
            std::process::exit(1);
        }
        None => {
            print_help();
            std::process::exit(1);
        }
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn parse_team_ids(args: &[String]) -> Option<Vec<i32>> {
    let mut ids = Vec::new();
    for a in args {
        if a.starts_with("--") {
            break;
        }
        match a.parse::<i32>() {
            Ok(id) => ids.push(id),
            Err(_) => {
                eprintln!("invalid team id: {a}");
                std::process::exit(2);
            }
        }
    }
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

fn parse_team_ids_with_all(args: &[String]) -> (Option<Vec<i32>>, bool) {
    let mut all = false;
    let mut rest = Vec::new();
    for a in args {
        if a == "--all" {
            all = true;
        } else {
            rest.push(a.clone());
        }
    }
    (parse_team_ids(&rest), all)
}
