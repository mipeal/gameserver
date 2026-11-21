//! Port of `vpn/on-connect.py` — marks a team as "online" (up script from
//! OpenVPN). ARGUMENTS: Team-ID [cloudhosted]

use sqlx::PgPool;

async fn run(db: PgPool, argv: Vec<String>) -> Result<(), String> {
    let team_id: i32 = argv
        .get(1)
        .ok_or("missing team id argument")?
        .parse()
        .map_err(|e| format!("ValueError: {e}"))?;
    let is_vpn_2 = argv.get(2).map(|s| s == "cloudhosted").unwrap_or(false);
    let changes = if is_vpn_2 {
        sqlx::query(
            "UPDATE teams SET vpn2_connected = TRUE, vpn_last_connect = now() WHERE id = $1",
        )
        .bind(team_id)
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected()
    } else {
        sqlx::query("UPDATE teams SET vpn_connected = TRUE, vpn_last_connect = now() WHERE id = $1")
            .bind(team_id)
            .execute(&db)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected()
    };
    if changes > 0 {
        println!("Updated connection status (connected) of team #{team_id}.");
    } else if team_id > 0 {
        // dummy team entry (Team() model defaults == DB column defaults)
        let r = if is_vpn_2 {
            sqlx::query(
                "INSERT INTO teams (id, name, vpn2_connected, vpn_last_connect) VALUES ($1, $2, TRUE, now())",
            )
        } else {
            sqlx::query(
                "INSERT INTO teams (id, name, vpn_connected, vpn_last_connect) VALUES ($1, $2, TRUE, now())",
            )
        }
        .bind(team_id)
        .bind(format!("unnamed team #{team_id}"))
        .execute(&db)
        .await
        .map_err(|e| e.to_string())?;
        if r.rows_affected() > 0 {
            println!("Updated connection status (connected) of team #{team_id}. Created new dummy team entry for that.");
        }
    } else {
        println!("Team #{team_id} not found.");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    match ctf_scripts_rs::init("on-connect").await {
        Ok((_cfg, db, _redis)) => {
            if let Err(e) = run(db, argv.clone()).await {
                ctf_scripts_rs::vpnlib::log_hook_error("=== CONNECT ===", &argv, &e);
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            ctf_scripts_rs::vpnlib::log_hook_error("=== CONNECT ===", &argv, &e);
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
