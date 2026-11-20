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
use leptos_router::components::{Redirect, Route, Router, Routes, A};
use leptos_router::hooks::use_location;
use leptos_router::path;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

/// `app.component.html` — header band, router outlet, credits.
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
            <div class="flexible-container">
                <div class="bg-primary-theme">
                    <div class="header-inner">
                        <Settings />
                        <div class="container-fluid">
                            <h1 class="page-header">
                                {"\u{00a0}CTF Scoreboard"}
                                <small>
                                    <A href="/dashboard"><span class="fas fa-th"></span></A>
                                    {"\u{00a0}"}
                                    <A href="/graphs"><span class="fas fa-chart-line"></span></A>
                                </small>
                                <BackLink />
                                <span class="pull-right"><CurrentTick /></span>
                            </h1>
                        </div>
                        <br/>
                    </div>
                </div>
                <div class="container-fluid flexible-container">
                    <div id="main-content">
                        <Routes fallback=not_found>
                            <Route path=path!("/") view=move || pages::scoretable(store, ui) />
                            <Route path=path!("/dashboard") view=move || pages::dashboard(store, ui) />
                            <Route path=path!("/team/:teamid") view=move || team::team_page(store, ui) />
                            <Route path=path!("/graphs") view=move || graphs::graphs_page(store, ui) />
                        </Routes>
                    </div>
                </div>
                <br/>
            </div>
        </Router>
    }
}

/// backlink, hidden (exact) while on `/`.
#[component]
fn BackLink() -> impl IntoView {
    let path = use_location().pathname;
    view! {
        <a href="/" class="backlink" class:hidden=move || path.get() == "/" >
            <i class="fas fa-chevron-left"></i> " back to scoreboard"
        </a>
    }
}

/// `settings.component` — dropdown toggling the UiService preferences.
#[component]
fn Settings() -> impl IntoView {
    let ui = expect_context::<Ui>();
    let open = RwSignal::new(false);

    view! {
        <div class="pull-right btn-group">
            <button
                id="settings-button"
                type="button"
                class="btn btn-link dropdown-toggle"
                aria-label="Settings"
                aria-haspopup="true"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <i class="fas fa-cog"></i><span class="caret"></span>
            </button>
            {move || {
                if !open.get() { return ().into_any(); }
                let items = [
                    (ui.show_history, "Hide differences to last tick", "Show differences to last tick"),
                    (ui.show_only_sums, "Show detailed scores", "Hide detailed scores (slim mode)"),
                    (ui.show_images, "Hide team logos", "Show team logos"),
                    (ui.show_notifications, "Hide notifications", "Show notifications"),
                    (ui.sound, "Disable first blood sound", "Enable first blood sound"),
                    (ui.darkmode, "Light mode", "Dark mode"),
                ];
                view! {
                    <ul id="dropdown-settings" class="dropdown-menu dropdown-menu-right show" role="menu" aria-labelledby="settings-button">
                        {items.iter().map(|(sig, off_label, on_label)| {
                            let (sig, off_label, on_label) = (*sig, *off_label, *on_label);
                            view! {
                                <li role="menuitem">
                                    <a class="dropdown-item href" on:click=move |_| sig.update(|v| *v = !*v)>
                                        {move || if sig.get() { off_label } else { on_label }}
                                    </a>
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
        <div class="current-tick" aria-live="polite">
            <p class="state">
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
            </p>
            {move || {
                let t = store.state.get().current_tick;
                if t >= 0 {
                    view! { <p class="tick">{format!("Tick {t}")}</p> }.into_any()
                } else { ().into_any() }
            }}
            <p class="time">
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
                            <span>{format!("{m:02}:{s:02}")}</span>
                        }.into_any()
                    } else { ().into_any() }
                }}
            </p>
        </div>
    }
}

fn not_found() -> impl IntoView {
    view! {
        <div class="alert alert-danger">"Page not found"</div>
        <Redirect path="/" />
    }
}
