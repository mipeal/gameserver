//! Port of `page-team`: stacked per-service point chart for one team, with
//! the neighbours as dashed reference lines, plus the per-tick detail table
//! with infinite scroll (7 more ticks per batch, as upstream `loadMore`).

use crate::charts::{dataset, y_scale, ChartHandle};
// allow: rustc's liveness analysis misses uses inside leptos `view!` closures.
#[allow(unused_imports)]
use crate::pages::{float_to_string, line_cells, service_header_cells};
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsValue;
use wasm_bindgen::JsCast;

/// `COLORS_LIGHT` from `chart-colorschemes.ts`, re-tuned for the Japandi
/// themes: [0]/[1] are the dashed neighbour lines, the rest the series palette.
pub const COLORS_DARK: &[&str] = &[
    "#6b6357", "#c25e4c", "#8aa07c", "#c9a86a", "#c67f53", "#8ca0b3", "#b08ea2",
    "#7f9e9b", "#a8986e", "#94794f", "#6e8a5e", "#bd7d6e", "#88919d",
];
pub const COLORS_LIGHT: &[&str] = &[
    "#857b6c", "#b04a35", "#54684a", "#7d6128", "#96542a", "#47617b", "#8a5f75",
    "#4f7a76", "#7d6a3a", "#6b4e2a", "#48683a", "#a05548", "#4f5a68",
];

/// Series palette for the active theme.
pub fn palette(dark: bool) -> &'static [&'static str] {
    if dark { COLORS_DARK } else { COLORS_LIGHT }
}

pub fn team_page(store: Store, ui: Ui) -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let team_id = Memo::new(move |_| {
        params
            .get()
            .get("teamid")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(-1)
    });
    let num_results = RwSignal::new(7i64);
    // upstream currentTick = newestScoreboardTick
    let current_tick = store.newest_scoreboard_tick;
    let current_round_info = Memo::new(move |_| store.ranking(current_tick.get()));

    // infinite scroll: IntersectionObserver on the sentinel + 500 ms poll.
    // Both must be torn down when the page unmounts — the interval closure
    // calls get_untracked(), which panics on disposed signals.
    let sentinel = NodeRef::<leptos::html::Div>::new();
    let spinner_visible = RwSignal::new(false);
    let observer_cell: Arc<Mutex<Option<web_sys::IntersectionObserver>>> = Arc::new(Mutex::new(None));
    let interval_cell: Arc<Mutex<Option<leptos::leptos_dom::helpers::IntervalHandle>>> = Arc::new(Mutex::new(None));
    {
        let observer_cell = observer_cell.clone();
        let interval_cell = interval_cell.clone();
        on_cleanup(move || {
            if let Ok(mut obs) = observer_cell.lock() {
                if let Some(o) = obs.take() {
                    o.disconnect();
                }
            }
            if let Ok(mut h) = interval_cell.lock() {
                if let Some(h) = h.take() {
                    h.clear();
                }
            }
        });
    }
    Effect::new(move || {
        let Some(el) = sentinel.get() else { return };
        let cb = wasm_bindgen::closure::Closure::wrap(
            Box::new(move |entries: Vec<web_sys::IntersectionObserverEntry>| {
                spinner_visible.set(entries.first().map(|e| e.is_intersecting()).unwrap_or(false));
            }) as Box<dyn Fn(_)>,
        );
        if let Ok(obs) = web_sys::IntersectionObserver::new(cb.as_ref().unchecked_ref()) {
            obs.observe(&el);
            if let Ok(mut cell) = observer_cell.lock() {
                *cell = Some(obs);
            }
            cb.forget(); // disconnected in on_cleanup; leaked closure keeps the observer alive
        } else {
            drop(cb);
        }
        let handle = set_interval_with_handle(
            move || {
                if spinner_visible.get_untracked() && num_results.get_untracked() <= current_tick.get_untracked() {
                    num_results.update(|n| *n += 7); // upstream loadMore
                }
            },
            std::time::Duration::from_millis(500),
        );
        if let Ok(mut cell) = interval_cell.lock() {
            *cell = handle.ok();
        }
    });

    let canvas = NodeRef::<leptos::html::Canvas>::new();
    // drop (destroy) the previous chart before reusing the canvas
    let live: Rc<RefCell<Option<ChartHandle>>> = Rc::new(RefCell::new(None));

    // chart: neighbour lines + per-service stacked series (updateGraph)
    Effect::new(move || {
        // darkmode is a dependency so the chart rebuilds on theme toggle
        let dark = ui.darkmode.get();
        let colors = palette(dark);
        let Some(info) = current_round_info.get() else { return };
        if info.tick < 0 {
            return;
        }
        let id = team_id.get();
        let Some(canvas) = canvas.get() else { return };
        // team before/after in the ranking (by index, not rank value)
        let pos = info.scoreboard.iter().position(|r| r.team_id == id);
        let name_of = |tid: i64| -> String {
            store.team(tid).map(|t| t.name).unwrap_or_else(|| format!("Team {tid}"))
        };
        let mut sets: Vec<JsValue> = vec![];
        if let Some(p) = pos {
            if p > 0 {
                // dataBeforeUs: colors[1], dash [10,5]
                let nid = info.scoreboard[p - 1].team_id;
                let np = store.team_history_simple(nid);
                if !np.is_empty() {
                    sets.push(dataset(&format!("Team {}", name_of(nid)), &np, colors[1], false, None, Some(&[10, 5])));
                }
            }
        }
        if let Some(p) = pos {
            if p + 1 < info.scoreboard.len() {
                // dataAfterUs: colors[0], dash [5,5]
                let nid = info.scoreboard[p + 1].team_id;
                let np = store.team_history_simple(nid);
                if !np.is_empty() {
                    sets.push(dataset(&format!("Team {}", name_of(nid)), &np, colors[0], false, None, Some(&[5, 5])));
                }
            }
        }
        let Some(points) = store.team_history(id) else { return };
        let n_ticks = points.first().map(|p| p.len()).unwrap_or(0);
        if n_ticks == 0 {
            return;
        }
        for (i, series) in points.iter().enumerate() {
            let color = colors[(i % (colors.len() - 2)) + 2];
            let name = info
                .services
                .get(i)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("service {i}"));
            sets.push(dataset(&name, series, color, true, None, None));
        }
        let ds: JsValue = sets.into();
        let scales = y_scale(true, Some(0.0));
        let labels: Vec<String> = (0..n_ticks).map(|t| t.to_string()).collect();
        live.borrow_mut().take();
        *live.borrow_mut() = Some(ChartHandle::line(&canvas, &labels, &ds, &scales));
    });

    view! {
        <div class="page-team">
            {move || {
                let st = current_tick.get();
                if st < 0 { return ().into_any(); }
                view! { <div class="chart-container"><canvas node_ref=canvas /></div> }.into_any()
            }}
            {move || {
                let id = team_id.get();
                if id < 0 { return ().into_any(); }
                let team = store.team(id);
                let (Some(team), Some(info)) = (team, current_round_info.get()) else {
                    return view! { <p class="loading"><span class="fas fa-spinner fa-spin"></span></p> }.into_any();
                };
                let name = team.name.clone();
                let logo = team.logo.clone();
                let banned = store.banned(id);
                let services = info.services.clone();
                // per-tick rows, newest first, only already-fetched ticks
                let max = current_tick.get_untracked().max(0);
                let from = (max + 1 - num_results.get_untracked()).max(0);
                let ticks: Vec<i64> = (from..=max)
                    .rev()
                    .filter(|t| store.ranking(*t).is_some())
                    .collect();
                let all_loaded = ticks.len() as i64 >= max + 1;
                view! {
                    <table
                        class="table table-bordered table-stickyhead"
                        class:hideHistory=move || !ui.show_history.get()
                        class:onlySums=move || ui.show_only_sums.get()
                    >
                        <thead>
                            <tr>
                                <th colspan="2" class="headercell">
                                    {move || {
                                        if !ui.show_images.get() { return ().into_any(); }
                                        view! {
                                            <div class="media-left team-logo-media">
                                                <img class="media-object" src={format!("logos/{logo}")} alt="" loading="lazy" />
                                            </div>
                                        }.into_any()
                                    }}
                                    <div class="media-body">
                                        <h4 class="team-name">
                                            {name.clone()}
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
                                        {if current_round_info.get().is_none() {
                                            view! { <span class="fas fa-spinner fa-spin"></span> }.into_any()
                                        } else { ().into_any() }}
                                        {if banned {
                                            view! {
                                                <a class="label label-danger"
                                                    title="Network access is currently blocked. Please contact the organizers in IRC!">
                                                    "! network blocked !"
                                                </a>
                                            }.into_any()
                                        } else { ().into_any() }}
                                    </div>
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
                            {ticks.iter().filter_map(|t| {
                                let ri = store.ranking(*t)?;
                                let rank = ri.scoreboard.iter().find(|r| r.team_id == id)?.clone();
                                let tick = *t;
                                Some(view! {
                                    <tr>
                                        <td class="rankcell"><h4>{rank.rank}</h4></td>
                                        <td class="teamcell">
                                            <h4 class="team-name">{format!("Tick {tick}")}</h4>
                                            <span class="team-vulnbox"></span><br/>
                                            <span class="team-total-points">
                                                <i class="fas fa-chart-line"></i>{"\u{00a0}"}{format!("{:.1}", rank.points)}
                                                <span class="delta">
                                                    <small title="New points this tick">{format!("({})", float_to_string(rank.do_ + rank.dd + rank.ds))}</small>
                                                </span>
                                            </span>
                                        </td>
                                        {line_cells(&rank, tick, ui.show_history, ui.show_only_sums)}
                                    </tr>
                                })
                            }).collect_view()}
                        </tbody>
                    </table>
                    <div class="text-center">
                        {move || {
                            if all_loaded {
                                view! { <span class="fas fa-spinner fa-spin" hidden></span> }.into_any()
                            } else {
                                view! { <span class="fas fa-spinner fa-spin"></span> }.into_any()
                            }
                        }}
                    </div>
                    <br/>
                }.into_any()
            }}
            <div node_ref=sentinel></div>
        </div>
    }
}
