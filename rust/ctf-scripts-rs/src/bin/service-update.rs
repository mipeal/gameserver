//! `scripts/service_update.py` — clone/update service repositories and update
//! metadata in the database. ARGUMENTS: none

use controlserver_rs::service_mgr::ServiceRepoManager;

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = ctf_scripts_rs::init("service_update.rs")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));

    let mut mgr = ServiceRepoManager::default();
    mgr.update_all_services(&cfg, &db)
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));
    for msg in &mgr.messages {
        println!("{msg}");
    }
    println!("[OK] Service update complete");
    println!("     You might want to update the checker scripts in UI.");
}
