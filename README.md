CTF Gameserver (Rust)
=====================

An attack-defense CTF gameserver: tick timer, checker dispatch, flag submission,
scoring, scoreboard, admin GUI and VPN tooling. This repository is a Rust rework
of the original Python + Angular gameserver — the services are axum/tokio
binaries and both web frontends are Leptos 0.8 CSR apps.

Components
----------
- **controlserver** (`rust/controlserver-rs`) — axum API, master tick timer,
  checker dispatcher, scoring, scoreboard JSON generation; also serves the
  admin GUI (`rust/admin-gui-rs`, Leptos).
- **checker_runner** — Redis Streams consumers that run the checker scripts
  (Python, via the `checker-exec` PyO3 shim, ADR-017).
- **scoreboard-daemon** + nginx — writes the scoreboard JSON files and serves
  the Leptos scoreboard SPA (`rust/scoreboard-rs`).
- **submitter** (`flag-submission-server/`) — C++ flag submission server.
- **PostgreSQL** and **Redis** — no RabbitMQ/Celery (ADR-012/015).
- VPN/network tooling lives in `rust/ctf-scripts-rs`, `rust/wireguard-sync-rs`
  and `rust/vpnboard-rs`.

Layout
------
| path | contents |
|---|---|
| `rust/` | Cargo workspace, production compose stack (`docker-compose.rust.yml`), simulation stack, ops binaries |
| `flag-submission-server/` | C++ submission server |
| `gamelib/` | Python checker SDK — checker scripts stay Python (ADR-013) |
| `ansible/` | bare-metal deployment playbook |
| `docs/` | guides — start at [docs/README.md](docs/README.md) |

Architecture decisions and the port map: [rust/DECISIONS.md](rust/DECISIONS.md),
[rust/MAP.md](rust/MAP.md).

Quickstart: run the simulation
------------------------------
The simulation stack runs the whole pipeline in Docker against per-team
vulnbox fixture containers. Needs Docker with Compose v2:

```sh
# one-time: the base image the simulation image is layered on
docker build -t gameserver-rust -f rust/controlserver-rs/Dockerfile.rust .

cd rust/simulation
./simctl up
```

- Scoreboard SPA: <http://localhost:8080>
- Admin GUI / admin API: <http://localhost:15000>
- Flag submitter: `localhost:1337` (raw TCP)

Teardown is `./simctl down`; a clean reset is `./simctl down -v && ./simctl up`.
Full guide (topology, redeploying rebuilt web apps, troubleshooting):
[docs/simulation.md](docs/simulation.md). Internals:
[rust/simulation/README.md](rust/simulation/README.md).

Deploying
---------
Bare-metal hosting (Debian/Ubuntu, Docker installed by the playbook):
[docs/hosting-baremetal.md](docs/hosting-baremetal.md). The real-deployment
compose file is `rust/docker-compose.rust.yml`.

Configuration
-------------
The controlserver, checker workers and scoreboard daemon read a YAML config
file (`CONFIG_FILE`, see [config.sample.yaml](config.sample.yaml)); the
submitter reads the same file mounted at `/config.yaml` and gets its connection
settings from the environment. The `POSTGRES_*`, `REDIS_*`, `SECRET_FLAG_KEY`
and scoreboard-path variables are listed in [.env.example](.env.example).
Flags are `PREFIX{...}`; the prefix is the `flag_prefix` config key (default
`FLAG`).

Credits
-------
- **Upstream:** this is a fork of the [saarCTF gameserver](https://github.com/MarkusBauer/saarctf-gameserver) built by Markus Bauer with contributions from Jonas Bushart, Patrick Schmelzeisen, Niklas Beierl and Simon Einzinger (MIT license, see [LICENSE](LICENSE)); the upstream Python package `saarctf_commons/` keeps its name because the pinned `gamelib` submodule imports it (ADR-013/017).
- **Rust port:** the incremental Python-to-Rust port was driven by the
  **rust-anything** toolchain (record → diff → gate per unit; see
  [rust/MAP.md](rust/MAP.md) and ADR-016).
- **Development workflow:** planned, implemented and reviewed with the
  **dev-team** agent workflow (sliced plan → implementer → reviewer, ledger in
  [docs/progress.md](docs/progress.md)).
