//! `controlserver/scoring/scoreboard_process.py` — the scoreboard daemon.
use controlserver_rs::{connect_db, load_config, scoreboard};

#[tokio::main]
async fn main() {
    let cfg = load_config().expect("config");
    cfg.validate().expect("config invalid");
    controlserver_rs::logger::setup_script_logging(None);
    let db = connect_db(&cfg).await.expect("database");
    sqlx::migrate!().run(&db).await.expect("migrations");
    scoreboard::run_scoreboard_generator(&cfg, db)
        .await
        .expect("scoreboard generator");
}
