//! `scripts/recreate_ranking.py` — recreate points/ranking from checker
//! results and submitted_flags. ARGUMENTS: start_tick end_tick (inclusive,
//! optional) ; --scoreboard recreates the scoreboard after each tick.

use controlserver_rs::scoreboard::default_scoreboards;
use controlserver_rs::scoring::ScoringCalculation;
use controlserver_rs::timer::{spawn_timer, CtfState, SharedTimer};
use sqlx::PgPool;
use std::sync::Arc;

async fn end_tick(timer: &SharedTimer) -> i64 {
    let t = timer.lock().await;
    if t.t.state != CtfState::Running {
        t.t.current_tick
    } else {
        t.t.current_tick - 1
    }
}

async fn recreate_ranking(
    cfg: &saarctf_commons_rs::config::Config,
    db: &PgPool,
    timer: &SharedTimer,
    tick_start: i64,
    tick_end: Option<i64>,
    refresh_scoreboard: bool,
) -> Result<i64, String> {
    let scoring = Arc::new(ScoringCalculation::new(db, &cfg.scoring).await?);
    let mut scoreboards = default_scoreboards(cfg, db.clone(), scoring.clone(), false);
    let tick_end_game = end_tick(timer).await;
    if refresh_scoreboard && tick_start <= 1 && tick_end_game > 0 {
        for sb in scoreboards.iter_mut() {
            sb.create_scoreboard(0, false, false).await?;
            sb.create_scoreboard(0, true, false).await?;
        }
    }
    let mut rn = tick_start;
    while rn <= tick_end.unwrap_or(tick_end_game) {
        let ts = std::time::Instant::now();
        // Remove old points/ranking from DB
        sqlx::query("DELETE FROM team_points WHERE tick=$1")
            .bind(rn as i32)
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM team_ranking WHERE tick=$1")
            .bind(rn as i32)
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;

        scoring.calculate_scoring_for_tick(db, rn).await?;
        scoring.calculate_ranking_per_tick(db, rn).await?;
        if refresh_scoreboard {
            let tick_end_game = end_tick(timer).await;
            let state = { timer.lock().await.t.state };
            for sb in scoreboards.iter_mut() {
                sb.create_scoreboard(rn, state != CtfState::Stopped, rn == tick_end_game)
                    .await?;
            }
        }
        println!(
            "- Round {rn} recalculated in {:.2} seconds",
            ts.elapsed().as_secs_f64()
        );
        rn += 1;
    }
    Ok(rn - 1)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (tick_start, tick_end) = if args.len() <= 2 {
        (1, None)
    } else {
        let start: i64 = args[1]
            .parse()
            .unwrap_or_else(|_| saarctf_scripts_rs::die("start tick must be an integer"));
        let end: Option<i64> = if args[2] == "current" {
            None
        } else {
            Some(
                args[2]
                    .parse()
                    .unwrap_or_else(|_| saarctf_scripts_rs::die("end tick must be an integer")),
            )
        };
        (start, end)
    };
    let scoreboard = args.iter().any(|a| a == "--scoreboard");

    let (cfg, db, _redis) = saarctf_scripts_rs::init("recreate_ranking.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let timer = spawn_timer(
        &cfg,
        false,
        Vec::<Box<dyn controlserver_rs::timer::CtfEvents>>::new(),
    )
    .await
    .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("timer: {e}")));

    let t = std::time::Instant::now();
    println!(
        "Recreating ranking from tick {tick_start} to tick {}...",
        tick_end
            .map(|t| t.to_string())
            .unwrap_or("<current>".into())
    );
    let tick = recreate_ranking(&cfg, &db, &timer, tick_start, tick_end, scoreboard)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoring: {e}")));
    println!("Done, took {:.1} sec.", t.elapsed().as_secs_f64());
    if !scoreboard {
        println!("Suggestion: > recreate-scoreboard.rs");
    }
    let _ = tick;
}
