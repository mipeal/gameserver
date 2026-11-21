//! Port of `page-index` (scoretable) + the team-only dashboard and the
//! shared `table-line-cells` / `table-service-header-cell` fragments.

use crate::models::*;
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;

/// `floatToString`: `(n < 0 ? '' : '+') + n.toFixed(1)`.
// allow: rustc's liveness analysis misses uses inside leptos `view!` closures.
#[allow(dead_code)]
pub fn float_to_string(n: f64) -> String {
    // exact zero (including -0.0) must not print as "-0.0"
    if n == 0.0 { return "+0.0".into(); }
    format!("{n:+.1}")
}

/// `floatToString(n, zeroIsNegative)`; zero prints unsigned so real data's
/// float noise never shows up as "-0.0".
#[allow(dead_code)]
pub fn float_to_string_zn(n: f64) -> String {
    if n == 0.0 { return "0.0".into(); }
    let prefix = if n > 0.0 { "+" } else { "" };
    format!("{prefix}{n:.1}")
}

/// Unsigned one-decimal point value; exact zero (also -0.0) prints "0.0".
pub fn float_value(n: f64) -> String {
    if n == 0.0 { return "0.0".into(); }
    format!("{n:.1}")
}

/// "-n" for a positive count, plain "0" for zero (never "-0").
pub fn neg_count(n: i64) -> String {
    if n == 0 { "0".into() } else { format!("-{n}") }
}

/// intToString(n, zeroIsNegative) — used for the flag-count deltas.
#[allow(dead_code)]
fn int_to_string_zn(n: i64) -> String {
    if n > 0 { "+".into() } else if n == 0 { "-".into() } else { String::new() }
}

/// `table-service-header-cell`: one `<th>` per service.
pub fn service_header_cells(services: &[Service]) -> impl IntoView {
    services.iter().map(|service| {
        let name = service.name.clone();
        let attackers = service.attackers.clone();
        let victims = service.victims.clone();
        let first_blood = service.first_blood.clone();
        let (flag_stores, flag_stores_exploited) = (service.flag_stores, service.flag_stores_exploited);
        // full first-blood list for the (possibly ellipsed) single-line header
        let first_blood_title = first_blood
            .iter()
            .map(|fb| if fb.confirmed { fb.name.clone() } else { format!("{}?", fb.name) })
            .collect::<Vec<_>>()
            .join(", ");
        view! {
            <th class="servicecell" scope="col">
                <p><strong>{name}</strong></p>
                <p class="attacker-victim-count">
                    <span class="av-line">
                        <i class="fas fa-crosshairs"></i>
                        {move || count_with_unit(&attackers, "attackers", "attacker")}
                    </span>
                    <span class="av-line">
                        <i class="fas fa-shield-alt"></i>
                        {move || count_with_unit(&victims, "victims", "victim")}
                    </span>
                    {move || {
                        if first_blood.is_empty() { return ().into_any(); }
                        let fbs = first_blood.clone();
                        let fbs_first = fbs.clone();
                        view! {
                            <span class="av-line fb-line" title=first_blood_title.clone()>
                                {move || {
                                    if fbs_first.first().map(|fb| fb.confirmed).unwrap_or(false) {
                                        view! { <i class="fas fa-tint"></i> }.into_any()
                                    } else {
                                        view! { <i class="fas fa-tint text-muted"></i> }.into_any()
                                    }
                                }}
                                {fbs.iter().map(|fb| {
                                    let (name, confirmed) = (fb.name.clone(), fb.confirmed);
                                    if confirmed {
                                        view! { <span class="firstblood confirmed">{name}</span> }.into_any()
                                    } else {
                                        view! {
                                            <span class="firstblood text-muted unconfirmed" title="Not confirmed until more teams get pwned.">
                                                {format!("{name}?")}
                                            </span>
                                        }.into_any()
                                    }
                                }).collect_view()}
                            </span>
                        }.into_any()
                    }}
                    {move || {
                        if flag_stores_exploited > 0 && flag_stores_exploited < flag_stores {
                            let unit = if flag_stores != 1 { "flag stores" } else { "flag store" };
                            view! {
                                <small class="av-line" title={format!("This services stores flags in {flag_stores} different locations or flags of {flag_stores} different types. Only {flag_stores_exploited} flag stores / types have been exploited so far.")}>
                                    {format!(" {}/{flag_stores} ", flag_stores - flag_stores_exploited)}{unit}
                                    "unexploited"
                                </small>
                            }.into_any()
                        } else { ().into_any() }
                    }}
                </p>
            </th>
        }
    }).collect_view()
}

/// `N  attackers` / frozen `? attackers` rendering.
fn count_with_unit(v: &serde_json::Value, plural: &str, singular: &str) -> AnyView {
    if v.as_i64() == Some(1) {
        view! { {format!("1  {singular}")} }.into_any()
    } else if let Some(n) = v.as_i64() {
        view! { {format!("{n}  {plural}")} }.into_any()
    } else {
        view! {
            <span class="frozen" title="Scoreboard is frozen, no new data available">"█"</span>
            {format!(" {plural}")}
        }.into_any()
    }
}

/// `table-line-cells`: one merged SCORE cell (attack / defense / SLA, icons only)
/// plus one cell per service. The team page (`with_history == false`) keeps its
/// per-tick deltas and the total row; the scoretable shows neither.
#[allow(clippy::too_many_arguments)]
pub fn line_cells(
    rank: &Rank,
    tick: i64,
    with_history: bool,
    show_history: RwSignal<bool>,
    show_only_sums: RwSignal<bool>,
) -> impl IntoView {
    let services = rank.services.clone();
    let delta = move |value: String, title: &'static str| -> AnyView {
        view! { <span class="delta"><small title=title>{value}</small></span> }.into_any()
    };
    let (d_att, d_def, d_sla, d_tot) = (
        format!("({})", float_to_string(rank.do_)),
        format!("({})", float_to_string_zn(rank.dd)),
        format!("({})", float_to_string(rank.ds)),
        format!("({})", float_to_string(rank.do_ + rank.dd + rank.ds)),
    );

    view! {
        <td class="num score">
            <div class="score-line att">
                <span class="sr-only">"Attack"</span>
                <i class="fas fa-crosshairs" aria-hidden="true" title="Attack"></i>
                <span class="v">{float_value(rank.o)}</span>
                {if with_history { ().into_any() } else { delta(d_att.clone(), "New offensive points this tick") }}
            </div>
            <div class="score-line def">
                <span class="sr-only">"Defense"</span>
                <i class="fas fa-shield-alt" aria-hidden="true" title="Defense"></i>
                <span class="v">{float_value(rank.d)}</span>
                {if with_history { ().into_any() } else { delta(d_def.clone(), "Lost defensive points this tick") }}
            </div>
            <div class="score-line sla">
                <span class="sr-only">"SLA"</span>
                <i class="fas fa-tachometer-alt" aria-hidden="true" title="SLA"></i>
                <span class="v">{float_value(rank.s)}</span>
                {if with_history { ().into_any() } else { delta(d_sla.clone(), "New SLA points this tick") }}
            </div>
            {(!with_history).then(|| view! {
                <div class="score-line total">
                    <span class="sr-only">"Total"</span>
                    <b>{float_value(rank.points)}</b>
                    {delta(d_tot.clone(), "New points this tick")}
                </div>
            })}
        </td>
        {services.iter().map(|sr| {
            let sr = sr.clone();
            let sr_h = sr.clone();
            let sr_s = sr.clone();
            let dc = sr.dc.clone();
            let (m, tick) = (sr.m.clone(), tick);
            view! {
                <td>
                    <div class="result-container">
                        // slim (sum) mode cell contents
                        {move || {
                            let sr = sr_s.clone();
                            if !show_only_sums.get() { return ().into_any(); }
                            view! {
                                <i class="fas fa-chart-line sum-mode-only"></i>
                                <div title="Points" class="text-nowrap points sum-mode-only">
                                    {float_value(sr.o + sr.d + sr.s)}
                                </div>
                                <span class="delta sum-mode-only">
                                    <small title="New points this tick">{format!("({})", float_to_string(sr.do_ + sr.dd + sr.ds))}</small>
                                </span>
                                <i class="fas fa-flag sum-mode-only" title="Captured / stolen flags"></i>
                                <div title="Captured / stolen flags" class="text-nowrap sum-mode-only">
                                    {format!("{} / -{}", sr.cap, sr.st)}
                                </div>
                                <span class="delta sum-mode-only">
                                    <small title="Captured flags last tick">{format!("(+{}/{})", sr.dcap, neg_count(sr.dst))}</small>
                                </span>
                            }.into_any()
                        }}
                        // detailed mode: two columns — points left, flags right
                        {move || {
                            let sr = sr_h.clone();
                            if show_only_sums.get() { return ().into_any(); }
                            view! {
                                <div class="svc-grid not-in-sum-mode">
                                    <div class="svc-col">
                                        <div class="svc-row" title="Offensive points">
                                            <i class="fas fa-crosshairs"></i>
                                            <span class="points">{float_value(sr.o)}</span>
                                            <span class="delta"><small>{format!("({})", float_to_string(sr.do_))}</small></span>
                                        </div>
                                        <div class="svc-row" title="Defensive points">
                                            <i class="fas fa-shield-alt"></i>
                                            <span class="points">{float_value(sr.d)}</span>
                                            <span class="delta"><small>{format!("({})", float_to_string_zn(sr.dd))}</small></span>
                                        </div>
                                        <div class="svc-row" title="SLA points">
                                            <i class="fas fa-tachometer-alt"></i>
                                            <span class="points">{float_value(sr.s)}</span>
                                            <span class="delta"><small>{format!("({})", float_to_string(sr.ds))}</small></span>
                                        </div>
                                    </div>
                                    <div class="svc-col">
                                        <div class="svc-row" title="Captured flags">
                                            <i class="fas fa-flag"></i>
                                            <span class="points">{sr.cap}</span>
                                            <span class="delta"><small>{format!("(+{})", sr.dcap)}</small></span>
                                        </div>
                                        <div class="svc-row" title="Lost flags">
                                            <i class="fas fa-flag-checkered"></i>
                                            <span class="points">{neg_count(sr.st)}</span>
                                            <span class="delta"><small>{format!("({})", neg_count(sr.dst))}</small></span>
                                        </div>
                                    </div>
                                </div>
                            }.into_any()
                        }}
                        // status column (always)
                        {move || {
                            let sr = sr.clone();
                            let dc = dc.clone();
                            let m = m.clone();
                            let status = sr.c.clone();
                            if !show_history.get() { return ().into_any(); }
                            view! {
                                <div class="sla-col offset-left">
                                    {(0..3).filter_map(|i| {
                                        // dc[2] → history-3 (tick-3), dc[1] → history-2, dc[0] → history-1
                                        let st = dc.get(2 - i)?;
                                        let idx = 3 - i;
                                        let title = format!("Tick {}: {}", tick - idx as i64, status_to_tooltip(st));
                                        Some(view! {
                                            <span class={format!("label history-{idx} {}", status_to_class(st))} title=title></span>
                                        })
                                    }).collect_view()}
                                    <span class="sep" aria-hidden="true"></span>
                                    <span class={format!("status label {}", status_to_class(&status))} title={format!("Tick {}: {}", tick, status_to_tooltip(&status))}>
                                        {status_to_text(&status)}
                                    </span>
                                    {move || {
                                        let m = m.clone();
                                        m.map(|m| view! {
                                            <span class="pop-host">
                                                <button type="button" class="btn btn-xs btn-link info-link" aria-label="Checker Infos">
                                                    <span class="fas fa-info"></span>
                                                </button>
                                                <span class="popover fade show" role="tooltip">
                                                    <span class="arrow"></span>
                                                    <h3 class="popover-title">{format!("Tick {}: {}", tick, sr.c)}</h3>
                                                    <div class="popover-content">{m.clone()}</div>
                                                </span>
                                            </span>
                                        }.into_any()).unwrap_or_else(|| ().into_any())
                                    }}
                                </div>
                            }.into_any()
                        }}
                    </div>
                </td>
            }
        }).collect_view().into_any()}
    }
}

/// `ScoretableComponent` (page-index).
pub fn scoretable(store: Store, ui: Ui) -> impl IntoView {
    let shown_tick = RwSignal::new(-1i64);
    // upstream shownTickIsRecent: auto-follow until the user navigates away
    let recent = RwSignal::new(true);

    Effect::new(move || {
        let newest = store.newest_scoreboard_tick.get();
        if recent.get() && newest >= 0 {
            shown_tick.set(newest);
        }
    });

    let info = Memo::new(move |_| {
        let tick = shown_tick.get();
        if tick < 0 { None } else { store.ranking(tick) }
    });

    let max_tick = Memo::new(move |_| store.state.get().scoreboard_tick);

    let nav = move |target: i64| {
        recent.set(target == max_tick.get_untracked());
        shown_tick.set(target);
    };

    view! {
        {move || match info.get() {
            None => view! { <p class="loading"><span class="fas fa-spinner fa-spin"></span></p> }.into_any(),
            Some(info) => {
                let tick = info.tick;
                let services = info.services.clone();
                let mut ranks = info.scoreboard.clone();
                ranks.sort_by_key(|r| r.rank);
                let shown = shown_tick.get();
                let max = max_tick.get_untracked();
                let spinner = shown != tick;
                // confirmed first bloods per team name, across all services
                let mut first_bloods: std::collections::HashMap<String, usize> = Default::default();
                for svc in &info.services {
                    for fb in svc.first_blood.iter().filter(|fb| fb.confirmed) {
                        *first_bloods.entry(fb.name.clone()).or_default() += 1;
                    }
                }
                view! {
                    <table
                        class="table table-bordered table-stickyhead"
                        class:hideHistory=move || !ui.show_history.get()
                        class:onlySums=move || ui.show_only_sums.get()
                    >
                        <thead>
                            <tr>
                                <th colspan="2" scope="col" class="headercell">
                                    <h4>
                                        {if tick < 0 { "No results yet" } else { "Results from:" }}
                                        {if shown > 0 {
                                            view! {
                                                <a class="btn btn-link" title="Go to first tick" on:click=move |_| nav(1)>
                                                    <i class="fas fa-fast-backward"></i>
                                                </a>
                                                <a class="btn btn-link" title="Previous tick" on:click=move |_| nav(shown - 1)>
                                                    <i class="fas fa-chevron-left"></i>
                                                </a>
                                            }.into_any()
                                        } else { ().into_any() }}
                                        {if tick < 0 { "".into_any() } else { format!("Tick {tick}").into_any() }}
                                        {if shown < max {
                                            view! {
                                                <a class="btn btn-link" title="Next tick" on:click=move |_| nav(shown + 1)>
                                                    <i class="fas fa-chevron-right"></i>
                                                </a>
                                                <a class="btn btn-link" title="Go to current tick" on:click=move |_| nav(max)>
                                                    <i class="fas fa-fast-forward"></i>
                                                </a>
                                            }.into_any()
                                        } else { ().into_any() }}
                                        {if spinner {
                                            view! { <span class="fas fa-spinner fa-spin"></span> }.into_any()
                                        } else { ().into_any() }}
                                    </h4>
                                </th>
                                <th class="num score" scope="col" title="Attack / defense / SLA / total points">"Score"</th>
                                {service_header_cells(&services)}
                            </tr>
                        </thead>
                        <tbody>
                            {ranks.iter().filter_map(|rank| {
                                let team = store.team(rank.team_id)?;
                                let rank = rank.clone();
                                let team_id = rank.team_id;
                                let name = team.name.clone();
                                let logo = team.logo.clone();
                                let banned = store.banned(team_id);
                                let title = if banned {
                                    "Network access is currently blocked. Please contact the organizers in IRC!".to_string()
                                } else {
                                    format!("{name} - See team's details")
                                };
                                let aria = format!("{name} Details");
                                let fb = first_bloods.get(&name).copied().unwrap_or(0);
                                Some(view! {
                                    <tr
                                        class:top1=(rank.rank == 1 && rank.points > 0.0)
                                        class:top2=(rank.rank == 2 && rank.points > 0.0)
                                        class:top3=(rank.rank == 3 && rank.points > 0.0)
                                    >
                                        <td class="rankcell"><h4>{rank.rank}</h4></td>
                                        <td class="teamcell">
                                            {move || {
                                                if !ui.show_images.get() || logo.is_empty() || logo == "false" {
                                                    return ().into_any();
                                                }
                                                view! {
                                                    <div class="media-left team-logo-media">
                                                        <a href={format!("/team/{team_id}")} aria-label={aria.clone()}>
                                                            <img class="media-object" src={format!("logos/{logo}")} alt="" loading="lazy" />
                                                        </a>
                                                    </div>
                                                }.into_any()
                                            }}
                                            <div class="media-body">
                                                <h4 class="team-name" class:blocked=banned>
                                                    <a href={format!("/team/{team_id}")} title=title>{name.clone()}</a>
                                                    {(!team.aff.is_empty() || !team.web.is_empty()).then(|| view! {
                                                        <span class="pop-host">
                                                            <button type="button" class="btn btn-xs btn-link info-link" aria-label="Team Infos">
                                                                <span class="fas fa-info"></span>
                                                            </button>
                                                            <span class="popover fade show" role="tooltip">
                                                                <span class="arrow"></span>
                                                                <div class="popover-content">
                                                                    {format!("Affiliation: {}; Website: {}",
                                                                        if team.aff.is_empty() { "-" } else { &team.aff },
                                                                        if team.web.is_empty() { "-" } else { &team.web })}
                                                                </div>
                                                            </span>
                                                        </span>
                                                    })}
                                                </h4>
                                                <span class="team-vulnbox">{team.vulnbox.clone()}</span>
                                                {(fb > 0).then(|| view! {
                                                    <span
                                                        class="fb-count"
                                                        title="Confirmed first bloods this game"
                                                        aria-label={format!("{fb} confirmed first bloods")}
                                                    >
                                                        <svg class="drop" viewBox="0 0 16 16" aria-hidden="true">
                                                            <path d="M8 1.5c2.7 3.4 4.6 6 4.6 8.2a4.6 4.6 0 0 1-9.2 0C3.4 7.5 5.3 4.9 8 1.5z"/>
                                                        </svg>
                                                        {fb}
                                                    </span>
                                                })}
                                            </div>
                                        </td>
                                        {line_cells(&rank, tick, true, ui.show_history, ui.show_only_sums)}
                                    </tr>
                                })
                            }).collect_view().into_any()}
                        </tbody>
                    </table>
                }.into_any()
            }
        }}
    }
}

const DASH_TEAM_KEY: &str = "dashboardTeam";

fn ls_get_team_id() -> Option<i64> {
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    storage.get_item(DASH_TEAM_KEY).ok().flatten()?.parse().ok()
}

fn ls_set_team_id(id: i64) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(DASH_TEAM_KEY, &id.to_string());
    }
}

/// Team-only dashboard: a persisted team picker, then that team's place,
/// points and flag totals plus its per-service checker status. Only fields
/// backed by the round/team models are shown (no vulnbox checks, no per-theft
/// attacker log — the API carries neither).
pub fn dashboard(store: Store, _ui: Ui) -> impl IntoView {
    // localStorage-backed, mirroring the Ui preference signals
    let selected = RwSignal::new(ls_get_team_id().unwrap_or(-1));

    let team_list = Memo::new(move |_| {
        let mut teams: Vec<Team> = store.teams.get().values().cloned().collect();
        teams.sort_by_key(|t| t.id);
        teams
    });

    // default to the first team when nothing (valid) is stored
    Effect::new(move || {
        let teams = team_list.get();
        let current = selected.get();
        if teams.is_empty() {
            return;
        }
        if current < 0 || !teams.iter().any(|t| t.id == current) {
            selected.set(teams[0].id);
        }
    });
    Effect::new(move || {
        let id = selected.get();
        if id >= 0 {
            ls_set_team_id(id);
        }
    });

    let info = Memo::new(move |_| {
        let tick = store.newest_scoreboard_tick.get();
        let id = selected.get();
        if tick < 0 || id < 0 {
            return None;
        }
        let round = store.ranking(tick)?;
        let rank = round.scoreboard.iter().find(|r| r.team_id == id)?.clone();
        Some((round, rank))
    });

    view! {
        <div class="page-dashboard">
            <h1>"MY DASHBOARD"</h1>
            <div class="sub">
                {move || {
                    let id = selected.get();
                    match team_list.get().iter().find(|t| t.id == id) {
                        Some(team) if id >= 0 => {
                            format!("private view for team {id} — {}", team.name)
                        }
                        _ => "private view for your team only · checks and flags".to_string(),
                    }
                }}
            </div>
            {move || {
                let teams = team_list.get();
                if teams.is_empty() {
                    return view! {
                        <p class="loading"><span class="fas fa-spinner fa-spin"></span>" loading..."</p>
                    }.into_any();
                }
                view! {
                    <div class="dash-toolbar">
                        <label for="dash-team-picker">"team"</label>
                        <select
                            id="dash-team-picker"
                            prop:value=move || selected.get().to_string()
                            on:change=move |ev| {
                                if let Ok(id) = event_target_value(&ev).parse::<i64>() {
                                    selected.set(id);
                                }
                            }
                        >
                            {teams.iter().map(|team| {
                                let (id, name) = (team.id, team.name.clone());
                                view! {
                                    <option value=id.to_string()>
                                        {format!("team {id} — {name}")}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    {move || match info.get() {
                        None => view! {
                            <p class="loading"><span class="fas fa-spinner fa-spin"></span>" loading..."</p>
                        }.into_any(),
                        Some((round, rank)) => {
                            let services = round.services.clone();
                            let results = rank.services.clone();
                            let captured: i64 = results.iter().map(|s| s.cap).sum();
                            let stolen: i64 = results.iter().map(|s| s.st).sum();
                            view! {
                                <div class="facts">
                                    <div class="tile">
                                        <div class="k">"place"</div>
                                        <div class="v">
                                            {if rank.rank > 0 { rank.rank.to_string() } else { "-".into() }}
                                        </div>
                                    </div>
                                    <div class="tile">
                                        <div class="k">"total"</div>
                                        <div class="v">{float_value(rank.points)}</div>
                                    </div>
                                    <div class="tile">
                                        <div class="k">"flags captured"</div>
                                        <div class="v ok">{captured}</div>
                                    </div>
                                    <div class="tile">
                                        <div class="k">"flags stolen from you"</div>
                                        <div class="v err">{stolen}</div>
                                    </div>
                                </div>
                                <div class="cols">
                                    <div>
                                        <h2>"Your checks (last tick)"</h2>
                                        <table class="table table-bordered dash-table">
                                            <thead>
                                                <tr>
                                                    <th scope="col">"Service"</th>
                                                    <th scope="col">"Status"</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {services.iter().enumerate().map(|(i, service)| {
                                                    let status = results
                                                        .get(i)
                                                        .map(|r| r.c.clone())
                                                        .filter(|s| !s.is_empty())
                                                        .unwrap_or_else(|| "PENDING".to_string());
                                                    let class = status_to_class(&status).to_string();
                                                    let title = status_to_tooltip(&status).to_string();
                                                    view! {
                                                        <tr>
                                                            <td>{service.name.clone()}</td>
                                                            <td>
                                                                <span class=format!("label {class} status-pill") title=title>
                                                                    {status}
                                                                </span>
                                                            </td>
                                                        </tr>
                                                    }
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                    <div>
                                        <h2>"Your flags"</h2>
                                        <table class="table table-bordered dash-table">
                                            <thead>
                                                <tr>
                                                    <th scope="col">"Service"</th>
                                                    <th scope="col" class="num">"Captured"</th>
                                                    <th scope="col" class="num">"Stolen from you"</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {services.iter().enumerate().map(|(i, service)| {
                                                    let (cap, stolen) = results
                                                        .get(i)
                                                        .map(|r| (r.cap, r.st))
                                                        .unwrap_or((0, 0));
                                                    view! {
                                                        <tr>
                                                            <td>{service.name.clone()}</td>
                                                            <td class="num ok">{cap}</td>
                                                            <td class="num err">{stolen}</td>
                                                        </tr>
                                                    }
                                                }).collect_view()}
                                            </tbody>
                                        </table>
                                    </div>
                                </div>
                            }.into_any()
                        }
                    }}
                }.into_any()
            }}
        </div>
    }
}
