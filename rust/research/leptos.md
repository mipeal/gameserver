# Leptos for the gameserver scoreboard rewrite (researched 2026-11-19)

## Verdict

Yes, a good fit. Leptos is alive and current: crate `leptos` is at **0.8.23** (published
2026-10-31, by gbj) with a **0.9.0-rc1** pre-release; 6.3M total downloads on crates.io.
The repo (github.com/leptos-rs/leptos) was last pushed **2026-11-17**, has 21.5k stars,
117 open issues. Not stale, not abandoned. No evidence points to Yew or Dioxus being
the better call for a pure-client SPA; Leptos's CSR story is a first-class, documented
part of the book (Part 1 of the book is entirely CSR, from `mount_to_body` through
deployment).

## Recommended setup for a pure-client WASM SPA

Build tool: **Trunk**, not cargo-leptos. The book's Getting Started chapter prescribes
Trunk for CSR apps; cargo-leptos is introduced only in the SSR part of the book
(ch. "cargo-leptos" sits under "Part 2: Server Side Rendering"). Book commands:

- `cargo install --locked trunk`
- `cargo init <dir>`; `cargo add leptos --features=csr`
- `rustup target add wasm32-unknown-unknown`
- minimal `index.html` next to Cargo.toml with an empty `<body>`
- `leptos::mount::mount_to_body(|| view! { <p>"Hello, world!"</p> })`
- dev: `trunk serve --open`

## Routing

`leptos_router` is a separate crate (book: users "are completely free" to swap it).
One `<Router/>` near the root (using two is an explicit anti-pattern), then
`<Routes fallback=...>` with `path!` macros — static segments (`/users`), named
params (`/:id`), wildcards (`/*any`):

```rust
use leptos_router::components::{Router, Route, Routes};
// <Route path=path!("/users/:id") view=UserProfile/>
```

Nested routing (ParentRoute/Outlet), `<A/>` links, `<Form/>`, params and queries are
covered in book ch. 9.2-9.5. For a dashboard/team/settings app this maps directly
onto the Angular routes.

## Reactivity pattern for a polling dashboard

Signals (`RwSignal`), `Memo`, effects; async data via resources (book ch. "Loading
Data with Resources"):

- **`LocalResource::new(move || fetcher)`** is the book's choice for browser-only
  async work ("can only be done on the browser", `!Send`-friendly); the fetcher is
  any Rust `Future` and re-runs when reactive values read inside it change. Read with
  `.get()` — `None` before resolution, `Some(T)` after, so the scoretable shows a
  loading row for free. `Resource::new` is recommended only for SSR (it serializes
  from server); for this SPA use `LocalResource` / `OnceResource`.
- Polling: the book has no dedicated intervals chapter; the community crate
  **`leptos-use` (0.20.0, published 2026-10-20, 1.77M downloads, very active)**
  provides VueUse-style utilities (intervals, event listeners, document access) —
  use `use_interval` (or `set_interval_with_handle` + cleanup) to tick a signal, and
  let the `LocalResource` fetcher read that signal so every tick re-fetches the
  scoretable.
- HTTP: fetch via `gloo-net` (typed JSON deserialization) or plain `web_sys`
  `fetch` — the book's ecosystem page defers HTTP clients to "Awesome Leptos";
  gloo-net is the common CSR choice.

## Charting — honest maturity check

- **`leptos_chart`**: stale. Latest 0.3.0 (2024-05-03), depends on `leptos ^0.6`
  (verified via crates.io dependencies endpoint), only 36 recent downloads. Not
  compatible with Leptos 0.8 without forking. Do not plan around it.
- **`plotters` + `plotters-canvas`**: maintained, framework-agnostic. `plotters`
  0.3.7 (2024-09-08, 279M downloads), `plotters-canvas` 0.3.7 backend at 0.3.1
  (2025-03-07). Renders to an HTML5 canvas you own inside a `view!`. Solid for
  static/simple charts; you handle redraw-on-new-data yourself (fine for a poll
  loop). No reactive integration — that is the price.
- **Chart.js via wasm-bindgen**: no maintained Rust binding crate surfaced
  (crates.io `chart-js` 404s). Keeping the existing Chart.js from a `<script>` tag
  in index.html and writing thin `wasm-bindgen` glue (`Chart(canvas, config)` with
  `js_sys`/`serde-wasm-bindgen` for the config object) is viable and keeps chart
  rendering/animation identical to the Angular app, at the cost of hand-written
  bindings. This is the lowest-risk path for pixel-parity with the current charts.

**Recommendation**: Chart.js via a small hand-rolled wasm-bindgen shim (reuse the
existing chart configs), with plotters-canvas as the pure-Rust fallback if the
interop feels heavy. Avoid leptos_chart.

## Semantic traps

- **WASM size** (book ch. "Optimizing WASM Binary Size"): debug builds are "much,
  much larger". Use a size profile — `opt-level = 'z'`, `lto = true`,
  `codegen-units = 1` — and always serve the WASM compressed ("typically shrinking
  to less than 50%"). `serde` and especially `regex` inflate binaries (~500kb for
  regex's Unicode tables; `leptos_router` deliberately uses browser regex APIs
  instead). Expect a several-MB uncompressed .wasm for a dashboard app; gzip/brotli
  is what makes it shippable.
- **Fetch response typing**: in CSR use `LocalResource`, not `Resource` — the book
  notes `Resource` is for SSR where the value "serializes its value from the server
  to the client". The fetcher must `serde`-deserialize the JSON itself (gloo-net
  `.json::<T>()`); there is no Angular `HttpClient` equivalent typing, and a wrong
  `Deserialize` model fails at runtime, not compile time.
- **Timezone formatting**: Rust date-time crates do not automatically behave like
  JS `Intl.DateTimeFormat`. `chrono` on wasm historically returns UTC for
  `Local::now()` unless the `iana-time-zone` feature resolves the browser TZ, and
  locale-aware formatting (month names, 12/24h) that Angular pipes did for free has
  to be done via `js_sys` `Intl.DateTimeFormat` interop or hand-rolled. Budget real
  time for scoreboard timestamp rendering.
- Router is a separate crate: import from `leptos_router::components::...`, not
  `leptos::`.

## Sources

- crates.io API: /crates/leptos, /leptos_chart (+ /0.3.0/dependencies),
  /plotters, /plotters-canvas, /leptos-use (fetched 2026-11-19)
- github.com/leptos-rs/leptos repo metadata (pushed 2026-11-17)
- book.leptos.dev: /, /getting_started/index.html, /router/index.html,
  /router/16_routes.html, /async/10_resources.html, /deployment/binary_size.html,
  /getting_started/community_crates.html, /csr_wrapping_up.html
