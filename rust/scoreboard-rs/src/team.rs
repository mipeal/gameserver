//! Port of `page-team`: stacked per-service point chart for one team, with
//! the neighbours as dashed reference lines (upstream infinite scroll is a
//! load-all here: the dataset is a single JSON file anyway).

use crate::charts::{dataset, y_scale, ChartHandle};
use crate::store::Store;
use leptos_router::hooks::use_params_map;
use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// Upstream `chart-colorschemes.ts` (light scheme; chart colors stay fixed
/// regardless of dark mode, as in the original default).
const COLORS: &[&str] = &[
    "#4dc9f6", "#f6704b", "#94a7e1", "#f1e15a", "#5ac1a0", "#b466d9", "#f1945a", "#75d977",
    "#e06a9c", "#6aa8e0", "#c8b653", "#59c3d9", "#dd7f9e", "#8fdc7f", "#e0b768",
];

pub fn team_page(store: Store) -> impl IntoView {
    let params = use_params_map();
    let team_id = Memo::new(move |_| {
        params
            .get()
            .get("teamid")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(-1)
    });

    // neighbours from the newest ranking
    let neighbours = Memo::new(move |_| {
        let id = team_id.get();
        let info = store.ranking(store.newest_scoreboard_tick.get())?;
        let mut ranks = info.scoreboard.clone();
        ranks.sort_by_key(|r| r.rank);
        let pos = ranks.iter().position(|r| r.team_id == id)?;
        let team_name = |id: i64| -> String {
            store
                .team(id)
                .map(|t| t.name)
                .unwrap_or_else(|| format!("Team {id}"))
        };
        let mut n: Vec<(i64, String)> = vec![];
        if pos > 0 {
            let r = &ranks[pos - 1];
            n.push((r.team_id, team_name(r.team_id)));
        }
        if pos + 1 < ranks.len() {
            let r = &ranks[pos + 1];
            n.push((r.team_id, team_name(r.team_id)));
        }
        Some(n)
    });

    let canvas = NodeRef::<leptos::html::Canvas>::new();
    // replacing the handle drops (and thereby destroys) the previous chart
    let live: Rc<RefCell<Option<ChartHandle>>> = Rc::new(RefCell::new(None));

    Effect::new(move || {
        // tracked: re-runs when the history cache fills or the tick advances
        let Some(points) = store.team_history(team_id.get()) else { return };
        let Some(canvas) = canvas.get() else { return };
        let n_ticks = points.first().map(|p| p.len()).unwrap_or(0);
        if n_ticks == 0 {
            return;
        }
        let mut sets: Vec<JsValue> = vec![];
        for (i, series) in points.iter().enumerate() {
            sets.push(dataset(
                &format!("service {i}"),
                series,
                COLORS[i % COLORS.len()],
                true,
                Some("main"),
                None,
            ));
        }
        for (nid, name) in neighbours.get().unwrap_or_default() {
            let np = store.team_history_simple(nid);
            if !np.is_empty() {
                sets.push(dataset(&name, &np, "#888888", false, None, Some(&[6, 3])));
            }
        }
        let ds: JsValue = sets.into();
        let scales = y_scale(true, Some(0.0));
        let labels: Vec<String> = (0..n_ticks).map(|t| t.to_string()).collect();
        *live.borrow_mut() = Some(ChartHandle::line(&canvas, &labels, &ds, &scales));
    });

    view! {
        <div>
            {move || {
                let id = team_id.get();
                let name = store.team(id).map(|t| t.name).unwrap_or_default();
                view! { <h3>{name}</h3> }
            }}
            <div class="chart-container">
                <canvas node_ref=canvas />
            </div>
        </div>
    }
}
