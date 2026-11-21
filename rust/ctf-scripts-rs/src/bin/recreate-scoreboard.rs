//! `scripts/recreate_scoreboard.py` (default path: full rebuild from scratch;
//! the tick-range variant stays Python for now).

use controlserver_rs::timer::{spawn_timer, CtfEvents};

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = ctf_scripts_rs::init("recreate_scoreboard.rs")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));
    // init_slave_timer(): state from redis (read-only use here)
    let timer = spawn_timer(&cfg, false, Vec::<Box<dyn CtfEvents>>::new())
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("timer: {e}")));

    let t = std::time::Instant::now();
    if let Err((msg, trace)) =
        controlserver_rs::endpoints::recreate_scoreboard_inner(&cfg, db.clone(), timer).await
    {
        eprintln!("[ERROR] Scoreboard failed: {msg}");
        if !trace.is_empty() {
            eprintln!("{trace}");
        }
        std::process::exit(1);
    }
    println!("Done, took {:.1} sec.", t.elapsed().as_secs_f64());
}
