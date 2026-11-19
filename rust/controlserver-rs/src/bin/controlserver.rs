//! `controlserver/app.py create_app` + `main` — the wave-1 build-out fills in
//! timer, dispatcher, endpoints; for now: config + database bring-up.
use controlserver_rs::{connect_db, load_config};

#[tokio::main]
async fn main() {
    let cfg = load_config().expect("config");
    cfg.validate().expect("config invalid");
    let db = connect_db(&cfg).await.expect("database");
    sqlx::migrate!().run(&db).await.expect("migrations");
    println!("controlserver: config ok, db ok");
}
