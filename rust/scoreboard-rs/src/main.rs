//! CTF public scoreboard — Leptos CSR port of the Angular app.

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
use leptos_router::components::{Redirect, Route, Router, Routes};
use leptos_router::hooks::use_location;
use leptos_router::path;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

/// `app.component.html` — header band, router outlet.
#[component]
fn App() -> impl IntoView {
    let store = Store::new();
    let ui = Ui::new();
    provide_context(store);
    provide_context(ui);

    view! {
        <Router>
            <a class="skip-link" href="#main-content">"Skip to scoreboard"</a>
            <overlay::NotificationOverlay />
            <header class="app-header">
                <a class="logo" href="/" aria-label="CTF scoreboard home">"◼ "<span>"CTF"</span>" SCOREBOARD"</a>
                <nav class="mainnav" aria-label="Scoreboard">
                    <NavLink href="/" label="scoretable" icon="fa-th-large" />
                    <NavLink href="/graphs" label="graphs" icon="fa-chart-line" />
                    <NavLink href="/dashboard" label="dashboard" icon="fa-tachometer-alt" />
                </nav>
                <BackLink />
                <CurrentTick />
                <Settings />
            </header>
            <main id="main-content" tabindex="-1" class="app-main">
                <Routes fallback=not_found>
                    <Route path=path!("/") view=move || pages::scoretable(store, ui) />
                    <Route path=path!("/dashboard") view=move || pages::dashboard(store, ui) />
                    <Route path=path!("/team/:teamid") view=move || team::team_page(store, ui) />
                    <Route path=path!("/graphs") view=move || graphs::graphs_page(store, ui) />
                </Routes>
            </main>
        </Router>
    }
}

/// Header navigation entry: icon-only, label kept as accessible name and
/// tooltip; `aria-current` marks the active route.
#[component]
fn NavLink(href: &'static str, label: &'static str, icon: &'static str) -> impl IntoView {
    let path = use_location().pathname;
    let current = move || {
        let p = path.get();
        if href == "/" { (p == "/").to_string() } else { p.starts_with(href).to_string() }
    };
    view! {
        <a href=href title=label aria-current=move || if current() == "true" { "page" } else { "false" }>
            <span class=format!("fas {icon}") aria-hidden="true"></span>
            <span class="sr-only">{label}</span>
        </a>
    }
}

/// backlink, hidden (exact) while on `/`.
#[component]
fn BackLink() -> impl IntoView {
    let path = use_location().pathname;
    view! {
        <a href="/" class="backlink" class:hidden=move || path.get() == "/" >
            "\u{2190} back to scoreboard"
        </a>
    }
}

/// `settings.component` — dropdown for every preference.
#[component]
fn Settings() -> impl IntoView {
    let ui = expect_context::<Ui>();
    let open = RwSignal::new(false);

    view! {
        <div class="settings">
            <button
                id="settings-button"
                type="button"
                class="gear"
                aria-label="Settings"
                aria-haspopup="true"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <span class="fas fa-cog" aria-hidden="true"></span>
            </button>
            {move || {
                if !open.get() { return ().into_any(); }
                let items = [
                    (ui.darkmode, "Switch to light mode", "Switch to dark mode"),
                    (ui.show_history, "Hide tick history", "Show tick history"),
                    (ui.show_only_sums, "Show full table", "Show only sums"),
                    (ui.show_images, "Hide team logos", "Show team logos"),
                    (ui.show_notifications, "Hide notifications", "Show notifications"),
                    (ui.sound, "Disable first blood sound", "Enable first blood sound"),
                ];
                view! {
                    <ul id="dropdown-settings" class="dropdown-menu show" role="menu" aria-labelledby="settings-button">
                        {items.iter().map(|(sig, off_label, on_label)| {
                            let (sig, off_label, on_label) = (*sig, *off_label, *on_label);
                            view! {
                                <li role="menuitem">
                                    <button
                                        type="button"
                                        class="dropdown-item"
                                        aria-pressed=move || sig.get().to_string()
                                        on:click=move |_| sig.update(|v| *v = !*v)
                                    >
                                        {move || if sig.get() { off_label } else { on_label }}
                                    </button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}

/// `current-tick.component` — 500 ms ticker, remaining time and state badge.
#[component]
fn CurrentTick() -> impl IntoView {
    let store = expect_context::<Store>();
    let current_time = RwSignal::new((js_sys::Date::now() / 1000.0) as i64);

    Effect::new(move || {
        let update = move || current_time.set((js_sys::Date::now() / 1000.0) as i64);
        update();
        let handle = set_interval_with_handle(update, std::time::Duration::from_millis(500));
        let _ = handle; // interval lives for the page lifetime
    });

    view! {
        <div class="tickclock" aria-live="polite">
            <span class="state">
                {move || {
                    let st = store.state.get();
                    if st.frozen {
                        view! {
                            <span class="label label-info" title="Points are no longer updated. You can still see your service status.">
                                "❄ frozen"
                            </span>
                        }.into_any()
                    } else { ().into_any() }
                }}
                {move || {
                    let st = store.state.get();
                    if st.state == STATE_SUSPENDED {
                        view! { <span class="label label-warning">"game suspended"</span> }.into_any()
                    } else { ().into_any() }
                }}
                {move || {
                    let st = store.state.get();
                    if st.state == STATE_STOPPED && st.current_tick > 0 {
                        view! { <span class="label label-success">"game is over"</span> }.into_any()
                    } else { ().into_any() }
                }}
                {move || {
                    let st = store.state.get();
                    if st.state == STATE_STOPPED && st.current_tick <= 0 {
                        view! { <span class="label label-danger">"not started"</span> }.into_any()
                    } else { ().into_any() }
                }}
            </span>
            {move || {
                let t = store.state.get().current_tick;
                if t >= 0 {
                    view! { <span class="tickline">"tick " <b>{t}</b></span> }.into_any()
                } else { ().into_any() }
            }}
            {move || {
                let st = store.state.get();
                if current_time.get() > 0 && (st.state == STATE_RUNNING || st.current_tick < 0) {
                    // formatRemainingTime: remaining = tick_until - client_now + (client - server)
                    let mut remaining =
                        st.current_tick_until - current_time.get_untracked()
                            + store.delta_client_to_server.get_untracked();
                    if remaining < 0 { remaining = 0; }
                    let m = remaining / 60;
                    let s = remaining % 60;
                    view! {
                        <span class="round">"· round ends in\u{00a0}"<span class="time">{format!("{m:02}:{s:02}")}</span></span>
                    }.into_any()
                } else { ().into_any() }
            }}
        </div>
    }
}

fn not_found() -> impl IntoView {
    view! {
        <div class="alert alert-danger" role="alert">"Page not found"</div>
        <Redirect path="/" />
    }
}
