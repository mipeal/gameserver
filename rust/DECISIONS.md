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
