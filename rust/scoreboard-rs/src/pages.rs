//! Port of `page-index` (scoretable) + `page-dashboard` (teamgrid) and the
//! shared `table-line-cells` / `table-service-header-cell` fragments.

use crate::models::*;
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;

/// `floatToString`: `(n < 0 ? '' : '+') + n.toFixed(1)`.
// allow: rustc's liveness analysis misses uses inside leptos `view!` closures.
#[allow(dead_code)]
pub fn float_to_string(n: f64) -> String {
    format!("{n:+.1}")
}

/// `floatToString(n, zeroIsNegative)`.
#[allow(dead_code)]
pub fn float_to_string_zn(n: f64) -> String {
    let prefix = if n > 0.0 { "+" } else if n == 0.0 { "-" } else { "" };
    format!("{prefix}{n:.1}")
}

/// intToString(n, zeroIsNegative) — used for the flag-count deltas.
#[allow(dead_code)]
fn int_to_string_zn(n: i64) -> String {
    if n > 0 { "+".into() } else if n == 0 { "-".into() } else { String::new() }
}

/// `table-service-header-cell`: one `<td>` per service.
pub fn service_header_cells(services: &[Service]) -> impl IntoView {
    services.iter().map(|service| {
        let name = service.name.clone();
        let attackers = service.attackers.clone();
        let victims = service.victims.clone();
        let first_blood = service.first_blood.clone();
        let (flag_stores, flag_stores_exploited) = (service.flag_stores, service.flag_stores_exploited);
        view! {
            <td class="servicecell">
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
                            <span class="av-line" title="First Blood">
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
            </td>
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

/// `table-line-cells`: sum cell + one cell per service.
pub fn line_cells(rank: &Rank, tick: i64, show_history: RwSignal<bool>, show_only_sums: RwSignal<bool>) -> impl IntoView {
    let rank0 = rank.clone();
    let services = rank.services.clone();

    view! {
        // sum cell (not shown in slim mode)
        {move || {
            let rank = rank0.clone();
            if show_only_sums.get() { return ().into_any(); }
            view! {
                <td>
                    <div class="sum-container">
                        <i class="fas fa-crosshairs"></i>
                        <span title="Offensive points" class="text-nowrap points">{format!("{:.1}", rank.o)}</span>
                        <i class="fas fa-shield-alt"></i>
                        <span title="Defensive points" class="text-nowrap points">{format!("{:.1}", rank.d)}</span>
                        <i class="fas fa-tachometer-alt"></i>
                        <span title="SLA points" class="text-nowrap points">{format!("{:.1}", rank.s)}</span>
                    </div>
                </td>
            }.into_any()
        }}
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
                                    {format!("{:.1}", sr.o + sr.d + sr.s)}
                                </div>
                                <span class="delta sum-mode-only">
                                    <small title="New points this tick">{format!("({})", float_to_string(sr.do_ + sr.dd + sr.ds))}</small>
                                </span>
                                <i class="fas fa-flag sum-mode-only" title="Captured / stolen flags"></i>
                                <div title="Captured / stolen flags" class="text-nowrap sum-mode-only">
                                    {format!("{} / -{}", sr.cap, sr.st)}
                                </div>
                                <span class="delta sum-mode-only">
                                    <small title="Captured flags last tick">{format!("(+{}/-{})", sr.dcap, sr.dst)}</small>
                                </span>
                            }.into_any()
                        }}
                        // detailed mode contents
                        {move || {
                            let sr = sr_h.clone();
                            if show_only_sums.get() { return ().into_any(); }
                            view! {
                                <i class="fas fa-crosshairs not-in-sum-mode"></i>
                                <div title="Offensive points" class="text-nowrap points not-in-sum-mode">{format!("{:.1}", sr.o)}</div>
                                <span class="delta not-in-sum-mode">
                                    <small title="New offensive points this tick">{format!("({})", float_to_string(sr.do_))}</small>
                                </span>
                                <i class="fas fa-flag offset-left not-in-sum-mode" title="Captured flags"></i>
                                <div title="Captured flags" class="text-nowrap points not-in-sum-mode">{sr.cap}</div>
                                <div class="text-nowrap">
                                    <span class="delta not-in-sum-mode"><small title="Captured flags last tick">{format!("(+{})", sr.dcap)}</small></span>
                                </div>
                                <i class="fas fa-shield-alt not-in-sum-mode"></i>
                                <div title="Defensive points" class="text-nowrap points not-in-sum-mode">{format!("{:.1}", sr.d)}</div>
                                <span class="delta not-in-sum-mode">
                                    <small title="Lost defensive points this tick">{format!("({})", float_to_string_zn(sr.dd))}</small>
                                </span>
                                <i class="fas fa-flag-checkered offset-left not-in-sum-mode" title="Lost flags"></i>
                                <div title="Lost flags" class="text-nowrap points not-in-sum-mode">{format!("-{}", sr.st)}</div>
                                <div class="text-nowrap">
                                    <span class="delta not-in-sum-mode"><small title="Lost flags last tick">{format!("(-{})", sr.dst)}</small></span>
                                </div>
                                <i class="fas fa-tachometer-alt not-in-sum-mode"></i>
                                <div title="SLA points" class="text-nowrap points not-in-sum-mode">{format!("{:.1}", sr.s)}</div>
                                <div class="delta not-in-sum-mode"><small title="New SLA points this tick">{format!("({})", float_to_string(sr.ds))}</small></div>
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
                                    <span class={format!("status label {}", status_to_class(&status))} title={status_to_tooltip(&status).to_string()}>
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
                view! {
                    <table
                        class="table table-bordered table-stickyhead"
                        class:hideHistory=move || !ui.show_history.get()
                        class:onlySums=move || ui.show_only_sums.get()
                    >
                        <thead>
                            <tr>
                                <th colspan="2" class="headercell">
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
                                // the sum cell (team o/d/s totals) only exists outside slim mode
                                {move || {
                                    if ui.show_only_sums.get() { return ().into_any(); }
                                    view! { <th class="headercell totalhead">"Total"</th> }.into_any()
                                }}
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
                                Some(view! {
                                    <tr>
                                        <td class="rankcell"><h4>{rank.rank}</h4></td>
                                        <td class="teamcell">
                                            {move || {
                                                if !ui.show_images.get() { return ().into_any(); }
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
                                                <span class="team-vulnbox">{team.vulnbox.clone()}</span><br/>
                                                <span class="team-total-points">
                                                    <i class="fas fa-chart-line"></i>{"\u{00a0}"}{format!("{:.1}", rank.points)}
                                                    <span class="delta">
                                                        <small title="New points this tick">{format!("({})", float_to_string(rank.do_ + rank.dd + rank.ds))}</small>
                                                    </span>
                                                </span>
                                            </div>
                                        </td>
                                        {line_cells(&rank, tick, ui.show_history, ui.show_only_sums)}
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

/// `PageDashboard`.
pub fn dashboard(store: Store, ui: Ui) -> impl IntoView {
    let info = Memo::new(move |_| {
        let tick = store.newest_scoreboard_tick.get();
        if tick < 0 { None } else { store.ranking(tick) }
    });

    view! {
        <div class="page-dashboard">
            {move || match info.get() {
                None => view! { <div class="loading"><span class="fas fa-spinner fa-spin"></span> " loading..."</div> }.into_any(),
                Some(info) => {
                    let services = info.services.clone();
                    let mut ranks = info.scoreboard.clone();
                    ranks.sort_by_key(|r| r.rank);
                    view! {
                        <table class="table table-bordered table-fixed">
                            <thead>
                                <tr>{service_header_cells(&services)}</tr>
                            </thead>
                        </table>
                        <div class="teamgrid">
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
                                Some(view! {
                                    <div class="team">
                                        <h4 class="rank"
                                            class:rank1=(rank.rank == 1 && rank.points > 0.0)
                                            class:rank2=(rank.rank == 2 && rank.points > 0.0)
                                            class:rank3=(rank.rank == 3 && rank.points > 0.0)>
                                            {rank.rank}
                                        </h4>
                                        <div class="teamcell">
                                            {move || {
                                                if !ui.show_images.get() { return ().into_any(); }
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
                                                    <a href={format!("/team/{team_id}")} title=title>{name}</a>
                                                </h4>
                                                <span class="team-total-points">
                                                    <i class="fas fa-chart-line"></i>{"\u{00a0}"}{format!("{:.1}", rank.points)}
                                                    <span class="delta">
                                                        <small title="New points this tick">{format!("({})", float_to_string(rank.do_ + rank.dd + rank.ds))}</small>
                                                    </span>
                                                </span>
                                                {"\u{00a0}"}
                                                <div class="team-status-row">
                                                    {rank.services.iter().enumerate().map(|(i, svc)| {
                                                        let svc_name = info.services.get(i).map(|s| s.name.clone()).unwrap_or_default();
                                                        let status = svc.c.clone();
                                                        let dcap = svc.dcap;
                                                        let cap = svc.cap;
                                                        let title = format!("{}: {}{}",
                                                            svc_name,
                                                            status_to_tooltip(&status),
                                                            if dcap > 0 { format!(" ({dcap} flags captured)") } else { String::new() });
                                                        view! {
                                                            <span
                                                                class={format!("label {}", status_to_class(&status))}
                                                                class:fresh=(dcap >= cap && cap > 0)
                                                                title=title
                                                            >
                                                                {if dcap > 0 { view! { <i class="fas fa-flag"></i> }.into_any() } else { ().into_any() }}
                                                            </span>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            </div>
                                        </div>
                                    </div>
                                })
                            }).collect_view().into_any()}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}
