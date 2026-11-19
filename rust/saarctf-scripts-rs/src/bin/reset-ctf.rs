//! `scripts/reset_ctf.py` — wipe redis, broker (same redis under ADR-015),
//! database and scoreboard outputs. ARGUMENTS: --force skips the prompt.

use controlserver_rs::scoreboard::{default_scoreboards, Scoreboard};
use controlserver_rs::scoring::ScoringCalculation;
use controlserver_rs::timer::{spawn_timer, CtfEvents};

async fn reset_redis(cfg: &saarctf_commons_rs::config::Config) -> Result<(), String> {
    let mut conn = saarctf_commons_rs::redis::get_redis_connection(&cfg.redis)
        .await
        .map_err(|e| e.to_string())?;
    redis::cmd("FLUSHDB")
        .query_async::<()>(&mut conn)
        .await
        .map_err(|e| e.to_string())?;
    // update without init - write default values
    let timer = spawn_timer(cfg, false, Vec::<Box<dyn CtfEvents>>::new())
        .await
        .map_err(|e| e.to_string())?;
    let mut t = timer.lock().await;
    t.on_update_times().await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn reset_database(db: &sqlx::PgPool, include_storage: bool) -> Result<(), String> {
    for table in [
        "team_points",
        "team_ranking",
        "submitted_flags",
        "checker_results",
        "log_messages",
        "ticks",
    ] {
        let count = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(db)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
        println!("- dropped {count} entries from {table}");
    }
    if include_storage {
        for table in ["checker_files", "checker_filesystem"] {
            let count = sqlx::query(&format!("DELETE FROM {table}"))
                .execute(db)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
            println!("- dropped {count} entries from {table}");
        }
        sqlx::query("UPDATE services SET package=NULL")
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn reset_scoreboard(
    cfg: &saarctf_commons_rs::config::Config,
    db: sqlx::PgPool,
) -> Result<(), String> {
    let scoring = ScoringCalculation::new(&db, &cfg.scoring)
        .await
        .map_err(|e| format!("ValueError: {e}"))?;
    let scoreboards: Vec<Scoreboard> =
        default_scoreboards(cfg, db, std::sync::Arc::new(scoring), false);
    for scoreboard in &scoreboards {
        let path = scoreboard.output.join("api");
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let force = std::env::args().any(|a| a == "--force");
    if !force
        && !saarctf_scripts_rs::query_yes_no("Do you really want to wipe the whole CTF?", "no")
    {
        return;
    }
    let (cfg, db, _redis) = saarctf_scripts_rs::init("reset_ctf.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));

    println!("Wiping redis ...");
    reset_redis(&cfg)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("redis: {e}")));
    // broker: ADR-015 runs the pipeline on the same redis instance, already flushed
    println!("Wiping broker ...");
    println!("Wiping database ...");
    reset_database(&db, false)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
    reset_scoreboard(&cfg, db)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
    println!("Done. I suggest restarting other components.");
}
