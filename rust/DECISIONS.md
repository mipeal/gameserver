## ADR-014: saarctf_commons — plain Rust library, golden-file equivalence (no PyO3 bridge)
Status: accepted (2026-11-19)
Context: The bridge analysis keeps everything that matters in `Config` (a 40-attribute dataclass) and the
stateful infrastructure (redis pool, logging/metrics setup) out of the PyO3 surface (`js_only`); the
remaining exports are side-effecting one-liners whose recorded cases would be "Config not initialized"
noise. No Python service ever calls Rust here: Python services keep Python saarctf_commons during the
staged cutover (ADR-006), Rust services use the Rust crate directly.
Decision: `saarctf-commons-rs` is a plain workspace library crate, not a bridge unit. Equivalence is
verified by golden tests: the Python original's `Config.load_from_file(...).to_dict()`, network math
(`team_id_to_*`, `network_ip_to_id`) and Influx line formatting are captured as JSON goldens from
fixture configs (yaml + json), and the Rust port must reproduce them exactly. Redis connection helper,
sql retry helper, logging setup and metrics writers are ported directly and exercised in the wave-1
service replays.
Consequences: This unit has no `rust-anything gate` verdict; its proof is `cargo test` against the
goldens. The scaffolded bridge crate files are replaced by the library.

## ADR-015: checker pipeline on Redis Streams — worker writes results, dispatcher collects incompletes
Status: accepted (2026-11-19)
Context: ADR-012 removes Celery. The Python pipeline used Celery groups: dispatch saves a
GroupResult ref per tick, workers write SUCCESS/FAILURE rows themselves, and the dispatcher's
collect phase maps every group member's Celery state (PENDING/RETRY→REVOKED, STARTED→TIMEOUT,
FAILURE→TIMEOUT/CRASHED) into `checker_results`.
Decision: Redis Streams with consumer groups replace Celery queues. Per tick the dispatcher
XADDs one message per (team, service) into the service's route stream and stores
`dispatcher:order:{tick}` / `dispatcher:ref:{tick}` (min/max entry id) as before. Workers
XREADGROUP, run the checker (subprocess or in-process), XACK, and upsert the result row —
same as the Python worker did. The dispatcher's collect phase no longer reads a result
backend: it queries `checker_results` for the tick and fills in what is missing — rows that
never appeared become REVOKED ("Not started before the tick ended") unless an in-flight
marker (worker XADDs `checker:running:{tick}` member heartbeat) says TIMEOUT. Revocation of
unstarted tasks is a per-tick redis flag `dispatcher:revoke:{tick}` that workers check
before starting a message (Celery had revoke-on-not-started).
Consequences: The DB-visible semantics (statuses, upserts, messages) stay identical; the
in-flight Celery states are reconstructed from DB + redis flags. `dispatcher:ref:{tick}` now
holds {stream, first_id, last_id, count} JSON. The DelayingCeleryDispatcher's spread-out
scheduling becomes per-message `XADD` with a delay payload the worker sleeps for (or is
scheduled by the dispatcher loop, whichever is simpler at implementation time).

## ADR-017: checker subprocess program becomes Rust (embedded CPython); controlserver/ Python deleted
Status: accepted (2026-11-20)
Context: After the cutover, the only Python in the runtime path is the checker subprocess
bootstrap (`checker_runner/runners/saarctf.py` + `checker_execution.py` + `package_loader.py`,
spawned per task by the Rust worker) plus the checker-script ecosystem it runs (gamelib SDK,
saarctf_commons). The bootstrap keeps 2,372 lines of dead `controlserver/` Python alive
(models.Service row read, db_filesystem, init_database). The user chose "migrate to pure rust"
for the bootstrap (overriding the keep-Python recommendation).
Decision: The worker keeps spawning one subprocess per task (RLIMIT_AS, kill-on-timeout and
MemoryError isolation are process-level; the Python original also used a discrete subprocess).
Only the program inside changes: a Rust `checker-exec` binary embedding CPython via PyO3.
It takes the service config (name, flag_ids, interface file/class) via argv from the worker
instead of reading the DB, loads checker modules from the package folder on disk (the worker
preloads packages; no DB fallback), and runs the same lifecycle (initialize_team, check_integrity,
store_flags, retrieve_flags, finalize_team) with the same output contract: console output, then
SEPARATOR + "status|message", exit 0. The embedded interpreter only drives Python objects
(gamelib ServiceInterface, handle_checker_exceptions) — checker scripts stay Python (ADR-013).
`checker_runner/demo_checker.py` stays for the worker's `pendingtest` special.
Consequences: `checker_runner/` Python (except demo_checker) and `controlserver/` Python are
deleted after gate; `controlserver/static/less` moves to the image build path. The original
bootstrap is kept until the port's record/diff is green, then deleted (recoverable from git).
Python scripts/ originals stay as recording reference (no recorded parity cases exist for the
30 saarctf-scripts-rs bins).
### ADR-017 implementation record (2026-11-20)
`rust/checker-exec/`: ~30-line Rust launcher (PyO3 0.30, `auto-initialize`, abi3-py39) +
`src/shim.py` (~110 lines, embedded via include_str!) holding the orchestration that must run
inside CPython (Team/ServiceConfig construction, lifecycle calls, handle_checker_exceptions
mapping, traceback printing). Worker (`runners.rs`) spawns `checker-exec <package> <script>
<service_id> <team_id> <tick> <name> <flag_ids>`; `TaskMessage` carries `service_name` +
`flag_ids` from the dispatcher's Service row (serde defaults keep old messages deserializable).
The package-less branch (pendingtest → `checker_runner.demo_checker:TimeoutService`) uses
`importlib.import_module`, matching the original's empty-package path.
Verification: recorded the original against a scratch sim DB (11 cases, 2x each, noise-
identified); the port matches all 10 representable cases 1:1 — status|message line, stdout
order (gamelogger lines, separator), stderr exception lines. `missing_service` (CRASHED "not
found in DB") is unrepresentable by design: the config comes from argv, the worker owns the
Service row. Accepted console differences (stderr side only): traceback file paths/line
numbers differ (embedded shim is not a real file, so frames print without source lines), and
merged stdout/stderr interleaving differs (the embedded interpreter is not finalized, so
stdout is flushed explicitly at the end of main; the original flushed at process exit). The
DB-fallback one-shot "Package <hash> loaded" line is gone (packages load from disk only).
Also deleted with user approval: `controlserver/` Python entirely (less sources moved to
`rust/controlserver-rs/less`; css_build path updated; the image keeps
/opt/controlserver/static/css/index.css for the scoreboard generator); checker_runner/
wrapper code (celery_cmd, checker_execution, package_loader, runner, user_agents, runners/).
`scripts/` and `sample_files/` dropped from the runtime image (ops use saarctf-scripts-rs);
they stay in the repo as reference — note that scripts importing controlserver are now
import-broken by design. `ci/` scripts still reference the deleted flask app (controlserver/
app.py) and cannot run until the CI harness is reworked for the Rust stack.

### ADR-017 addendum: DB package materialization moved into the worker (2026-11-20)
Sim end-to-end exposed a gap the disk-only recording harness could not see: with packages loading
from disk only, nothing materialized them — the original did `PackageLoader.ensure_package_exists`
(DBFilesystem → /tmp/packages) inside the subprocess. Ported as `runners::ensure_package_exists`
(checker_filesystem + checker_files → `<packages_path>/<package>`, >500 KB files symlinked from the
LFS dir, `.tmp`-dir + rename for atomicity). The worker calls it before spawning checker-exec;
a load error returns CRASHED, matching what the original subprocess crashed with. The concurrent
first-load race (original: FileLock) resolves as "already loaded" once the target folder exists.
Verified live: sim stack ticks 624+ all 9 results (3 services x 3 teams) SUCCESS; the vpnboard
static assets (css/favicon) that copy_static_files needs moved to rust/vpnboard-rs/static/.
