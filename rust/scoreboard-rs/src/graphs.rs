//! Port of `page-graphs`: top-10 team point charts (overall + per service)
//! and attackers/victims charts per service.

use crate::charts::{dataset, y_scale, ChartHandle};
use crate::store::Store;
use crate::team::palette;
use crate::ui::Ui;
use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// One chart: title + datasets + labels.
#[derive(Clone, PartialEq)]
struct Group {
    title: String,
    datasets: Vec<JsValue>,
    n_ticks: usize,
}

pub fn graphs_page(store: Store, ui: Ui) -> impl IntoView {
    let current_round = Memo::new(move |_| {
        let tick = store.newest_scoreboard_tick.get();
        if tick < 0 { None } else { store.ranking(tick) }
    });

    let stats = LocalResource::new(move || async move { store.service_stat_history().await });

    // team charts: top 10 teams × (overall + per-service) + 2 service charts
    let top_teams = Memo::new(move |_| -> Option<Vec<_>> {
        let info = current_round.get()?;
        let mut ranks = info.scoreboard.clone();
        ranks.sort_by_key(|r| r.rank);
        let teams: Vec<_> = ranks
            .iter()
            .take(10)
            .filter_map(|r| store.team(r.team_id).and_then(|t| store.team_history(r.team_id).map(|h| (t, h))))
            .collect();
        if teams.is_empty() { None } else { Some(teams) }
    });

    // build all chart groups reactively (darkmode is a dependency so the
    // datasets rebuild with the theme's palette)
    let groups = Memo::new(move |_| -> Vec<Group> {
        let dark = ui.darkmode.get();
        let colors = palette(dark);
        let mut out: Vec<Group> = vec![];
        if let Some(teams) = top_teams.get() {
            let n_ticks = teams.first().and_then(|(_, p)| p.first()).map(|p| p.len()).unwrap_or(0);
            if n_ticks > 0 {
                // "Points (overall)": per-tick sum over the team's services
                let sets: Vec<JsValue> = teams.iter().map(|(team, points)| {
                    let mut total = vec![0.0; n_ticks];
                    for series in points {
                        for (i, p) in series.iter().enumerate() {
                            if let Some(t) = total.get_mut(i) { *t += p; }
                        }
                    }
                    let color = colors[(team.id % (colors.len() as i64 - 2) + 2) as usize];
                    dataset(&team.name, &total, color, false, None, None)
                }).collect();
                out.push(Group { title: "Points (overall)".into(), datasets: sets, n_ticks });
                // "Points (service X)"
                let svc_names: Vec<String> = current_round
                    .get_untracked()
                    .map(|ri| ri.services.iter().map(|s| s.name.clone()).collect())
                    .unwrap_or_default();
                for si in 0..teams.iter().map(|(_, p)| p.len()).max().unwrap_or(0) {
                    let sets: Vec<JsValue> = teams.iter().map(|(team, points)| {
                        let color = colors[(team.id % (colors.len() as i64 - 2) + 2) as usize];
                        let data: Vec<f64> = points.get(si).cloned().unwrap_or_default();
                        dataset(&team.name, &data, color, false, None, None)
                    }).collect();
                    let name = svc_names.get(si).cloned().unwrap_or_else(|| format!("service {si}"));
                    out.push(Group { title: format!("Points (service {name})"), datasets: sets, n_ticks });
                }
            }
        }
        // service charts (attackers / victims)
        if let Some(Some(stats_info)) = stats.get() {
            let n_ticks = stats_info.stats.first().map(|s| s.len()).unwrap_or(0);
            if n_ticks > 0 {
                for (title, key) in [("No. of Attackers", 0), ("No. of Victims", 1)] {
                    let sets: Vec<JsValue> = stats_info.services.iter().enumerate().map(|(si, name)| {
                        let color = colors[(si % (colors.len() - 2)) + 2];
                        let data: Vec<f64> = stats_info
                            .stats
                            .get(si)
                            .map(|row| {
                                row.iter()
                                    .map(|s| {
                                        let v = if key == 0 { &s.a } else { &s.v };
                                        v.as_f64().unwrap_or(0.0) // "?" (frozen) skipped upstream
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        dataset(name, &data, color, false, None, None)
                    }).collect();
                    out.push(Group { title: title.into(), datasets: sets, n_ticks });
                }
            }
        }
        out
    });

    let canvas_set: Vec<NodeRef<leptos::html::Canvas>> =
        (0..40).map(|_| NodeRef::new()).collect();
    let effect_canvases = canvas_set.clone();
    let view_canvases = canvas_set.clone();
    let live: Vec<Rc<RefCell<Option<ChartHandle>>>> =
        (0..40).map(|_| Rc::new(RefCell::new(None))).collect();

    // draw every group onto its canvas (darkmode dep: redraw on theme toggle)
    Effect::new(move || {
        let _dark = ui.darkmode.get();
        let groups = groups.get();
        if groups.is_empty() {
            return;
        }
        for (gi, group) in groups.iter().enumerate() {
            let (Some(canvas), Some(live)) = (effect_canvases.get(gi), live.get(gi)) else {
                continue;
            };
            let Some(canvas) = canvas.get() else { continue };
            let ds: JsValue = group.datasets.clone().into();
            let scales = y_scale(false, Some(0.0));
            let labels: Vec<String> = (0..group.n_ticks).map(|t| t.to_string()).collect();
            // drop (destroy) the previous chart before reusing the canvas
            live.borrow_mut().take();
            *live.borrow_mut() = Some(ChartHandle::line(&canvas, &labels, &ds, &scales));
        }
    });

    view! {
        <div class="page-graphs">
            {move || if current_round.get().is_none() {
                view! { <p class="lead"><span class="fas fa-spinner fa-spin"></span></p> }.into_any()
            } else { ().into_any() }}
            {move || {
                let groups = groups.get();
                if groups.is_empty() { return ().into_any(); }
                groups.iter().enumerate().map(|(gi, group)| {
                    view! {
                        <div class="chart-group">
                            <h3>{group.title.clone()}</h3>
                            <div class="chart-container">
                                <canvas node_ref=view_canvases[gi] />
                            </div>
                        </div>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}
