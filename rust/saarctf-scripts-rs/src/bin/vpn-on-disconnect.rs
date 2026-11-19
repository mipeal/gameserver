//! Port of `vpn/on-disconnect.py` — marks a team as "offline" (down script
//! from OpenVPN). ARGUMENTS: Team-ID [cloudhosted]. The id may arrive as
//! "team<id>-<common-name>" — the 'team' prefix and '-…' suffix are stripped.

use sqlx::PgPool;

async fn run(db: PgPool, argv: Vec<String>) -> Result<(), String> {
    let mut team_id_str = argv.get(1).ok_or("missing team id argument")?.clone();
    if let Some(stripped) = team_id_str.strip_prefix("team") {
        team_id_str = stripped.to_string();
        if team_id_str.contains('-') {
            team_id_str = team_id_str.split('-').next().unwrap_or("").to_string();
        }
    }
    let team_id: i32 = team_id_str
        .parse()
        .map_err(|e| format!("ValueError: {e}"))?;
    let is_vpn_2 = argv.get(2).map(|s| s == "cloudhosted").unwrap_or(false);
    let changes = if is_vpn_2 {
        sqlx::query(
            "UPDATE teams SET vpn2_connected = FALSE, vpn_last_disconnect = now() \
             WHERE id = $1 AND vpn2_connected",
        )
    } else {
        sqlx::query(
            "UPDATE teams SET vpn_connected = FALSE, vpn_last_disconnect = now() \
             WHERE id = $1 AND vpn_connected",
        )
    }
    .bind(team_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    if changes > 0 {
        println!("Updated connection status (disconnected) of team #{team_id}.");
    } else {
        println!("Team #{team_id} already disconnected or not found.");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    match saarctf_scripts_rs::init("on-disconnect").await {
        Ok((_cfg, db, _redis)) => {
            if let Err(e) = run(db, argv.clone()).await {
                saarctf_scripts_rs::vpnlib::log_hook_error("=== DISCONNECT ===", &argv, &e);
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            saarctf_scripts_rs::vpnlib::log_hook_error("=== DISCONNECT ===", &argv, &e);
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
