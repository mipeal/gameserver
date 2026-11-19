//! Port of `vpn/on-cloud-connect.py` — increments the cloud connection
//! counter (up script from the cloud-hosted OpenVPN). ARGUMENTS: Team-ID

use sqlx::PgPool;

async fn run(db: PgPool, argv: Vec<String>) -> Result<(), String> {
    let team_id: i32 = argv
        .get(1)
        .ok_or("missing team id argument")?
        .parse()
        .map_err(|e| format!("ValueError: {e}"))?;
    let changes = sqlx::query(
        "UPDATE teams SET vpn_connection_count = vpn_connection_count + 1 WHERE id = $1",
    )
    .bind(team_id)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    if changes > 0 {
        println!("Updated connection status (connected) of team #{team_id}.");
    } else if team_id > 0 {
        sqlx::query("INSERT INTO teams (id, name, vpn_connection_count) VALUES ($1, $2, 1)")
            .bind(team_id)
            .bind(format!("unnamed team #{team_id}"))
            .execute(&db)
            .await
            .map_err(|e| e.to_string())?;
        println!("Updated connection status (connected) of team #{team_id}. Created new dummy team entry for that.");
    } else {
        println!("Team #{team_id} not found.");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    match saarctf_scripts_rs::init("on-cloud-connect").await {
        Ok((_cfg, db, _redis)) => {
            if let Err(e) = run(db, argv.clone()).await {
                saarctf_scripts_rs::vpnlib::log_hook_error("=== CONNECT ===", &argv, &e);
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            saarctf_scripts_rs::vpnlib::log_hook_error("=== CONNECT ===", &argv, &e);
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
