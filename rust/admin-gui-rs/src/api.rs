//! JSON fetch helpers — everything is `serde_json::Value` (the server shape is
//! ported 1:1; no point re-modeling it client-side).

use leptos::prelude::*;
use serde_json::Value;

pub async fn get_json(url: &str) -> Result<Value, String> {
    gloo_net::http::Request::get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("GET {url}: bad JSON: {e}"))
}

/// POST with a JSON body; returns response text (server answers plain text for
/// most form-style endpoints).
pub async fn post_json(url: &str, body: Value) -> Result<String, String> {
    let resp = gloo_net::http::Request::post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .map_err(|e| format!("POST {url}: {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        Ok(text)
    } else {
        Err(format!("{}: {}", resp.status(), text))
    }
}

/// POST as x-www-form-urlencoded (for axum `Form` handlers, e.g. /services/checker_status).
pub async fn post_form(url: &str, body: &str) -> Result<String, String> {
    let resp = gloo_net::http::Request::post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body.to_string())
        .map_err(|e| format!("POST {url}: {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        Ok(text)
    } else {
        Err(format!("{}: {}", resp.status(), text))
    }
}

pub async fn post_raw(url: &str, body: Vec<u8>) -> Result<String, String> {
    let resp = gloo_net::http::Request::post(url)
        .body(body)
        .map_err(|e| format!("POST {url}: {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if resp.ok() {
        Ok(text)
    } else {
        Err(format!("{}: {}", resp.status(), text))
    }
}

pub fn s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn s_opt(v: &Value) -> String {
    if v.is_null() { "—".to_string() } else { s(v) }
}

pub fn ts(v: &Value) -> String {
    // unix seconds → local-ish display (UTC)
    v.as_f64()
        .and_then(|t| chrono_dash(t as i64))
        .unwrap_or_else(|| {
            // ISO strings (e.g. checker_results.finished): "2026-11-20T20:06:36.095611+00:00"
            // → "2026-11-20 20:06:36"
            let raw = s_opt(v);
            // ISO strings (e.g. "2026-11-20T20:06:36.095611+00:00") → local time via Date::parse
            let ms = js_sys::Date::parse(&raw);
            if !ms.is_nan() {
                chrono_dash((ms / 1000.0) as i64).unwrap_or(raw)
            } else {
                raw
            }
        })
}

fn chrono_dash(unix: i64) -> Option<String> {
    // no chrono in wasm: format in the browser's local timezone via JS Date
    let d = js_sys::Date::new(&(unix as f64 * 1000.0).into());
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        d.get_full_year() as i64,
        d.get_month() as i64 + 1,
        d.get_date() as i64,
        d.get_hours() as i64,
        d.get_minutes() as i64,
        d.get_seconds() as i64,
    ))
}

/// Polling helper: increments a signal every `secs` so LocalResources re-fetch.
pub fn poll(secs: u32, tick: leptos::prelude::RwSignal<u64>) {
    // the handle is only for cancelling; dropping it lets the interval run
    let _ = leptos::prelude::set_interval_with_handle(
        move || tick.set(tick.get_untracked() + 1),
        std::time::Duration::from_secs(secs as u64),
    );
}

/// Percent-encode for query strings (no `urlencoding` crate in wasm).
pub fn urlencoding(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}
