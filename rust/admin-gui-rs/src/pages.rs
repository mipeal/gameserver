//! Teams / Services / Checker pages.

use crate::api::*;
use crate::{err_box, table};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde_json::Value;

fn pager(name: &'static str, cur: impl Fn() -> usize + Send + Copy + 'static, pages: impl Fn() -> usize + Send + Copy + 'static, nav: impl Fn(isize) + Send + Copy + 'static) -> impl IntoView {
    view! {
        <p>
            <button type="button" on:click=move |_| { if cur() > 1 { nav(-1) } }>{"‹ prev"}</button>
            {move || format!(" page {} / {} ", cur(), pages())}
            <button type="button" on:click=move |_| { if cur() < pages() { nav(1) } }>{"next ›"}</button>
            <a href=name style="margin-left: 12px">{"back to "}{name.replace('_', " ")}</a>
        </p>
    }
}

// ---- Teams -------------------------------------------------------------------

#[component]
pub fn TeamsList() -> impl IntoView {
    let (page, set_page) = signal(1usize);
    let (sort, set_sort) = signal("name".to_string());
    let (dir, set_dir) = signal("asc".to_string());
    let (filter, set_filter) = signal(String::new());
    let teams = LocalResource::new(move || {
        let url = format!("/api/teams/list?page={}&sort={}&dir={}&filter_level={}", page.get(), sort.get(), dir.get(), filter.get());
        async move { get_json(&url).await }
    });
    let th = move |name: &'static str, key: &'static str| {
        view! {
            <th on:click=move |_| { set_sort.set(key.to_string()); set_dir.set(if dir.get() == "desc" { "asc".into() } else { "desc".into() }); }>{name}</th>
        }
    };
    view! {
        <h1>"Teams"</h1>
        <form class="inline">
            <select on:change=move |e| set_filter.set(event_target_value(&e))>
                <option value="">"all"</option>
                <option value="online">"online"</option>
                <option value="ever online">"ever online"</option>
                <option value="offline">"offline"</option>
            </select>
        </form>
        {move || match teams.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let pages = v["pages"].as_u64().unwrap_or(1) as usize;
                let rows = v["teams"].as_array().cloned().unwrap_or_default().iter().map(|t| vec![
                    view! { <a href={format!("/teams/view/{}", s(&t["id"]))}>{s(&t["id"])}</a> }.into_any(),
                    s(&t["name"]).into_any(),
                    s_opt(&t["affiliation"]).into_any(),
                    s_opt(&t["website"]).into_any(),
                    {if t["vpn_connected"].as_bool().unwrap_or(false) { view! { <span class="ok">"online"</span> }.into_any() }
                     else if t["vpn_last_connect"].is_null() { view! { <span class="err">"offline"</span> }.into_any() }
                     else { view! { <span class="warn">"offline (was online)"</span> }.into_any() }},
                    s(&t["traffic"]["down_teams_bytes"]).into_any(),
                    s(&t["traffic"]["up_teams_bytes"]).into_any(),
                ]).collect();
                view! {
                    {pager("teams", move || page.get(), move || pages, move |d| set_page.set((page.get() as isize + d).max(1) as usize))}
                    {table(&["ID", "Name", "Affiliation", "Website", "VPN", "Download (teams)", "Upload (teams)"], rows)}
                }.into_any()
            }
        }}
        <span hidden>{th("id", "id")}</span>
    }
}

#[component]
pub fn TeamView() -> impl IntoView {
    let id = move || use_params_map().get_untracked().get("id").unwrap_or_default();
    let team = LocalResource::new(move || {
        let id = id();
        async move { get_json(&format!("/api/teams/view/{id}")).await }
    });
    view! {
        <h1>"Team " {move || id()}</h1>
        <p><a href="/teams">"← all teams"</a></p>
        {move || match team.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let graph = |title: &'static str, key: &'static str| {
                    let rows: Vec<Vec<AnyView>> = v[key].as_array().cloned().unwrap_or_default().iter().map(|series| {
                        vec![s(&series["label"]).into_any(),
                             series["data"].as_array().cloned().unwrap_or_default().iter().map(|x| s(x)).collect::<Vec<_>>().join(", ").into_any()]
                    }).collect();
                    view! { <h2>{title}</h2> {table(&["Series", "Samples (most recent last)"], rows)} }
                };
                view! {
                    <p>{s(&v["name"])}" — " {s_opt(&v["affiliation"])}" — " {s_opt(&v["website"])}" — IP " {s_opt(&v["ip"])}</p>
                    {graph("Download / Upload", "graph1")}
                    {graph("Bytes: team-to-team vs game", "graph2")}
                    {graph("SYNs: team-to-team vs game", "graph3")}
                }.into_any()
            }
        }}
    }
}

// ---- Services ----------------------------------------------------------------

#[component]
pub fn Services() -> impl IntoView {
    let services = LocalResource::new(|| async { get_json("/api/services/list").await });
    view! {
        <h1>"Services"</h1>
        {move || match services.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let fbs = svc["first_bloods"].as_array().cloned().unwrap_or_default();
                    let fb_names = fbs.iter().filter_map(|fb| {
                        let tid = fb["team_id"].as_i64()?;
                        let team = v["teams"].as_array()?.iter().find(|t| t["id"].as_i64() == Some(tid))?;
                        Some(format!("#{} {} (tick {})", tid, s(team), s(&fb["tick_submitted"])))
                    }).collect::<Vec<_>>().join(", ");
                    vec![
                        s(&svc["id"]).into_any(),
                        s(&svc["name"]).into_any(),
                        {if svc["checker_enabled"].as_bool().unwrap_or(false) { view! { <span class="ok">"enabled"</span> }.into_any() } else { view! { <span class="err">"disabled"</span> }.into_any() }},
                        s(&svc["checker_runner"]).into_any(),
                        s_opt(&svc["package"]).into_any(),
                        fb_names.into_any(),
                    ]
                }).collect();
                table(&["ID", "Name", "Checker", "Runner", "Package", "First bloods"], rows).into_any()
            }
        }}
    }
}

// ---- Checker status ----------------------------------------------------------

#[component]
pub fn CheckerStatus() -> impl IntoView {
    let (tick, set_tick) = signal(String::new());
    let status = LocalResource::new(move || {
        let t = tick.get();
        let url = if t.is_empty() { "/api/checker_status".to_string() } else { format!("/api/checker_status?tick={t}") };
        async move { get_json(&url).await }
    });
    view! {
        <h1>"Checker Status"</h1>
        <p><a href="/checker_status/overview">"Overview of ticks →"</a></p>
        <form class="inline">
            <input placeholder="tick (empty = current)" prop:value=tick on:input=move |e| set_tick.set(event_target_value(&e)) />
            <button type="button" on:click=move |_| status.refetch()>"Show"</button>
        </form>
        {move || match status.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let statuses = svc["status"].as_object().cloned().unwrap_or_default().iter()
                        .map(|(k, n)| format!("{k}: {}", s(n))).collect::<Vec<_>>().join("  ");
                    vec![
                        s(&svc["name"]).into_any(),
                        s(&svc["dispatched"]).into_any(),
                        s(&svc["finished"]).into_any(),
                        format!("{:.1}s avg", svc["time"].as_f64().unwrap_or(0.0) / svc["time_count"].as_f64().unwrap_or(1.0).max(1.0)).into_any(),
                        s(&svc["toolate"]).into_any(),
                        statuses.into_any(),
                    ]
                }).collect();
                view! {
                    <p>"Tick " {s(&v["tick"])} " — " {s(&v["count_finished"])} " finished of " {s(&v["count_dispatched"])}
                       " dispatched (tick " {ts(&v["tick_start"])} " → " {ts(&v["tick_end"])} ")"</p>
                    {table(&["Service", "Dispatched", "Finished", "Avg time", "Too late", "Statuses"], rows)}
                }.into_any()
            }
        }}
    }
}

#[component]
pub fn CheckerStatusOverview() -> impl IntoView {
    let data = LocalResource::new(|| async { get_json("/api/checker_status/overview").await });
    view! {
        <h1>"Checker Status — Overview"</h1>
        <p><a href="/checker_status">"← current tick"</a></p>
        {move || match data.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["ticks"].as_array().cloned().unwrap_or_default().iter().map(|t| vec![
                    view! { <a href={format!("/checker_status?tick={}", s(&t["number"]))}>{s(&t["number"])}</a> }.into_any(),
                    ts(&t["start"]).into_any(),
                    s(&t["time"]).into_any(),
                    s(&t["dispatched"]).into_any(),
                    {let ok = t["tasks_ok"].as_i64().unwrap_or(0);
                     view! { <span class={if ok > 0 { "ok" } else { "" }}>{s(&t["tasks_ok"])}</span> }.into_any()},
                    s(&t["tasks_warn"]).into_any(),
                    {if t["tasks_error"].as_i64().unwrap_or(0) > 0 { view! { <span class="err">{s(&t["tasks_error"])}</span> }.into_any() } else { s(&t["tasks_error"]).into_any() }},
                    s(&t["tasks_revoked"]).into_any(),
                    s(&t["tasks_toolate"]).into_any(),
                    ts(&t["last_finished"]).into_any(),
                ]).collect();
                table(&["Tick", "Start", "Time", "Dispatched", "OK", "Timeout", "Crashed", "Revoked", "Too late", "Last finished"], rows).into_any()
            }
        }}
    }
}

// ---- Checker results ---------------------------------------------------------

#[component]
pub fn CheckerResults() -> impl IntoView {
    let (page, set_page) = signal(1usize);
    let (team, set_team) = signal(String::new());
    let (service, set_service) = signal(String::new());
    let (status, set_status) = signal(String::new());
    let data = LocalResource::new(move || {
        let url = format!(
            "/api/checker_results?page={}&filter_team={}&filter_service={}&filter_status={}",
            page.get(), team.get(), service.get(), status.get());
        async move { get_json(&url).await }
    });
    view! {
        <h1>"Checker Results"</h1>
        <form class="inline">
            <input placeholder="team id" prop:value=team on:input=move |e| { set_team.set(event_target_value(&e)); set_page.set(1); } />
            <input placeholder="service id" prop:value=service on:input=move |e| { set_service.set(event_target_value(&e)); set_page.set(1); } />
            <select prop:value=status on:change=move |e| { set_status.set(event_target_value(&e)); set_page.set(1); }>
                <option value="">"any status"</option>
                {["SUCCESS", "FLAGMISSING", "MUMBLE", "OFFLINE", "RECOVERING", "TIMEOUT", "REVOKED", "CRASHED", "PENDING"].iter()
                    .map(|s| view! { <option value={*s}>{*s}</option> }).collect::<Vec<_>>()}
            </select>
        </form>
        {move || match data.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let pages = v["pages"].as_u64().unwrap_or(1) as usize;
                let rows = v["results"].as_array().cloned().unwrap_or_default().iter().map(|r| {
                    let st = s(&r["status"]);
                    let cls = match st.as_str() {
                        "SUCCESS" => "ok", "TIMEOUT" | "MUMBLE" | "FLAGMISSING" | "OFFLINE" | "RECOVERING" => "warn",
                        "REVOKED" | "CRASHED" => "err", _ => "",
                    };
                    vec![
                        view! { <a href={format!("/checker_results/view/{}", s(&r["id"]))}>{s(&r["id"])}</a> }.into_any(),
                        s(&r["tick"]).into_any(),
                        s(&r["team_id"]).into_any(),
                        s(&r["service_id"]).into_any(),
                        view! { <span class={cls}>{st}</span> }.into_any(),
                        s_opt(&r["message"]).into_any(),
                        {r["time"].as_f64().map(|t| format!("{t:.2}s")).unwrap_or_default().into_any()},
                        ts(&r["finished"]).into_any(),
                    ]
                }).collect();
                view! {
                    {pager("checker_results", move || page.get(), move || pages, move |d| set_page.set((page.get() as isize + d).max(1) as usize))}
                    {table(&["ID", "Tick", "Team", "Service", "Status", "Message", "Time", "Finished"], rows)}
                }.into_any()
            }
        }}
    }
}

#[component]
pub fn CheckerResultView() -> impl IntoView {
    let id = move || use_params_map().get_untracked().get("id").unwrap_or_default();
    let result = LocalResource::new(move || {
        let id = id();
        async move {
            gloo_net::http::Request::get(&format!("/checker_results/view/{id}"))
                .header("Accept", "application/json")
                .send().await.map_err(|e| e.to_string())?
                .json::<Value>().await.map_err(|e| e.to_string())
        }
    });
    view! {
        <h1>"Checker Result " {move || id()}</h1>
        <p><a href="/checker_results">"← all results"</a></p>
        {move || match result.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => view! {
                <p>"Tick " {s(&v["tick"])} ", team " {s(&v["team_id"])} ", service " {s(&v["service_id"])} ": " {s(&v["status"])}</p>
                <pre>{s_opt(&v["output"])}</pre>
            }.into_any(),
        }}
    }
}

// ---- Flags -------------------------------------------------------------------

#[component]
pub fn Flags() -> impl IntoView {
    let (flag, set_flag) = signal(String::new());
    let (submitted, set_submitted) = signal(String::new());
    let data = LocalResource::new(move || {
        let f = submitted.get();
        let url = if f.is_empty() { "/api/flags/analyze".to_string() } else { format!("/api/flags/analyze?flag={}", urlencoding(&f)) };
        async move { get_json(&url).await }
    });
    view! {
        <h1>"Flags"</h1>
        <form class="inline" on:submit=|e| e.prevent_default()>
            <input placeholder="SAAR{...}" style="width: 420px" prop:value=flag on:input=move |e| set_flag.set(event_target_value(&e)) />
            <button type="button" on:click=move |_| set_submitted.set(flag.get())>"Analyze"</button>
        </form>
        {move || match data.get() {
            None => "".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => view! {
                {if v["demo_flags"].is_array() {
                    view! {
                        <h2>"Demo flags (status checks)"</h2>
                        {table(&["Purpose", "Expected", "Flag"],
                            v["demo_flags"].as_array().cloned().unwrap_or_default().iter().map(|d| vec![
                                s(&d["title"]).into_any(), s(&d["expect"]).into_any(),
                                view! { <code>{s(&d["flag"])}</code> }.into_any(),
                            ]).collect())}
                    }.into_any()
                } else { "".into_any() }}
                {if v["flag"].is_null() { "".into_any() } else {
                    let valid = v["valid"].as_bool().unwrap_or(false);
                    view! {
                        <h2>"Analysis"</h2>
                        {if let Some(err) = v["err"].as_str() { view! { <p class="err">{"Error: "}{err}</p> }.into_any() } else { "".into_any() }}
                        <p class={if valid { "ok" } else { "err" }}>{if valid { "VALID flag" } else { "INVALID flag" }}</p>
                        <p>"Tick " {s(&v["stored_tick"])} ", team " {s(&v["teamid"])} " (" {s_opt(&v["team"]["name"])} "), service " {s(&v["serviceid"])} " ("
                           {s_opt(&v["service"]["name"])} "), payload " {s(&v["payload"])} ", MAC " {s(&v["mac"])} " (expected " {s(&v["real_mac"])} ")"</p>
                        {if let Some(repaired) = v["repaired_flag"].as_str() {
                            view! { <p class="warn">{"Repaired flag: "}<code>{repaired}</code></p> }.into_any()
                        } else { "".into_any() }}
                        {if valid && v["submissions"].is_array() {
                            view! {
                                <h2>"Submissions"</h2>
                                {table(&["Submitter", "Tick", "TS"],
                                    v["submissions"].as_array().cloned().unwrap_or_default().iter().map(|f| vec![
                                        s(&f["submitted_by"]).into_any(), s(&f["tick_submitted"]).into_any(), s(&f["ts"]).into_any(),
                                    ]).collect())}
                            }.into_any()
                        } else { "".into_any() }}
                    }.into_any()
                }}
            }.into_any(),
        }}
    }
}
