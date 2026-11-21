# Simulation deployment

A one-command Docker deployment of the whole CTF pipeline — controlserver,
checker runners, flag submitter, scoreboard, admin GUI, and one vulnbox
fixture container per team — for demoing and testing the platform end to end.
It is gated by `CTF_SIMULATION=1`: with the variable unset, the real pipeline
is unchanged.

Sources of truth: `rust/docker-compose.simulation.yml` (base stack),
`rust/simulation/gen_compose.py` (TEAMS/VPN-dependent files), and
`rust/simulation/README.md` (internals). Historical design and plan:
`docs/superpowers/specs/2026-11-20-docker-simulation-deployment-design.md`,
`docs/superpowers/plans/2026-11-20-docker-simulation-deployment.md`.

## What runs

`simctl up` assembles three compose files:

- `rust/docker-compose.simulation.yml` — base stack (project name `ctf-sim`)
- `rust/simulation/docker-compose.vulnboxes.yml` — one `vulnbox-N` per team,
  generated from `TEAMS`
- `rust/simulation/docker-compose.sim-vpn.yml` — WireGuard plumbing, generated
  only when `SIM_VPN=1`

| service | role | host port | notes |
|---|---|---|---|
| `postgres` | database | — | `postgres:17.2`, volume `postgres`, healthcheck `pg_isready`; internal only |
| `redis` | streams/state store | — | `redis:7.4-alpine`, volume `redis`; internal only |
| `controlserver` | API + tick timer + sim boot | `15000` → `5000` | runs migrations, seed, service import, enables checkers, starts the CTF; serves the **admin GUI** SPA and admin API (no auth) |
| `checker_runner` | checker workers | — | 4 replicas; spawn the Python checker scripts per tick |
| `submitter` | flag submission (raw TCP) | `1337` → `31337` | the C++ flag-submission-server image; vulnbox attack loops target `submitter:31337` internally |
| `scoreboard-daemon` | scoreboard JSON generator | — | writes into the `scoreboard` volume (`/var/scoreboard` in the container) |
| `scoreboard` | scoreboard SPA + JSON | `8080` → `80` | nginx serving the `scoreboard` volume |
| `vulnbox-1` … `vulnbox-N` | per-team vulnerable services | — | demoservice :1337, cloud-api :8080, flagboard :31337 inside each container; also runs that team's attack loop |

There is **no RabbitMQ** in this stack: the Rust pipeline dispatches over Redis
Streams (ADR-012/015). The historical Python stack used Celery + RabbitMQ.

`SIM_VPN=1` (default) additionally starts:

| service | role |
|---|---|
| `sim-init` | one-shot: generates WireGuard keys into the `keys`/`wg-keystore` volumes (idempotent) |
| `host-route` | one-shot, privileged: routes `10.32.0.0/16` on the Docker host to the controlserver |
| `vpnboard` | VPN status daemon, shares the controlserver network namespace |

and makes `controlserver` privileged (it is the sim's router), adds
`NET_ADMIN`/`/dev/net/tun` to vulnboxes and `NET_ADMIN` + a route to
`checker_runner`. With `SIM_VPN=1` the Docker bridge is `10.33.0.0/16` because
the WireGuard tunnels own `10.32.0.0/16` (team N's vulnbox is `10.32.N.2`).
With `SIM_VPN=0` the bridge itself is `10.32.0.0/16` and no privileges are
needed.

Teams come from `rust/simulation/seed.sql` (8 rows, idempotent). `TEAMS` above
8 generates vulnboxes that have no team row and take no part in the game.

## Prerequisites

- Docker with Compose v2.
- A local `gameserver-rust` image: the simulation image (`Dockerfile.sim`) is
  `FROM ${BASE_IMAGE:-gameserver-rust}` and does not build that base. Build it
  once from the repo root:
  `docker build -t gameserver-rust -f rust/controlserver-rs/Dockerfile.rust .`
  If you already built the base under a registry prefix (for example
  `rust/cutover.sh build`), set `BASE_IMAGE=<registry>/gameserver-rust` in
  `rust/simulation/.env.sim` instead.
- `SIM_VPN=1`: `/dev/net/tun` and container privileges (Docker Desktop's
  default VM works). Rootless Docker / Podman: use `SIM_VPN=0`.
- Free host ports `8080`, `1337`, `15000`.

## Start, stop, reset

```sh
cd rust/simulation
cp .env.sim.example .env.sim   # first run only; simctl creates it if missing
./simctl up
```

`simctl up` regenerates the TEAMS/VPN compose files, then runs
`docker compose ... up -d --build`. The first run builds `gameserver-sim`
(layered on the `gameserver-rust` base) and `sim-vulnbox` and can take several
minutes.
Subsequent runs reuse the build cache and the existing volumes.

- Stop, keep volumes/data: `./simctl down`
- Stop and wipe volumes (clean reset): `./simctl down -v`
- Reset to seeded state: `./simctl down -v && ./simctl up`

On every controlserver start the sim boot gate re-applies the seed, re-imports
services and re-enables checkers (all idempotent), then auto-starts the game.

### What to watch

```sh
curl -s localhost:8080/api/scoreboard_round_<N>.json   # scores rise each tick
curl -s localhost:15000/overview/logs | tail                # "New tick: N", dispatch/first-blood events
cd rust/simulation && ./simctl demo-commands          # more one-liners
```

First checker results appear within about one tick (30 s in the sim config).
With `SIM_ATTACK_RATE > 0` the attack loops steal checker-stored flags and
submit them, so attack points and first-blood overlays appear within a few
ticks.

## Re-deploying rebuilt web apps into the running stack

Both SPAs are built on a workstation with [Trunk](https://trunkrs.dev) and
copied into the images:

```sh
cd rust/scoreboard-rs && trunk build --release
cd rust/admin-gui-rs  && trunk build --release
```

The image bakes in whatever is in `dist/` at build time. To update a running
stack without rebuilding and recreating containers, copy the new `dist` into
the container that serves it.

Define the compose invocation once (from the repo root; add
`-f rust/simulation/docker-compose.sim-vpn.yml` when `SIM_VPN=1`, exactly as
`simctl` does):

```sh
sim() { docker compose --env-file rust/simulation/.env.sim \
  -f rust/docker-compose.simulation.yml \
  -f rust/simulation/docker-compose.vulnboxes.yml "$@"; }
```

### Scoreboard (`:8080`)

The scoreboard daemon writes JSON **and** holds the SPA files in the
`scoreboard` volume, which nginx serves:

```sh
docker cp rust/scoreboard-rs/dist/. "$(sim ps -q scoreboard-daemon)":/var/scoreboard/
```

### Admin GUI (`:15000`)

The controlserver serves the SPA from `ADMIN_GUI_DIST` (`/opt/admin-gui/dist`,
set in `rust/controlserver-rs/Dockerfile.rust`):

```sh
docker cp rust/admin-gui-rs/dist/. "$(sim ps -q controlserver)":/opt/admin-gui/dist/
```

### Verify what is actually served

Hashed filenames differ per build; compare the served `index.html` with your
local `dist/index.html`:

```sh
curl -s localhost:8080/ | grep -oE 'scoreboard-rs-[a-f0-9]+_bg\.wasm'   # SPA
curl -s localhost:15000/ | grep -oE 'admin-gui-rs-[a-f0-9]+_bg\.wasm'   # admin GUI
grep -oE '(scoreboard|admin-gui)-rs-[a-f0-9]+_bg\.wasm' \
  rust/scoreboard-rs/dist/index.html rust/admin-gui-rs/dist/index.html
```

The hashes must match. If they differ, the served files are stale — the
container is still serving an older build.

### Gotchas

- **A scoreboard-daemon restart resurrects the image's static files.** On the
  first scoreboard generation after the daemon starts, it copies its baked-in
  SPA into the output volume, overwriting what you `docker cp`'ed in. After any restart/recreate of
  `scoreboard-daemon` (`docker restart`, `simctl up`, crash), re-run the
  `docker cp` above and re-check the served hash. Recreating the controlserver
  container has the same effect for the admin GUI.
- **`index.html` is cached by the browser** (nginx sends `max-age=30` for HTML,
  one day for hashed assets). Use a query-parameter bust after a deploy, e.g.
  `http://localhost:8080/?v=2` and `http://localhost:15000/?v=2`, or a hard
  reload. Hashed JS/WASM URLs change per build and need no bust.
- Copying a new `dist` leaves the previous hashed files in place (harmless —
  nothing references them). `simctl down -v` wipes the volume.

## Common gotchas

| symptom | cause / fix |
|---|---|
| Served hash does not match `dist/index.html` | stale files in the volume/container (see above); re-copy and verify |
| UI looks unchanged after a deploy | browser cache — append `?v=<n>` to the URL |
| Editing `controlserver-rs` sources has no effect | `gameserver-sim` is layered on the cached `gameserver-rust` base image; rebuild both (`simctl up` after rebuilding the base, or build with `--no-cache`) |
| Vulnbox restart loses flags | the vulnbox services keep flags **in memory**; the next `retrieve_flags` reports `FlagMissing` until a `store_flags` tick repopulates them |
| No `wg show` handshakes (`SIM_VPN=1`) | `/dev/net/tun` missing or `NET_ADMIN` dropped; check `docker logs ctf-sim-controlserver-1` and fall back to `SIM_VPN=0` |
| Team goes OFFLINE after `wg-quick down wg0` on a vulnbox | expected; vpnboard needs two missed ~120 s handshakes, so allow up to 5 minutes. `wg-quick up wg0` restores it (equally slow) |
| Submitter restart-loops | it reads `CTF_CONFIG`; it must point at the mounted `config.sim.yaml` |
| Checker results `CRASHED` | read the `output` column of the newest `checker_results` row; usually a missing Python dependency in the base image |
| Host ports already in use | another stack holds `8080`/`1337`/`15000`; stop it (the sim does not remap these) |

Container names follow `ctf-sim-<service>-1` (from the compose `name:`).
A stack brought up from an older checkout may carry a different project
prefix; `./simctl down && ./simctl up` recreates it under the current name.
Prefer `docker compose ps` / `ps -q` over hard-coded container names.

## Environment (`rust/simulation/.env.sim`)

| variable | default | meaning |
|---|---|---|
| `TEAMS` | 8 | number of vulnbox containers (team 1 is NOP and is never attacked) |
| `SIM_ATTACK_RATE` | 0.25 | share of (team, service) pairs attacked per tick; `0` = defense-only |
| `SIM_VPN` | 1 | `1` = real WireGuard tunnels (`/dev/net/tun` required); `0` = plain bridge, no privileges |
| `SECRET_FLAG_KEY` | zero placeholder | HMAC flag-signing key shared by controlserver and submitter; the demo placeholder is public by design |
| `POSTGRES_PASSWORD` | `demo` | database password for the sim-local Postgres |

Never commit a real `SECRET_FLAG_KEY`; the sim placeholder must stay a
placeholder.

## What is simulated vs real

- **Real:** tick timer, checker subprocess protocol and Python checker scripts,
  HMAC flag generation/validation, checker results over Redis Streams,
  dispatcher, scoring, scoreboard JSON, flag submission.
- **Simulated:** the vulnbox services (HTTP fixtures that store/retrieve flags
  per service and tick), the attacking side (per-team loops inside the
  vulnboxes), and the team rows (8 generic fixture teams).

VPN details (key flow, endpoints, handshake verification):
`rust/simulation/wg/README.md`.
