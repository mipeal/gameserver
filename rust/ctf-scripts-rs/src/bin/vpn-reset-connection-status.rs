//! Port of `vpn/reset-connection-status.py` — reset the connection status of
//! all teams that connected before the last reboot. ARGUMENTS: [--all]

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};

async fn run(db: sqlx::PgPool) -> Result<(), String> {
    let out = std::process::Command::new("uptime")
        .arg("-s")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("CalledProcessError: uptime -s".to_string());
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // local naive timestamp → UTC (psycopg2 binds naive datetimes in server tz)
    let naive = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| format!("ValueError: {e}"))?;
    let last_boot: DateTime<Utc> = Local
        .from_local_datetime(&naive)
        .single()
        .ok_or("ambiguous local time")?
        .with_timezone(&Utc);
    println!(
        "Last boot: {}",
        Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.format("%d.%m.%Y %H:%M:%S").to_string())
            .unwrap_or_else(|| s.clone())
    );

    let count = sqlx::query(
        "UPDATE teams SET vpn_connected = FALSE, vpn2_connected = FALSE, \
         wg_boxes_connected = FALSE, wg_vulnbox_connected = FALSE, vpn_last_disconnect = $1 \
         WHERE (vpn_connected OR vpn2_connected OR wg_boxes_connected OR wg_vulnbox_connected) \
         AND vpn_last_connect < $1",
    )
    .bind(last_boot)
    .execute(&db)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    println!("{count} teams were disconnected due to last reboot (and have not reconnected yet).");

    if std::env::args().any(|a| a == "--all") {
        let count =
            sqlx::query("UPDATE teams SET vpn_connection_count = 0 WHERE vpn_connection_count > 0")
                .execute(&db)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
        println!("{count} cloud connections were disconnected.");
    } else {
        println!("Use --all to reset the cloud-style connection counter");
    }
    println!("[DONE]");
    Ok(())
}

#[tokio::main]
async fn main() {
    match ctf_scripts_rs::init("reset-connection-status").await {
        Ok((_cfg, db, _redis)) => {
            if let Err(e) = run(db).await {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
