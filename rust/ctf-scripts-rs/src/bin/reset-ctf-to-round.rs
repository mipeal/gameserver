//! `scripts/reset_ctf_to_round.py` — wipe the CTF after a tick.
//! ARGUMENTS: tick [--force]

use controlserver_rs::scoreboard::{default_scoreboards, Scoreboard};
use controlserver_rs::scoring::ScoringCalculation;
use controlserver_rs::timer::{redis_set_and_publish, spawn_timer, CtfEvents};
use redis::AsyncCommands;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(tick_str) = args.get(1) else {
        println!("USAGE: reset-ctf-to-round <tick>");
        return;
    };
    let tick: i64 = tick_str
        .parse()
        .unwrap_or_else(|_| ctf_scripts_rs::die("tick must be an integer"));
    let force = args.iter().any(|a| a == "--force");

    let (cfg, db, mut redis) = ctf_scripts_rs::init("reset_ctf_to_round.rs")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));
    let timer = spawn_timer(&cfg, false, Vec::<Box<dyn CtfEvents>>::new())
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("timer: {e}")));

    if !force
        && !ctf_scripts_rs::query_yes_no(
            &format!("Do you really want to wipe the whole CTF after tick {tick}?"),
            "no",
        )
    {
        return;
    }

    if timer.lock().await.t.state == controlserver_rs::timer::CtfState::Running {
        println!("CTF must not be running.");
        return;
    }
    if ctf_scripts_rs::count_master_timer(&mut redis)
        .await
        .unwrap_or(0)
        > 0
    {
        println!("Please stop all master timers.");
        return;
    }

    println!("Wiping redis ...");
    // timing:currentRound = tick (set + publish)
    redis_set_and_publish(&mut redis, "timing:currentRound", Some(&tick.to_string()))
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("redis: {e}")));
    let keys: Vec<String> = redis
        .keys("services:*")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("redis: {e}")));
    let mut wiped = 0;
    for key in keys {
        // Python: int(key.split(b":")[3]) — the 4th colon-separated field
        let keep = key
            .split(':')
            .nth(3)
            .and_then(|t| t.parse::<i64>().ok())
            .map(|t| t <= tick)
            .unwrap_or(true);
        if !keep {
            let _: Result<i64, _> = redis.del(&key).await;
            wiped += 1;
        }
    }
    println!("Wiped {wiped} keys.");

    println!("Wiping database ...");
    for table in ["team_points", "team_ranking", "checker_results", "ticks"] {
        let count = sqlx::query(&format!("DELETE FROM {table} WHERE tick > $1"))
            .bind(tick as i32)
            .execute(&db)
            .await
            .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("db: {e}")))
            .rows_affected();
        println!("- dropped {count} entries from {table}");
    }
    let count = sqlx::query("DELETE FROM submitted_flags WHERE tick_submitted > $1")
        .bind(tick as i16)
        .execute(&db)
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("db: {e}")))
        .rows_affected();
    println!("- dropped {count} entries from submitted_flags");

    // scoreboard
    let scoring = ScoringCalculation::new(&db, &cfg.scoring)
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("ValueError: {e}")));
    let mut scoreboards: Vec<Scoreboard> =
        default_scoreboards(&cfg, db.clone(), std::sync::Arc::new(scoring), false);
    for sb in scoreboards.iter_mut() {
        let path = sb.output.join("api");
        if !path.exists() {
            continue;
        }
        let entries = std::fs::read_dir(&path)
            .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("fs: {e}")));
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("scoreboard_round_") && name.ends_with(".json") {
                let num: i64 = name[17..name.len() - 5].parse().unwrap_or(0);
                if num > tick {
                    std::fs::remove_file(entry.path())
                        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("fs: {e}")));
                    println!("- deleted {}", entry.path().to_string_lossy());
                }
            }
        }
        // Python also rglob()s, so team files can live in subdirectories
        for dir in [path.clone()] {
            let entries = walkdir_shallow(&dir);
            for file in entries {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with("scoreboard_team_") && name.ends_with(".json") {
                    let _ = std::fs::remove_file(&file);
                }
            }
        }
        let _ = std::fs::remove_file(path.join("scoreboard_service_stats.json"));
        let scoreboard_tick = sb
            .update_tick_info(None)
            .await
            .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("scoreboard: {e}")));
        sb.reset_to_tick(std::cmp::min(scoreboard_tick, tick))
            .await
            .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("scoreboard: {e}")));
    }

    println!("Done. I suggest restarting other components.");
}

/// one-level recursive listing (Python's `path.rglob` used for team json files)
fn walkdir_shallow(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(walkdir_shallow(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
