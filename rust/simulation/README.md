# saarCTF simulation deployment

A Docker-only deployment of the whole saarCTF pipeline (controlserver,
checker runners, flag submitter, scoreboard, and per-team vulnbox fixture
containers) for demoing and testing the Rust gameserver end to end. Gated by
`SAARCTF_SIMULATION=1`: with the env unset, the real pipeline is byte-identical
(ADR-006) — no sim code runs.

## Layout

| path | what |
|---|---|
| `Dockerfile.sim` | `gameserver-rust` + wg tooling + `wireguard-sync`/`vpnboard` binaries |
| `config.sim.yaml` | sim config: compose hostnames, 10.32 team math, `dispatcher_check_vpn_status: true`, tick 30s |
| `seed.sql` | 8 fixture teams (idempotent), applied by the boot gate |
| `services/` | checker sources for the 3 fixture services (demoservice :1337, cloud-api :8080, flagboard :31337); imported + enabled by the boot gate |
| `vulnbox/` | vulnbox image: HTTP shims that store/retrieve flags per (service, tick) + a per-team attack loop (`attack.py`) |
| `wg/` | WireGuard plumbing for SIM_VPN=1 (see `wg/README.md`) |
| `gen_compose.py` | emits `docker-compose.vulnboxes.yml` (+ `docker-compose.sim-vpn.yml` when SIM_VPN=1) from `.env.sim` |
| `simctl` | `up` / `down` / `demo-commands` |

## Prerequisites

- Docker with compose v2. **SIM_VPN=1** additionally needs `/dev/net/tun` and
  container privileges (`NET_ADMIN`) — on macOS that means Docker Desktop with
  the default VM; on rootless/podman setups use `SIM_VPN=0`, which needs no
  special privileges at all.
- ~2 GB free RAM and the repo root as build context (both images build from
  the repo root via `simctl up`).
- Host port 8080 (scoreboard), 1337 (flag submitter), 15000 (controlserver
  admin API) must be free. 15000 instead of 5000 avoids the macOS AirPlay
  listener on 5000.

## Ports (host)

| port | service |
|---|---|
| 8080 | scoreboard (SPA + JSON API) |
| 1337 | flag submitter (vulnbox attack loops target `submitter:31337` internally) |
| 15000 | controlserver: admin GUI (management UI) + admin API (no auth, ADR-011) |

## Bring-up

```sh
cd rust/simulation
./simctl up          # creates .env.sim from .env.sim.example on first run
./simctl demo-commands
```

`simctl up` builds `gameserver-sim` and `sim-vulnbox`, starts postgres/redis,
the controlserver (which runs migrations → sim seed → `import_services` →
enable checkers → auto-start the CTF), 4 checker runners, the Python flag
submitter (:1337), the scoreboard daemon + nginx (:8080), and one vulnbox per
team at `10.32.<team>.2`.

First checker results appear within ~1 tick (30s). First-blood overlay and
attacker points show on the scoreboard at :8080. Each vulnbox (except NOP
team 1) runs its own attack loop: it steals checker-stored flags from every
opponent vulnbox and submits them from its own team IP, so the submitter
attributes flags to the right attacker — the real CTF attack model.

### What to watch

- `curl -s localhost:8080/api/scoreboard_round_<N>.json` — scores rise each tick once
  checkers report SUCCESS.
- `curl -s localhost:15000/api/logs | tail` — "New tick: N", dispatcher
  warnings, first-blood events.
- With `SIM_ATTACK_RATE > 0`: the scoreboard's attacker column and
  first-blood overlay (also audible in the SPA) fill in within a few ticks;
  `scoreboard_round_*.json` files appear under the `scoreboard` volume.
- Vulnboxes hold flags **in memory**: restarting a vulnbox drops its stored
  flags, so the next `retrieve_flags` reports `FlagMissing` until the next
  `store_flags` tick repopulates it. Expected, not a bug.

## Teardown

```sh
./simctl down          # stops and removes all sim containers + volumes
```

## Troubleshooting

| symptom | cause / fix |
|---|---|
| No `wg show` handshakes (SIM_VPN=1) | `/dev/net/tun` missing or NET_ADMIN dropped — check `docker logs saarctf-sim-controlserver-1` for the wg setup; fall back to SIM_VPN=0 |
| Vulnboxes crash-looping with `RTNETLINK answers: File exists` right after bring-up | the wg route (10.32.0.0/16) and the bridge subnet collided — regenerate compose (`simctl up`); with SIM_VPN=1 the bridge is 10.33.0.0/16 |
| Vulnboxes crash-looping with `Name or service not known: controlserver:51820` | startup race: wg-quick resolved the Endpoint before the controlserver joined the network — the restart policy self-heals within a few seconds |
| Team goes OFFLINE after `wg-quick down wg0` on a vulnbox | expected (that is the demo); vpnboard marks it down after two missed ~120 s handshakes, so allow up to 5 min on the scoreboard. Reconnect with `wg-quick up wg0` — recovery is equally slow. |
| Checker results stuck at CRASHED | read `output` of the newest `checker_results` row; usually a missing Python dep in the base image (see `rust/controlserver-rs/Dockerfile.rust`) |
| Submitter restart-looping | it reads `SAARCTF_CONFIG` (C++ convention) — must point at the mounted `config.sim.yaml` |
| Stale behavior after editing controlserver-rs sources | both images must be rebuilt — `gameserver-sim` reuses the cached `gameserver-rust` base and does not rebuild it |

## What is simulated vs real

- **Real pipeline:** tick timer, checker subprocess protocol
  (`checker_runner/runners/saarctf.py`), flag generation (HMAC via
  `SECRET_FLAG_KEY`), checker-result Redis Streams, dispatcher, scoreboard
  scoring, flag submission + validation.
- **Simulated:** the vulnbox services (fixture HTTP shims), the attack side
  (per-team loops stealing what checkers stored), teams (8 fixture rows).

## Environment (`.env.sim`)

| var | default | meaning |
|---|---|---|
| `TEAMS` | 8 | number of teams (vulnboxes); team 1 = NOP, never attacked |
| `SIM_ATTACK_RATE` | 0.25 | 0.0–1.0: share of (team, service) pairs attacked per tick; `0` = defense-only |
| `SIM_VPN` | 1 | `1` = real WireGuard tunnels (see `wg/README.md`); `0` = plain bridge, no privileges |
| `SECRET_FLAG_KEY` | zero placeholder | HMAC flag-signing key — must match between controlserver and submitter; the demo placeholder is public by design |

## VPN mode (SIM_VPN=1)

Vulnbox containers get a real wg tunnel (`10.32.<team>.2` is the tunnel
address); checker traffic to `team.ip` genuinely flows through it, so
`wg-quick down wg0` on a vulnbox makes that team offline and the scoreboard
shows the damage. The compose bridge network moves to `10.33.0.0/16` — the
tunnel owns `10.32.0.0/16`. Server side (`wg0` at 10.32.250.1, port 51820) is
set up by `wg/server-setup.sh` inside the controlserver container (NET_ADMIN);
keys are pregenerated by the `sim-init` one-shot (`wireguard-sync generate`,
split by `wg/split-keystore.py`). Details and verification steps:
`wg/README.md`.

SIM_VPN=0 needs no NET_ADMIN anywhere: vulnboxes are plain bridge containers
at their static IPs.
