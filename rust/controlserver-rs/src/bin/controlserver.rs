//! `controlserver/app.py` (wave 1): timer + events + dispatcher wiring.
//! The HTTP endpoints are the next wave-1 piece (pages, teams, flags, ...).
use controlserver_rs::{connect_db, endpoints, events, load_config};
use controlserver_rs::timer;

#[tokio::main]
async fn main() {
    let cfg = load_config().expect("config");
    cfg.validate().expect("config invalid");
    controlserver_rs::logger::setup_script_logging(None);
    saarctf_commons_rs::metrics::setup_default_metrics();
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

    // HTTP API (endpoints/*.py JSON routes; HTML admin pages become the Leptos SPA)
    let state = endpoints::AppState {
        cfg: cfg.clone(),
        db: db.clone(),
        timer: shared_timer.clone(),
    };
    let http = tokio::spawn(endpoints::run_http_server(state));
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        res = http => {
            if let Ok(Err(e)) = res {
                tracing::error!("HTTP server failed: {e}");
            }
        }
    }
}
