//! Patches and Packages pages (form-driven ops tools).

use crate::api::*;
use crate::{err_box, table};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use serde_json::{json, Value};

#[component]
pub fn Patches() -> impl IntoView {
    let data = LocalResource::new(|| async { get_json("/api/patches").await });
    let (msg, set_msg) = signal(String::new());
    let (new_name, set_new_name) = signal(String::new());
    let (new_type, set_new_type) = signal("bash".to_string());
    let (selected, set_selected) = signal(String::new());
    let reload = move || data.refetch();
    let upload = move |_| {
        spawn_local(async move {
            let doc = web_sys::window().expect("window").document().expect("document");
            let input = doc.get_element_by_id("patchfile-input").expect("#patchfile-input")
                .unchecked_into::<web_sys::HtmlInputElement>();
            let Some(file) = input.files().and_then(|f| f.get(0)) else {
                set_msg.set("No file chosen".into());
                return;
            };
            let name = file.name();
            let buf = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(file.array_buffer()))
                .await.expect("arrayBuffer");
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            set_msg.set(match post_raw(&format!("/api/patches/upload?filename={}", urlencoding(&name)), bytes).await {
                Ok(t) => t,
                Err(e) => e,
            });
            reload();
        });
    };
    let run = move |target: &'static str| {
        spawn_local(async move {
            let f = selected.get();
            match post_json("/api/patches/run", json!({"filename": f, "target": target})).await {
                Ok(_) => set_msg.set("Ran playbook".into()),
                Err(e) => set_msg.set(e),
            }
            reload();
        });
    };
    view! {
        <h1>"Patches"</h1>
        {move || if !msg.get().is_empty() { view! { <p class="warn">{msg.get()}</p> }.into_any() } else { "".into_any() }}
        {move || match data.get() {
            None => "Loading…".into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["files"].as_array().cloned().unwrap_or_default().iter().map(|f| {
                    let name = s(&f["name"]);
                    vec![
                        {let n = name.clone();
                         view! { <label><input type="radio" name="patchfile" prop:value=name.clone()
                            on:change=move |_| set_selected.set(n.clone()) /> <code>{name.clone()}</code></label> }.into_any()},
                        {match f["url"].as_str() {
                            Some(url) => view! { <a href={url.to_string()}>"download"</a> }.into_any(),
                            None => "".into_any(),
                        }}
                    ]
                }).collect();
                view! {
                    {table(&["File", "Public URL"], rows)}
                    <h2>"Run playbook"</h2>
                    <form class="inline">
                        <button type="button" on:click=move |_| run("test")>"Test on NOP team"</button>
                        <button type="button" on:click=move |_| run("deploy")>"Deploy"</button>
                        <button type="button" on:click=move |_| {
                            spawn_local(async move { let _ = post_json("/api/patches/hosts", json!({})).await; reload(); });
                        }>"Regenerate hosts.yaml"</button>
                    </form>
                    <h2>"Create new patch"</h2>
                    <form class="inline" on:submit=|e| e.prevent_default()>
                        <input placeholder="filename" prop:value=new_name on:input=move |e| set_new_name.set(event_target_value(&e)) />
                        <select prop:value=new_type on:change=move |e| set_new_type.set(event_target_value(&e))>
                            <option value="bash">"bash"</option>
                            <option value="python">"python"</option>
                        </select>
                        <button type="button" on:click=move |_| {
                            let (n, t) = (new_name.get(), new_type.get());
                            spawn_local(async move {
                                set_msg.set(post_json("/api/patches/create", json!({"filename": n, "type": t})).await.unwrap_or_else(|e| e));
                                reload();
                            });
                        }>"Create"</button>
                    </form>
                    <h2>"Publish selected"</h2>
                    <form class="inline">
                        <button type="button" on:click=move |_| {
                            let f = selected.get();
                            spawn_local(async move {
                                set_msg.set(post_json("/api/patches/publish", json!({"filename": f})).await.unwrap_or_else(|e| e));
                                reload();
                            });
                        }>"Publish"</button>
                    </form>
                    <h2>"Upload file"</h2>
                    <form class="inline" on:submit=|e| e.prevent_default()>
                        <input type="file" id="patchfile-input" />
                        <button type="button" on:click=upload>"Upload"</button>
                    </form>
                }.into_any()
            }
        }}
    }
}

#[component]
pub fn Packages() -> impl IntoView {
    let (msg, set_msg) = signal(String::new());
    let (busy, set_busy) = signal(false);
    let do_call = move |endpoint: &'static str, body: Value| {
        spawn_local(async move {
            set_busy.set(true);
            set_msg.set(match post_json(endpoint, body).await {
                Ok(text) => text,
                Err(e) => e,
            });
            set_busy.set(false);
        });
    };
    view! {
        <h1>"Checker packages"</h1>
        <p>"These operations broadcast to all running celery workers (may take a while)."</p>
        <form class="inline">
            <button type="button" disabled=busy on:click=move |_| do_call("/packages/update", json!({}))>"Update packages"</button>
            <button type="button" disabled=busy on:click=move |_| do_call("/packages/push", json!({}))>"Push packages"</button>
        </form>
        {move || if !msg.get().is_empty() { view! { <pre>{msg.get()}</pre> }.into_any() } else { "".into_any() }}
    }
}
