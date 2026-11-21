//! Teams / Services / Checker pages.

use crate::api::*;
use crate::{err_box, fmt_bytes, short_ts, status_cls, table};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde_json::Value;

fn pager(name: &'static str, cur: impl Fn() -> usize + Send + Copy + 'static, pages: impl Fn() -> usize + Send + Copy + 'static, nav: impl Fn(isize) + Send + Copy + 'static) -> impl IntoView {
    view! {
        <div class="pager">
            <button type="button" on:click=move |_| { if cur() > 1 { nav(-1) } }>{"‹ prev"}</button>
            <span>{move || format!("page {} / {}", cur(), pages())}</span>
            <button type="button" on:click=move |_| { if cur() < pages() { nav(1) } }>{"next ›"}</button>
            <a href=name class="dim">{"back to "}{name.replace('_', " ")}</a>
        </div>
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
            <th class="sort" on:click=move |_| {
                if sort.get() == key {
                    set_dir.set(if dir.get() == "desc" { "asc".into() } else { "desc".into() });
                } else {
                    set_sort.set(key.to_string());
                    set_dir.set("asc".into());
                }
            }>
                {name}
                {move || if sort.get() == key {
                    if dir.get() == "asc" { " ▲" } else { " ▼" }
                } else { "" }}
            </th>
        }
    };
    view! {
        <h1>"Teams"</h1>
        <div class="sub">
            {move || match teams.get() {
                Some(Ok(v)) => format!(
                    "{} teams · live VPN state · sort: click header",
                    v["teams"].as_array().map(|a| a.len()).unwrap_or(0)
                ),
                _ => "live VPN state · sort: click header".to_string(),
            }}
        </div>
        <div class="toolbar">
            <span class="dim">"filter"</span>
            <select on:change=move |e| { set_filter.set(event_target_value(&e)); set_page.set(1); }>
                <option value="">"all"</option>
                <option value="online">"online"</option>
                <option value="ever online">"ever online"</option>
                <option value="offline">"offline"</option>
            </select>
        </div>
        {move || match teams.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let pages = v["pages"].as_u64().unwrap_or(1) as usize;
                let rows = v["teams"].as_array().cloned().unwrap_or_default().iter().map(|t| {
                    let id = s(&t["id"]);
                    let href = format!("/teams/view/{id}");
                    let online = t["vpn_connected"].as_bool().unwrap_or(false)
                        || t["vpn2_connected"].as_bool().unwrap_or(false)
                        || t["wg_vulnbox_connected"].as_bool().unwrap_or(false);
                    let (vpn_cls, vpn_txt) = if online {
                        ("ok", "online")
                    } else if t["vpn_last_connect"].is_null() {
                        ("err", "offline")
                    } else {
                        ("warn", "offline (was online)")
                    };
                    view! {
                        <tr>
                            <td><a href=href.clone()>{id.clone()}</a></td>
                            <td>{s(&t["name"])}</td>
                            <td>{s_opt(&t["affiliation"])}</td>
                            <td>{s_opt(&t["website"])}</td>
                            <td class=vpn_cls>{vpn_txt}</td>
                            <td>{fmt_bytes(t["traffic"]["down_teams_bytes"].as_f64().unwrap_or(0.0))}</td>
                            <td>{fmt_bytes(t["traffic"]["up_teams_bytes"].as_f64().unwrap_or(0.0))}</td>
                            <td><a href=href>"inspect →"</a></td>
                        </tr>
                    }.into_any()
                }).collect::<Vec<_>>();
                view! {
                    <table>
                        <thead><tr>
                            {th("ID", "id")}
                            {th("Name", "name")}
                            {th("Affiliation", "affiliation")}
                            {th("Website", "website")}
                            {th("VPN", "vpn_connected")}
                            {th("↓ teams", "traffic.down_teams_bytes")}
                            {th("↑ teams", "traffic.up_teams_bytes")}
                            <th></th>
                        </tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                    {pager("teams", move || page.get(), move || pages, move |d| set_page.set((page.get() as isize + d).max(1) as usize))}
                }.into_any()
            }
        }}
    }
}

// ---- Team detail -------------------------------------------------------------

fn any_online(v: &Value) -> bool {
    v["vpn_connected"].as_bool().unwrap_or(false)
        || v["vpn2_connected"].as_bool().unwrap_or(false)
        || v["wg_vulnbox_connected"].as_bool().unwrap_or(false)
}

fn vpn_chip(v: &Value) -> impl IntoView {
    if any_online(v) {
        let since = if v["vpn_last_connect"].is_null() {
            String::new()
        } else {
            format!(" since {}", short_ts(&v["vpn_last_connect"]))
        };
        view! { <span class="ok">{format!("VPN online{since}")}</span> }.into_any()
    } else {
        view! { <span class="err">"VPN offline"</span> }.into_any()
    }
}

fn graph_chart(v: &Value, title: &'static str, key: &'static str, bytes: bool) -> impl IntoView {
    let series = v[key].as_array().cloned().unwrap_or_default();
    let rows: Vec<AnyView> = series.iter().enumerate().map(|(i, srs)| {
        let data: Vec<f64> = srs["data"].as_array().cloned().unwrap_or_default()
            .iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
        let max = data.iter().cloned().fold(0.0f64, f64::max);
        let cls = match i { 0 => "b", 1 => "b g", _ => "b r" };
        let maxlabel = if bytes { fmt_bytes(max) } else { format!("{}", max as i64) };
        let bars: Vec<AnyView> = data.iter().map(|d| {
            let h = if max > 0.0 { (d / max * 100.0).max(1.0) } else { 1.0 };
            view! { <span class=cls style=format!("height:{h:.0}%")></span> }.into_any()
        }).collect();
        view! {
            <div class="row">
                <span class="lab">{s_opt(&srs["label"])}</span>
                <div class="bars">{bars}</div>
                <span class="max">{maxlabel}</span>
            </div>
        }.into_any()
    }).collect();
    view! { <h2>{title}</h2> <div class="chart">{rows}</div> }
}

fn recent_results(res: LocalResource<Result<Value, String>>) -> impl IntoView {
    view! {
        {move || match res.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(_)) => view! { <p class="dim">"unavailable"</p> }.into_any(),
            Some(Ok(v)) => {
                let services = v["services"].as_array().cloned().unwrap_or_default();
                let rows = v["results"].as_array().cloned().unwrap_or_default().iter().take(6).map(|r| {
                    let sid = r["service_id"].as_i64().unwrap_or(0);
                    let name = services.iter()
                        .find(|svc| svc["id"].as_i64() == Some(sid))
                        .map(|svc| s(&svc["name"]))
                        .unwrap_or_else(|| sid.to_string());
                    let st = s(&r["status"]);
                    let cls = status_cls(&st);
                    vec![
                        s(&r["tick"]).into_any(),
                        name.into_any(),
                        view! { <span class=cls>{st}</span> }.into_any(),
                    ]
                }).collect::<Vec<_>>();
                if rows.is_empty() {
                    view! { <p class="dim">"no checker results for this team"</p> }.into_any()
                } else {
                    table(&["Tick", "Service", "Status"], rows).into_any()
                }
            }
        }}
    }
}

#[component]
pub fn TeamView() -> impl IntoView {
    let id = move || use_params_map().get_untracked().get("id").unwrap_or_default();
    let team = LocalResource::new(move || {
        let id = id();
        async move { get_json(&format!("/api/teams/view/{id}")).await }
    });
    let results = LocalResource::new(move || {
        let id = id();
        async move {
            get_json(&format!("/api/checker_results?page=1&filter_team={id}&filter_service=&filter_status=")).await
        }
    });
    view! {
        <h1>
            {move || match team.get() {
                Some(Ok(v)) => format!("TEAM {} — {}", s(&v["id"]), s(&v["name"]).to_uppercase()),
                _ => format!("TEAM {}", id()),
            }}
        </h1>
        <div class="sub">
            <a href="/teams">"← all teams"</a>
            {move || match team.get() {
                Some(Ok(v)) => view! {
                    " · " {s_opt(&v["ip"])}
                    " · " {s_opt(&v["affiliation"])}
                    " · " {s_opt(&v["website"])}
                    " · " {vpn_chip(&v)}
                }.into_any(),
                _ => "".into_any(),
            }}
        </div>
        {move || match team.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let online = any_online(&v);
                let samples = v["labels"].as_array().map(|a| a.len()).unwrap_or(0);
                view! {
                    <div class="tiles">
                        <div class="tile"><div class="k">"ip"</div><div class="v">{s_opt(&v["ip"])}</div></div>
                        <div class="tile"><div class="k">"affiliation"</div><div class="v">{s_opt(&v["affiliation"])}</div></div>
                        <div class="tile">
                            <div class="k">"VPN"</div>
                            <div class="v">{if online {
                                view! { <span class="ok">"online"</span> }.into_any()
                            } else {
                                view! { <span class="err">"offline"</span> }.into_any()
                            }}</div>
                        </div>
                        <div class="tile">
                            <div class="k">"traffic samples"</div>
                            <div class="v">{samples.to_string()}</div>
                        </div>
                    </div>

                    <div class="two">
                        <div>
                            {graph_chart(&v, "Download / Upload — 60 samples", "graph1", true)}
                            {graph_chart(&v, "Bytes: team-to-team vs game", "graph2", true)}
                        </div>
                        <div>
                            {graph_chart(&v, "SYNs: team-to-team vs game", "graph3", false)}
                            <h2>"Recent checker results"</h2>
                            {recent_results(results)}
                            <p class="empty">{"→ full history under Checker Results, filter team="}{id()}</p>
                        </div>
                    </div>
                }.into_any()
            }
        }}
    }
}

// ---- Services ----------------------------------------------------------------

fn set_checker<F: Fn() + 'static>(set_op_msg: WriteSignal<String>, id: String, enable: bool, reload: F) {
    leptos::task::spawn_local(async move {
        set_op_msg.set(match crate::api::post_form("/services/checker_status",
            &format!("id={}&status={}", urlencoding(&id), if enable { 1 } else { 0 })).await {
            Ok(t) => t, Err(e) => e,
        });
        reload();
    });
}

#[component]
pub fn Services() -> impl IntoView {
    let services = LocalResource::new(|| async { get_json("/api/services/list").await });
    let (op_msg, set_op_msg) = signal(String::new());
    view! {
        <h1>"Services"</h1>
        <div class="sub">"checker enable/disable takes effect next tick"</div>
        {move || if !op_msg.get().is_empty() { view! { <p class="warn">{op_msg.get()}</p> }.into_any() } else { "".into_any() }}
        {move || match services.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let reload = move || services.refetch();
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let fbs = svc["first_bloods"].as_array().cloned().unwrap_or_default();
                    let fb_names = fbs.iter().filter_map(|fb| {
                        let tid = fb["team_id"].as_i64()?;
                        let team = v["teams"].as_array()?.iter().find(|t| t["id"].as_i64() == Some(tid))?;
                        Some(format!("#{} {} (tick {})", tid, s(&team["name"]), s(&fb["tick_submitted"])))
                    }).collect::<Vec<_>>().join(", ");
                    let id = s(&svc["id"]);
                    let enabled = svc["checker_enabled"].as_bool().unwrap_or(false);
                    let toggle_id = id.clone();
                    let toggle = if enabled {
                        view! { <button type="button" class="danger" on:click=move |_| set_checker(set_op_msg, toggle_id.clone(), false, reload)>"Disable"</button> }.into_any()
                    } else {
                        view! { <button type="button" class="primary" on:click=move |_| set_checker(set_op_msg, toggle_id.clone(), true, reload)>"Enable"</button> }.into_any()
                    };
                    vec![
                        id.into_any(),
                        s(&svc["name"]).into_any(),
                        view! { <span class=if enabled { "ok" } else { "err" }>{if enabled { "enabled" } else { "disabled" }}</span> }.into_any(),
                        toggle,
                        s_opt(&svc["checker_runner"]).into_any(),
                        view! { <span class="hash">{s_opt(&svc["package"])}</span> }.into_any(),
                        fb_names.into_any(),
                    ]
                }).collect();
                table(&["ID", "Name", "Checker", "", "Runner", "Package", "First bloods"], rows).into_any()
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
        <div class="sub">
            <a href="/checker_status/overview">"overview of ticks →"</a>
            " · tick "
            <input placeholder="tick" style="width: 90px" prop:value=tick on:input=move |e| set_tick.set(event_target_value(&e)) />
            <button type="button" class="primary" on:click=move |_| status.refetch()>"Show"</button>
        </div>
        {move || match status.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let statuses: Vec<AnyView> = svc["status"].as_object().cloned().unwrap_or_default().iter().map(|(k, n)| {
                        let cls = status_cls(k);
                        view! { <span class=cls>{format!("{k}: {n}  ")}</span> }.into_any()
                    }).collect();
                    let finished = svc["finished"].as_i64().unwrap_or(0);
                    let dispatched = svc["dispatched"].as_i64().unwrap_or(0);
                    let fin_cls = if dispatched > 0 && finished >= dispatched { "ok" } else { "warn" };
                    let avg = svc["time"].as_f64().unwrap_or(0.0)
                        / svc["time_count"].as_f64().unwrap_or(1.0).max(1.0);
                    let late = svc["toolate"].as_i64().unwrap_or(0);
                    view! {
                        <tr>
                            <td>{s(&svc["name"])}</td>
                            <td>{s(&svc["dispatched"])}</td>
                            <td class=fin_cls>{s(&svc["finished"])}</td>
                            <td>{format!("{avg:.2}s")}</td>
                            <td class=(if late > 0 { "warn" } else { "" })>{s(&svc["toolate"])}</td>
                            <td>{statuses}</td>
                        </tr>
                    }.into_any()
                }).collect::<Vec<_>>();
                view! {
                    <table>
                        <thead><tr>
                            <th>"Service"</th><th>"Dispatched"</th><th>"Finished"</th>
                            <th>"Avg time"</th><th>"Too late"</th><th>"Statuses"</th>
                        </tr></thead>
                        <tbody>
                            <tr><td colspan="6" class="dim">
                                {format!("tick {} — {} finished of {} dispatched · start {} → end {}",
                                    s(&v["tick"]), s(&v["count_finished"]), s(&v["count_dispatched"]),
                                    short_ts(&v["tick_start"]), short_ts(&v["tick_end"]))}
                            </td></tr>
                            {rows}
                        </tbody>
                    </table>
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
        <div class="sub">
            <a href="/checker_status">"← current tick"</a>
            " · last 31 ticks · click a tick to inspect"
        </div>
        {move || match data.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["ticks"].as_array().cloned().unwrap_or_default().iter().map(|t| {
                    let ok = t["tasks_ok"].as_i64().unwrap_or(0);
                    let warn = t["tasks_warn"].as_i64().unwrap_or(0);
                    let err = t["tasks_error"].as_i64().unwrap_or(0);
                    let revoked = t["tasks_revoked"].as_i64().unwrap_or(0);
                    let late = t["tasks_toolate"].as_i64().unwrap_or(0);
                    vec![
                        view! { <a href={format!("/checker_status?tick={}", s(&t["number"]))}>{s(&t["number"])}</a> }.into_any(),
                        short_ts(&t["start"]).into_any(),
                        s(&t["time"]).into_any(),
                        s(&t["dispatched"]).into_any(),
                        view! { <span class=(if ok > 0 { "ok" } else { "" })>{s(&t["tasks_ok"])}</span> }.into_any(),
                        view! { <span class=(if warn > 0 { "warn" } else { "" })>{s(&t["tasks_warn"])}</span> }.into_any(),
                        view! { <span class=(if err > 0 { "err" } else { "" })>{s(&t["tasks_error"])}</span> }.into_any(),
                        view! { <span class=(if revoked > 0 { "err" } else { "" })>{s(&t["tasks_revoked"])}</span> }.into_any(),
                        view! { <span class=(if late > 0 { "warn" } else { "" })>{s(&t["tasks_toolate"])}</span> }.into_any(),
                        short_ts(&t["last_finished"]).into_any(),
                    ]
                }).collect();
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
        <div class="sub">
            "filters apply live · status colors: "
            <span class="ok">"SUCCESS"</span> " "
            <span class="warn">"TIMEOUT/MUMBLE/OFFLINE"</span> " "
            <span class="err">"REVOKED/CRASHED"</span>
        </div>
        <div class="toolbar">
            <label>"team"</label>
            <input style="width: 70px" placeholder="—" prop:value=team on:input=move |e| { set_team.set(event_target_value(&e)); set_page.set(1); } />
            <label>"service"</label>
            <input style="width: 70px" placeholder="—" prop:value=service on:input=move |e| { set_service.set(event_target_value(&e)); set_page.set(1); } />
            <label>"status"</label>
            <select prop:value=status on:change=move |e| { set_status.set(event_target_value(&e)); set_page.set(1); }>
                <option value="">"any status"</option>
                {["SUCCESS", "FLAGMISSING", "MUMBLE", "OFFLINE", "RECOVERING", "TIMEOUT", "REVOKED", "CRASHED", "PENDING"].iter()
                    .map(|s| view! { <option value={*s}>{*s}</option> }).collect::<Vec<_>>()}
            </select>
            <span class="dim">
                {move || match data.get() {
                    Some(Ok(v)) => format!("{} results total", s(&v["count"])),
                    _ => String::new(),
                }}
            </span>
        </div>
        {move || match data.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let pages = v["pages"].as_u64().unwrap_or(1) as usize;
                let teams = v["teams"].as_array().cloned().unwrap_or_default();
                let svcs = v["services"].as_array().cloned().unwrap_or_default();
                let team_label = |id: i64| teams.iter()
                    .find(|t| t["id"].as_i64() == Some(id))
                    .map(|t| format!("{id} {}", s(&t["name"])))
                    .unwrap_or_else(|| id.to_string());
                let svc_label = |id: i64| svcs.iter()
                    .find(|svc| svc["id"].as_i64() == Some(id))
                    .map(|svc| s(&svc["name"]))
                    .unwrap_or_else(|| id.to_string());
                let rows = v["results"].as_array().cloned().unwrap_or_default().iter().map(|r| {
                    let st = s(&r["status"]);
                    let cls = status_cls(&st);
                    let pill_cls = if cls.is_empty() { "pill".to_string() } else { format!("pill {cls}") };
                    let status_view = view! { <span class=pill_cls>{st.clone()}</span> }.into_any();
                    vec![
                        view! { <a href={format!("/checker_results/view/{}", s(&r["id"]))}>{s(&r["id"])}</a> }.into_any(),
                        s(&r["tick"]).into_any(),
                        team_label(r["team_id"].as_i64().unwrap_or(0)).into_any(),
                        svc_label(r["service_id"].as_i64().unwrap_or(0)).into_any(),
                        status_view,
                        s_opt(&r["message"]).into_any(),
                        r["time"].as_f64().map(|t| format!("{t:.2}s")).unwrap_or_default().into_any(),
                        short_ts(&r["finished"]).into_any(),
                    ]
                }).collect();
                view! {
                    {table(&["ID", "Tick", "Team", "Service", "Status", "Message", "Time", "Finished"], rows)}
                    {pager("checker_results", move || page.get(), move || pages, move |d| set_page.set((page.get() as isize + d).max(1) as usize))}
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
        <h1>{move || format!("Checker Result {}", id())}</h1>
        <div class="sub"><a href="/checker_results">"← all results"</a></div>
        {move || match result.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let st = s(&v["status"]);
                let cls = status_cls(&st);
                let out = v["output"].as_str().unwrap_or("").to_string();
                let lines: Vec<AnyView> = if out.is_empty() {
                    vec![view! { <span class="dim">"(no output)"</span> }.into_any()]
                } else {
                    out.split_inclusive('\n').enumerate().map(|(i, l)| {
                        let line_cls = if i == 0 { "cin" } else if is_error_line(l) { "cerr" } else { "cout" };
                        view! { <span class=line_cls>{l.to_string()}</span> }.into_any()
                    }).collect()
                };
                view! {
                    <div class="facts">
                        <div class="tile"><div class="k">"tick"</div><div class="v">{s(&v["tick"])}</div></div>
                        <div class="tile"><div class="k">"team"</div><div class="v">{s(&v["team_id"])}</div></div>
                        <div class="tile"><div class="k">"service"</div><div class="v">{s(&v["service_id"])}</div></div>
                        <div class="tile"><div class="k">"status"</div><div class="v"><span class=cls>{st}</span></div></div>
                        <div class="tile">
                            <div class="k">"time"</div>
                            <div class="v">{v["time"].as_f64().map(|t| format!("{t:.2}s")).unwrap_or_else(|| "—".into())}</div>
                        </div>
                    </div>
                    <h2>"Checker output"</h2>
                    <pre>{lines}</pre>
                }.into_any()
            }
        }}
    }
}

fn is_error_line(line: &str) -> bool {
    let low = line.to_lowercase();
    ["error", "exception", "failed", "not found", "mumble", "crashed", "timeout", "refused", "denied"]
        .iter().any(|needle| low.contains(needle))
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
        <div class="sub">"paste a flag to decode it (HMAC + tick/team/service fields)"</div>
        <form class="row" on:submit=|e| e.prevent_default()>
            <input class="flag-input" placeholder="FLAG{...}" prop:value=flag on:input=move |e| set_flag.set(event_target_value(&e)) />
            <button type="button" class="primary" on:click=move |_| set_submitted.set(flag.get())>"Analyze"</button>
        </form>
        {move || match data.get() {
            None => "".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let analysis = if v["flag"].is_null() {
                    "".into_any()
                } else {
                    let valid = v["valid"].as_bool().unwrap_or(false);
                    let repaired = v["repaired_flag"].as_str().map(str::to_string);
                    let has_repaired = repaired.is_some();
                    view! {
                        <h2>"Analysis"</h2>
                        {if let Some(err) = v["err"].as_str() {
                            view! { <p class="err">{format!("Error: {err}")}</p> }.into_any()
                        } else { "".into_any() }}
                        <div class="decode">
                            <span class="k">"valid"</span>
                            <span class=if valid { "ok" } else { "err" }>{if valid { "VALID flag (MAC verified)" } else { "INVALID flag" }}</span>
                            <span class="k">"stored tick"</span><span>{s(&v["stored_tick"])}</span>
                            <span class="k">"team"</span>
                            <span>{format!("{} — {}", s(&v["teamid"]), s_opt(&v["team"]["name"]))}</span>
                            <span class="k">"service"</span>
                            <span>{format!("{} — {}", s(&v["serviceid"]), s_opt(&v["service"]["name"]))}</span>
                            <span class="k">"payload"</span><span><code>{s(&v["payload"])}</code></span>
                            <span class="k">"MAC"</span>
                            <span class=if valid { "ok" } else { "err" }>{format!("{} (expected {})", s(&v["mac"]), s(&v["real_mac"]))}</span>
                            <span class="k">"repaired"</span>
                            <span class=if has_repaired { "warn" } else { "" }>
                                {repaired.unwrap_or_else(|| "—".into())}
                            </span>
                        </div>
                    }.into_any()
                };
                let submissions = if valid_submissions(&v) {
                    view! {
                        <h2>"Submissions of this flag"</h2>
                        {table(&["Submitter", "Tick", "Timestamp"],
                            v["submissions"].as_array().cloned().unwrap_or_default().iter().map(|f| vec![
                                format!("team {}", s(&f["submitted_by"])).into_any(),
                                s(&f["tick_submitted"]).into_any(),
                                short_ts(&f["ts"]).into_any(),
                            ]).collect())}
                    }.into_any()
                } else { "".into_any() };
                let demo = if v["demo_flags"].is_array() {
                    view! {
                        <h2>"Demo flags (status checks)"</h2>
                        {table(&["Purpose", "Expected", "Flag"],
                            v["demo_flags"].as_array().cloned().unwrap_or_default().iter().map(|d| {
                                let expect = s(&d["expect"]);
                                let low = expect.to_lowercase();
                                let cls = if low.contains("repair") {
                                    "warn"
                                } else if low.contains("invalid") || low.contains("expired") || low.contains("error") {
                                    "err"
                                } else {
                                    "ok"
                                };
                                vec![
                                    s(&d["title"]).into_any(),
                                    view! { <span class=cls>{expect}</span> }.into_any(),
                                    view! { <code>{s(&d["flag"])}</code> }.into_any(),
                                ]
                            }).collect())}
                    }.into_any()
                } else { "".into_any() };
                view! { {analysis} {submissions} {demo} }.into_any()
            }
        }}
    }
}

fn valid_submissions(v: &Value) -> bool {
    v["valid"].as_bool().unwrap_or(false) && v["submissions"].is_array()
}
