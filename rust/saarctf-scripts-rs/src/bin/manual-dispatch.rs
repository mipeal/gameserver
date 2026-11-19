//! `scripts/manual_dispatch.py` — dispatch the checker scripts for a tick.
//! ARGUMENTS: tick (optional, default: current)

#[tokio::main]
async fn main() {
    let (cfg, db, mut redis) = saarctf_scripts_rs::init("manual_dispatch.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let tick: i64 = match std::env::args().nth(1) {
        Some(t) => t
            .parse()
            .unwrap_or_else(|_| saarctf_scripts_rs::die("tick must be an integer")),
        None => {
            let timing = controlserver_rs::timer::read_timing_state(&cfg)
                .await
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("timer: {e}")));
            timing.current_tick
        }
    };

    let t = std::time::Instant::now();
    let dispatcher = controlserver_rs::dispatcher::Dispatcher::from_config(&cfg);
    let timing = controlserver_rs::timer::read_timing_state(&cfg)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("timer: {e}")));
    dispatcher
        .dispatch_checker_scripts(&db, &mut redis, &timing, tick)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("dispatch: {e}")));
    println!(
        "Checker scripts for tick {} dispatched. Took {:.1} sec",
        tick,
        t.elapsed().as_secs_f64()
    );
}
