//! `controlserver/app.py` (wave 1): timer + events + dispatcher wiring.
//! The HTTP endpoints are the next wave-1 piece (pages, teams, flags, ...).
use controlserver_rs::{connect_db, events, load_config};
use controlserver_rs::timer;

#[tokio::main]
async fn main() {
    let cfg = load_config().expect("config");
    cfg.validate().expect("config invalid");
    controlserver_rs::logger::setup_script_logging(None);
    let db = connect_db(&cfg).await.expect("database");
    sqlx::migrate!().run(&db).await.expect("migrations");

    let events_impl = events::DeferredCtfEvents::new(&cfg, db.clone())
        .await
        .expect("scoring/scoreboards");
    let master = !cfg.external_timer;
    let shared_timer = timer::spawn_timer(
        &cfg,
        master,
        vec![
            Box::new(events::LogCtfEvents(db.clone())),
            Box::new(events::DatabaseTickRecording(db.clone())),
            Box::new(events_impl),
        ],
    )
    .await
    .expect("timer");
    if master {
        tracing::info!("Master timer initialized");
    }

    // stay alive; the timer's tasks do the work (endpoints arrive next wave-1 step)
    let _ = shared_timer;
    tokio::signal::ctrl_c().await.ok();
}
