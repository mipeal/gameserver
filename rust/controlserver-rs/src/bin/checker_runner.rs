//! `checker_runner/main.py` + `celery_cmd.py` — the Rust worker: config, DB,
//! then the Redis Streams consumer loop (ADR-015).
use controlserver_rs::{connect_db, load_config, worker};

#[tokio::main]
async fn main() {
    let cfg = load_config().expect("config");
    cfg.validate().expect("config invalid");
    controlserver_rs::logger::setup_script_logging(None);
    let db = connect_db(&cfg).await.expect("database");
    sqlx::migrate!().run(&db).await.expect("migrations");
    let concurrency = std::env::var("CTF_RUNNER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });
    worker::run_worker(&cfg, db, concurrency)
        .await
        .expect("worker");
}
