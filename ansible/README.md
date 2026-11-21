# Ansible deployment

Deploys the Rust stack (`rust/docker-compose.rust.yml`) to one bare-metal
Debian or Ubuntu host. The guide is
[docs/hosting-baremetal.md](../docs/hosting-baremetal.md); this file is the
playbook reference.

## Requirements

- Ansible 2.14+ on the machine you run it from. **No collections are
  required** — the playbook uses `ansible.builtin` modules only and installs
  Docker from Docker's official apt repository.
- The target host: Debian 12+ or Ubuntu 22.04+, reachable over SSH with a
  sudo-capable user, and `python3`/`python3-apt` present (default on Debian and
  Ubuntu server images).
- The host needs outbound network access for apt, the Docker repository and
  (unless images are pre-built) image builds.

## Files

| file | purpose |
|---|---|
| `inventory.example.ini` | one host entry — copy to `inventory.ini` and edit |
| `site.yml` | the playbook: Docker install, repo sync, config/env, compose deploy, health wait |
| `templates/env.j2` | renders `rust/.env` for the compose stack |

## Inputs

Pass these with `-e` (or a `-e @file.yml`). Secrets belong in a file outside
this repository — the playbook requires them as variables, not committed
values.

| variable | default | meaning |
|---|---|---|
| `gameserver_repo_url` | `https://github.com/example/gameserver.git` | repository to clone on the host |
| `gameserver_repo_version` | `master` | branch/tag/commit to deploy |
| `gameserver_repo_dest` | `/opt/gameserver` | checkout directory |
| `gameserver_config_src` | *required* | local path to your `config.yaml` (start from `config.sample.yaml`); copied to `rust/config.yaml` |
| `gameserver_secret_flag_key` | *required* | 64 hex chars, `openssl rand -hex 32`; must not be a placeholder |
| `gameserver_postgres_local` | `false` | `true` runs the bundled `postgres` service (compose profile `donotstart`) |
| `gameserver_postgres_server` / `_port` / `_db` / `_user` / `_password` | `""` / `5432` / `ctf` / `ctf` / *required* | external database |
| `gameserver_redis_host` / `_port` / `_db` / `_password` | `redis` / `6379` / `0` / `""` | the compose stack always runs redis; point this at it |
| `gameserver_image_registry` | `localhost` | images are `<registry>/gameserver-rust` and `<registry>/gameserver/submitter` |
| `gameserver_build_images` | `true` | build the images on the host; set `false` to pull them from the registry |
| `gameserver_telegraf_enabled` | `false` | enables the `submitter_telegraf` metrics sidecar (needs TimescaleDB credentials) |
| `gameserver_timescale_*` | `""` | TimescaleDB connection for the telegraf sidecar |
| `gameserver_public_host` | `ansible_host` | host name used in the printed URLs |

`rust/.env` is rendered with mode `0600`; `rust/config.yaml` with `0640`.

## Run

```sh
cp ansible/inventory.example.ini ansible/inventory.ini   # edit host/user
openssl rand -hex 32                                     # SECRET_FLAG_KEY
$EDITOR ~/.ctf-gameserver-secrets.yml                    # see below

ansible-playbook -i ansible/inventory.ini ansible/site.yml \
  -e @~/.ctf-gameserver-secrets.yml \
  -e gameserver_repo_url=git@github.com:you/gameserver.git \
  -e gameserver_config_src=$PWD/my-config.yaml
```

Example secrets file (keep it outside the repository):

```yaml
gameserver_secret_flag_key: "<64 hex chars>"
gameserver_postgres_password: "<db password>"
gameserver_redis_password: "<redis password or empty>"
```

What the playbook does, in order:

1. validates the inputs (distro, config path, 64-hex flag key, database);
2. installs `ca-certificates`, `curl`, `gnupg`, `git`;
3. adds Docker's apt repository and installs Docker Engine,
   `docker-buildx-plugin` and `docker-compose-plugin`; enables the service;
4. clones/updates the repository at `gameserver_repo_version`;
5. copies your config to `rust/config.yaml` and templates `rust/.env`;
6. builds the images (`gameserver_build_images=true`) or pulls them from
   `gameserver_image_registry` (`false`);
7. runs `docker compose -f rust/docker-compose.rust.yml up -d --wait
   --remove-orphans` (plus `--profile donotstart` for the bundled database,
   `--scale submitter_telegraf=0` when the metrics sidecar is disabled);
8. waits for `http://127.0.0.1:5000/overview/logs` and
   `http://127.0.0.1:8080/` to return 200;
9. prints the service URLs.

## Idempotency

- Package and repository setup tasks are convergent; re-running makes no
  changes once the repository and packages exist.
- `git` sync leaves the checkout on `gameserver_repo_version` and discards
  local modifications (`force: true`); the config file and env file are then
  re-rendered from your inputs, so the server is not the place to hand-edit
  them.
- `docker compose up -d --wait` recreates only services whose definition or
  image changed. Data lives in named volumes (`postgres`, `redis`,
  `public-scoreboard`, `internal-scoreboard`) and survives a re-run.
- The build/pull task always reports `changed`; run with
  `-e gameserver_build_images=false` for a pull-only deploy (the host must be
  logged into the registry if it is private).

## Verify and tear down

```sh
ansible -i ansible/inventory.ini gameservers -m shell -a \
  'docker compose -f /opt/gameserver/rust/docker-compose.rust.yml --env-file /opt/gameserver/rust/.env ps'
```

Stopping the stack keeps the volumes; `down -v` (run manually) destroys the
data. See the guide's troubleshooting section for the common failures
(metrics sidecar crash-loop, unreachable external database, port conflicts).
