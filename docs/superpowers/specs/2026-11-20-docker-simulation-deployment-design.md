# Docker simulation deployment — design

Date: 2026-11-20
Status: draft (awaiting approval)

## Intent and why

Demonstrate the **whole saarCTF gameserver pipeline** — flag submission, checker
workers probing vulnerable services, scoring, scoreboard generation, and the
Leptos scoreboard SPA — running entirely in Docker, with **containerized
vulnboxes standing in for real teams**. Purpose: a self-contained demo you can
bring up on one machine and watch ticks, attack points, defense points and
first-blood notifications appear live on the scoreboard.

This replaces the ad-hoc local simulation (three `http.server` processes on
8123/8124/8125 plus manually started binaries) with a proper one-command
deployment.

## Goals

- `docker compose -f rust/docker-compose.simulation.yml up` brings up the full
  pipeline and it runs unattended: ticks advance, checkers run against the
  vulnbox containers, an attack simulator submits stolen flags, the scoreboard
  moves, first-bloods fire.
- Vulnboxes are containers with static IPs matching the gameserver's own IP
  math (`10.32.<team>.2` from `team_id_to_vulnbox_ip`) — the simulation
  config overrides `network.vulnbox_ip` (the demo config's value computes
  `127.0.N.2`, loopback, unusable for containers), so **zero code change** is
  needed for the gameserver to reach them.
- Gated by an environment variable (`SAARCTF_SIMULATION=1`) so that with it
  unset, controlserver-rs behaves **exactly** as it does today (ADR-004
  semantic equivalence; nothing in the real pipeline may change).
- Configurable scale: `TEAMS` (default 8, the seed fixture) vulnbox containers.
- Switchable attacker activity: `SIM_ATTACK_RATE` env on the simulator sidecar
  (0 = no attacks, only defense/SLA from real checker traffic; default > 0 =
  flag theft + submissions, producing first-bloods and attack points).
- Testable VPN: vulnbox containers connect over **real WireGuard tunnels** to
  the gameserver; `wireguard-sync` syncs peers, `vpnboard` shows live
  handshake status, and with `dispatcher_check_vpn_status: true` a team whose
  tunnel is down has its checkers skipped — the VPN gating pipeline is
  exercised for real. `SIM_VPN=0` falls back to plain simnet networking (no
  privileges needed) with teams online via the seed.

## Non-goals

- Not a production deployment story (no TLS, no multi-host).
- Not porting the flag-submission server to Rust (ADR-001 unchanged — the sim
  reuses the existing `flag-submission-server` image from the Python side).
- The demo fixture **services** are reused as-is (`sample_files/` +
  demo seed: demoservice:1337, cloud-api:8080, flagboard:31337; 8 teams), but
  the demo **checker** scripts cannot be (user-approved exemption, 2026-11-20):
  the sample checkers hardcode `localhost` and cloud-api/flagboard have none,
  so the sim ships small Python checker classes per service that target
  `team.host` (the vulnbox IP). Demo tooling, not fixture changes.

## Anti-goals (failure modes)

- Any change to controlserver-rs/checker_runner/scoreboard behavior when
  `SAARCTF_SIMULATION` is unset — would break the real pipeline.
- Printing or persisting `SECRET_FLAG_KEY` in logs, compose output or fixtures
  (demo config's zero placeholder is fine).
- Simulation logic living in the hot path (timer tick, scoring) beyond a
  checked env flag.

## Constraints

- ADR-013: checker scripts stay Python (the `gameserver-rust` image already
  ships python + gamelib + checker_runner; vulnbox services are plain HTTP/DB
  demo services, language is whatever the fixtures use).
- ADR-010: axum/tokio binaries unchanged except the env-gated bits below.
- ADR-005: sqlx migrations run by controlserver at boot, as today.
- Existing `rust/docker-compose.rust.yml` and `Dockerfile.rust` remain the
  real-deployment files; the simulation is an **additional** compose file.

## Architecture

### Components (all in one new compose file, `rust/docker-compose.simulation.yml`)

1. **Backing stores** — postgres 17 and redis 7.4 (as in the existing compose;
   sim-local volumes so the demo never touches real data).
2. **controlserver** — the existing `gameserver-rust` image, env
   `SAARCTF_SIMULATION=1`, seeded from the demo seed (fixture teams/services).
3. **checker_runner** ×N — same image, spawns the Python checker subprocesses
   each tick, as today.
4. **submitter** — the existing Python flag-submission-server image (ports
   exposed for the simulator and for curl demos).
5. **scoreboard-daemon + nginx** — writes JSON files, serves the Leptos SPA on
   port 8080 (as in the existing compose).
6. **Vulnbox containers** — one per team (count = `TEAMS`), each running the
   fixture service implementations so real checkers can store/retrieve flags.
   A small `Dockerfile.vulnbox` (base `php:apache` plus python) serves
   `sample_files/demoservice.php` and small HTTP/DB service shims for
   cloud-api and flagboard on their seed ports.
7. **Attack simulator sidecar** — a small python container running a tick loop:
   each tick, with probability/rate `SIM_ATTACK_RATE`, it **steals real flags**
   via the vulnbox read path and submits them to the submitter for random
   (victim team, service) pairs. The submitter validates via HMAC with
   `SECRET_FLAG_KEY`, so the simulator needs no key and submitted state is
   honest by construction. Rate 0 disables it entirely (defense-only demo).
8. **wireguard-sync** — generates team key pairs, initializes the gameserver's
   WireGuard interface, runs the peer sync loop (existing `wireguard-sync-rs`
   binary). In `SIM_VPN=0` mode it is omitted.
9. **vpnboard** — the existing `vpnboard-rs` board, served on its own port,
   showing real peer/handshake status. In `SIM_VPN=0` mode it is omitted.

### Seed and checker package (user-approved modified sim seed)

The demo seed ships all services with `checker_enabled = FALSE` and
`checker_script: 'demo'` — checkers would never dispatch. The sim therefore
uses its own seed: `checker_enabled = TRUE`, real
`checker_script: '<file>:<Class>'` rows, `vpn_connected = TRUE` for all
teams, and a checker package present in the DB (package upload endpoint or
seed insert — builder's call).

### Networking

- One bridge network `simnet`, subnet `10.32.0.0/16` (a /15 can be refused by
  Docker; /16 comfortably holds all teams), static container IPs: team N's
  vulnbox gets `10.32.N.2`. No MTU override (1290 in the real compose is
  VPN-specific). controlserver/checker_runner/submitter/simulator join
  `simnet` too.
- **SIM_VPN=1 (default):** the gameserver container runs a WireGuard interface
  (`wg0` = the config's `vpn_host`, e.g. 10.32.250.1) with `CAP_NET_ADMIN` +
  `/dev/net/tun`; each vulnbox container (same capabilities) brings up its own
  `wg0` using its team key pair from the keystore, tunnel address =
  `vpn_peer_ip(N)`, peer endpoint = the gameserver's exposed wg port on
  simnet, and routes `vulnbox_ip` (10.32.N.2) through the tunnel. The checker
  then talks to a vulnbox only when its tunnel is up. Sim config sets
  `dispatcher_check_vpn_status: true` so the gating pipeline is exercised.
  Kicking a tunnel down (docker exec wg show / interface down) must visibly
  take that team offline for checkers and vpnboard.
- **SIM_VPN=0:** no wg anywhere; teams online via seed; everything else
  unchanged.
- The config for the sim (`config.sim.yaml`) mirrors
  `/tmp/saarctf-demo/config.yaml` but: points all host names at compose
  service names, overrides `network.vulnbox_ip` (plus
  gameserver_ip/gateway/testbox for consistency) so
  `team_id_to_vulnbox_ip(N) = 10.32.N.2`, and sets
  `dispatcher_check_vpn_status` per SIM_VPN mode.

### Env-gated code changes (controlserver-rs, minimal)

- `SAARCTF_SIMULATION=1` only: auto-start the game timer on boot when no game
  is running (no manual admin call needed for the demo). Team-online gating
  needs no code — `dispatcher_check_vpn_status: false` already selects all
  teams, and the seed sets `vpn_connected = TRUE` for the cosmetic consumers.
- Everything else (dispatch, scoring, flag storage, events) runs the real
  code paths — that is the point of the demo.
- With the var unset or `0`: no behavior change whatsoever (single check at
  startup, stored in app state).

## What's left to the builder

- Vulnbox service shims: which fixture implementation backs cloud-api /
  flagboard and what the new sim checkers request from them (checker and shim
  are written together, protocol is the builder's).
- Package upload mechanics: `/packages` admin endpoint vs direct seed insert
  for getting the checker package into the DB.
- TEAMS compose plumbing (anchors vs tiny generator; prefer plain compose).
- Exact sim-config values (ports, paths, ignored rabbitmq section,
  scoreboard_url); whether team 1 (NOP) is attacked.
- How `wireguard-sync-rs` and `vpnboard-rs` binaries get into the sim image
  (extend `Dockerfile.rust`'s build stage vs a sim-specific Dockerfile —
  builder's call; the real image must not change behavior).
- wg key/keystore plumbing for vulnbox containers (keystore volume shared with
  wireguard-sync vs pregenerated key files).

## Deliverables

- `rust/docker-compose.simulation.yml` (+ `Dockerfile.vulnbox`, sim config,
  simulation seed, sim checker scripts + checker package, simulator sidecar
  source, demo env file), with wireguard-sync + vpnboard wired in for
  `SIM_VPN=1`.
- Env-gated startup logic in controlserver-rs (one flag, two behaviors).
- `rust/simulation/README.md`: bring-up, env vars, what to watch (scoreboard
  on :8080, first-blood overlay+sound in the SPA), teardown.
- Verification: sim up from clean state → ≥3 ticks with checker results, ≥1
  first-blood event with `SIM_ATTACK_RATE > 0`, scoreboard JSON regenerated
  per tick, SPA renders it; VPN: wg handshakes visible on vpnboard, taking a
  vulnbox tunnel down stops that team's checker results and the board shows
  it, bringing it back up restores them; and with `SAARCTF_SIMULATION` unset
  the real compose path is byte-for-byte unaffected (no diff in default
  env/config files, no new default behavior).
