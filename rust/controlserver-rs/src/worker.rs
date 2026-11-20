//! Port of the Celery worker side of `checker_runner/runner.py` onto Redis
//! Streams (ADR-015). N consumer tasks XREADGROUP from every `checker:*`
//! stream, run the checker, upsert the result row themselves, then XACK.
//! Revocation is the per-tick flag `dispatcher:revoke:{tick}`; in-flight tasks
//! are marked in the set `checker:running:{tick}`.

use crate::dispatcher::TaskMessage;
use crate::models::CheckerResultUpsert;
use crate::runners;
use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

const GROUP: &str = "rust-workers";
const RUNNING_TTL_SECS: i64 = 2 * 60 * 60;

/// The worker process (celery_cmd.py + the celery worker loop).
pub async fn run_worker(cfg: &Config, db: PgPool, concurrency: usize) -> Result<(), String> {
    let client = redis::Client::open(
        saarctf_commons_rs::redis::redis_url(&cfg.redis).map_err(|e| format!("redis url: {e}"))?,
    )
    .map_err(|e| format!("redis client: {e}"))?;
    let write_conn = ConnectionManager::new(client.clone())
        .await
        .map_err(|e| e.to_string())?;
    let state = Arc::new(WorkerState {
        db,
        client,
        write_conn: tokio::sync::Mutex::new(write_conn),
        streams: tokio::sync::RwLock::new(Vec::new()),
        python_dir: saarctf_commons_rs::config::python_dir(cfg),
        cfg: cfg.clone(),
        checker_packages_path: cfg.checker_packages_path.clone(),
        checker_packages_lfs: cfg.checker_packages_lfs.clone(),
    });

    // stream discovery: every checker:* stream gets a consumer group
    {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                discover_streams(&state).await;
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    // control broadcasts (celery broadcast tasks): preload_packages, run_command
    {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = control_loop(state).await {
                tracing::error!("control loop failed: {e}");
            }
        });
    }

    // N consumers, like celery's worker concurrency
    let mut handles = Vec::new();
    for i in 0..concurrency {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            let consumer = format!("{}-{i}", uuid::Uuid::new_v4());
            let mut conn = match state.client.get_connection_manager().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("worker {i}: redis connection failed: {e}");
                    return;
                }
            };
            consumer_loop(state.clone(), &mut conn, &consumer).await;
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

struct WorkerState {
    db: PgPool,
    client: redis::Client,
    write_conn: tokio::sync::Mutex<ConnectionManager>,
    streams: tokio::sync::RwLock<Vec<String>>,
    python_dir: std::path::PathBuf,
    checker_packages_path: String,
    checker_packages_lfs: Option<String>,
    cfg: saarctf_commons_rs::config::Config,
}

/// SCAN for `checker:*` streams (type filter drops the `checker:running:*`
/// sets), ensure the consumer group exists on each.
async fn discover_streams(state: &WorkerState) {
    let mut conn = state.write_conn.lock().await;
    let mut cursor = String::from("0");
    let mut found = Vec::new();
    loop {
        let (next, keys): (String, Vec<String>) = match redis::cmd("SCAN")
            .arg(&cursor)
            .arg("MATCH")
            .arg("checker:*")
            .arg("COUNT")
            .arg(100)
            .query_async(&mut *conn)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("stream scan failed: {e}");
                return;
            }
        };
        for key in keys {
            if key.starts_with("checker:running:") {
                continue;
            }
            found.push(key);
        }
        cursor = next;
        if cursor == "0" {
            break;
        }
    }
    for stream in &found {
        let _: Result<String, _> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(GROUP)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut *conn)
            .await; // BUSYGROUP → already exists, fine
    }
    *state.streams.write().await = found;
}

async fn consumer_loop(state: Arc<WorkerState>, conn: &mut ConnectionManager, consumer: &str) {
    let mut last_reclaim = std::time::Instant::now();
    loop {
        // reclaim tasks from dead consumers (celery's visibility timeout)
        if last_reclaim.elapsed() > std::time::Duration::from_secs(30) {
            last_reclaim = std::time::Instant::now();
            let streams = state.streams.read().await.clone();
            for stream in &streams {
                if let Ok((_, msgs, _)) = xautoclaim(conn, stream, consumer).await {
                    for (id, fields) in msgs {
                        handle_message(&state, conn, stream, &id, &fields).await;
                    }
                }
            }
        }
        let streams = state.streams.read().await.clone();
        if streams.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }
        let mut cmd = redis::cmd("XREADGROUP");
        cmd.arg("GROUP")
            .arg(GROUP)
            .arg(consumer)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(5000);
        cmd.arg("STREAMS");
        for s in &streams {
            cmd.arg(s);
        }
        for _ in 0..streams.len() {
            cmd.arg(">");
        }
        match cmd
            .query_async::<Option<Vec<(String, Vec<(String, HashMap<String, String>)>)>>>(conn)
            .await
        {
            Ok(Some(entries)) => {
                for (stream, msgs) in entries {
                    for (id, fields) in msgs {
                        handle_message(&state, conn, &stream, &id, &fields).await;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("XREADGROUP failed: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

/// XAUTOCLAIM stream group consumer min-idle-time start → (cursor, msgs, _deleted).
async fn xautoclaim(
    conn: &mut ConnectionManager,
    stream: &str,
    consumer: &str,
) -> Result<(String, Vec<(String, HashMap<String, String>)>, String), redis::RedisError> {
    redis::cmd("XAUTOCLAIM")
        .arg(stream)
        .arg(GROUP)
        .arg(consumer)
        .arg(60_000)
        .arg("0-0")
        .query_async(conn)
        .await
}

/// One checker task: deliver delay, revoke check, running marker, execute,
/// save the result row (workers write their own results per ADR-015), ack.
async fn handle_message(
    state: &WorkerState,
    conn: &mut ConnectionManager,
    stream: &str,
    id: &str,
    fields: &HashMap<String, String>,
) {
    let Some(msg_json) = fields.get("msg") else {
        return;
    };
    let task: TaskMessage = match serde_json::from_str(msg_json) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("unparseable task message {id}: {e}");
            ack(conn, stream, id).await;
            return;
        }
    };
    let result = execute_task(state, &task).await;

    // save (retry_on_sql_error attempts=3); on persistent failure leave the
    // message unacked so it is redelivered once the DB is back.
    let mut saved = false;
    for _ in 0..3 {
        let entry = CheckerResultUpsert {
            tick: task.tick as i32,
            team_id: task.team_id as i16,
            service_id: task.service_id as i16,
            status: result.status.clone(),
            message: result.message.clone(),
            time: Some(result.runtime),
            celery_id: task.task_id.clone(),
            output: result.output.clone(),
            run_over_time: Some(false),
            finished: Some(Utc::now()),
            data: result.data.clone().or_else(|| Some(serde_json::json!({}))),
        };
        match entry.execute(&state.db).await {
            Ok(()) => {
                saved = true;
                break;
            }
            Err(e) => {
                tracing::error!("save_checker_result failed: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }

    let _: Result<i64, _> = conn
        .srem(format!("checker:running:{}", task.tick), &task.task_id)
        .await;
    if saved {
        ack(conn, stream, id).await;
    }
}

async fn ack(conn: &mut ConnectionManager, stream: &str, id: &str) {
    let _: Result<i64, _> = redis::cmd("XACK")
        .arg(stream)
        .arg(GROUP)
        .arg(id)
        .query_async(conn)
        .await;
}

/// `checker:control` pubsub loop (Dispatcher::broadcast, celery broadcasts).
async fn control_loop(state: Arc<WorkerState>) -> Result<(), String> {
    use futures_util::StreamExt;
    let mut pubsub = state
        .client
        .get_async_pubsub()
        .await
        .map_err(|e| e.to_string())?;
    pubsub
        .subscribe(crate::dispatcher::CONTROL_CHANNEL)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = pubsub.into_on_message();
    while let Some(msg) = stream.next().await {
        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(_) => continue,
        };
        handle_control(&state, &payload).await;
    }
    Ok(())
}

/// One broadcast task on this worker: preload packages or run a command.
async fn handle_control(state: &WorkerState, payload: &str) {
    let Ok(task) = serde_json::from_str::<TaskMessage>(payload) else {
        tracing::warn!("unparseable control message");
        return;
    };
    match task.kind.as_deref() {
        Some("preload_packages") => {
            let Some(args) = task.args.as_ref().and_then(|a| a.as_array().cloned()) else {
                return;
            };
            for p in args.iter().filter_map(|v| v.as_str()) {
                println!("Preloading {p} ...");
                let folder = std::path::Path::new(&state.checker_packages_path).join(p);
                let lfs = state
                    .checker_packages_lfs
                    .as_ref()
                    .map(std::path::PathBuf::from);
                if let Err(e) = crate::db_filesystem::load_package_to_folder(
                    &state.db,
                    p,
                    &folder,
                    lfs.as_deref(),
                )
                .await
                {
                    tracing::error!("preload {p} failed: {e}");
                }
            }
            println!("Done.");
        }
        Some("run_command") => {
            let Some(cmd) = task.args.as_ref().and_then(|a| a.as_str()) else {
                return;
            };
            // run_command: shell=True, cwd=repo root, timeout 90 — result goes to
            // the celery redis result backend key so the SPA can poll it.
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(90),
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .current_dir(&state.python_dir)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output(),
            )
            .await;
            let (status, result) = match out {
                Ok(Ok(out)) if out.status.success() => (
                    "SUCCESS",
                    String::from_utf8_lossy(&out.stdout).into_owned()
                        + &String::from_utf8_lossy(&out.stderr),
                ),
                Ok(Ok(out)) => (
                    "FAILURE",
                    format!(
                        "Process failed with code {}\n{}",
                        out.status.code().unwrap_or(-1),
                        String::from_utf8_lossy(&out.stdout)
                    ) + &String::from_utf8_lossy(&out.stderr),
                ),
                Ok(Err(e)) => ("FAILURE", format!("ERROR: {e}")),
                Err(_) => ("FAILURE", "TIMEOUT".to_string()),
            };
            let meta = serde_json::json!({"status": status, "result": result});
            let mut wc = state.write_conn.lock().await;
            let _: Result<(), _> = redis::AsyncCommands::set_ex(
                &mut *wc,
                format!("celery-task-meta-{}", task.task_id),
                meta.to_string(),
                86_400,
            )
            .await;
        }
        other => tracing::warn!("unknown control kind {other:?}"),
    }
}

/// `run_checkerscript` body minus the DB write: delay, revoke check, marker, run.
struct TaskResult {
    status: String,
    message: Option<String>,
    output: Option<String>,
    data: Option<serde_json::Value>,
    runtime: f64,
}

async fn execute_task(state: &WorkerState, task: &TaskMessage) -> TaskResult {
    // celery countdown → broker-held ETA (ADR-015: worker-side delay)
    let now = crate::timer::now_secs();
    if let Some(deliver_at) = task.deliver_at {
        if deliver_at > now {
            tokio::time::sleep(std::time::Duration::from_secs((deliver_at - now) as u64)).await;
        }
    }
    // revoke-on-not-started: the row is written here with collect's exact
    // REVOKED content (celery discarded revoked tasks before they started)
    let revoked: Option<String> = {
        let mut wc = state.write_conn.lock().await;
        wc.get(format!("dispatcher:revoke:{}", task.tick))
            .await
            .unwrap_or(None)
    };
    if revoked.as_deref() == Some("1") {
        return TaskResult {
            status: "REVOKED".into(),
            message: Some("Service not checked".into()),
            output: Some("Not started before the tick ended".into()),
            data: None,
            runtime: 0.0,
        };
    }

    let start = std::time::Instant::now();
    // task_track_started equivalent: collect maps a still-running task to TIMEOUT
    let running_key = format!("checker:running:{}", task.tick);
    {
        let mut wc = state.write_conn.lock().await;
        let _: Result<i64, _> = wc.sadd(&running_key, &task.task_id).await;
        let _: Result<i64, _> = redis::cmd("EXPIRE")
            .arg(&running_key)
            .arg(RUNNING_TTL_SECS)
            .query_async(&mut *wc)
            .await;
    }
    let result = if task.runner_spec.starts_with("eno:") {
        crate::eno::run_eno_checker(task, &state.cfg).await
    } else {
        runners::run_checker(task, &state.python_dir).await
    };

    TaskResult {
        status: result.status,
        message: result.message,
        output: result.output,
        data: result.data,
        runtime: start.elapsed().as_secs_f64(),
    }
}
