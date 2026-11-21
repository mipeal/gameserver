# Documentation

## Guides

- [simulation.md](simulation.md) — run the whole pipeline locally in Docker:
  stack topology, start/stop/reset, re-deploying rebuilt web apps, gotchas.
- [hosting-baremetal.md](hosting-baremetal.md) — deploy the Rust stack to a
  Debian/Ubuntu host with the Ansible playbook.

## Project records

- [progress.md](progress.md) — delivery ledger for the de-brand + docs
  milestone (acceptance criteria, decisions, per-slice evidence).
- [superpowers/specs/2026-11-20-docker-simulation-deployment-design.md](superpowers/specs/2026-11-20-docker-simulation-deployment-design.md)
  and the matching [plan](superpowers/plans/2026-11-20-docker-simulation-deployment.md) —
  the historical design and implementation plan for the simulation stack.

## Rust port

The port's own documentation lives with the code:

- [rust/MAP.md](../rust/MAP.md) — port map and unit status
- [rust/DECISIONS.md](../rust/DECISIONS.md) — ADRs
- [rust/GLOSSARY.md](../rust/GLOSSARY.md) — terms
- [rust/HANDOFF.md](../rust/HANDOFF.md) — session handoff notes
- [rust/simulation/README.md](../rust/simulation/README.md) — simulation
  internals (image layout, VPN wiring, env vars)
