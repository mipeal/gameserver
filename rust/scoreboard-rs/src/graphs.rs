//! Port of `page-graphs`: attackers/victims chart per service.

use crate::charts::{dataset, y_scale, ChartHandle};
use crate::store::Store;
use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;

fn stat_num(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or(0.0) // "?" (frozen) counts as 0
}

pub fn graphs_page(store: Store) -> impl IntoView {
    let stats = LocalResource::new(move || async move { store.service_stat_history().await });

    let canvas_set: Vec<NodeRef<leptos::html::Canvas>> =
        (0..20).map(|_| NodeRef::new()).collect();
    let effect_canvases = canvas_set.clone();
    let view_canvases = canvas_set.clone();
    let live: Vec<Rc<RefCell<Option<ChartHandle>>>> =
        (0..20).map(|_| Rc::new(RefCell::new(None))).collect();

    Effect::new(move || {
        let Some(Some(stats_info)) = stats.get() else { return };
        if stats_info.stats.first().map(|s| s.len()).unwrap_or(0) == 0 {
            return;
        }
        for (si, _service) in stats_info.services.iter().enumerate() {
            let (Some(canvas), Some(live)) = (effect_canvases.get(si), live.get(si)) else {
                continue;
            };
            let Some(canvas) = canvas.get() else { continue };
            let Some(row0) = stats_info.stats.first() else { continue };
            let n_ticks = row0.len();
            let column = |side: &str| -> Vec<f64> {
                (0..n_ticks)
                    .map(|t| {
                        stats_info
                            .stats
                            .iter()
                            .map(|row| {
                                let cell = row.get(t);
                                let v = cell
                                    .and_then(|c| {
                                        if side == "a" { Some(&c.a) } else { Some(&c.v) }
                                    });
                                v.map(|v| stat_num(v)).unwrap_or(0.0)
                            })
                            .sum()
                    })
                    .collect()
            };
            let sets: Vec<JsValue> = vec![
                dataset("attackers", &column("a"), "#b02a37", true, None, None),
                dataset("victims", &column("v"), "#0d6efd", true, None, None),
            ];
            let ds: JsValue = sets.into();
            let scales = y_scale(false, Some(0.0));
            let labels: Vec<String> = (0..n_ticks).map(|t| t.to_string()).collect();
            *live.borrow_mut() = Some(ChartHandle::line(&canvas, &labels, &ds, &scales));
        }
    });

    view! {
        <div>
            {move || match stats.get() {
                None => view! { <p class="loading">"loading…"</p> }.into_any(),
                Some(None) => ().into_any(),
                Some(Some(info)) => {
                    info.services.iter().enumerate().map(|(si, s)| {
                        view! {
                            <div class="chart-group">
                                <h3>{s.name.clone()}</h3>
                                <div class="chart-container">
                                    <canvas node_ref=view_canvases[si] />
                                </div>
                            </div>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}
