//! Port of `notification-overlay.component`: firstblood greetings (10 s) and
//! the final top-3 "pyro" table, with the upstream 12-second rate limiter.

use crate::models::STATE_RUNNING;
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// What the overlay currently shows: one coalesced first-blood group (several
/// services can be first-blooded in the same tick) or the final greeting.
#[derive(Clone)]
enum Current {
    FirstBlood(Vec<(String, String)>),
    Final,
}

/// First-blood sound: two synthesized low sine hits + one spoken announcement
/// covering every coalesced pair. Requires a prior user gesture (autoplay
/// policy) — at a live CTF the viewer has interacted with the page long
/// before the first tick.
fn play_first_blood(pairs: &[(String, String)]) {
    let Some(win) = web_sys::window() else { return };
    if let Ok(speech) = win.speech_synthesis() {
        if let Ok(u) = web_sys::SpeechSynthesisUtterance::new() {
            let mut text = String::from("First blood.");
            for (service, team) in pairs {
                text.push_str(&format!(" {service}, by {team}."));
            }
            u.set_text(&text);
            u.set_volume(1.0);
            speech.speak(&u);
        }
    }
    let Ok(ctx) = web_sys::AudioContext::new() else { return };
    let _ = ctx.resume();
    let dst_ok = ctx.destination().dyn_into::<web_sys::AudioNode>().ok();
    let Some(dst) = dst_ok else { return };
    // two thumps: (start freq, end freq, start s, peak gain, dur s)
    let hits: [(f64, f64, f64, f64, f64); 2] = [(660.0, 180.0, 0.0, 0.25, 1.2), (132.0, 60.0, 0.15, 0.35, 0.9)];
    for (f0, f1, at, peak, dur) in hits {
        let Ok(osc) = ctx.create_oscillator() else { return };
        let Ok(gain) = ctx.create_gain() else { return };
        let now = ctx.current_time() + at;
        let _ = osc.set_type(web_sys::OscillatorType::Sine);
        let f = osc.frequency();
        let g = gain.gain();
        let _ = f.set_value_at_time(f0 as f32, now);
        let _ = f.exponential_ramp_to_value_at_time(f1 as f32, now + dur);
        let _ = g.set_value_at_time(0.0, now);
        let _ = g.linear_ramp_to_value_at_time(peak as f32, now + 0.02);
        let _ = g.exponential_ramp_to_value_at_time(0.001, now + dur);
        let _ = osc.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&dst);
        let _ = osc.start_with_when(now);
        let _ = osc.stop_with_when(now + dur);
    }
}

#[component]
pub fn NotificationOverlay() -> impl IntoView {
    let store = expect_context::<Store>();
    let ui = expect_context::<Ui>();
    let current = RwSignal::new(None::<Current>);
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
        let hide_after;
        if ev.0 == "final" {
            current.set(Some(Current::Final));
            hide_after = 15_000;
        } else {
            // coalesce every queued consecutive firstblood (same tick, often
            // several services) into ONE overlay with ONE sound
            let mut pairs = vec![(ev.1.clone(), ev.2.clone())];
            loop {
                let next = store.events.get_untracked().first().cloned();
                match next {
                    Some(e2) if e2.0 == "firstblood" => {
                        store.events.update(|e| { e.remove(0); });
                        pairs.push((e2.1, e2.2));
                    }
                    _ => break,
                }
            }
            current.set(Some(Current::FirstBlood(pairs)));
            if ui.sound.get_untracked() {
                let pairs_now = match current.get_untracked() {
                    Some(Current::FirstBlood(p)) => p,
                    _ => vec![],
                };
                play_first_blood(&pairs_now);
            }
            hide_after = 10_000;
        }
        let hide = current;
        let _ = gloo_timers::callback::Timeout::new(hide_after, move || hide.set(None));
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
            Some(Current::FirstBlood(pairs)) => {
                let many = pairs.len() > 1;
                view! {
                    <>
                        <div class="backdrop"></div>
                        <div class="msgcontainer" class:many=many>
                            <div class="message-what">
                                "First Blood"
                                <div class="bigicon"><i class="fas fa-tint"></i></div>
                            </div>
                            {pairs.iter().map(|(service, team)| {
                                let (service, team) = (service.clone(), team.clone());
                                view! {
                                    <div class="message-service"><small>"on "</small><strong>{service.clone()}</strong></div>
                                    <div class="message-team"><small>"by "</small><strong>{team}</strong></div>
                                }
                            }).collect_view()}
                        </div>
                    </>
                }.into_any()
            }
            Some(Current::Final) => view! {
                <>
                    <div class="pyrocontainer">
                        <div class="message-what">"Game is over!"</div>
                        <FinalTable store />
                        <div class="message-sub">
                            <strong>"Thanks for playing!"</strong><br/>
                            <small>"this game was powered by the organizers"</small>
                        </div>
                    </div>
                </>
            }.into_any(),
            _ => ().into_any(),
        }}
    }
}

#[component]
fn FinalTable(store: Store) -> impl IntoView {
    let st = store.state.get_untracked();
    let info = store.ranking(st.scoreboard_tick);
    let running = st.state == STATE_RUNNING; // upstream only shows pyro when stopped
    let mut ranks = match info {
        Some(i) if !running => i.scoreboard.clone(),
        _ => vec![],
    };
    ranks.sort_by_key(|r| r.rank);
    ranks.truncate(3);
    view! {
        <div class="teams">
            <table class="finaltable">
                {ranks.iter().enumerate().map(|(i, rank)| {
                    let team = store.team(rank.team_id);
                    let name = team.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| format!("Team {}", rank.team_id));
                    let logo = team.map(|t| t.logo).unwrap_or_default();
                    view! {
                        <tr>
                            <td>{format!("{}.", i + 1)}</td>
                            <td>
                                {(!logo.is_empty() && logo != "false").then(|| view! {
                                    <img class="media-object" src={format!("logos/{logo}")} alt="" loading="lazy" />
                                })}
                            </td>
                            <td><strong>{name}</strong></td>
                            <td><small>{format!("{:.1} points", rank.points)}</small></td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </table>
        </div>
    }
}
