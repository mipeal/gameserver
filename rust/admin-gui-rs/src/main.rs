//! saarCTF management GUI (replaces the Flask HTML templates) — Leptos CSR SPA.

use leptos::prelude::*;
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;

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
            <nav>
                <A href="/">Overview</A>
                <A href="/teams">Teams</A>
                <A href="/services">Services</A>
                <A href="/checker_status">Checker Status</A>
                <A href="/checker_results">Checker Results</A>
                <A href="/flags">Flags</A>
                <A href="/patches">Patches</A>
                <A href="/packages">Packages</A>
            </nav>
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

// ---- Overview ----------------------------------------------------------------

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
    let vpn = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/vpn").await }
    });
    let logs = LocalResource::new(move || {
        let _ = tick.get();
        async { get_json("/overview/logs").await }
    });
    let (log_limit, set_log_limit) = signal(50.0f64);
    let timing_reload = move || timing.refetch();
    let vpn_reload = move || vpn.refetch();
    let set_vpn = move |state: &'static str| {
        leptos::task::spawn_local(async move {
            let _ = crate::api::post_json("/overview/set_vpn", serde_json::json!({"state": state})).await;
            vpn_reload();
        });
    };
    view! {
        <h1>"Overview"</h1>
        {move || match status.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => view! {
                <div class="grid">
                    <div class="card">
                        <h2>"State"</h2>
                        <p>"Timer: " <span class={if s(&v["state"]) == "RUNNING" { "ok" } else { "warn" }}>{s(&v["state"])}</span>
                           " (desired " {s(&v["desiredState"])} "), tick " {s(&v["currentRound"])}</p>
                        <p class={if v["missing_scoreboard"].as_bool().unwrap_or(false) { "err" } else { "ok" }}>
                            {if v["missing_scoreboard"].as_bool().unwrap_or(false) { "Scoreboard is MISSING" } else { "Scoreboard present" }}
                        </p>
                        {if !s(&v["missing_scripts"]).is_empty() {
                            view! { <p class="err">{"Missing scripts: "}{s(&v["missing_scripts"])}</p> }.into_any()
                        } else { "".into_any() }}
                    </div>
                    <div class="card">
                        <h2>"Teams"</h2>
                        {move || match vpn.get() {
                            None => "Loading…".into_any(),
                            Some(Err(e)) => err_box(&e).into_any(),
                            Some(Ok(v)) => view! {
                                <p>
                                    <span class="pill ok">{"online "}{s(&v["teams_online"])}</span>
                                    <span class="pill warn">{"once "}{s(&v["teams_online_once"])}</span>
                                    <span class="pill err">{"offline "}{s(&v["teams_offline"])}</span>
                                </p>
                                <p>"VPN state: " <b>{s(&v["state"])}</b></p>
                                <form class="inline">
                                    <button type="button" on:click=move |_| set_vpn("on")>"Open"</button>
                                    <button type="button" on:click=move |_| set_vpn("team")>"Teams only"</button>
                                    <button type="button" on:click=move |_| set_vpn("team-no-vulnbox")>"Teams-only-no-vulnbox"</button>
                                    <button type="button" on:click=move |_| set_vpn("off")>"Close"</button>
                                </form>
                            }.into_any(),
                        }}
                    </div>
                </div>
            }.into_any(),
        }}
        {move || match timing.get() {
            None => "".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => view! {
                <h2>"Timing"</h2>
                <p>"Tick " {s(&v["currentRound"])} " — start " {api::ts(&v["roundStart"])} " / end " {api::ts(&v["roundEnd"])}
                   " (" {s(&v["roundTime"])} "s), last round " {s_opt(&v["lastRound"])}
                   ", master timers: " {s(&v["masterTimers"])}</p>
                <p><button on:click=move |_| { leptos::task::spawn_local(async move { let _ = crate::api::post_json("/overview/set_timing", serde_json::json!({"state": 3})).await; timing_reload(); }); }>"Start"</button>" "
                   <button on:click=move |_| { leptos::task::spawn_local(async move { let _ = crate::api::post_json("/overview/set_timing", serde_json::json!({"state": 1})).await; timing_reload(); }); }>"Stop"</button></p>
            }.into_any(),
        }}
        <h2>"Logs"</h2>
        <form class="inline">
            <label>"Min ID / limit: "</label>
            <input type="number" min=0 prop:value=log_limit on:input=move |e| set_log_limit.set(event_target_value(&e).parse().unwrap_or(50.0)) />
            <button type="button" on:click=move |_| logs.refetch()>"Reload"</button>
        </form>
        {move || match logs.get() {
            None => "".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let arr = v.as_array().cloned().unwrap_or_default()
                    .into_iter().take(log_limit.get() as usize).collect::<Vec<_>>();
                table(&["ID", "Time", "Component", "Level", "Title", "Text"],
                    arr.iter().map(|l| vec![
                        s(&l["id"]).into_any(), ts(&l["created"]).into_any(), s(&l["component"]).into_any(),
                        s(&l["level"]).into_any(), s(&l["title"]).into_any(), s_opt(&l["text"]).into_any(),
                    ]).collect()).into_any()
            }
        }}
    }
}
