//! Patches and Packages pages (form-driven ops tools).

use crate::api::*;
use crate::{err_box, table};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use serde_json::{json, Value};

fn patch_type(name: &str) -> &'static str {
    if name.ends_with(".py") {
        "python"
    } else if name.ends_with(".sh") {
        "bash"
    } else {
        "—"
    }
}

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
        <div class="sub">"shell/python scripts deployed via ansible to vulnboxes · test before deploy"</div>
        {move || match data.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["files"].as_array().cloned().unwrap_or_default().iter().map(|f| {
                    let name = s(&f["name"]);
                    let n = name.clone();
                    let download = match f["url"].as_str() {
                        Some(url) => view! { <a href={url.to_string()} class="dim">"↓ download"</a> }.into_any(),
                        None => "".into_any(),
                    };
                    view! {
                        <tr>
                            <td>
                                <input type="radio" name="patchfile" aria-label=name.clone() prop:value=name.clone()
                                    on:change=move |_| set_selected.set(n.clone()) />
                            </td>
                            <td><code>{name.clone()}</code> " " {download}</td>
                            <td>{patch_type(&name)}</td>
                            <td class="dim">{format!("patches/ansible/{name}.yml")}</td>
                        </tr>
                    }.into_any()
                }).collect::<Vec<_>>();
                view! {
                    <table>
                        <thead><tr>
                            <th></th><th>"Script"</th><th>"Type"</th><th>"Generated playbook"</th>
                        </tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                }.into_any()
            }
        }}

        <h2>"Deploy"</h2>
        <div class="row">
            <button type="button" on:click=move |_| {
                let f = selected.get();
                spawn_local(async move {
                    set_msg.set(post_json("/api/patches/publish", json!({"filename": f})).await.unwrap_or_else(|e| e));
                    reload();
                });
            }>"Publish"</button>
            <button type="button" class="warnbtn" title="runs ansible against NOP team only" on:click=move |_| run("test")>"Test on NOP team"</button>
            <button type="button" class="primary" title="runs ansible against all vulnboxes" on:click=move |_| run("deploy")>"Deploy"</button>
            <button type="button" on:click=move |_| {
                spawn_local(async move { let _ = post_json("/api/patches/hosts", json!({})).await; reload(); });
            }>"Regenerate hosts.yaml"</button>
        </div>
        <div class="logbox">
            {move || if msg.get().is_empty() {
                view! { <span class="dim">"no output yet"</span> }.into_any()
            } else {
                view! { <span>{msg.get()}</span> }.into_any()
            }}
        </div>

        <h2>"Create / upload"</h2>
        <form class="row" on:submit=|e| e.prevent_default()>
            <span class="dim">"filename"</span>
            <input placeholder="filename" style="width: 180px" prop:value=new_name on:input=move |e| set_new_name.set(event_target_value(&e)) />
            <span class="dim">"type"</span>
            <select prop:value=new_type on:change=move |e| set_new_type.set(event_target_value(&e))>
                <option value="bash">"bash"</option>
                <option value="python">"python"</option>
            </select>
            <button type="button" class="primary" on:click=move |_| {
                let (n, t) = (new_name.get(), new_type.get());
                spawn_local(async move {
                    set_msg.set(post_json("/api/patches/create", json!({"filename": n, "type": t})).await.unwrap_or_else(|e| e));
                    reload();
                });
            }>"Create"</button>
            <input type="file" id="patchfile-input" />
            <button type="button" on:click=upload>"Upload file…"</button>
        </form>
        <p class="dim" style="font-size: 11px; margin-top: 8px">"→ appears in the table above; hosts.yaml regenerated automatically on deploy"</p>
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
    let services = LocalResource::new(|| async { get_json("/api/services/list").await });
    view! {
        <h1>"Packages"</h1>
        <div class="sub">"checker packages build + distribution to vulnboxes"</div>

        <h2>"Actions"</h2>
        <div class="row">
            <button type="button" class="primary" disabled=busy on:click=move |_| do_call("/packages/update", json!({}))>"Update packages"</button>
            <button type="button" disabled=busy on:click=move |_| do_call("/packages/push", json!({}))>"Push packages"</button>
            <span class="dim">"— broadcast to all running checker workers (may take a while)"</span>
        </div>

        <h2>"Packages"</h2>
        {move || match services.get() {
            None => view! { <p class="dim">"Loading…"</p> }.into_any(),
            Some(Err(e)) => err_box(&e).into_any(),
            Some(Ok(v)) => {
                let rows = v["services"].as_array().cloned().unwrap_or_default().iter().map(|svc| {
                    let built = !svc["package"].is_null();
                    vec![
                        s(&svc["name"]).into_any(),
                        view! { <span class="hash">{s_opt(&svc["package"])}</span> }.into_any(),
                        view! {
                            <span class=if built { "ok" } else { "warn" }>
                                {if built { "built" } else { "stale — no package" }}
                            </span>
                        }.into_any(),
                    ]
                }).collect();
                table(&["Service", "Package", "State"], rows).into_any()
            }
        }}

        <h2>"Last job output"</h2>
        <div class="logbox">
            {move || if msg.get().is_empty() {
                view! { <span class="dim">"no job run yet"</span> }.into_any()
            } else {
                view! { <span>{msg.get()}</span> }.into_any()
            }}
        </div>
    }
}
