//! `scripts/export_scoreboard.py` — build the full scoreboard HTML export.
//! ARGUMENTS: <output-dir> [--exclude 1,2,3] [--subtract-nop]

use controlserver_rs::scoreboard::Scoreboard;
use controlserver_rs::scoring::ScoringCalculation;
use controlserver_rs::timer::{spawn_timer, CtfState, SharedTimer};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

async fn end_tick(timer: &SharedTimer) -> i64 {
    let t = timer.lock().await;
    if t.state != CtfState::Running {
        t.current_tick
    } else {
        t.current_tick - 1
    }
}

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = saarctf_scripts_rs::init("export_scoreboard.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));

    // argparse: <output> positional, --exclude, --subtract-nop
    let mut exclude: HashSet<i16> = HashSet::new();
    let mut subtract_nop = false;
    let mut output: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--exclude" => {
                let val = args.next().unwrap_or_default();
                for x in val.split(',') {
                    if let Ok(id) = x.trim().parse::<i16>() {
                        exclude.insert(id);
                    }
                }
            }
            "--subtract-nop" => subtract_nop = true,
            other => output = Some(other.to_string()),
        }
    }
    let output = output.unwrap_or_else(|| {
        saarctf_scripts_rs::die(
            "usage: export_scoreboard <output-dir> [--exclude 1,2,3] [--subtract-nop]",
        )
    });

    // init_slave_timer(): restore timing state from redis (read-only use)
    let timer = spawn_timer(
        &cfg,
        false,
        Vec::<Box<dyn controlserver_rs::timer::CtfEvents>>::new(),
    )
    .await
    .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("timer: {e}")));

    let scoring = Arc::new(
        ScoringCalculation::filtered(&db, &cfg.scoring, exclude, subtract_nop)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("ValueError: {e}"))),
    );
    let mut scoreboard = Scoreboard::new(&cfg, db, scoring, PathBuf::from(&output), false, false);

    scoreboard
        .check_scoreboard_prepared(true)
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
    scoreboard
        .update_team_info()
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));

    // allow hosting under any path
    let index = PathBuf::from(&output).join("index.html");
    let html = std::fs::read_to_string(&index)
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("reading index.html: {e}")));
    let html = html.replace("<base href=\"/\">", "");
    std::fs::write(&index, html)
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("writing index.html: {e}")));

    let mut tick_end_game = end_tick(&timer).await;
    println!("last tick: {tick_end_game}");
    // tick "-1"
    scoreboard
        .create_scoreboard(0, false, false)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
    // tick "0"
    scoreboard
        .create_scoreboard(0, true, false)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
    // tick "1"..n
    let mut rn = 1;
    while rn <= tick_end_game {
        let state = timer.lock().await.state != CtfState::Stopped;
        scoreboard
            .create_scoreboard(rn, state, rn == tick_end_game)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
        println!("- Scoreboard for tick {rn} created");
        tick_end_game = end_tick(&timer).await;
        rn += 1;
    }
}
