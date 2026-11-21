//! CTF management GUI (replaces the Flask HTML templates) — Leptos CSR SPA.

use leptos::prelude::*;
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;
use serde_json::Value;

mod api;
mod ops;
mod pages;
use api::{get_json, poll, s, s_opt, ts};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <Router>
            <aside>
                <div class="logo">"◼ "<span>"CTF"</span></div>
                <nav>
                    <A href="/" exact=true>"Overview"</A>
                    <A href="/teams">"Teams"</A>
                    <A href="/services">"Services"</A>
                    <A href="/checker_status">"Checker"</A>
                    <A href="/checker_results">"Results"</A>
                    <A href="/flags">"Flags"</A>
                    <A href="/patches">"Patches"</A>
                    <A href="/packages">"Packages"</A>
                </nav>
                <div class="auth">"⚑ no login — ADR-011"<br/>"(opt-in auth shell)"</div>
            </aside>
            <main>
                <Routes fallback=|| view! { <h1>"Not found"</h1> }>
                    <Route path=path!("/") view=Overview/>
                    <Route path=path!("/teams") view=pages::TeamsList/>
                    <Route path=path!("/teams/view/:id") view=pages::TeamView/>
                    <Route path=path!("/services") view=pages::Services/>
                    <Route path=path!("/checker_status") view=pages::CheckerStatus/>
                    <Route path=path!("/checker_status/overview") view=pages::CheckerStatusOverview/>
                    <Route path=path!("/checker_results") view=pages::CheckerResults/>
                    <Route path=path!("/checker_results/view/:id") view=pages::CheckerResultView/>
                    <Route path=path!("/flags") view=pages::Flags/>
                    <Route path=path!("/patches") view=ops::Patches/>
                    <Route path=path!("/packages") view=ops::Packages/>
                </Routes>
            </main>
        </Router>
    }
}

fn err_box(e: &String) -> impl IntoView {
    view! { <p class="err">{format!("Error: {e}")}</p> }
}

fn table(headers: &[&str], rows: Vec<Vec<AnyView>>) -> impl IntoView {
    view! {
        <table>
            <thead><tr>{headers.iter().map(|h| view! { <th>{h.to_string()}</th> }).collect::<Vec<_>>()}</tr></thead>
            <tbody>
                {rows.into_iter().map(|row| view! { <tr>{row.into_iter().map(|c| view! { <td>{c}</td> }).collect::<Vec<_>>()}</tr> }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}

// ---- shared formatting helpers ------------------------------------------------

/// Decimal byte formatting used by the tiles/teams tables ("225 kB", "1.1 MB").
pub fn fmt_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes.abs();
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        return format!("{v:.0} B");
    }
    let txt = format!("{v:.1}");
    let txt = txt.strip_suffix(".0").unwrap_or(&txt);
    format!("{txt} {}", UNITS[i])
}

/// "YYYY-MM-DD HH:MM:SS" (or raw) → "HH:MM:SS".
pub fn short_ts(v: &Value) -> String {
    ts(v).rsplit(' ').next().unwrap_or_default().to_string()
}

/// Seconds remaining → "h:mm:ss" or "m:ss" (clamped at zero).
fn fmt_countdown(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// CSS class for a checker result status.
pub fn status_cls(st: &str) -> &'static str {
    match st {
        "SUCCESS" => "ok",
        "TIMEOUT" | "MUMBLE" | "OFFLINE" | "FLAGMISSING" | "RECOVERING" => "warn",
        "REVOKED" | "CRASHED" => "err",
        "PENDING" => "dim",
        _ => "",
    }
}

/// Text sparkline ▁▂▃▄▅▆▇█ scaled to the series max.
pub fn sparkline(values: &[f64]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().cloned().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return "▁".repeat(values.len());
    }
    values
        .iter()
        .map(|v| BLOCKS[((v / max * 7.0).round() as usize).min(7)])
        .collect()
}

// ---- Overview ----------------------------------------------------------------

fn traffic_series(v: &Value) -> Vec<f64> {
    let stats = v["traffic_stats"].as_array().cloned().unwrap_or_default();
    let start = stats.len().saturating_sub(12);
    stats[start..]
        .iter()
        .map(|p| {
            p.as_array()
                .and_then(|a| a.first())
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0)
        })
        .collect()
}

fn last_traffic(v: &Value) -> (f64, f64) {
    let stats = v["traffic_stats"].as_array().cloned().unwrap_or_default();
    stats
        .last()
        .map(|p| {
            let a = p.as_array().cloned().unwrap_or_default();
            let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0);
            (g(0) + g(2), g(4) + g(6))
        })
        .unwrap_or((0.0, 0.0))
}

fn checker_table(res: LocalResource<Result<Value, String>>) -> impl IntoView {
    view! {
        {move || match res.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(_)) => view! { <p class="dim">"unavailable"</p> }.into_any(),
            Some(Ok(v)) => {
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let statuses: Vec<AnyView> = svc["status"].as_object().cloned().unwrap_or_default().iter().map(|(k, n)| {
                        let cls = status_cls(k);
                        view! { <span class=cls>{format!("{k} {n}  ")}</span> }.into_any()
                    }).collect();
                    let avg = svc["time"].as_f64().unwrap_or(0.0)
                        / svc["time_count"].as_f64().unwrap_or(1.0).max(1.0);
                    view! {
                        <tr>
                            <td>{s(&svc["name"])}</td>
                            <td>{statuses}</td>
                            <td>{format!("{avg:.2}s")}</td>
                        </tr>
                    }.into_any()
                }).collect::<Vec<_>>();
                view! {
                    <table>
                        <thead><tr><th>"service"</th><th>"state"</th><th>"avg"</th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }.into_any()
            }
        }}
    }
}

fn components_table(res: LocalResource<Result<Value, String>>) -> impl IntoView {
    view! {
        {move || match res.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(_)) => view! { <p class="dim">"unavailable"</p> }.into_any(),
            Some(Ok(v)) => {
                let clients = v["redis"].as_array().cloned().unwrap_or_default();
                let total = clients.len();
                let mut rows: Vec<AnyView> = clients.iter().take(8).map(|c| {
                    let name = c["name"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
                        .or_else(|| c["addr"].as_str().map(str::to_string))
                        .unwrap_or_else(|| "client".to_string());
                    view! { <tr><td>{name}</td><td class="ok">"connected"</td></tr> }.into_any()
                }).collect();
                if rows.is_empty() {
                    rows.push(view! { <tr><td>"redis streams"</td><td class="dim">"no clients"</td></tr> }.into_any());
                } else if total > 8 {
                    rows.push(view! {
                        <tr><td colspan="2" class="dim">{format!("… {} more clients", total - 8)}</td></tr>
                    }.into_any());
                }
                view! {
                    <table>
                        <thead><tr><th>"component"</th><th>"state"</th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }.into_any()
            }
        }}
    }
}

#[component]
fn Overview() -> impl IntoView {
    let tick = RwSignal::new(0u64);
    poll(5, tick);
    let status = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/api/overview/status").await }
    });
    let timing = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/timing").await }
    });
    // live round countdown: 1s ticker + base from the last timing poll
    let sec = RwSignal::new(0u64);
    poll(1, sec);
    let round_left = Memo::new(move |prev: Option<&Option<(i64, i64, u64)>>| -> Option<(i64, i64, u64)> {
        let sec_now = sec.get();
        let v = timing.get().and_then(|r| r.ok())?;
        let base = v["roundEnd"].as_i64()?.saturating_sub(v["serverTime"].as_i64()?);
        let (left, psec) = match prev.copied().flatten() {
            Some((pbase, _, psec)) if pbase == base => (base.saturating_sub(sec_now.saturating_sub(psec) as i64), psec),
            _ => (base, sec_now),
        };
        Some((base, left, psec))
    });
    let vpn = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/vpn").await }
    });
    let logs = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/logs").await }
    });
    let checker = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/api/checker_status").await }
    });
    let components = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/components").await }
    });
    let (log_limit, set_log_limit) = signal(50.0f64);
    let (roundtime, set_roundtime) = signal(String::new());
    let (lastround, set_lastround) = signal(String::new());
    let (op_msg, set_op_msg) = signal(String::new());
    let total_teams = move || {
        status
            .get()
            .and_then(|r| r.ok())
            .and_then(|v| v["teams"].as_array().map(|a| a.len()))
            .unwrap_or(0)
    };
    let vpn_reload = move || vpn.refetch();
    let set_timing = move |body: serde_json::Value, reload: bool| {
        leptos::task::spawn_local(async move {
            set_op_msg.set(match crate::api::post_json("/overview/set_timing", body).await {
                Ok(t) => t,
                Err(e) => e,
            });
            if reload {
                timing.refetch();
            }
        });
    };
    let set_vpn = move |state: &'static str| {
        leptos::task::spawn_local(async move {
            let _ = crate::api::post_json("/overview/set_vpn", serde_json::json!({"state": state})).await;
            vpn_reload();
        });
    };
    view! {
        <div class="topbar">
            <h1>"Overview"</h1>
            <span class="clock">
                {move || match status.get() {
                    Some(Ok(v)) => format!("⏱ tick {} · ", s(&v["currentRound"])),
                    _ => "⏱ tick … · ".to_string(),
                }}
                {move || match timing.get() {
                    Some(Ok(v)) => format!("round ends in {} · {} · ", fmt_countdown(round_left.get().unwrap_or((0, 0, 0)).1), short_ts(&v["roundEnd"])),
                    _ => String::new(),
                }}
                "polling 5s " <span class="ok">"●"</span>
            </span>
        </div>

        <div class="tiles">
            <div class="tile">
                <div class="k">"timer"</div>
                <div class="v">
                    {move || match timing.get() {
                        Some(Ok(v)) => {
                            let st = s(&v["state"]);
                            let cls = if st == "RUNNING" { "ok" } else { "warn" };
                            view! { <span class=cls>{st}</span> }.into_any()
                        }
                        Some(Err(e)) => err_box(&e).into_any(),
                        None => "…".into_any(),
                    }}
                </div>
                <div class="s">
                    {move || match timing.get() {
                        Some(Ok(v)) => format!(
                            "desired: {} · roundTime {}s",
                            s(&v["desiredState"]).to_lowercase(),
                            s(&v["roundTime"])
                        ),
                        _ => String::new(),
                    }}
                </div>
            </div>
            <div class="tile">
                <div class="k">"tick"</div>
                <div class="v">
                    {move || match status.get() {
                        Some(Ok(v)) => s(&v["currentRound"]).into_any(),
                        Some(Err(e)) => err_box(&e).into_any(),
                        None => "…".into_any(),
                    }}
                </div>
                <div class="s">
                    {move || match timing.get() {
                        Some(Ok(v)) => format!("started {}", short_ts(&v["roundStart"])),
                        _ => String::new(),
                    }}
                    {move || match status.get() {
                        Some(Ok(v)) => if v["missing_scoreboard"].as_bool().unwrap_or(false) {
                            view! { <span class="err">" · scoreboard MISSING"</span> }.into_any()
                        } else {
                            view! { <span class="ok">" · scoreboard OK"</span> }.into_any()
                        },
                        _ => "".into_any(),
                    }}
                    {move || match status.get() {
                        Some(Ok(v)) => {
                            let missing = s(&v["missing_scripts"]);
                            if missing.is_empty() {
                                "".into_any()
                            } else {
                                view! { <span class="err">" · missing scripts: "{missing}</span> }.into_any()
                            }
                        }
                        _ => "".into_any(),
                    }}
                </div>
            </div>
            <div class="tile">
                <div class="k">"teams"</div>
                <div class="v">
                    {move || match vpn.get() {
                        Some(Ok(v)) => {
                            let total = total_teams();
                            view! {
                                {s(&v["teams_online"])} <span class="u">{format!(" / {total} online")}</span>
                            }.into_any()
                        }
                        Some(Err(e)) => err_box(&e).into_any(),
                        None => "…".into_any(),
                    }}
                </div>
                <div class="s">
                    {move || match vpn.get() {
                        Some(Ok(v)) => format!(
                            "once: {} · offline: {}",
                            s(&v["teams_online_once"]),
                            s(&v["teams_offline"])
                        ),
                        _ => String::new(),
                    }}
                </div>
            </div>
            <div class="tile">
                <div class="k">"traffic ↓ / ↑"</div>
                <div class="v">
                    {move || match vpn.get() {
                        Some(Ok(v)) => {
                            let (down, up) = last_traffic(&v);
                            view! { {fmt_bytes(down)} <span class="u">"/"</span> {fmt_bytes(up)} }.into_any()
                        }
                        Some(Err(e)) => err_box(&e).into_any(),
                        None => "…".into_any(),
                    }}
                </div>
                <div class="s spark">
                    {move || match vpn.get() {
                        Some(Ok(v)) => sparkline(&traffic_series(&v)),
                        _ => String::new(),
                    }}
                </div>
            </div>
        </div>

        <h2>"VPN"</h2>
        <div class="btns">
            <button type="button" class="primary" on:click=move |_| set_vpn("on")>"Open"</button>
            <button type="button" on:click=move |_| set_vpn("team")>"Teams only"</button>
            <button type="button" on:click=move |_| set_vpn("team-no-vulnbox")>"Teams-only-no-vulnbox"</button>
            <button type="button" class="danger" on:click=move |_| set_vpn("off")>"Close"</button>
            <span class="dim">
                {move || match vpn.get() {
                    Some(Ok(v)) => format!(
                        "— state: {} · peers {}/{}",
                        s(&v["state"]),
                        s(&v["teams_online"]),
                        total_teams()
                    ),
                    _ => String::new(),
                }}
            </span>
        </div>

        <h2>"Timing"</h2>
        <div class="btns">
            <button type="button" class="primary" on:click=move |_| set_timing(serde_json::json!({"state": 3}), true)>"Start"</button>
            <button type="button" on:click=move |_| set_timing(serde_json::json!({"state": 1}), true)>"Stop after tick"</button>
            <button type="button" on:click=move |_| set_timing(serde_json::json!({"state": 2}), true)>"Pause after tick"</button>
            <button type="button" on:click=move |_| {
                leptos::task::spawn_local(async move {
                    set_op_msg.set(match crate::api::post_json("/scripts/recreate_scoreboard", serde_json::json!({})).await {
                        Ok(t) => t, Err(e) => e,
                    });
                });
            }>"Recreate scoreboard"</button>
        </div>
        <p class="poll">
            {move || match timing.get() {
                Some(Ok(v)) => format!(
                    "last round {} · master timers {}",
                    s_opt(&v["lastRound"]),
                    s(&v["masterTimers"])
                ),
                _ => String::new(),
            }}
        </p>
        <form class="row" on:submit=|e| e.prevent_default()>
            <span class="dim">"tick time"</span>
            <input type="number" min=5 max=999 style="width: 80px" placeholder="30" prop:value=roundtime on:input=move |e| set_roundtime.set(event_target_value(&e)) />
            <span class="dim">"s"</span>
            <span class="dim">"stop after round"</span>
            <input type="number" min=1 style="width: 80px" placeholder="—" prop:value=lastround on:input=move |e| set_lastround.set(event_target_value(&e)) />
            <button class="primary" type="button" on:click=move |_| {
                let mut body = serde_json::Map::new();
                if let Ok(v) = roundtime.get_untracked().parse::<i64>() { body.insert("roundtime".into(), serde_json::json!(v)); }
                if let Ok(v) = lastround.get_untracked().parse::<i64>() { body.insert("lastround".into(), serde_json::json!(v)); }
                if !body.is_empty() { set_timing(serde_json::Value::Object(body), true); }
            }>"Apply"</button>
        </form>
        <p>{move || if !op_msg.get().is_empty() { view! { <span class="warn">{op_msg.get()}</span> }.into_any() } else { "".into_any() }}</p>

        <div class="cols" style="margin-top:4px">
            <div>
                <h2>"Logs — live"</h2>
                {move || match logs.get() {
                    None => view! { <p class="dim">"Loading…"</p> }.into_any(),
                    Some(Err(e)) => err_box(&e).into_any(),
                    Some(Ok(v)) => {
                        let arr = v.as_array().cloned().unwrap_or_default()
                            .into_iter().take(log_limit.get() as usize).collect::<Vec<_>>();
                        let rows = arr.iter().map(|l| {
                            let level = l["level"].as_i64().unwrap_or(0);
                            let cls = if level >= 30 { "err" } else if level >= 20 { "warn" } else { "" };
                            view! {
                                <tr>
                                    <td>{short_ts(&l["created"])}</td>
                                    <td>{s(&l["component"])}</td>
                                    <td class=cls>{s(&l["title"])}</td>
                                </tr>
                            }.into_any()
                        }).collect::<Vec<_>>();
                        view! {
                            <table>
                                <thead><tr><th>"ts"</th><th>"component"</th><th>"event"</th></tr></thead>
                                <tbody>{rows}</tbody>
                            </table>
                        }.into_any()
                    }
                }}
                <p class="poll">
                    "limit "
                    <input type="number" min=0 style="width: 70px" prop:value=log_limit on:input=move |e| set_log_limit.set(event_target_value(&e).parse().unwrap_or(50.0)) />
                    <button type="button" on:click=move |_| logs.refetch()>"Reload"</button>
                </p>
            </div>
            <div>
                <h2>"Checker — current tick"</h2>
                {checker_table(checker)}
                <h2>"Redis / components"</h2>
                {components_table(components)}
            </div>
        </div>
    }
}
