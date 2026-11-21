# Hosting on bare metal (Ansible)

How to deploy the Rust gameserver stack to a single bare-metal or VPS host
running Debian or Ubuntu. The playbook in `ansible/` installs Docker, syncs
this repository, installs the config and env file, and converges
`rust/docker-compose.rust.yml`.

Everything the playbook deploys comes from that compose file; the playbook
only wraps it. Local demo/simulation is a different stack — see
[simulation.md](simulation.md).

## What gets deployed

From `rust/docker-compose.rust.yml` (project name `gameserver`):

| service | host port | notes |
|---|---|---|
| `controlserver` | `5000` | axum API, tick timer, dispatcher, scoring, admin GUI; runs sqlx migrations at boot |
| `submitter` | `1337` → `31337` | C++ flag submission server (raw TCP) |
| `scoreboard` | `8080` → `80` | nginx serving the scoreboard SPA + JSON from the `internal-scoreboard` volume |
| `scoreboard-daemon` | — | writes scoreboard JSON and SPA files into the volumes |
| `checker` | — | 8 checker runner replicas (redis streams consumers) |
| `redis` | `127.0.0.1:6379` | always started; reachable from the host loopback only |
| `postgres` | `127.0.0.1:5432` | only started with the `donotstart` profile (bundled database); external DB is the default |
| `submitter_telegraf` | — | metrics sidecar; disabled by default by the playbook (needs a TimescaleDB) |

Not deployed by this compose file: WireGuard/VPN services. The binaries exist
(`rust/wireguard-sync-rs`, `rust/vpnboard-rs`) but there are no compose service
definitions for them, so VPN hosting is out of scope for this guide.

## Prerequisites

- A Debian 12+ or Ubuntu 22.04+ host with SSH access and a sudo-capable user.
- Ansible 2.14+ on the control machine; no collections needed.
- Outbound network on the host (apt, Docker repository) and enough disk/RAM for
  the image builds (Rust and C++ compile on the host by default).
- A real `config.yaml` (start from `config.sample.yaml`) and a flag-signing
  key generated with `openssl rand -hex 32`.

## Inventory

```sh
cp ansible/inventory.example.ini ansible/inventory.ini
$EDITOR ansible/inventory.ini
```

One host in the `gameservers` group:

```ini
[gameservers]
ctf1 ansible_host=203.0.113.10 ansible_user=deploy
```

Add `ansible_ssh_private_key_file=...` if needed. Run the playbook with `-K`
if the user does not have passwordless sudo.

## First deploy

Secrets go in a file outside the repository:

```yaml
# ~/.ctf-gameserver-secrets.yml
gameserver_secret_flag_key: "<openssl rand -hex 32>"
gameserver_postgres_password: "<db password>"
gameserver_redis_password: "<redis password, or empty>"
```

Then:

```sh
ansible-playbook -i ansible/inventory.ini ansible/site.yml \
  -e @~/.ctf-gameserver-secrets.yml \
  -e gameserver_repo_url=git@github.com:you/gameserver.git \
  -e gameserver_config_src=$PWD/my-config.yaml
```

External database (default) — pass:

```sh
-e gameserver_postgres_server=db.internal -e gameserver_postgres_db=ctf \
-e gameserver_postgres_user=ctf
```

Bundled database instead (starts the `postgres` service and keeps its data in
the `postgres` volume):

```sh
-e gameserver_postgres_local=true
```

The first run compiles the Rust binaries and the C++ submitter inside Docker,
so it takes substantially longer than later runs. The playbook finishes by
printing:

```
Admin GUI / admin API: http://<host>:5000  (unauthenticated — firewall it)
Scoreboard:            http://<host>:8080
Flag submitter:        <host>:1337  (raw TCP)
```

## The config file

The compose file mounts `rust/config.yaml` into the controlserver
(`CONFIG_FILE`, default `/opt/config.yaml`) and the submitter (`/config.yaml`).
The repository's `rust/config.yaml` is the **simulation** config, so the
playbook replaces it with `gameserver_config_src` on every run. The config
provides the network ranges, tick duration, scoring factors, flag prefix and
paths; the env file overrides the database/redis connection and
`SECRET_FLAG_KEY`.

Relevant env variables (rendered to `rust/.env`, mode `0600`; the same names
the compose stack reads, cf. `.env.example`):

| variable | used by | meaning |
|---|---|---|
| `IMAGE_REGISTRY` | all images | registry prefix for `<registry>/gameserver-rust` and `<registry>/gameserver/submitter` |
| `POSTGRES_SERVER` / `_PORT` / `_DB` / `_USER` / `_PASSWORD` | controlserver, checker, scoreboard-daemon, submitter | database connection |
| `REDIS_HOST` / `_PORT` / `_DB` / `_PASSWORD` | same services | redis connection |
| `SECRET_FLAG_KEY` | controlserver, checker, submitter | HMAC flag-signing key (64 hex chars) |
| `CONFIG_FILE` | compose mounts | path where `rust/config.yaml` is mounted (`/opt/config.yaml`) |
| `PUBLIC_SCOREBOARD_PATH` / `INTERNAL_SCOREBOARD_PATH` | compose volumes | scoreboard output mount points; nginx serves the internal volume on `:8080` |
| `TIMESCALE_*` | `submitter_telegraf` | metrics database; unused unless the sidecar is enabled |

Build vs pull: `gameserver_build_images=true` (the default) builds both images
on the server. To deploy pre-built images instead, push them to
`<gameserver_image_registry>/gameserver-rust` and
`<gameserver_image_registry>/gameserver/submitter`, set
`gameserver_build_images=false`, and log the host into the registry first.

## Subsequent deploys

Re-run the same command with the new revision:

```sh
ansible-playbook -i ansible/inventory.ini ansible/site.yml \
  -e @~/.ctf-gameserver-secrets.yml \
  -e gameserver_repo_url=git@github.com:you/gameserver.git \
  -e gameserver_config_src=$PWD/my-config.yaml \
  -e gameserver_repo_version=v1.2
```

- The checkout is updated, the env file re-rendered, and
  `docker compose up -d --wait` recreates only what changed.
- Named volumes (`postgres`, `redis`, `public-scoreboard`,
  `internal-scoreboard`) persist across deploys.
- Changing the config file alone does not recreate containers: after a config
  edit, run `docker compose -f rust/docker-compose.rust.yml --env-file
  rust/.env up -d --force-recreate controlserver submitter` on the host (or
  remove the containers) so the new file is reloaded.
- Back up the database (or the `postgres` volume) before upgrading.

## Verification

```sh
ssh deploy@<host>
cd /opt/gameserver
docker compose -f rust/docker-compose.rust.yml --env-file rust/.env ps
curl -s localhost:5000/overview/logs | head -c 200; echo
curl -s -o /dev/null -w '%{http_code}\n' localhost:8080/
nc -z localhost 1337 && echo "submitter reachable"
```

The playbook already waits for `5000/overview/logs` and `8080/` to return 200, so a
successful run means both are up. Right after a cold start the scoreboard can
return a non-200 until the daemon has written the SPA files.

## Troubleshooting

| symptom | cause / fix |
|---|---|
| `up --wait` fails on `submitter_telegraf` | the metrics sidecar needs a reachable TimescaleDB; keep `gameserver_telegraf_enabled=false` (the playbook then scales it to 0) |
| controlserver exits or `/overview/logs` never returns 200 | the database is unreachable or credentials are wrong — check `docker compose ... logs controlserver`; migrations run at boot |
| `postgres` service missing from `ps` | it only starts with `--profile donotstart`; the playbook adds it when `gameserver_postgres_local=true` |
| image build fails with a registry error | `IMAGE_REGISTRY` points at a registry you cannot reach; use `gameserver_build_images=true` (local build) |
| port already in use on the host | the compose file fixes host ports `5000`, `8080`, `1337`; stop the conflicting service or add a local compose override. `rust/docker-compose.override.yml` is *not* auto-loaded when `-f` is given |
| scoreboard/admin UI shows an old build | stale web assets; copy the built `dist` into the `scoreboard-daemon` container's `INTERNAL_SCOREBOARD_PATH` (default `/var/internal/scoreboard`) and the controlserver's `/opt/admin-gui/dist`, then verify the served hashes as in [simulation.md](simulation.md#re-deploying-rebuilt-web-apps-into-the-running-stack) |
| config change has no effect | containers still mount the old file contents; recreate `controlserver` and `submitter` |

## Security notes

- The admin API and GUI on `:5000` have **no authentication** (ADR-011). Never
  expose them publicly; firewall the port or front it with an authenticating
  proxy.
- The flag submitter (`:1337`) and scoreboard (`:8080`) are meant to be public,
  but the stack does not terminate TLS. Put a TLS reverse proxy in front if the
  event requires HTTPS, and firewall `5000`.
- No production credentials belong in this repository; the playbook requires
  them as variables and writes them to a `0600` env file on the host.
- The compose file does not pin image tags for `nginx`, `telegraf` and the
  gameserver images. For reproducible deploys, build/tag your images at a
  fixed version and pin them in an override file.
