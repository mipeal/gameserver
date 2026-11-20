# Docker Simulation Deployment — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-command Docker demo of the full gameserver pipeline — real checkers probing containerized vulnboxes over real WireGuard tunnels, an attack simulator producing first-bloods, scoreboard SPA live — gated by `SAARCTF_SIMULATION=1` so the real pipeline is untouched.

**Architecture:** A `rust/simulation/` tree (fixtures, checker sources, vulnbox, simulator, compose generator) plus a minimal env-gated boot hook in controlserver-rs (seed → import services → enable checkers → start timer). Vulnbox containers get static IPs `10.32.N.2` on a `/16` bridge matching the sim config's `vulnbox_ip` override; WireGuard runs for real when `SIM_VPN=1`.

**Tech Stack:** docker compose, existing `gameserver-rust` image (python3.13 + gamelib + checker_runner + rust bins), Python http.server-based vulnbox shims, Python attack simulator, wireguard-tools.

**Spec:** `docs/superpowers/specs/2026-11-20-docker-simulation-deployment-design.md` (approved, with the user's three amendments: modified sim seed, new sim checker scripts, 10.32.N.2 on a /16; plus real WireGuard tunnels with `SIM_VPN=0` fallback).

## Global Constraints

- Never print or persist `SECRET_FLAG_KEY` / `cfg.secret_flag_key` in logs, compose output, or fixtures. The demo config's zero placeholder for `secret_flags` is safe.
- `SAARCTF_SIMULATION` unset or `0` ⇒ controlserver-rs behavior byte-identical to today (single check at startup, stored in state).
- Sim-only files live under `rust/simulation/`; Rust changes touch only `rust/controlserver-rs/src/bin/controlserver.rs` (+ a small new module) — no dispatcher/scoring/worker changes.
- ADR-013 (checkers stay Python), ADR-001 (submitter stays the existing Python image), ADR-011 (admin API unauthenticated), ADR-004 (semantic equivalence).
- Real-deployment files `rust/docker-compose.rust.yml` and `rust/controlserver-rs/Dockerfile.rust` are not modified.
- Demo tick: `tick_duration_default: 30` in the sim config (demo-friendly; config value, not code).
- Team 1 is NOP (`nop_team_id: 1`) — the simulator does not attack team 1.

## Review Focus

1. **Checker target host:** checker scripts must connect to `team.ip` (gamelib `Team.ip` = vulnbox IP), never `localhost`. Test: Task 2 Step 4 runs a checker inside the gameserver-rust image against a vulnbox at `10.32.2.2` and asserts `SUCCESS`.
2. **`SAARCTF_SIMULATION` unset = zero change:** Test: Task 3 Step 2's unit test asserts gating off ⇒ no seed/import/start calls; Task 7 Step 4 diff-checks default compose behavior.
3. **Vulnbox IP math:** sim config must yield `team_id_to_vulnbox_ip(2) == "10.32.2.2"` (the demo config yields `127.0.2.2` — do not copy it). Test: Task 3 Step 1.
4. **Attack rate 0 = no submissions:** Test: Task 4 Step 1's unit test asserts zero submissions at rate 0.
5. **VPN gating honesty:** a team whose tunnel is down must produce no checker results and show offline on vpnboard, and recover when the tunnel returns. Test: Task 7 Step 6 (live down/up cycle).

---

### Task 1: Vulnbox image + service shims

**Files:**
- Create: `rust/simulation/vulnbox/Dockerfile.vulnbox`
- Create: `rust/simulation/vulnbox/services.py`
- Create: `rust/simulation/vulnbox/entrypoint.sh`

**Interfaces:**
- Consumes: nothing (standalone container).
- Produces: one container per team serving all 3 services — demoservice on **1337**, cloud-api on **8080**, flagboard on **31337**, all HTTP with the same protocol: `POST /` body form-encoded `team_id, service_id, tick, flag` (multi-line) ⇒ 200 stores; `GET /?team_id=&service_id=&tick=` ⇒ 200, response text contains the stored flags; `GET /` (no params) ⇒ 200 (integrity probe). Flags are keyed `(service_id, tick)` in memory. In `SIM_VPN=1` mode the entrypoint also brings up `wg0` from `/run/sim/keys/team<id>.json` (Task 6). Simulator "steals" via the same `GET`.

- [ ] **Step 1: Write `services.py`** — stdlib-only (`http.server.ThreadingHTTPServer`, one server thread per port; `json` module). Handlers per the protocol above. No persistence: vulnbox restart loses flags, acceptable for a demo (document in README, Task 7).
- [ ] **Step 2: Write `Dockerfile.vulnbox`** — `FROM php:8.3-cli` (base is small and ubiquitous), `RUN apt-get update && apt-get install -y --no-install-recommends python3 wireguard-tools iproute2` (wg tools only needed in SIM_VPN=1; harmless otherwise), COPY `services.py` + `entrypoint.sh`, `ENTRYPOINT ["./entrypoint.sh"]`.
- [ ] **Step 3: Write `entrypoint.sh`** — if `/run/sim/keys/team$TEAM_ID.json` exists, bring up wg (Task 6 defines the JSON shape and the wg0 setup); always start `python3 services.py`.
- [ ] **Step 4: Verify locally** —
  Run: `docker build -f rust/simulation/vulnbox/Dockerfile.vulnbox -t sim-vulnbox rust/simulation/vulnbox && docker run -d --rm --name sim-vulnbox-test -p 1337:1337 sim-vulnbox && curl -sf -d 'team_id=2&service_id=1&tick=5&flag=SAAR%3Ddemo' http://localhost:1337/ && curl -sf 'http://localhost:1337/?team_id=2&service_id=1&tick=5' | grep -q 'SAAR=demo' && docker rm -f sim-vulnbox-test`
  Expected: command exits 0.
- [ ] **Step 5: Commit** — `git add rust/simulation/vulnbox && git commit -m "simulation: vulnbox image with HTTP service shims"`

### Task 2: Sim checker sources (one per service)

**Files:**
- Create: `rust/simulation/services/demoservice/checkers/config.toml`
- Create: `rust/simulation/services/demoservice/checkers/checker.py`
- Create: `rust/simulation/services/cloud-api/checkers/config.toml`
- Create: `rust/simulation/services/cloud-api/checkers/checker.py`
- Create: `rust/simulation/services/flagboard/checkers/config.toml`
- Create: `rust/simulation/services/flagboard/checkers/checker.py`

**Interfaces:**
- Consumes: Python `gamelib.ServiceInterface` (in the gameserver-rust image): subclass with `check_integrity(team, tick)`, `store_flags(team, tick)`, `retrieve_flags(team, tick)`; `self.get_flag(team, tick, i)`, `self.search_flags(text)`, `self.check_flag(flag, team.id, tick)`; `team.ip`, `team.id`; exceptions `OfflineException`/`MumbleException`/`FlagMissingException`.
- Produces: TOML fields exactly as `ServiceConfig::from_file` reads them (see `rust/gamelib-rs/src/service_config.rs`): `name`, `interface_file` = `"checker"`, `interface_class` = `"<Name>Service"`, `ports` = `["1337"]` / `["8080"]` / `["31337"]`, `flags_per_tick` = 4 / 4 / 2, `num_payloads` = 2 / 2 / 1. `import_services` will register rows with `checker_script = "checker:<Name>Service"`.

- [ ] **Step 1: Write `demoservice` config.toml** — fields above; name `demoservice`.
- [ ] **Step 2: Write `demoservice/checker.py`** — class `DemoServiceService(ServiceInterface)`: `check_integrity` GETs `http://{team.ip}:1337/` (ConnectionError ⇒ `OfflineException`, non-200 ⇒ `MumbleException`); `store_flags` POSTs 3 flags (form fields `team_id, service_id, tick, flag`) via `self.get_flag(team, tick, i)`, returns 3; `retrieve_flags` GETs the same, `search_flags` + `check_flag` each (invalid/missing ⇒ `FlagMissingException`), returns 3. This is the upstream demo checker shape with `URL` replaced by `team.ip` — the whole point of the sim checkers.
- [ ] **Step 3: Write `cloud-api` and `flagboard`** — same class shape on 8080 / 31337 with class names `CloudApiService` / `FlagboardService`; flagboard with `flags_per_tick` 2 / 1 payload (fewer flags in store/retrieve).
- [ ] **Step 4: Verify a checker end-to-end in the real image** —
  Run:
  ```bash
  docker network create --subnet=10.32.0.0/16 simtest && \
  docker run -d --name vtest --network simtest --ip 10.32.2.2 sim-vulnbox && \
  docker run --rm --network simtest -v "$PWD/rust/simulation/services/demoservice/checkers:/work" \
    -w /opt -e SAARCTF_NO_RLIMIT=1 ${IMAGE_REGISTRY:-gameserver-rust} \
    python3 checker_runner/runners/saarctf.py "" checker:DemoServiceService 1 2 5; \
  docker rm -f vtest; docker network rm simtest
  ```
  (If the image tag differs locally, use the tag from `docker images | grep gameserver-rust`.) Expected: output contains `SUCCESS` (check_integrity + empty store/retrieve pass). If the runner expects a non-empty package, pass `test` and note it — package identity is set by Task 3's import, not here.
- [ ] **Step 5: Commit** — `git add rust/simulation/services && git commit -m "simulation: sim checker sources for all 3 fixture services"`

### Task 3: Env-gated sim boot in controlserver-rs + sim config + seed

**Files:**
- Modify: `rust/controlserver-rs/src/bin/controlserver.rs`
- Create: `rust/controlserver-rs/src/sim_boot.rs`
- Test: `rust/controlserver-rs/tests/sim_boot.rs`
- Create: `rust/simulation/config.sim.yaml`
- Create: `rust/simulation/seed.sql`

**Interfaces:**
- Consumes: `Config` (saarctf-commons), `ServiceRepoManager::import_services(&mut self, cfg: &Config, db: &PgPool) -> Result<(), String>` (`rust/controlserver-rs/src/service_mgr.rs:226`), `timer.start_ctf()` (`rust/controlserver-rs/src/timer.rs:312`), migrations already applied before this point in `bin/controlserver.rs`.
- Produces: `sim_boot::simulation_enabled_with(value: Option<&str>) -> bool` (injectable core; env `SAARCTF_SIMULATION` == "1" ⇒ true, anything else false), `sim_boot::simulation_enabled() -> bool` (reads the real env), `sim_boot::run(cfg, db, seed_path: Option<&str>) -> Result<(), String>` — executes `seed.sql` if the path is set and the file exists, then `import_services`, then `UPDATE services SET checker_enabled = TRUE`, then idempotency: re-running must be a no-op (import skips existing names; UPDATE is idempotent; seed uses `ON CONFLICT (id) DO NOTHING`).

- [ ] **Step 1: Write the failing test** — `tests/sim_boot.rs`:
  ```rust
  #[test]
  fn gating_off_by_default() {
      assert!(!sim_boot::simulation_enabled_with(Some("")));
      assert!(!sim_boot::simulation_enabled_with(None));
  }
  #[test]
  fn gating_on() { assert!(sim_boot::simulation_enabled_with(Some("1"))); }
  ```
  (`simulation_enabled_with(value: Option<&str>)` is the injectable core; `simulation_enabled()` reads the real env.)
  Also a config test (same file or `tests/sim_config.rs`): load `rust/simulation/config.sim.yaml` via the same `Config` loader used at boot (`saarctf_commons_rs::config`, `CONFIG_FILE` env) and assert `cfg.network.team_id_to_vulnbox_ip(2) == "10.32.2.2"`, `cfg.dispatcher_check_vpn_status == true`, `cfg.services_path == "/opt/services"`, `cfg.tick_duration_default == 30`. Run it with `CONFIG_FILE` pointed at the sim config; needs no DB.
- [ ] **Step 2: Run tests to verify they fail** — `cargo test -p controlserver-rs --test sim_boot --test sim_config`. Expected: compile failure (module missing).
- [ ] **Step 3: Implement `sim_boot.rs`** — the two functions above; `run()` order: seed (only if path set), import, enable. In `bin/controlserver.rs`, after migrations and before/after timer spawn: if `sim_boot::simulation_enabled()`, call `sim_boot::run(cfg, db, env SAARCTF_SIM_SEED)`, log a `tracing::info!("simulation mode: boot steps applied")`, and after `spawn_timer` call `start_ctf()` if state is not already Running. No other file changes.
- [ ] **Step 4: Write `config.sim.yaml`** — copy of `/tmp/saarctf-demo/config.yaml` with: `dispatcher_check_vpn_status: true`; `services_path: /opt/services`; `checker_packages_path: /tmp/packages`; `scoreboard_path: /var/scoreboard`; `vpnboard_path: /tmp/vpnboard`; database/redis hosts = compose service names (`postgres`, `redis:6379`); `network.vulnbox_ip` = `[10, 32, [1, 256, 0], 2]` and `gameserver_ip`/`gateway_ip`/`testbox_ip`/`vpn_peer_ips` overridden to the same 10.32-math shape (use `[10, 32, [1, 256, 0], 1]` for gateway etc.); `vpn_host: 10.32.250.1` kept; `secret_flag_key: "0"`-style placeholder per the demo config (never a real key). No `rabbitmq` dependence (Rust stack uses Redis Streams, ADR-012).
- [ ] **Step 5: Write `seed.sql`** — the 8 fixture teams from the demo seed, one statement per concern: `INSERT INTO teams (id, name, affiliation, website, vpn_connected) VALUES ... ON CONFLICT (id) DO NOTHING;` with `vpn_connected = TRUE` (cosmetic consumers; dispatcher uses real wg state in SIM_VPN=1).
- [ ] **Step 6: Run tests to verify they pass** — `cargo test -p controlserver-rs --test sim_boot --test sim_config`. Expected: PASS.
- [ ] **Step 7: Build gate** — `cargo clippy -p controlserver-rs --all-targets -- -D warnings && cargo build -p controlserver-rs --bins`. Expected: clean.
- [ ] **Step 8: Commit** — `git add rust/controlserver-rs/src/bin/controlserver.rs rust/controlserver-rs/src/sim_boot.rs rust/controlserver-rs/tests/ rust/simulation/config.sim.yaml rust/simulation/seed.sql && git commit -m "simulation: SAARCTF_SIMULATION boot gate (seed, import, enable, auto-start) + sim config/seed"`

### Task 4: Attack simulator sidecar

**Files:**
- Create: `rust/simulation/attack_simulator/sim.py`
- Create: `rust/simulation/attack_simulator/Dockerfile`
- Test: `rust/simulation/attack_simulator/test_sim.py`

**Interfaces:**
- Consumes: vulnbox `GET /?team_id&service_id&tick` (Task 1) to steal; flag submission protocol — reuse the exact wire format from `sample_files/submit-random-flags.py` against the submitter at `submitter:31337` (raw TCP; read that script before writing).
- Produces: env `SIM_ATTACK_RATE` (float 0.0–1.0, default 0.25; 0 = disabled), `TEAMS` (default 8), `TICK_SECONDS` (default 30), `SUBMITTER_ADDR` (default `submitter:31337`), `VULNBOX_IP_PREFIX` (default `10.32`), `NOP_TEAM_ID` (default 1). Loop: sleep a tick, for each service and a rate-proportioned random subset of victim teams (never NOP), steal stored flags and submit them to the submitter, logging `stole N flags for team T service S`.

- [ ] **Step 1: Write the failing test** — `test_sim.py` (stdlib `unittest`): spin up two mock servers (a vulnbox HTTP mock returning flags, a submitter TCP mock capturing data); run one `Simulator(...).tick()` iteration with rate 1.0 ⇒ assert ≥1 captured submission whose flag text matches what the vulnbox mock served; run with rate 0.0 ⇒ assert 0 captured submissions and no NOP-team victim ever chosen across 50 iterations at rate 1.0 (team 1 excluded).
- [ ] **Step 2: Run test to verify it fails** — `python3 rust/simulation/attack_simulator/test_sim.py`. Expected: FAIL (ImportError: `sim`).
- [ ] **Step 3: Implement `sim.py`** — stdlib only (`http.client`, `socket`, `random`, `time`, `os`). Structure: `class Simulator: __init__(self, teams, rate, tick_seconds, submitter, vulnbox_prefix, nop_team_id)`, `.tick() -> int` (returns stolen-and-submitted count), `main()` loop. Flag validity: stolen flags are real signed flags (checker-stored via `get_flag`), so the submitter accepts them; do NOT fabricate flags.
- [ ] **Step 4: Run test to verify it passes** — expected PASS.
- [ ] **Step 5: Write `Dockerfile`** — `FROM python:3.13-alpine`, COPY `sim.py`, `CMD ["python3", "-u", "sim.py"]`.
- [ ] **Step 6: Commit** — `git add rust/simulation/attack_simulator && git commit -m "simulation: attack simulator sidecar (rate-controlled flag theft + submission)"`

### Task 5: WireGuard plumbing (keystore, vulnbox client, server side)

**Files:**
- Create: `rust/simulation/wg/entrypoint.sh` (vulnbox wg client bring-up, called from Task 1's `entrypoint.sh` when keys exist)
- Create: `rust/simulation/wg/README.md` (one page: key flow, endpoints, how to verify a handshake)

**Interfaces:**
- Consumes: `wireguard-sync` binary (`generate` pregenerates `keystore.json` for teams 1..N; default command syncs DB peers → the local wg interface and reports handshakes to the controlserver API) — binary exists in `rust/wireguard-sync-rs`; the sim image gets it via Task 6's build (COPY both crates into the build context — extend the *sim* image, not `Dockerfile.rust`).
- Produces: key shape `/run/sim/keys/team<id>.json` = `{"private": "<wg private key>", "public": "<wg public key>"}` (extract from wireguard-sync's `keystore.json` format — read `rust/wireguard-sync-rs/src/keystore.rs` before writing; adapt this file's shape to what it actually stores). Vulnbox `wg0`: address = `vpn_peer_ip(team_id)` from the sim config math, `PrivateKey`, `Peer` = gameserver public key, `Endpoint = controlserver:51820`, `AllowedIPs = 10.32.0.0/16`, `PersistentKeepalive = 15`. Server side (controlserver container): `wg0` at `vpn_host` (10.32.250.1), `ListenPort 51820`, created by wireguard-sync's normal init (its interface-up hooks handle routing/iptables from the repo `vpn/` dir already present at `/opt/vpn` in the image).

- [ ] **Step 1: Read `rust/wireguard-sync-rs/src/keystore.rs` and `network.rs`** — confirm keystore JSON shape and how `generate` writes keys; adjust the key-file extraction snippet in this task accordingly.
- [ ] **Step 2: Write vulnbox `wg/entrypoint.sh`** — reads `/run/sim/keys/team$TEAM_ID.json` + env `SERVER_PUBLIC_KEY`, writes `/etc/wireguard/wg0.conf` per the shape above, `wg-quick up wg0`.
- [ ] **Step 3: Write `wg/README.md`** — the key flow: `wireguard-sync generate -a $TEAMS` once (init container) → shared `keys` volume; gameserver's public key read from the keystore by an init step exported as `SERVER_PUBLIC_KEY`.
- [ ] **Step 4: Verify a tunnel in isolation** — two containers on one docker network (server-side wg setup by hand with `wg` commands per README, client via the entrypoint), `ping 10.32.250.1` from the vulnbox side ⇒ succeeds; `wg show` on the server lists the peer with a recent handshake.
- [ ] **Step 5: Commit** — `git add rust/simulation/wg && git commit -m "simulation: wireguard client plumbing for vulnboxes"`

### Task 6: Sim image + compose (vulnbox generator, full stack)

**Files:**
- Create: `rust/simulation/Dockerfile.sim` — `FROM` the built `gameserver-rust` image; adds `wireguard-tools iproute2 iptables` plus the `wireguard-sync` + `vpnboard` binaries (build them in a `rust_build` stage copying `rust/wireguard-sync-rs` and `rust/vpnboard-rs` — copy the sibling crates the same way `Dockerfile.rust` does for controlserver).
- Create: `rust/simulation/gen_compose.py` — emits `rust/simulation/docker-compose.vulnboxes.yml`: for team N in 2..=TEAMS (team 1 is the gameserver's own vulnbox? No — team 1 is NOP and still needs a vulnbox: emit for 1..=TEAMS) one service `vulnbox-N`: image `sim-vulnbox`+`sim-tools` layering per SIM_VPN, `ipv4_address: 10.32.N.2` on network `simnet` (subnet `10.32.0.0/16`, gateway `10.32.0.1`), env `TEAM_ID=N`, mounts `keys:/run/sim/keys:ro` (SIM_VPN=1), `cap_add: [NET_ADMIN]` + `devices: [/dev/net/tun]` in SIM_VPN=1.
- Create: `rust/docker-compose.simulation.yml` — full stack: `postgres` (with `seed.sql` NOT auto-mounted — seed runs via the env gate; ports internal only), `redis`, `controlserver` (image from `Dockerfile.sim`, `SAARCTF_SIMULATION=1`, `SAARCTF_SIM_SEED=/opt/sim/seed.sql`, mounts: `rust/simulation/seed.sql:/opt/sim/seed.sql:ro`, `rust/simulation/services:/opt/services:ro`, `config.sim.yaml` as `CONFIG_FILE`, wg caps + port `51820/udp` in SIM_VPN=1), `checker_runner` (same image, replicas 4), `submitter` (existing Python image from `flag-submission-server`, port 1337:31337), `scoreboard-daemon` + `scoreboard` nginx (port 8080:80), `sim-init` (one-shot: runs `wireguard-sync generate -a $TEAMS` and exports the server public key to the keys volume; only in SIM_VPN=1), `wireguard-sync` (long-running, privileged wg server side; SIM_VPN=1), `vpnboard` (`network_mode: service:controlserver`, port via the controlserver container; SIM_VPN=1), `attack-simulator` (Task 4 image). SIM_VPN is resolved by `gen_compose.py` emitting a small override file (`docker-compose.sim-vpn.yml`) rather than YAML anchors, so the base file never contains privileged entries. Never inline `SECRET_FLAG_KEY` — pass through `${SECRET_FLAG_KEY}` from `.env.sim`.
- Create: `rust/simulation/.env.sim.example` — `TEAMS=8`, `SIM_ATTACK_RATE=0.25`, `SIM_VPN=1`, `SECRET_FLAG_KEY=<zero placeholder for demo>`, `POSTGRES_PASSWORD=demo`, `REDIS_PASSWORD=demo`.
- Create: `rust/simulation/simctl` — tiny shell wrapper: `simctl up` (gen compose files → `docker compose -f ... up -d --build`), `simctl down`, `simctl demo-commands` (prints curl/one-liners for the demo).

**Interfaces:**
- Consumes: all prior tasks; existing images (`gameserver-rust` base build via `rust/controlserver-rs/Dockerfile.rust` context, submitter image).
- Produces: `rust/simulation/simctl up` brings the whole demo from clean state.

- [ ] **Step 1: Write `Dockerfile.sim`** and build it (`docker build -f rust/simulation/Dockerfile.sim -t gameserver-sim rust/` — build context `rust/` so sibling crates resolve; adjust paths to match `Dockerfile.rust`'s COPY layout). Expected: image builds, `wireguard-sync` and `vpnboard` binaries present.
- [ ] **Step 2: Write `gen_compose.py`** (stdlib; emits the two YAML files deterministically from `TEAMS`/`SIM_VPN`) and run it: `python3 rust/simulation/gen_compose.py --env rust/simulation/.env.sim` → files appear. Add `--check` mode used by tests: emits to stdout; a `test_gen.py` asserts `TEAMS=3` output contains `vulnbox-1`, `vulnbox-3`, `10.32.3.2`, and SIM_VPN=0 output contains no `NET_ADMIN`. Run the test.
- [ ] **Step 3: Write `docker-compose.simulation.yml` + `.env.sim.example` + `simctl`** — cross-check every env name against `x-gameserver-common-env` in `rust/docker-compose.rust.yml` (the Rust bins read the same `POSTGRES_*`/`REDIS_*`/`CONFIG_FILE`/`SCOREBOARD_PATH` names).
- [ ] **Step 4: Config sanity inside the stack** — `docker compose -f rust/docker-compose.simulation.yml config` validates; no `SECRET_FLAG_KEY` literal appears in any emitted file (grep; only `${SECRET_FLAG_KEY}` references allowed).
- [ ] **Step 5: Commit** — `git add rust/simulation rust/docker-compose.simulation.yml && git commit -m "simulation: sim image, compose generator, full-stack compose, simctl"`

### Task 7: README + end-to-end bring-up

**Files:**
- Create: `rust/simulation/README.md`

**Interfaces:** Consumes `simctl` (Task 6). Produces the demo runbook.

- [ ] **Step 1: Write `README.md`** — what it is (link the spec), prerequisites (Docker with /dev/net/tun for SIM_VPN=1; rootless docker ⇒ SIM_VPN=0), quickstart (`cp .env.sim.example .env.sim && ./simctl up`), env var table, ports (scoreboard :8080, submitter :1337, vpnboard via controlserver), what to watch (ticks, first-blood overlay + sound in the SPA), the in-memory flag-store caveat (vulnbox restart drops stored flags — checkers will FlagMissing until next store tick; acceptable for demo), teardown, and troubleshooting (wg handshake absent ⇒ check port 51820/udp and capabilities).
- [ ] **Step 2: Clean-state bring-up (SIM_VPN=0 first)** — `SIM_VPN=0 TEAMS=3 ./simctl up`; wait 3 ticks (≥90 s). Verify: controlserver log shows `simulation mode: boot steps applied` and `New tick` lines; checker results rows appear (`docker compose exec postgres psql ... -c "SELECT status, count(*) FROM checker_results GROUP BY status"` — statuses are SUCCESS/MUMBLE/OFFLINE, no dispatch errors); scoreboard JSON at `http://localhost:8080/api/...` regenerates per tick; SPA loads.
- [ ] **Step 3: First-blood check (SIM_ATTACK_RATE=0.5)** — bring up with attacks on; within ~3 ticks the SPA shows a First Blood overlay (verify by watching `scoreboard_round_*.json` for a nonzero attacker count on some team, plus the SPA visually if interactive).
- [ ] **Step 4: Real-pipeline untouched** — `git diff master -- rust/docker-compose.rust.yml rust/controlserver-rs/Dockerfile.rust` is empty; and `SAARCTF_SIMULATION` unset start of controlserver (no env) logs no `simulation mode` line (run the binary briefly locally with the demo config as before — same as pre-existing behavior).
- [ ] **Step 5: VPN bring-up (SIM_VPN=1)** — `SIM_VPN=1 TEAMS=3 ./simctl up`; verify vpnboard shows all 3 peers with recent handshakes, and `docker compose exec controlserver wg show` lists them.
- [ ] **Step 6: VPN gating down/up cycle** — inside vulnbox-2: `wg-quick down wg0`; within ~2 vpnboard cycles (its offline threshold is two missed ~120 s handshakes — allow up to 5 min) vpnboard shows team 2 offline and its checker results stop arriving; `wg-quick up wg0` restores both. Record actual timings in the README troubleshooting section.
- [ ] **Step 7: Commit** — `git add rust/simulation/README.md && git commit -m "simulation: runbook + end-to-end verification notes"`

---

## Execution Notes

- Tasks 1→2→3 are strictly ordered (checkers need the vulnbox image; the boot gate needs both). Task 4 and Task 5 are independent of 3 and can run in parallel once 1/2 exist. Task 6 depends on all; Task 7 depends on 6.
- The Rust change (Task 3) is the only product-code change; everything else is additive under `rust/simulation/`.
- If `saarctf.py` requires a real package name in Task 2 Step 4, the correct package string comes from `import_services` in Task 3 — the smoke test may substitute the package name printed by that step when run against a live DB, or use `SAARCTF_PYTHON_DIR`-relative import of the mounted checker dir as a fallback smoke.
