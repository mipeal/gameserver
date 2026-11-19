//! Port of `vpn/manage-trafficstats.py` — periodically writes network
//! statistics (from a tc/bpf `counting_map`) into the database.
//! ctypes.CDLL("libbpf.so") becomes runtime dlopen via `libloading`, so this
//! binary also builds where libbpf.so is not installed.

use chrono::TimeZone;
use libloading::{Library, Symbol};
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

// how often you want results. Should be 60 for now, and must be a divisor or multiple of 60
const TICK_TIME: i64 = 60;
const BPF_RETRY_COUNT: i32 = 100;
const MAP_PATH: &str = "/sys/fs/bpf/tc/globals/counting_map";

static BPF_RETRY: AtomicI32 = AtomicI32::new(BPF_RETRY_COUNT);

struct Bpf {
    lib: Library,
}

unsafe impl Send for Bpf {}

impl Bpf {
    /// SAFETY: dlopen of libbpf.so at runtime; symbols are plain C functions.
    unsafe fn open() -> Result<Bpf, String> {
        let lib = Library::new("libbpf.so").map_err(|e| e.to_string())?;
        Ok(Bpf { lib })
    }

    /// SAFETY: `bpf_obj_get(const char*)` returns an fd or -1.
    unsafe fn bpf_obj_get(&self, path: *const std::os::raw::c_char) -> RawFd {
        let f: Symbol<unsafe extern "C" fn(*const std::os::raw::c_char) -> RawFd> = self
            .lib
            .get(b"bpf_obj_get\0")
            .expect("INFALLIBLE: libbpf.so exports bpf_obj_get");
        f(path)
    }

    /// SAFETY: `bpf_map_lookup_elem(fd, key, value)` writes into `value`.
    unsafe fn bpf_map_lookup_elem(&self, fd: RawFd, key: *const i32, value: *mut i64) -> i32 {
        let f: Symbol<
            unsafe extern "C" fn(
                RawFd,
                *const std::os::raw::c_void,
                *mut std::os::raw::c_void,
            ) -> i32,
        > = self
            .lib
            .get(b"bpf_map_lookup_elem\0")
            .expect("INFALLIBLE: libbpf.so exports bpf_map_lookup_elem");
        f(fd, key as *const _, value as *mut _)
    }

    /// SAFETY: `bpf_map_update_elem(fd, key, value, flags)`.
    unsafe fn bpf_map_update_elem(
        &self,
        fd: RawFd,
        key: *const i32,
        value: *const i32,
        flags: u64,
    ) -> i32 {
        let f: Symbol<
            unsafe extern "C" fn(
                RawFd,
                *const std::os::raw::c_void,
                *const std::os::raw::c_void,
                u64,
            ) -> i32,
        > = self
            .lib
            .get(b"bpf_map_update_elem\0")
            .expect("INFALLIBLE: libbpf.so exports bpf_map_update_elem");
        f(fd, key as *const _, value as *const _, flags)
    }
}

/// BPF_ANY from /usr/include/linux/bpf.h (the Python fallback when the
/// libbpf.so export is missing)
const BPF_ANY: u64 = 0;

fn open_bpfmap(bpf: &Bpf, fname: &str) -> Result<RawFd, String> {
    let cpath = std::ffi::CString::new(fname).map_err(|e| e.to_string())?;
    let mut fd = unsafe { bpf.bpf_obj_get(cpath.as_ptr()) };
    while fd < 0 && BPF_RETRY.load(Ordering::Relaxed) > 0 {
        tracing::info!(
            "[-] BPF map {:?} not available, retry later ...",
            fname.rsplit('/').next().unwrap_or(fname)
        );
        BPF_RETRY.fetch_sub(1, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_secs(10));
        fd = unsafe { bpf.bpf_obj_get(cpath.as_ptr()) };
    }
    if fd < 0 {
        return Err("Could not open BPF map".to_string());
    }
    Ok(fd)
}

/// value_list = [down_game, down_teams, up_game(corrected), up_teams, forward_self] × (packets, bytes, syns, syn_acks)
fn read_team_infos(
    bpf: &Bpf,
    fd: RawFd,
    team_ids: &[i32],
) -> Result<HashMap<i32, Vec<i64>>, String> {
    let mut result = HashMap::new();
    for &team_id in team_ids {
        let mut value = [0i64; 20];
        // SAFETY: value is a valid 20×i64 buffer; key points to team_id
        if unsafe { bpf.bpf_map_lookup_elem(fd, &team_id, value.as_mut_ptr()) } != 0 {
            return Err(format!("Key {team_id} not found"));
        }
        // correct "total" entry
        value[8] -= value[12] + value[16];
        value[9] -= value[13] + value[17];
        value[10] -= value[14] + value[18];
        value[11] -= value[15] + value[19];
        result.insert(team_id, value.to_vec());
    }
    Ok(result)
}

async fn wait_for_next_tick(offset: f64) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
        % TICK_TIME as f64;
    let mut waittime = TICK_TIME as f64 - (ts % TICK_TIME as f64);
    if offset < 0.0 && waittime - offset > TICK_TIME as f64 {
        return;
    }
    if waittime < offset {
        waittime += TICK_TIME as f64;
    }
    tracing::info!("...  next action in {waittime:.1} sec");
    tokio::time::sleep(std::time::Duration::from_secs_f64(waittime)).await;
}

async fn efficient_insert(
    db: &PgPool,
    timestamp: i64,
    items: &HashMap<i32, Vec<i64>>,
) -> Result<(), sqlx::Error> {
    let mut teams: Vec<i32> = Vec::with_capacity(items.len());
    let mut v: Vec<Vec<i64>> = (0..20).map(|_| Vec::with_capacity(items.len())).collect();
    for (team_id, values) in items {
        teams.push(*team_id);
        for i in 0..20 {
            v[i].push(values[i]);
        }
    }
    sqlx::query(
        "INSERT INTO team_traffic_stats (\"time\", team_id, \
         down_game_packets, down_game_bytes, down_game_syns, down_game_syn_acks, \
         down_teams_packets, down_teams_bytes, down_teams_syns, down_teams_syn_acks, \
         up_game_packets, up_game_bytes, up_game_syns, up_game_syn_acks, \
         up_teams_packets, up_teams_bytes, up_teams_syns, up_teams_syn_acks, \
         forward_self_packets, forward_self_bytes, forward_self_syns, forward_self_syn_acks) \
         SELECT to_timestamp($1), unnest($2), \
         unnest($3), unnest($4), unnest($5), unnest($6), \
         unnest($7), unnest($8), unnest($9), unnest($10), \
         unnest($11), unnest($12), unnest($13), unnest($14), \
         unnest($15), unnest($16), unnest($17), unnest($18), \
         unnest($19), unnest($20), unnest($21), unnest($22)",
    )
    .bind(timestamp)
    .bind(&teams)
    .bind(&v[0])
    .bind(&v[1])
    .bind(&v[2])
    .bind(&v[3])
    .bind(&v[4])
    .bind(&v[5])
    .bind(&v[6])
    .bind(&v[7])
    .bind(&v[8])
    .bind(&v[9])
    .bind(&v[10])
    .bind(&v[11])
    .bind(&v[12])
    .bind(&v[13])
    .bind(&v[14])
    .bind(&v[15])
    .bind(&v[16])
    .bind(&v[17])
    .bind(&v[18])
    .bind(&v[19])
    .execute(db)
    .await
    .map(|_| ())
}

async fn save_difference(
    db: &PgPool,
    timestamp: i64,
    new_results: &HashMap<i32, Vec<i64>>,
    last_minute_results: &HashMap<i32, Vec<i64>>,
    skip_diff: bool,
) -> Result<(), String> {
    let mut stuff_to_save: HashMap<i32, Vec<i64>> = HashMap::new();
    if !skip_diff {
        for (team_id, new_values) in new_results {
            if let Some(old_values) = last_minute_results.get(team_id) {
                let diff: Vec<i64> = new_values
                    .iter()
                    .zip(old_values.iter())
                    .map(|(n, o)| n - o)
                    .collect();
                stuff_to_save.insert(*team_id, diff);
            }
        }
    }
    efficient_insert(db, timestamp, &stuff_to_save)
        .await
        .map_err(|e| e.to_string())?;
    check_for_suspicious_numbers(db, new_results, last_minute_results).await;
    Ok(())
}

async fn check_for_suspicious_numbers(
    db: &PgPool,
    new_results: &HashMap<i32, Vec<i64>>,
    traffic_last_tick: &HashMap<i32, Vec<i64>>,
) {
    for (team_id, traffic) in traffic_last_tick {
        let Some(new_traffic) = new_results.get(team_id) else {
            continue;
        };
        // check for too much team upload (=VPN ingress)
        let team_upload = new_traffic[9] + new_traffic[13] - traffic[9] - traffic[13];
        if team_upload > 5_500_000 * TICK_TIME {
            let _ = controlserver_rs::logger::log(
                db,
                "traffic-control",
                &format!(
                    "Team #{team_id} uploaded too much last minute ({:.1} MB)",
                    team_upload as f64 / 1_000_000.0
                ),
                "",
                controlserver_rs::models::LOG_IMPORTANT,
            )
            .await;
        }
    }
}

fn write_port_numbers(bpf: &Bpf, bpf_map: &str, ports: &[(i32, i32)]) -> Result<(), String> {
    let fd = open_bpfmap(bpf, bpf_map)?;
    let result = (|| -> Result<(), String> {
        let mut entries: Vec<i32> = ports.iter().map(|(sid, p)| (sid << 16) | p).collect();
        entries.push(0);
        assert!(entries.len() <= 20);
        for (i, &entry) in entries.iter().enumerate() {
            // SAFETY: both point to valid locals
            if unsafe { bpf.bpf_map_update_elem(fd, &(i as i32), &entry, BPF_ANY) } != 0 {
                return Err(format!("cannot update port array element {i}"));
            }
        }
        Ok(())
    })();
    // SAFETY: fd from bpf_obj_get
    unsafe { libc_close(fd) };
    result
}

extern "C" {
    #[link_name = "close"]
    fn libc_close(fd: RawFd) -> i32;
}

async fn update_service_ports(bpf: &Bpf, db: &PgPool) -> Result<(), String> {
    let services: Vec<(i32, String)> = sqlx::query_as("SELECT id, ports FROM services")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    let mut tcp_ports = Vec::new();
    let mut udp_ports = Vec::new();
    for (id, ports) in services {
        for (proto, port) in
            controlserver_rs::models::parse_ports_str(&ports).map_err(|e| e.to_string())?
        {
            if proto == "tcp" {
                tcp_ports.push((id, port));
            } else if proto == "udp" {
                udp_ports.push((id, port));
            }
        }
    }
    write_port_numbers(bpf, "/sys/fs/bpf/tc/globals/service_ports_tcp", &tcp_ports)?;
    write_port_numbers(bpf, "/sys/fs/bpf/tc/globals/service_ports_udp", &udp_ports)?;
    tracing::info!("[OK] Wrote service ports to bpf maps");
    Ok(())
}

fn round_tick(t: f64) -> i64 {
    (t / TICK_TIME as f64).round() as i64 * TICK_TIME
}

fn floor_tick(t: f64) -> i64 {
    (t / TICK_TIME as f64).floor() as i64 * TICK_TIME
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedState {
    last_minute_results: HashMap<String, Vec<i64>>,
    old_vpn_status: bool,
    last_minute_tick: i64,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, db, redis_conn) = saarctf_scripts_rs::init("manage-trafficstats").await?;
    let bpf = unsafe { Bpf::open()? };

    update_service_ports(&bpf, &db).await?;
    let client = redis::Client::open(
        saarctf_commons_rs::redis::redis_url(&cfg.redis).map_err(|e| format!("redis url: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let mut conn = redis_conn;
    let fd = open_bpfmap(&bpf, MAP_PATH)?;

    let result = run_loop(&cfg, &db, &bpf, fd, &mut conn, &client).await;
    // SAFETY: fd from bpf_obj_get
    unsafe { libc_close(fd) };
    result
}

async fn run_loop(
    _cfg: &Config,
    db: &PgPool,
    bpf: &Bpf,
    fd: RawFd,
    conn: &mut redis::aio::ConnectionManager,
    _client: &redis::Client,
) -> Result<(), String> {
    let state_on: bool = matches!(
        redis::cmd("GET").arg("network:state").query_async::<Option<String>>(conn).await,
        Ok(Some(s)) if s == "on"
    );
    if !state_on {
        tracing::info!("[-]  VPN is offline, stats paused.");
    }

    let (mut last_minute_results, mut old_vpn_status, mut last_minute_tick): (
        HashMap<i32, Vec<i64>>,
        bool,
        i64,
    );
    // After a disk round-trip the Python dict has string keys, so the first
    // save finds no old values and stores nothing (dict.get(int) → None).
    let mut resume_miss = false;
    let mut loaded: Option<SavedState> = None;
    if std::path::Path::new("/tmp/vpn-stats-state.json").exists() {
        if let Ok(text) = std::fs::read_to_string("/tmp/vpn-stats-state.json") {
            if let Ok(data) = serde_json::from_str::<SavedState>(&text) {
                loaded = Some(data);
            }
        }
    }
    match loaded {
        Some(data) if data.last_minute_tick == floor_tick(now_secs()) => {
            // We have data from last minute on disk, read it
            last_minute_results = data
                .last_minute_results
                .into_iter()
                .map(|(k, v)| (k.parse::<i32>().unwrap_or(0), v))
                .collect();
            old_vpn_status = data.old_vpn_status;
            last_minute_tick = data.last_minute_tick;
            resume_miss = true;
            tracing::info!(
                "[OK] Initialized from saved state, {} teams.",
                last_minute_results.len()
            );
        }
        _ => {
            // Initially wait for the next minute to start
            wait_for_next_tick(-1.0).await;

            // Initially fill the cache
            let team_ids: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM teams WHERE vpn_last_connect IS NOT NULL")
                    .fetch_all(db)
                    .await
                    .map_err(|e| e.to_string())?;
            last_minute_results = read_team_infos(bpf, fd, &team_ids)?;
            old_vpn_status = matches!(
                redis::cmd("GET").arg("network:state").query_async::<Option<String>>(conn).await,
                Ok(Some(s)) if s == "on"
            );
            last_minute_tick = round_tick(now_secs());
            tracing::info!("[OK] Initialized, {} teams.", team_ids.len());
        }
    }

    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;

    loop {
        let waittime = wait_seconds(1.0);
        tracing::info!("...  next action in {waittime:.1} sec");
        let sleep = tokio::time::sleep_until(
            tokio::time::Instant::now() + std::time::Duration::from_secs_f64(waittime),
        );
        tokio::select! {
            _ = sleep => {}
            _ = sigint.recv() => {
                // KeyboardInterrupt: save state and terminate
                let saved = SavedState {
                    last_minute_results: last_minute_results
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect(),
                    old_vpn_status,
                    last_minute_tick,
                };
                if let Ok(json) = serde_json::to_string(&saved) {
                    let _ = std::fs::write("/tmp/vpn-stats-state.json", json);
                }
                tracing::info!("[Terminating]");
                return Ok(());
            }
        }
        let mut t = now_secs();
        let t_display = chrono::Utc
            .timestamp_opt(t as i64, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();

        // 1. Retrieve network status
        tracing::info!("...  Network status at {t_display}");
        let team_ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM teams WHERE vpn_last_connect IS NOT NULL")
                .fetch_all(db)
                .await
                .map_err(|e| e.to_string())?;
        let new_results = read_team_infos(bpf, fd, &team_ids)?;
        let new_vpn_status = matches!(
            redis::cmd("GET").arg("network:state").query_async::<Option<String>>(conn).await,
            Ok(Some(s)) if s == "on"
        );

        // 2. If network was enabled in between take stats
        if old_vpn_status || new_vpn_status {
            save_difference(
                db,
                round_tick(t),
                &new_results,
                &last_minute_results,
                resume_miss,
            )
            .await?;
            // NOTE: the Python original reassigns `t` to the elapsed duration
            // here — so `last_minute_tick` below is computed from the duration
            // (kept bug-compatible)
            t = now_secs() - t;
            let duration = t;
            tracing::info!(
                "[OK] Saved stats of {} teams in {duration:.3} seconds.",
                team_ids.len()
            );
        } else {
            tracing::info!("[-]  VPN is offline, no stats taken.");
        }
        resume_miss = false;

        // 3. Save current results as base for next tick
        last_minute_results = new_results;
        old_vpn_status = new_vpn_status;
        last_minute_tick = round_tick(t);
    }
}

/// The sleep part of `wait_for_next_tick` (logging happens in the Python one;
/// here the loop logs nothing extra) — waittime = TICK - ts + optional TICK.
fn wait_seconds(offset: f64) -> f64 {
    let ts = now_secs() % TICK_TIME as f64;
    let mut waittime = TICK_TIME as f64 - ts;
    if waittime < offset {
        waittime += TICK_TIME as f64;
    }
    waittime
}
