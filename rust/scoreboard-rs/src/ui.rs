//! Port of `ui.service.ts` — user preferences in sessionStorage (like the
//! original's @SessionStorage), plus the dark-mode flag mirroring to
//! `html.dark`.

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct Ui {
    pub show_history: RwSignal<bool>,
    pub show_only_sums: RwSignal<bool>,
    pub show_images: RwSignal<bool>,
    pub show_notifications: RwSignal<bool>,
    pub sound: RwSignal<bool>,
    pub darkmode: RwSignal<bool>,
}

fn ss_get(key: &str) -> Option<bool> {
    let w = web_sys::window()?;
    let s = w.session_storage().ok().flatten()?;
    s.get_item(key).ok().flatten()?.parse().ok()
}

fn ss_set(key: &str, v: bool) {
    if let Some(w) = web_sys::window() {
        if let Ok(Some(s)) = w.session_storage() {
            let _ = s.set_item(key, &v.to_string());
        }
    }
}

impl Ui {
    pub fn new() -> Ui {
        // darkmode default: the Ops Console is dark-first; a stored choice wins
        let ui = Ui {
            show_history: RwSignal::new(ss_get("showHistory").unwrap_or(true)),
            show_only_sums: RwSignal::new(ss_get("showOnlySums").unwrap_or(false)),
            show_images: RwSignal::new(ss_get("showImages").unwrap_or(true)),
            show_notifications: RwSignal::new(ss_get("showNotifications").unwrap_or(true)),
            sound: RwSignal::new(ss_get("sound").unwrap_or(true)),
            darkmode: RwSignal::new(ss_get("darkmode").unwrap_or(true)),
        };
        let this = ui;
        Effect::new(move || {
            let dark = this.darkmode.get();
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let class = doc.document_element().map(|e| e.class_list());
                if let Some(list) = class {
                    let _ = if dark { list.add_1("dark") } else { list.remove_1("dark") };
                }
            }
        });
        let this = ui;
        Effect::new(move || ss_set("showHistory", this.show_history.get()));
        let this = ui;
        Effect::new(move || ss_set("showOnlySums", this.show_only_sums.get()));
        let this = ui;
        Effect::new(move || ss_set("showImages", this.show_images.get()));
        let this = ui;
        Effect::new(move || ss_set("showNotifications", this.show_notifications.get()));
        let this = ui;
        Effect::new(move || ss_set("sound", this.sound.get()));
        let this = ui;
        Effect::new(move || ss_set("darkmode", this.darkmode.get()));
        ui
    }
}
