//! Port of `page-graphs`: top-10 team point charts (overall + per service)
//! and attackers/victims charts per service.

use crate::charts::{dataset, y_scale, ChartHandle};
use crate::store::Store;
use crate::team::palette;
use crate::ui::Ui;
use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

/// One chart: title + datasets + labels.
#[derive(Clone, PartialEq)]
struct Group {
    title: String,
    datasets: Vec<JsValue>,
    n_ticks: usize,
    /// (series label, line colour) for the page's dot legend.
    legend: Vec<(String, String)>,
}

/// Hide the canvas-drawn Chart.js legend; the card renders its own dot
/// legend from the same datasets/palette.
fn hide_chart_legend(canvas: &web_sys::HtmlCanvasElement) {
    let global = js_sys::global();
    let Ok(chart) = js_sys::Reflect::get(&global, &JsValue::from_str("Chart")) else { return };
    let Ok(get_chart) = js_sys::Reflect::get(&chart, &JsValue::from_str("getChart")) else { return };
    let Ok(get_chart) = get_chart.dyn_into::<js_sys::Function>() else { return };
    let Ok(instance) = get_chart.call1(&JsValue::UNDEFINED, canvas) else { return };
    if !instance.is_object() {
        return;
    }
    let Ok(options) = js_sys::Reflect::get(&instance, &JsValue::from_str("options")) else { return };
    let plugins = match js_sys::Reflect::get(&options, &JsValue::from_str("plugins")) {
        Ok(p) if p.is_object() => p,
        _ => {
            let p: JsValue = js_sys::Object::new().into();
            let _ = js_sys::Reflect::set(&options, &JsValue::from_str("plugins"), &p);
            p
        }
    };
    let legend: JsValue = js_sys::Object::new().into();
    let _ = js_sys::Reflect::set(&legend, &JsValue::from_str("display"), &JsValue::FALSE);
    let _ = js_sys::Reflect::set(&plugins, &JsValue::from_str("legend"), &legend);
    let Ok(update) = js_sys::Reflect::get(&instance, &JsValue::from_str("update")) else { return };
    if let Ok(update) = update.dyn_into::<js_sys::Function>() {
        let _ = update.call0(&instance);
    }
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
                let color_of = |team_id: i64| colors[(team_id % (colors.len() as i64 - 2) + 2) as usize];
                // "Points (overall)": per-tick sum over the team's services
                let mut legend = Vec::new();
                let sets: Vec<JsValue> = teams.iter().map(|(team, points)| {
                    let mut total = vec![0.0; n_ticks];
                    for series in points {
                        for (i, p) in series.iter().enumerate() {
                            if let Some(t) = total.get_mut(i) { *t += p; }
                        }
                    }
                    let color = color_of(team.id);
                    legend.push((team.name.clone(), color.to_string()));
                    dataset(&team.name, &total, color, false, None, None)
                }).collect();
                out.push(Group { title: "Points (overall)".into(), datasets: sets, n_ticks, legend });
                // "Points (service X)"
                let svc_names: Vec<String> = current_round
                    .get_untracked()
                    .map(|ri| ri.services.iter().map(|s| s.name.clone()).collect())
                    .unwrap_or_default();
                for si in 0..teams.iter().map(|(_, p)| p.len()).max().unwrap_or(0) {
                    let mut legend = Vec::new();
                    let sets: Vec<JsValue> = teams.iter().map(|(team, points)| {
                        let color = color_of(team.id);
                        legend.push((team.name.clone(), color.to_string()));
                        let data: Vec<f64> = points.get(si).cloned().unwrap_or_default();
                        dataset(&team.name, &data, color, false, None, None)
                    }).collect();
                    let name = svc_names.get(si).cloned().unwrap_or_else(|| format!("service {si}"));
                    out.push(Group {
                        title: format!("Points (service {name})"),
                        datasets: sets,
                        n_ticks,
                        legend,
                    });
                }
            }
        }
        // service charts (attackers / victims)
        if let Some(Some(stats_info)) = stats.get() {
            let n_ticks = stats_info.stats.first().map(|s| s.len()).unwrap_or(0);
            if n_ticks > 0 {
                for (title, key) in [("No. of Attackers", 0), ("No. of Victims", 1)] {
                    let mut legend = Vec::new();
                    let sets: Vec<JsValue> = stats_info.services.iter().enumerate().map(|(si, name)| {
                        let color = colors[(si % (colors.len() - 2)) + 2];
                        legend.push((name.clone(), color.to_string()));
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
                    out.push(Group { title: title.into(), datasets: sets, n_ticks, legend });
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
            hide_chart_legend(&canvas);
        }
    });

    view! {
        <div class="page-graphs">
            <h1>"GRAPHS"</h1>
            <div class="sub">"points and attacker / victim counts per tick · all teams"</div>
            {move || if current_round.get().is_none() {
                view! { <p class="lead loading"><span class="fas fa-spinner fa-spin"></span></p> }.into_any()
            } else { ().into_any() }}
            {move || {
                let groups = groups.get();
                if groups.is_empty() { return ().into_any(); }
                groups.iter().enumerate().map(|(gi, group)| {
                    let legend = group.legend.iter().map(|(label, color)| {
                        view! {
                            <span>
                                <i class="dot" style=format!("background:{color}") aria-hidden="true"></i>
                                {label.clone()}
                            </span>
                        }
                    }).collect_view();
                    view! {
                        <div class="chart-group">
                            <h2>{group.title.clone()}</h2>
                            <div class="card">
                                <div class="legend">{legend}</div>
                                <div class="chart-box">
                                    <canvas node_ref=view_canvases[gi] />
                                </div>
                            </div>
                        </div>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}
