# rusty-celery vs Python Celery 5.x (Redis broker) — interop research

## Verdict: NOT usable for our staged cutover

Only the *producer* direction (Rust sends, Python worker consumes) is wire-compatible;
the *worker* direction we actually need is blocked. The crate on crates.io is stale and
predates the one essential Redis-interop fix. Port `checker_runner` differently (Rust
worker behind its own Redis/HTTP contract, keep the thin Celery shell in Python).

## What our Python side actually does (repo facts)

- App `checker_runner.celery_cmd` in `checker_runner/runner.py:210-227`; broker =
  RabbitMQ preferred, Redis fallback (`saarctf_commons/config.py:504-530`, Celery on
  `REDIS["db"]+1`); result backend = Redis, `result_expires = None`.
- Tasks are auto-named from the module (registered `runner.py:223-227`):
  `checker_runner.runner.run_checkerscript`, `...run_checkerscript_external`,
  `...preload_packages`, `...run_command`. `run_checkerscript(self, runner_spec: str,
  package: str, script: str, service_id: int, team_id: int, tick: int, cfg: dict|None)`
  (`runner.py:81`) — bound task, positional args, returns a str.
- Producer is `controlserver/dispatcher.py` (same Python app, imported producer-side):
  builds `group([...signature(tuple...)])` with *positional* args (`dispatcher.py:140-166`),
  `apply_async()`, then `GroupResult.save()` / `restore()` (`dispatcher.py:142,169`),
  `revoke()` (`dispatcher.py:172`), plus per-message `time_limit`/`soft_time_limit`
  and `countdown` (ETA, `dispatcher.py:162-163`).
- Queues are dynamic: `queue=route or service.checker_route or 'celery'`
  (`dispatcher.py:164`) — queue names are data, plus a kombu `Broadcast` queue
  (`runner.py:219`, used by controlserver→all-workers commands, `pages.py:264-288`).
- flag-submission-server is C++ — it is not a Celery participant.

## Wire compatibility (what rusty-celery gets right)

- The crate is `celery` on crates.io, published from github.com/rusty-celery/rusty-celery;
  `rusty-celery` itself does not exist on crates.io (crates.io API, checked 2026-11).
- Its `Message` implements Celery **task protocol v2**: headers `task`/`id`/`lang`/`eta`/
  `expires`/`retries`/`timelimit`/`group`/`root_id`/`origin`, properties
  `correlation_id`/`content_type`, body `[args, kwargs, embed]`
  (src/protocol/mod.rs, main branch; Celery protocol doc:
  docs.celeryq.dev/en/stable/internals/protocol.html).
- It handles Python's positional-args-only body: JSON bodies `[args,[],embed]` are
  coerced into kwargs by parameter name via `T::ARGS` (protocol.rs) — matches our
  positional signatures, *if* the Rust task's declared param names and order match
  Python's exactly.
- AMQP and Redis brokers both supported; Redis transport uses queue-name LPUSH/BRPOP
  (src/broker/redis.rs) which matches Kombu's default Redis transport.

## Traps (each with a source)

1. **crates.io release predates the Python-Redis fix.** `celery` 0.5.5 is from
   2023-09 (crates.io API). PR #371 "fix sending a message from a rust client to a
   python worker with redis broker" (merged 2024-01) fixed Python workers crashing
   with `KeyError('exchange')` because Kombu's Redis transport needs `delivery_info`
   (`exchange`, `routing_key`) on every message. That fix is **only on git main** —
   you must depend on the git repo, not the published crate. Unfixed 0.5.5 breaks
   Python consumers outright.
2. **No result backend.** README "Backends: not supported yet"; the Rust worker
   cannot write task results anywhere. Our controlserver *requires* results:
   `GroupResult.save()/restore()` on the Redis backend (`dispatcher.py:142,169`)
   is how checker results are collected. PR #372 "implement backend integration +
   redis backend" was never merged (closed stale, 2024-05); issue #392 "Developing
   celery result backend with redis" still open (2026-01). Without it a Rust worker
   would have to hand-roll writing Celery's result-backend Redis format
   (`celery-task-meta-<id>` JSON with `status`/`result`/`traceback`/`children`,
   plus group metadata keys) — undocumented and unmaintained upstream.
3. **No `send_task` by name.** docs.rs 0.5.5 only exposes typed `send_task(task::new(...))`;
   consuming a Python-sent task is fine (match by header `task` name via
   `#[task(name = "...")]`), but producing "whatever the Python side defines" is not.
4. **Dynamic queues.** `checker_route` values are per-service data; Rust must
   `declare_queue` per name at runtime and the Python worker's queue list must agree
   (broker.rs `declare_queue`). Kombu `Broadcast` queues (fanout-to-all-workers,
   `runner.py:219`) have no rusty-celery equivalent — keep those producer/worker in
   Python.
5. **Datetime / ETA.** `countdown` becomes an ISO-8601 UTC `eta` header (protocol v2
   assumes UTC, docs.celeryq.dev protocol page). Rust side uses chrono `DateTime<Utc>`
   and serializes `to_rfc3339()` (protocol.rs:437) — compatible, but the *worker* must
   honor ETA; rusty-celery retry/ETA support exists (`retry_with_eta`, task/mod.rs:133).
6. **Headers/serialization.** JSON only in production builds (`application/json`);
   yaml/pickle are behind the `extra_content_types` feature (protocol.rs). Set Python
   side `task_serializer='json'` (default) and never pickle args. `timelimit` tuple is
   `(soft, hard)` — note the Python docs typo, flagged in protocol.rs:651-655.
7. **Revoke / worker control.** `revoke()` (`dispatcher.py:172`) works via Celery's
   Redis control commands (`_kombu.binding.celeryev`, worker PIDs) — Python-worker
   only. Rust workers would ignore revokes unless implemented separately.
8. **Maintenance risk.** Last release 2023, changelog stops at v0.5.5, result backends
   still "on-deck" after 3 years (README + CHANGELOG.md, main). Do not build the
   cutover plan on it gaining a backend.

## If we still staged it

- Rust dependency: `celery = { git = "https://github.com/rusty-celery/rusty-celery" }`
  (for #371), never `0.5.5`.
- Declare tasks with `#[task(name = "checker_runner.runner.run_checkerscript")]` and
  params *named and ordered* identically to the Python signature (positional coercion
  is by `T::ARGS` order).
- Broker URL: `redis://:pass@host:port/<REDIS.db+1>` (same as
  `config.celery_url()`); declare every queue the services use.
- Python producer → Rust worker: works (with #371 semantics); Rust worker → Python
  collector: does NOT (no result backend) unless we write `celery-task-meta-*` keys
  ourselves — uncharted, unmaintained upstream.
- RabbitMQ path: the repo prefers AMQP; producer-side interop over AMQP is the
  better-trodden path (first-class transport in rusty-celery), but the result-backend
  blocker stands regardless of broker.

## Bottom line

The only realistic rusty-celery role here is "Rust producer → Python worker", which is
the low-value direction (the controlserver producer is thin; checker_runner is the
CPU-heavy part we want in Rust). Porting the worker with rusty-celery requires
reimplementing the Redis result-backend format and revocation by hand against an
inactive upstream — the wrong foundation. Keep Celery in Python and give Rust its own
task interface (raw Redis lists / HTTP / gRPC behind a thin Celery task, mirroring what
`rust/research/redis.md` already covers).

## Sources

- Repo: `checker_runner/runner.py:200-227`, `controlserver/dispatcher.py:138-173,289`,
  `controlserver/endpoints/pages.py:264-288`, `saarctf_commons/config.py:504-530`.
- https://crates.io/api/v1/crates/celery (0.5.5, 2023-09-25) and
  https://crates.io/api/v1/crates/rusty-celery (does not exist).
- https://github.com/rusty-celery/rusty-celery (README feature table; PR #371; PR #372;
  issue #392; CHANGELOG.md; src/protocol/mod.rs; src/broker/redis.rs; src/task/mod.rs — main branch).
- https://docs.rs/celery/0.5.5/celery/ (typed `send_task`, no backend module).
- https://docs.celeryq.dev/en/stable/internals/protocol.html (v1 vs v2, headers, ISO-8601 UTC eta/expires).
