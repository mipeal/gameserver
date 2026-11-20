//! Port of `notification-overlay.component`: firstblood greetings (10 s) and
//! the final top-3 "pyro" table, with the upstream 12-second rate limiter.

use crate::models::STATE_RUNNING;
use crate::store::{Event, Store};
use crate::ui::Ui;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

pub fn notification_overlay(store: Store, ui: Ui) -> impl IntoView {
    // (what, service, team, shown_at)
    let current = RwSignal::new(None::<Event>);
    let last_shown = RwSignal::new(0f64);

    // drain store events → display, rate-limited to one per 12 s
    Effect::new(move || {
        if !ui.show_notifications.get() {
            return;
        }
        let events = store.events.get();
        if events.is_empty() {
            return;
        }
        let now = js_sys::Date::now() / 1000.0;
        if now - last_shown.get_untracked() < 12.0 {
            return; // dropped, like the upstream RateLimiter
        }
        let ev = events[0].clone();
        store.events.update(|e| { e.remove(0); });
        last_shown.set(now);
        current.set(Some(ev.clone()));
        let hide = current;
        let _ = gloo_timers::callback::Timeout::new(
            if ev.0 == "final" { 15_000 } else { 10_000 },
            move || hide.set(None),
        );
    });

    // escape dismisses
    Effect::new(move || {
        let on_key = move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Escape" {
                current.set(None);
            }
        };
        let Some(w) = web_sys::window() else { return };
        let cb = wasm_bindgen::closure::Closure::wrap(Box::new(on_key) as Box<dyn Fn(_)>);
        let _ = w.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        cb.forget(); // page-lifetime listener
    });

    view! {
        {move || match current.get() {
            None => view! { <span></span> }.into_any(),
            Some(("firstblood", service, team)) => view! {
                <>
                    <div class="backdrop"></div>
                    <div class="msgcontainer">
                        <div class="message-what"><span class="bigicon">"☠"</span></div>
                        <div class="message-what">"FIRST BLOOD"</div>
                        <div class="message-service">{service}</div>
                        <div class="message-team">{team}</div>
                    </div>
                </>
            }.into_any(),
            Some(("final", _, _)) => view! {
                <>
                    <div class="backdrop"></div>
                    <div class="msgcontainer">
                        <div class="message-what">"GAME OVER"</div>
                        <FinalTable store />
                    </div>
                </>
            }.into_any(),
            Some(_) => view! { <span></span> }.into_any(),
        }}
    }
}

#[component]
fn FinalTable(store: Store) -> impl IntoView {
    let st = store.state.get();
    let info = store.ranking(st.scoreboard_tick);
    let running = st.state == STATE_RUNNING; // upstream only shows pyro when stopped
    let mut ranks = match info {
        Some(i) if !running => i.scoreboard.clone(),
        _ => vec![],
    };
    ranks.sort_by_key(|r| r.rank);
    ranks.truncate(3);
    view! {
        <table class="finaltable">
            {ranks.iter().map(|rank| {
                let team = store.team(rank.team_id);
                let name = team.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| format!("Team {}", rank.team_id));
                let logo = team.map(|t| t.logo).unwrap_or_default();
                view! {
                    <tr>
                        <td>{rank.rank}</td>
                        <td>
                            {move || {
                                let logo = logo.clone();
                                if logo != "false" && !logo.is_empty() {
                                    view! { <img src={format!("logos/{logo}")} alt="" /> }.into_any()
                                } else { ().into_any() }
                            }}
                        </td>
                        <td>{name}</td>
                        <td>{rank.points}</td>
                    </tr>
                }
            }).collect::<Vec<_>>()}
        </table>
    }
}
