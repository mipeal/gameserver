//! `scripts/reconstruct_redis.py` — rebuild the minimal necessary redis data
//! from the ticks table. ARGUMENTS: none

use controlserver_rs::models::Tick;
use controlserver_rs::timer::{redis_set_and_publish, CtfState};
use redis::AsyncCommands;

#[tokio::main]
async fn main() {
    let (_cfg, db, mut redis) = ctf_scripts_rs::init("reconstruct_redis.rs")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));

    let ticks: Vec<Tick> = sqlx::query_as("SELECT * FROM ticks ORDER BY tick")
        .fetch_all(&db)
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("db: {e}")));
    let last_completed_tick = ticks
        .iter()
        .filter(|t| t.end.is_some())
        .map(|t| t.tick as i64)
        .max()
        .unwrap_or(0);
    let last_tick = ticks.iter().map(|t| t.tick as i64).max().unwrap_or(0);
    let state = if 0 < last_completed_tick && last_completed_tick < last_tick {
        CtfState::Suspended
    } else {
        CtfState::Stopped
    };
    let (mut current_start, mut current_end, mut estimated_time) = (None, None, None);
    if let Some(t) = ticks.iter().find(|t| t.tick as i64 == last_completed_tick) {
        if let (Some(start), Some(end)) = (t.start, t.end) {
            current_start = Some(start.timestamp());
            current_end = Some(end.timestamp());
            let dt = (end - start).num_seconds().max(0);
            estimated_time = Some(((dt as f64 / 5.0).round() as i64) * 5);
        }
    }
    let state_name = state.name();
    println!("  last tick: {last_tick}");
    println!("  last completed tick: {last_completed_tick}  <-- restore end of this tick");
    println!("  state: {state_name}");
    println!(
        "  estimated tick time: {} seconds",
        estimated_time
            .map(|t| t.to_string())
            .unwrap_or_else(|| "None".into())
    );

    async fn set(redis: &mut redis::aio::ConnectionManager, key: &str, val: Option<String>) {
        redis_set_and_publish(redis, key, val.as_deref())
            .await
            .unwrap_or_else(|e| ctf_scripts_rs::die(&format!("redis: {e}")));
    }
    set(&mut redis, "timing:state", Some(state_name.to_string())).await;
    set(
        &mut redis,
        "timing:desiredState",
        Some(state_name.to_string()),
    )
    .await;
    set(
        &mut redis,
        "timing:currentRound",
        Some(last_completed_tick.to_string()),
    )
    .await;
    set(
        &mut redis,
        "timing:roundStart",
        current_start.map(|v| v.to_string()),
    )
    .await;
    set(
        &mut redis,
        "timing:roundEnd",
        current_end.map(|v| v.to_string()),
    )
    .await;
    set(
        &mut redis,
        "timing:roundTime",
        estimated_time.map(|v| v.to_string()),
    )
    .await;
    set(&mut redis, "timing:stopAfterRound", None).await;
    set(&mut redis, "timing:startAt", None).await;

    for tick in &ticks {
        let n = tick.tick;
        if let Some(start) = tick.start {
            let _: Result<(), _> = redis
                .set::<_, _, ()>(format!("round.{n}.start"), start.timestamp())
                .await;
        }
        if let Some(end) = tick.end {
            let _: Result<(), _> = redis
                .set::<_, _, ()>(format!("round.{n}.end"), end.timestamp())
                .await;
        }
        if let (Some(start), Some(end)) = (tick.start, tick.end) {
            let _: Result<(), _> = redis
                .set::<_, _, ()>(
                    format!("round.{n}.time"),
                    end.timestamp() - start.timestamp(),
                )
                .await;
        }
    }

    println!("Done. Your checklist for now:");
    println!("1. Restart CTF Timer");
    println!("2. Check status in dashboard");
    println!("3. Set last tick");
    println!("4. Rebuild the scoreboard");
    println!("5. Continue the game");
}
