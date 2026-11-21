# MAP — gameserver → Rust whole-app port

Goal: standalone Rust gameserver replacing all Python services and the Angular scoreboard SPA;
C++ flag-submission-server stays (ADR-001). Original kept as reference until the user cuts over.

**2026-11-20 post-cutover cleanup:** the Python sources of all `done` units were removed from
the repo (`vpn/`, `vpnboard/`, `wireguard-sync/`, `scoreboard/` Angular, `migrations/`,
`Dockerfile`, `docker-compose.yml`, `tests/test_vpnboard.py`) — recoverable from git history.
`checker-exec` (ADR-017) later replaced the wrapper and both Python trees were deleted.

Files: decisions in [DECISIONS.md](DECISIONS.md), terms in [GLOSSARY.md](GLOSSARY.md),
session handoff in [HANDOFF.md](HANDOFF.md), dependency research in [research/](research/).

## ADR index
ADR-001 scope (all but C++ fss) · ADR-002 Leptos GUI · ADR-003 rusty-celery (SUPERSEDED by ADR-012) ·
ADR-004 semantic equivalence · ADR-005 sqlx baseline migration · ADR-006 staged cutover ·
ADR-007 multiple binaries · ADR-008 checkpoints per wave · ADR-009 tangled core one piece ·
ADR-010 axum + tokio · ADR-011 no sessions/no tornado corrections · ADR-012 no Celery in Rust,
atomic pipeline cutover (checker pipeline switches together with controlserver) ·
ADR-016 rust-anything front-door/app_lift superseded: multi-binary port (ADR-007) with
standalone ports per unit and an original that cannot run for recording on this machine —
per-unit record/diff/gate still applied where the original is runnable (gamelib, commons).

## Units

State: `open` | `claimed by <session> <date>` | `done (<gate>, <replay>)`

| Wave | Unit | State | Notes |
| ---: | --- | --- | --- |
| 0 | `gamelib` subset → `gamelib-rs` crate | done (gate, 878/878) | ADR-013. Flag encode/regex pattern, flag_ids + usernames wordlist, CPython-exact RNG. get_flag_regex/assert_*/OS-random generators stay Python (js_only). Hand patches listed in gamelib-rs/README.md. |
| 0 | `saarctf_commons/` (746 ln) | done (cargo test, 4 goldens green) | ADR-014: plain Rust crate, golden-file equivalence (Config to_dict + env overrides, network math incl. negative ids, Influx lines byte-exact). Redis/retry ported directly, verified at wave 1. `logging_utils.py` still Python-side only (Rust services use `tracing` at wave 1). |
| 0 | `migrations/versions/` (974 ln) | closed | Superseded by ADR-005: the sqlx baseline lives in controlserver-rs/migrations. |
| 0 | `controlserver/utils/` (28 ln) | done (wave 1) | eval registry folded into controlserver-rs (ADR-009). |
| 0 | `sample_files/checker_ok/`, `checker_fnf/` | stays Python | Demo checkers, ADR-013. |
| 1 | tangled core: `checker_runner/` + `runners/`, `controlserver/` + `endpoints/` + `scoring/` + `scoring/algorithms/` (~6,530 ln) | done (waves 1–5, clippy clean) | ADR-009. controlserver-rs (axum API + timer + scoring + scoreboard JSON) + checker_runner over Redis Streams (ADR-012/015). |
| 1 | `wireguard-sync/wireguard_sync/` (505 ln) | done (`wireguard-sync-rs`, clippy clean) | |
| 1 | `ci/` (248 ln) | stays Python | Test harness CLI, ADR-013. |
| 1 | `flag-submission-server/scripts/` (85 ln) | stays Python | ADR-013; C++ binary stays (ADR-001). |
| 1 | `sample_files/checker_demoservice/` (132 ln) | stays Python | ADR-013. |
| 2 | `migrations/` (71 ln) | closed | env.py — superseded by ADR-005. |
| 2 | `sample_files/` (240 ln) | stays Python | demo exploits, ADR-013. |
| 2 | `scripts/` (1,225 ln) | done (`ctf-scripts-rs`, 30 bins, clippy clean) | Ops CLIs incl. scoreboard exports (wave 3). |
| 2 | `vpn/` (1,663 ln) | done (wave 2) | iptables via subprocess, eBPF maps via libc, hcloud API only. Bins + daemons in ctf-scripts-rs. |
| 2 | `vpnboard/` (535 ln) | done (`vpnboard-rs`, clippy clean) | Status daemon + board. |
| 5 | `controlserver` admin HTML → Leptos SPA `rust/admin-gui-rs` | done (wave 5) | 15 JSON admin endpoints + SPA, served by controlserver-rs. |
| 6 | `scoreboard/src/app` (Angular) → Leptos SPA `rust/scoreboard-rs` | done (wasm build clean, trunk dist) | ADR-002. rust-anything front-door/app_lift superseded — see ADR-016. Cutover tooling: rust/docker-compose.rust.yml + rust/cutover.sh (multi-arch buildx, amd64+arm64). |
| 7 | `checker_runner/` bootstrap + `controlserver/` Python → `checker-exec` (PyO3 embed) | done (gate, 10/10 recorded cases match) | ADR-017. Rust launcher + embedded Python shim; service config via argv; packages from disk. controlserver/ deleted (less → rust/controlserver-rs/less), checker_runner keeps demo_checker.py. |

## Out of scope
- `flag-submission-server/src` (C++) — stays as-is (ADR-001).
- gamelib repo itself (service SDK) — stays Python; only its gameserver-used subset becomes `gamelib-rs` (ADR-013).
- Angular tooling (package.json, karma) — deleted at GUI cutover.

## Open questions
- Redis JSON key/value formats: inventoried in research/redis.md (spaced json.dumps output, "None" literal, empty-string values) — treat as contract during wave 0/1.
- Flask session/login: resolved (ADR-011) — none to port.
- Leptos charting: resolved (research/leptos.md) — Leptos 0.8 + Trunk CSR, charts via a thin hand-written chart.js wasm-bindgen shim (leptos_chart is stale); timezone formatting needs budgeting at GUI wave.

## Research
- [research/celery.md](research/celery.md) — rusty-celery ↔ Python interop (gates wave 1)
- [research/redis.md](research/redis.md) — redis-rs patterns, formats (gates wave 0/1)
- [research/sqlx.md](research/sqlx.md) — Postgres, baseline migration (gates wave 0)
- [research/axum.md](research/axum.md) — routes, sessions (gates wave 1)
- [research/leptos.md](research/leptos.md) — SPA + admin (gates GUI wave)
- [research/netlink.md](research/netlink.md) — rtnetlink for vpn/ (gates wave 2)
