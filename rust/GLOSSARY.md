# GLOSSARY — gameserver domain terms

Each term: the original's meaning, and its name in Rust. Add entries as units bring new terms.

| Term | Meaning | In Rust (planned) |
| --- | --- | --- |
| tick / round | One scoring period of the CTF; flags are validated per tick. Numbered from 1. | `Tick` (newtype u32/u64) |
| team | A participating team with an id, name, VPN address. | `Team` (saarctf_commons / sqlx row) |
| service | A vulnerable service teams exploit; has id, checker, flags. | `Service` |
| flag | `SAAR{32-websafe-base64-bytes}`; binary layout: 2B tick, 2B team, 2B service, 2B payload, 16B HMAC-SHA256 (little-endian) — see flag-submission-server/README.md. | `Flag` (encode/decode in shared crate) |
| checker | The script that probes/exploits a service each round; run by checker_runner via Celery. | `Checker` / checker result types |
| first blood | First team to capture a flag for a service; shown on scoreboard. | `FirstBlood` |
| scoreboard | Generated ranking JSON per tick, served by its own process (controlserver/scoring/scoreboard.py). | `scoreboard` crate/binary |
| storage flags / captured flags | DB tables distinguishing placed vs submitted-captured flags. | `stored_flags`, `submitted_flags` |
| ENV / environment | Deployment identity (intern/dev/prod) picked from config; drives ng build flags. | `environment` config field |
| config | config.json/config.yaml shared by all services (same formats stay). | `saarctf_commons::config` |
| EXPLOIT/FLAGSUBMIT/CHECKER states | Checker result states in checker_results tables. | enum `CheckerState` |
