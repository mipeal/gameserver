# axum — Flask controlserver port (wave 1)

Sources: docs.rs/axum 0.8.9 (crate page, `routing::Router`, `middleware` module),
docs.rs/tower-sessions 0.15.0, and repo files as cited. Resolves MAP.md open question
"Flask session/login: which mechanism in axum".

## 0. Premise correction: there are no tornado servers

The task brief assumed tornado in `timer.py`, `scoring/scoreboard.py`, `vpnboard/`.
`grep -rn tornado` over the whole repo: zero hits. What those actually are:

- `controlserver/timer.py` — Redis-backed singleton + `run_master_timer` thread
  (pubsub on `timing:*` keys, `timer.py:186-205`). No HTTP.
- `controlserver/scoring/scoreboard.py` — static-file generator: listens on Redis
  `timing:scoreboard_tick` pubsub (`scoreboard.py:737-743`) and writes JSON files
  into `<output>/api/` (`scoreboard.py:530-564`). No HTTP server; nginx serves the
  files (`nginx.default.conf`: `location /api/ { try_files $uri =404; }`).
- `vpnboard/` — `vpn_status_daemon.py` is a Redis pubsub + TCP-check thread pool;
  `vpn_board.py` is a jinja2 HTML file generator. No HTTP.

So the only Python HTTP server to port is the Flask app. The tornado→axum question
dissolves: timer, scoreboard and vpnboard become Redis-subscribing daemons /
file writers, not axum services (MAP.md already lists them as separate units).

## 1. Session approach for the organizer login: none exists — port none

- `app.py:94`: `app.config["SECRET_KEY"] = os.urandom(12).hex()` — fresh every
  process start, i.e. not usable for persistent sessions anyway.
- No `flask-login`, no `session[...]` usage, no auth decorator anywhere
  (`grep -rn "login\|session\[" controlserver/` → only DB sessions).
- Flask-Admin is mounted with **no auth view** (`model_admin.py:52-60`,
  `Admin(app, ...)` without an index view). All `/overview/*`, `/scripts/*`,
  `/packages/*` POST endpoints are unauthenticated today; access control is
  network-level.

Decision: **port no sessions** (ADR-004 semantic-equivalence bar: no behavior to
mirror). If a login is wanted later when the admin panel folds into the Leptos SPA
(ADR-002), use **tower-sessions 0.15** (`SessionManagerLayer` + `MemoryStore`;
SQLx/Redis stores exist via `tower-sessions-sqlx-store` /
`tower-sessions-redis-store` if multi-process). Hand-rolled signed cookies and
Redis-backed sessions would be extra code with no current requirement.

## 2. Flask blueprints → axum Router

Blueprints are just URL-prefix groupings; none of them use a `url_prefix`
(all rules are absolute, e.g. `pages.py:34` `@app.route("/")`), so the mapping is
one flat merge:

- `Router::new().route("/x", get(h))` per endpoint module; combine with
  `.merge(...)` (same-level paths, Router docs) — `nest` is unnecessary and would
  change URI semantics (nested routers strip the prefix; none of the blueprints
  has one).
- State: `AppState` (DB pool, Redis, `Timer`) via `Router<AppState>` +
  `.with_state(...)`, extracted with `State<AppState>` (docs.rs/axum: "prefer
  State over extensions since it's more type safe"). `FromRef` for substates.
- Path captures: Flask `/<int:page>` → axum `/{page}` with `Path<i64>`.
  Router docs: segments cannot be constrained to numeric; the typed extractor
  rejects non-numeric with 400/404 — handle in the handler, don't assume
  Flask's 404-on-converter-failure for free.
- Method routing: `.route("/overview/set_timing", post(handler))`; multiple
  methods on one rule (`patches.py:22` GET+POST) → `get(a).post(b)`.
- Timers/threads: Flask starts the master-timer thread in `create_app`
  (`app.py:109`); in axum spawn it as a tokio task in `main` before `axum::serve`.
- Templates (jinja2, `templates/`): under ADR-002 these pages become Leptos, so
  the axum port initially only needs the JSON routes; `render_template` pages can
  be stubbed (404.html) or served with a `minijinja` shim if kept alive longer.

## 3. Error-handler parity — the repo does NOT return {"error": ...}

There is no `@app.errorhandler` and no `{"error": ...}` JSON anywhere in
`controlserver/`. Actual error shapes (grep across `endpoints/*.py`):

| Status | Body | Where |
|---|---|---|
| 400 | plain text `"No ip"` | `teams.py:157` |
| 400 | plain text `"No string"`, `"No filename"`, `"Invalid filename"` | `pages.py:286`, `patches.py:141,144` |
| 404 | HTML `render_template('404.html')` | `log_messages.py:56`, `teams.py:126,160`, `checker_results.py:72`, `pages.py:360` |
| 404 | plain text `"Invalid filename"` | `patches.py:76` |
| 500 | plain text `"Invalid type"`, `"Invalid"` | `patches.py:117`, `api.py:18,20` |
| 500 | `jsonify(errors)` (list, not object) | `services.py:124` (`eno_config_valid`: 200 when valid, 500 when not) |

Port rule: `(StatusCode, "text")` / `(StatusCode, Html(...))`, not a JSON error
envelope. Success bodies use `jsonify` → axum `Json<T>`; `jsonify(messages)` of a
bare list is fine as `Json<Vec<String>>`. `Timer.state` is an IntEnum
(`timer.py:30-33`) and Flask serializes it as its **int** — Rust enum with
`#[serde(int)]`-style `u8` repr (`serde(int)`), not the name.

## 4. Exact endpoints the Angular scoreboard calls (backend.service.ts)

`url_base = environment.api_url` = relative `"api/"`
(`scoreboard/src/environments/environment.ts`). All five are **static JSON files**
written by `scoreboard.py`, not Flask routes:

1. `GET api/scoreboard_teams.json` (`backend.service.ts:39`)
2. `GET api/scoreboard_round_{tick}.json` (`:56`)
3. `GET api/scoreboard_team_{team_id}.json` (`:114`)
4. `GET api/scoreboard_service_stats.json` (`:140`)
5. `GET api/scoreboard_current.json` (`:167`, polled every 1–10 s)

It also reads the HTTP `Date` response header of `scoreboard_current.json` for
clock skew (`:179-182`) — whatever serves these files must emit a real `Date`
header (axum/hyper does; a hand-rolled file server must add it).

Plus the dev proxy (`scoreboard/proxy.conf.json`): `/api/` and `/logos/` →
`http://127.0.0.19:8080/` (the gameserver nginx). So in production **no HTTP
backend is involved at all for these five**; if the Rust port replaces nginx too
(ADR-007 multiple binaries), a single axum `Router` with
`get_service(ServeDir::new(scoreboard_output))` at `/api/` reproduces it.

## 5. Semantic traps

- **Trailing slashes.** Flask with `strict_slashes` default 308-redirects
  `/teams` → `/teams/`; axum never redirects — `/teams` and `/teams/` are
  distinct routes (Router docs: matching is exact, fallthrough to 404). Several
  rules exist only in trailing-slash form (`/teams/`, `/checker_results/`,
  `/log_messages/`, `/flags/`, `/patches/`, `/services/`), while others register
  both explicitly (`/packages` + `/packages/`, `pages.py:207-208`;
  `/checker_status` + `/checker_status/`). Port each rule verbatim and add a
  fallback or small redirect middleware to reproduce the 308s — recorded replay
  cases will hit `/checker_results/` with the slash.
- **CSRF: none.** No flask-wtf/secrets on POSTs (`/overview/set_vpn`,
  `/scripts/service_update`, ...). Do not add CSRF middleware in the port; it
  would break the same-origin organizer pages' POSTs.
- **CORS: none.** All clients are same-origin (relative `api/`, proxied).
  `grep -i "Access-Control|cors"` over repo: zero hits. No `CorsLayer` needed.
  Cookie flags likewise moot — no cookies are set anywhere.
- **`request.json`**: handlers index straight into it (`body["command"]`,
  `pages.py:284`) → malformed JSON raises 400/500 from Flask's BadRequest.
  axum's `Json` extractor rejection returns 422/400 by default — acceptable under
  ADR-004 (error text may differ), but note it in the record/normalize step.
- **`jsonify` list responses** (`packages_update` returns a list of strings) and
  bare-string returns (`return "OK"` → `text/html; charset=utf-8` in Flask, status
  200). Use `axum::response::Html`/`String` responses for those, not `Json`.
- **404 page**: HTML template, status 404, used as the "tick not dispatched" case
  (`pages.py:360`). Router `fallback` reproduces the unmatched-path case
  (Router docs); the in-handler 404s are explicit responses.

## 6. Minimal axum shape (for the wave-1 unit)

```rust
let app = Router::<AppState>::new()
    .route("/overview/timing", get(timing))          // pages.py:69
    .route("/overview/set_timing", post(set_timing)) // pages.py:89
    // ... one .route per Flask rule, verbatim paths incl. slashes
    .merge(teams_router()).merge(api_router())       // one per blueprint
    .with_state(state);
axum::serve(listener, app).await?;
```

Middleware only via tower-http `TraceLayer` if wanted; no auth, CORS, or CSRF
layer (sections 1, 5).
