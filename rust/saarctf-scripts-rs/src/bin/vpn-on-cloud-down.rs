//! Port of `vpn/on-cloud-down.py` — zeroes the cloud connection counter
//! (down script from the cloud-hosted OpenVPN). ARGUMENTS: Team-ID

use sqlx::PgPool;

async fn run(db: PgPool, argv: Vec<String>) -> Result<(), String> {
    let team_id: i32 = argv
        .get(1)
        .ok_or("missing team id argument")?
        .parse()
        .map_err(|e| format!("ValueError: {e}"))?;
    let changes = sqlx::query("UPDATE teams SET vpn_connection_count = 0 WHERE id = $1")
        .bind(team_id)
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    if changes > 0 {
        println!("Updated connection status (down) of team #{team_id}.");
    } else {
        println!("Team #{team_id} not found.");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    match saarctf_scripts_rs::init("on-cloud-down").await {
        Ok((_cfg, db, _redis)) => {
            if let Err(e) = run(db, argv.clone()).await {
                saarctf_scripts_rs::vpnlib::log_hook_error("=== DOWN ===", &argv, &e);
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            saarctf_scripts_rs::vpnlib::log_hook_error("=== DOWN ===", &argv, &e);
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
