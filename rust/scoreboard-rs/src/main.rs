//! saarCTF public scoreboard — Leptos CSR port of the Angular app.

mod charts;
mod graphs;
mod models;
mod overlay;
mod pages;
mod store;
mod team;
mod ui;

use crate::models::{STATE_RUNNING, STATE_STOPPED, STATE_SUSPENDED};
use crate::store::Store;
use crate::ui::Ui;
use leptos::prelude::*;
use leptos_router::components::{Redirect, Route, Router, Routes, A};
use leptos_router::path;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let store = Store::new();
    let ui = Ui::new();
    provide_context(store);
    provide_context(ui);

    view! {
        <Router>
            <Header />
            <main>
                <Routes fallback=not_found>
                    <Route path=path!("/") view=move || pages::scoretable(store, ui) />
                    <Route path=path!("/dashboard") view=move || pages::dashboard(store) />
                    <Route path=path!("/team/:teamid") view=move || team::team_page(store) />
                    <Route path=path!("/graphs") view=move || graphs::graphs_page(store) />
                </Routes>
            </main>
            <p class="credits">
                <a href="https://github.com/saarctf/scoreboard" rel="noopener" target="_blank">saarCTF scoreboard</a>
            </p>
            {overlay::notification_overlay(store, ui)}
        </Router>
    }
}

#[component]
fn Header() -> impl IntoView {
    let ui = expect_context::<Ui>();
    let settings_open = RwSignal::new(false);

    view! {
        <header class="sb">
            <h1>"saarCTF"</h1>
            <nav>
                <A href="/">"Scores"</A>
                <A href="/dashboard">"Dashboard"</A>
                <A href="/graphs">"Graphs"</A>
            </nav>
            <div class="header-right">
                <CurrentTick />
                <button class="settings-btn" on:click=move |_| settings_open.update(|o| *o = !*o)>
                    "⚙ Settings"
                </button>
            </div>
        </header>
        {move || {
            if !settings_open.get() { return ().into_any(); }
            let items = [
                ("Show scoreboard history", ui.show_history),
                ("Show only sums", ui.show_only_sums),
                ("Show team images", ui.show_images),
                ("Show notifications", ui.show_notifications),
                ("Dark mode", ui.darkmode),
            ];
            view! {
                <div class="settings-menu" on:mouseleave=move |_| settings_open.set(false)>
                    {items.iter().map(|(label, sig)| {
                        let (label, sig) = (*label, *sig);
                        view! {
                            <a on:click=move |_| sig.update(|v| *v = !*v)>
                                {label} ": " {move || if sig.get() { "yes" } else { "no" }}
                            </a>
                        }
                    }).collect_view()}
                </div>
            }.into_any()
        }}
    }
}

#[component]
fn CurrentTick() -> impl IntoView {
    let store = expect_context::<Store>();
    let remaining = RwSignal::new(String::new());

    // 1 s countdown of current_tick_until - (now - delta)
    Effect::new(move || {
        let update = move || {
            let st = store.state.get_untracked();
            let now = (js_sys::Date::now() / 1000.0) as i64
                - store.delta_client_to_server.get_untracked();
            let left = (st.current_tick_until - now).max(0);
            remaining.set(if st.state == STATE_RUNNING {
                fmt_mmss(left)
            } else {
                state_text(st.state).to_string()
            });
        };
        update();
        let handle = set_interval_with_handle(update, std::time::Duration::from_secs(1));
        let _ = handle; // interval lives for the page lifetime
    });

    view! {
        <div class="current-tick">
            <p class="tick">{move || {
                let t = store.state.get().current_tick;
                if t >= 0 { format!("Tick {t}") } else { String::new() }
            }}</p>
            <p class="time">{remaining}</p>
        </div>
    }
}

fn fmt_mmss(secs: i64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn state_text(state: u8) -> &'static str {
    match state {
        STATE_RUNNING => "RUNNING",
        STATE_SUSPENDED => "SUSPENDED",
        STATE_STOPPED => "STOPPED",
        _ => "?",
    }
}

fn not_found() -> impl IntoView {
    view! {
        <h2>"404 — not found"</h2>
        <Redirect path="/" />
    }
}
