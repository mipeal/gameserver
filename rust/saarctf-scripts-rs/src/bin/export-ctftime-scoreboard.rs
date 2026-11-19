//! `scripts/export_ctftime_scoreboard.py` — write the CTFinel JSON export.
//! ARGUMENTS: [output-file] (default: stdout) [--exclude 1,2,3] [--subtract-nop]

use controlserver_rs::scoreboard::Scoreboard;
use controlserver_rs::scoring::ScoringCalculation;
use controlserver_rs::timer::{spawn_timer, CtfState};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = saarctf_scripts_rs::init("export_ctftime_scoreboard.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));

    // argparse: [output] positional ('-' = stdout), --exclude, --subtract-nop
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
            other if output.is_none() => output = Some(other.to_string()),
            _ => {}
        }
    }

    // init_slave_timer(): restore timing state from redis (read-only use)
    let timer = spawn_timer(
        &cfg,
        false,
        Vec::<Box<dyn controlserver_rs::timer::CtfEvents>>::new(),
    )
    .await
    .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("timer: {e}")));

    // always exclude NOP team
    if let Some(nop) = cfg.scoring.nop_team_id.as_i64() {
        exclude.insert(nop as i16);
    }
    let scoring = Arc::new(
        ScoringCalculation::filtered(&db, &cfg.scoring, exclude, subtract_nop)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("ValueError: {e}"))),
    );
    let scoreboard = Scoreboard::new(
        &cfg,
        db,
        scoring,
        PathBuf::from(&cfg.scoreboard_path),
        false,
        false,
    );

    let tick = {
        let t = timer.lock().await;
        if t.state != CtfState::Running {
            t.current_tick
        } else {
            t.current_tick - 1
        }
    };
    let data = scoreboard
        .create_ctftime_json(tick)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoreboard: {e}")));
    match output {
        Some(fname) if fname != "-" => {
            let fname = std::env::current_dir()
                .map(|d| d.join(&fname))
                .unwrap_or_else(|_| PathBuf::from(&fname));
            std::fs::write(&fname, data)
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("writing {fname:?}: {e}")));
            println!("Saved scoreboard to \"{}\".", fname.display());
        }
        _ => println!("{data}"),
    }
}
