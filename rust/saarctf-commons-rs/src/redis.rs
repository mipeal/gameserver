//! Port of `saarctf_commons/redis.py` — a shared redis connection with a
//! `CLIENT SETNAME` identity (ADR-014: verified at wave-1 service replays).

use std::sync::RwLock;

static CLIENT_NAME: RwLock<String> = RwLock::new(String::new());

/// `NamedRedisConnection.set_clientname` — name for all future connections,
/// spaces become underscores.
pub fn set_clientname(name: &str, overwrite: bool) {
    let mut n = CLIENT_NAME
        .write()
        .expect("INFALLIBLE: name lock is never poisoned while panics are aborted");
    if overwrite || n.is_empty() {
        *n = name.replace(' ', "_");
    }
}

/// `redis://[:password@]host:port/db` from the config `databases.redis` dict.
pub fn redis_url(redis_cfg: &serde_json::Value) -> Result<String, String> {
    let d = redis_cfg
        .as_object()
        .ok_or("'redis' config must be a dict")?;
    let host = d.get("host").and_then(|v| v.as_str()).unwrap_or("");
    let port = d.get("port").map(|p| p.to_string()).unwrap_or_default();
    let db = d.get("db").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok(match d.get("password").and_then(|v| v.as_str()) {
        Some(pw) if !pw.is_empty() => format!("redis://:{pw}@{host}:{port}/{db}"),
        _ => format!("redis://{host}:{port}/{db}"),
    })
}

/// `get_redis_connection` — a pooled connection (redis-rs ConnectionManager)
/// with the client name already set.
pub async fn get_redis_connection(
    redis_cfg: &serde_json::Value,
) -> Result<redis::aio::ConnectionManager, String> {
    let url = redis_url(redis_cfg)?;
    let manager = redis::aio::ConnectionManager::new(
        redis::Client::open(url.as_str()).map_err(|e| format!("RedisError: {e}"))?,
    )
    .await
    .map_err(|e| format!("RedisError: {e}"))?;
    let name = CLIENT_NAME
        .read()
        .expect("INFALLIBLE: name lock is never poisoned")
        .clone();
    if !name.is_empty() {
        let mut conn = manager.clone();
        redis::cmd("CLIENT")
            .arg("SETNAME")
            .arg(&name)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| format!("RedisError: {e}"))?;
    }
    Ok(manager)
}
