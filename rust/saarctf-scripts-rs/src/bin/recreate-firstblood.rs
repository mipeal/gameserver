//! `scripts/recreate_firstblood.py` — drop and recompute all first-blood
//! markings. Restart other scoring calculators afterwards.

use controlserver_rs::scoring::ScoringCalculation;

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = saarctf_scripts_rs::init("recreate_firstblood.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let scoring = ScoringCalculation::new(&db, &cfg.scoring)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("ValueError: {e}")));
    println!("Recomputing first blood flags now, might take some time ...");
    let t = std::time::Instant::now();
    scoring
        .recompute_first_blood_flags(&db)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("scoring: {e}")));
    println!("First Blood: {:.1} sec", t.elapsed().as_secs_f64());
    println!("Done. Please restart the CTF Timer now (it needs a fresh cache)");
}
