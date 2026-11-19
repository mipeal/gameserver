# MAP — gameserver → Rust whole-app port

Goal: standalone Rust gameserver replacing all Python services and the Angular scoreboard SPA;
C++ flag-submission-server stays (ADR-001). Original kept as reference until the user cuts over.

Files: decisions in [DECISIONS.md](DECISIONS.md), terms in [GLOSSARY.md](GLOSSARY.md),
session handoff in [HANDOFF.md](HANDOFF.md), dependency research in [research/](research/).

## ADR index
ADR-001 scope (all but C++ fss) · ADR-002 Leptos GUI · ADR-003 rusty-celery (SUPERSEDED by ADR-012) ·
ADR-004 semantic equivalence · ADR-005 sqlx baseline migration · ADR-006 staged cutover ·
ADR-007 multiple binaries · ADR-008 checkpoints per wave · ADR-009 tangled core one piece ·
ADR-010 axum + tokio · ADR-011 no sessions/no tornado corrections · ADR-012 no Celery in Rust,
atomic pipeline cutover (checker pipeline switches together with controlserver)

## Units

State: `open` | `claimed by <session> <date>` | `done (<gate>, <replay>)`

| Wave | Unit | State | Notes |
| ---: | --- | --- | --- |
| 0 | `gamelib` subset → `gamelib-rs` crate | done (gate, 878/878) | ADR-013. Flag encode/regex pattern, flag_ids + usernames wordlist, CPython-exact RNG. get_flag_regex/assert_*/OS-random generators stay Python (js_only). Hand patches listed in gamelib-rs/README.md. |
| 0 | `saarctf_commons/` (746 ln) | done (cargo test, 4 goldens green) | ADR-014: plain Rust crate, golden-file equivalence (Config to_dict + env overrides, network math incl. negative ids, Influx lines byte-exact). Redis/retry ported directly, verified at wave 1. `logging_utils.py` still Python-side only (Rust services use `tracing` at wave 1). |
| 0 | `migrations/versions/` (974 ln) | open | Superseded by ADR-005: becomes the sqlx baseline, not a ported unit. |
| 0 | `controlserver/utils/` (28 ln) | open | Blocked on eval — becomes explicit registry inside tangled core. |
| 0 | `sample_files/checker_ok/`, `checker_fnf/` | open | Demo checkers. |
| 1 | tangled core: `checker_runner/` + `runners/`, `controlserver/` + `endpoints/` + `scoring/` + `scoring/algorithms/` (~6,530 ln) | open | ADR-009. Flask→axum, eval→registry. Biggest chunk. |
| 1 | `wireguard-sync/wireguard_sync/` (505 ln) | open | Async-heavy. |
| 1 | `ci/` (248 ln) | open | Test harness CLI. |
| 1 | `flag-submission-server/scripts/` (85 ln) | open | generate_flag etc. C++ binary itself stays (ADR-001). |
| 1 | `sample_files/checker_demoservice/` (132 ln) | open | |
| 2 | `migrations/` (71 ln) | open | env.py — folds into ADR-005 tooling. |
| 2 | `sample_files/` (240 ln) | open | demo exploits. |
| 2 | `scripts/` (1,225 ln) | open | Ops CLIs. |
| 2 | `vpn/` (1,663 ln) | open | No pyroute2 here (research finding): iptables via subprocess, eBPF maps via ctypes — port as-is. hcloud API only. |
| 2 | `vpnboard/` (535 ln) | open | Status daemon + board. |
| GUI | `scoreboard/src/app` (Angular) → Leptos SPA | open | ADR-002. After backend APIs are proven. |

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
