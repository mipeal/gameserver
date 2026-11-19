//! Port of `vpnboard/vpnchecks.py` — ping/nping subprocess parsing + testbox web check.

use regex::Regex;
use std::process::Stdio;

pub const USE_NPING_TIMEPINGS: bool = true;

/// Average ping time in ms, None if unreachable / unreliable.
pub async fn test_ping(ip: &str) -> Option<f64> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("ping")
            .args(["-c", "3", ip])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let re = Regex::new(r"time=([\d.]+) ms").ok()?;
    let times: Vec<f64> = re
        .captures_iter(&text)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    if times.len() != 3 {
        return None;
    }
    Some(times.iter().sum::<f64>() / times.len() as f64)
}

/// nping-based ping (gateway pings via nping are rate-limited → router uses test_ping).
pub async fn test_nping(ip: &str) -> Option<f64> {
    let mut cmd = vec![
        "nping".to_string(),
        "--privileged".into(),
        "--icmp".into(),
        "-c".into(),
        "3".into(),
    ];
    if USE_NPING_TIMEPINGS {
        cmd.extend(["--icmp-type".into(), "time".into()]);
    }
    cmd.push(ip.to_string());
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let stdout = &out.stdout;
    if !out.status.success() {
        return None;
    }
    if !contains_subslice(stdout, b"Lost: 0 (0.00%)") {
        return None;
    }
    if contains_subslice(stdout, b"unreachable (type=3") {
        return None;
    }
    let times = Regex::new(r"Avg rtt: ([\d.]+)ms").ok()?;
    let times: Vec<f64> = times
        .captures_iter(&String::from_utf8_lossy(stdout))
        .filter_map(|c| c[1].parse().ok())
        .collect();
    if times.len() != 1 {
        return None;
    }
    Some(times[0])
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// 'OK' or an error string (never None in Python — the Option is the author's lie).
pub async fn test_web(ip: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    match client.get(format!("http://{ip}/saarctf")).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.bytes().await.map(|b| b.to_vec());
            match body {
                Ok(body) => {
                    if status != 200 {
                        return format!("status {status}");
                    }
                    if String::from_utf8_lossy(&body).trim() != "saarctf-testbox" {
                        return "unexpected text".into();
                    }
                    "OK".into()
                }
                Err(e) => request_error(&e),
            }
        }
        Err(e) => request_error(&e),
    }
}

fn request_error(e: &reqwest::Error) -> String {
    // Python catches Timeout/ConnectionError → 'unreachable', RequestException → 'error: ...'
    if e.is_timeout() || e.is_connect() {
        "unreachable".into()
    } else {
        format!("error: {e}")
    }
}
