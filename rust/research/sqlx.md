# SQLAlchemy/Alembic → sqlx (Postgres)

## 1. What the migration chain actually does

- 40 revisions in `migrations/versions/` (root `4a67c6f412ac`, head `ca2c4a596673` "firstblood is now integer").
- They create/maintain exactly 12 tables: `teams`, `team_logos`, `services`, `team_points`,
  `team_rankings`, `team_traffic_stats`, `submitted_flags`, `checker_results`, `logmessages`,
  `checker_filesystem`, `checker_files`, `ticks`. Later revisions are small column/index churn
  (e.g. `d3f46a7baef6` renames round→tick, `4225a3d6edbd` converts 6 timestamp columns to timestamptz).
- `migrations/env.py` autogenerates against `controlserver.models.Base.metadata` — so **the current
  models file IS the final schema** and is the single source of truth for a baseline.

## 2. Generating the baseline

sqlx migrations are plain SQL files named `<VERSION>_<DESCRIPTION>.sql` (i64 version > 0)
— source: docs.rs `sqlx::migrate` module. Procedure:

1. `alembic upgrade head` on a scratch DB, then dump schema only:
   `pg_dump --schema-only --no-owner --no-privileges > rust/migrations/0001_baseline.sql`.
2. `cd rust && sqlx migrate add 0001_baseline` and paste the dump into the created file
   (sqlx-cli: `sqlx migrate add <name>` creates `migrations/<timestamp>_<name>.sql`;
   `sqlx migrate run` applies pending files — source: sqlx-cli README).
3. Verify: run `sqlx migrate run` on a second scratch DB and `pg_dump --schema-only`-diff it
   against a live migrated prod DB before trusting it.
4. Run at startup with the embedded migrator: `sqlx::migrate!().run(&pool)` (default dir `./migrations`,
   features `macros` + `migrate`; applied state incl. checksums lives in `_sqlx_migrations`).
   Caveat: sqlx verifies stored checksums — never edit `0001_baseline.sql` after any DB has run it.

## 3. Offline / compile-time query checking

- Use `sqlx::query_as!` / `query!` macros; to compile without a DB run `cargo sqlx prepare` once
  with `DATABASE_URL` set, check the generated `.sqlx/` directory into VCS, then build with
  `DATABASE_URL` unset (source: docs.rs `query!` macro, "Offline Mode"). Set `SQLX_OFFLINE=true`
  to force offline even with a URL present (source: sqlx-cli README).
- CI: `cargo sqlx prepare --check` exits nonzero if `.sqlx` is stale (sqlx-cli README).
- All commands read `DATABASE_URL` from env or `.env` (sqlx-cli README).

## 4. Column type mapping (from `controlserver/models.py`)

| SQLAlchemy | Postgres | Rust (sqlx `postgres`, `chrono`+`json` features) |
|---|---|---|
| `Integer` | `int4` | `i32` |
| `SmallInteger` (team_id/service_id/…) | `int2` | `i16` |
| `BigInteger` (traffic stats) | `int8` | `i64` |
| `Float` | `float8` | `f64` |
| `Boolean` | `bool` | `bool` |
| `String(n)` | `varchar` | `String` |
| `LargeBinary` (logos, checker files) | `bytea` | `Vec<u8>` |
| `JSON` (`services.runner_config`, `checker_results.data`) | `json` (NOT jsonb — SQLAlchemy `JSON` on PG emits `JSON`; keep it in the baseline) | `serde_json::Value` or `Json<T>` (docs.rs postgres types: `Json<T>`/`serde_json::Value` ↔ JSON, JSONB) |
| `TIMESTAMP(timezone=True)` | `timestamptz` | `chrono::DateTime<Utc>` (or `DateTime<Local>`) |
| `func.now()` server defaults | `DEFAULT now()` | carried over verbatim in baseline SQL |

No Postgres enums, no ARRAY columns anywhere in the schema.

## 5. Pool + retry (equivalent of `retry_on_sql_error`)

- `saarctf_commons/db_utils.py` wraps ~15 functions (scoring, scoreboard, runner, vpn daemon,
  attempts=2..40, sleep 0.5–1.5s) and retries on any `SQLAlchemyError`/`ConnectionResetError`.
- Rust equivalent: `PgPoolOptions::new().max_connections(60)` (models.py uses `pool_size=50,
  max_overflow=10`) `.acquire_timeout(...)`; `Pool::begin()` per logical unit
  (docs.rs `Transaction`: auto-rollback on drop if not committed).
- Retry helper (hand-written, ~20 lines): match `sqlx::Error` variants `Io(_)`,
  `IsConnectionClosed`, `PoolTimedOut`, `Database(e)` with a connection-failure sqlstate class
  (e.g. `08*`, `57P01..57P05`); sleep; retry whole function. Same semantics as the Python
  decorator: retry only full units that re-do their reads, never partial work inside a failed tx.

## 6. ORM semantics that must be hand-ported

- **No identity map / unit of work.** `session.query().filter()...first()` + attribute writes +
  `session.commit()` becomes explicit `UPDATE ... WHERE` or `INSERT ... ON CONFLICT DO UPDATE`
  via `sqlx::query`. The two `upsert()` helpers in models.py map 1:1 to Postgres
  `ON CONFLICT ON CONSTRAINT <name> DO UPDATE SET ...` (they were dialect-specific already).
- **No lazy loading.** Only one eager-load site in app code (`joinedload` in
  `controlserver/endpoints/checker_results.py`); `Team.points` / `TeamPoints.team` relationships
  become explicit JOINs in `query_as!`.
- **`efficient_insert` bypasses SQLAlchemy** and uses a raw psycopg2 cursor with
  `INSERT ... SELECT unnest(%(list)s)` (submitted_flags, checker_results, team_points,
  team_traffic_stats). sqlx equivalent: `UNNEST($1)` with bound `Vec<i16>`/`Vec<i32>`/`Vec<f64>`
  params, or `QueryBuilder::push_batch`. 11 call sites total.
- **Scoped sessions** (`scoped_session`, `session.remove()` on Flask teardown, `db_session_2()`
  context) become passing `&Pool` or `&mut Transaction` to `query_as!` methods
  (`fetch_one/fetch_optional/fetch_all` accept `&Pool`, `&mut Transaction` — docs.rs `query_as!`).

## 7. Semantic traps

- **Autocommit/transaction scope**: `sessionmaker(autocommit=False)` opens an implicit
  transaction on the first statement and holds it until commit/rollback. A sqlx `&Pool` executes
  each statement in its own implicit transaction. Read-modify-write sequences that relied on that
  (scoring updates, tick accounting) MUST wrap in `pool.begin()`; forgetting one is a data race,
  not a compile error.
- **NULL**: `server_default=text('NULL')` nullable columns (`services.checker_script_dir`,
  `checker_results.message/time/output/data/finished`, teams VPN timestamps, `logmessages.level`…) →
  `Option<T>` fields. `query_as!` enforces `Option` for nullable columns at compile time
  (docs.rs `query_as!`: "If a column may be NULL, the corresponding field's type must be wrapped in Option<_>").
- **Timestamps**: the `4225a3d6edbd` migration exists precisely because naive `timestamp` broke —
  do not regress to `TIMESTAMP WITHOUT TIME ZONE` in the baseline. Bind `DateTime<Utc>`;
  `TIMESTAMP` (naive) would need `NaiveDateTime`.
- **Integer overflow**: ids mix `int2` (team_id in most tables) and `int4` (teams.id) — bind the
  exact width or Postgres errors. Rust arithmetic on `i32` panics (debug) / wraps (release) where
  Python has bignums — score/counter arithmetic should go through checked ops or i64.
- **`sum(bigint)`**: `query_sum()`/`query_sum_lite()` aggregate `BigInteger` columns; Postgres
  `SUM(int8)` returns `numeric`, which sqlx decodes as `rust_decimal::Decimal`/`BigDecimal`
  (with the decimal feature), not `i64` — decode via `BigDecimal` or cast in SQL
  (`::int8` at your own overflow risk).
- **`SELECT *`** in macros is discouraged (column order varies) — enumerate columns,
  as the models already do for the Lite variants (docs.rs `query_as!`).

## Sources

- docs.rs sqlx 0.9: `query!` (Offline Mode), `query_as`/`query_as!`, `migrate!`, `migrate` module
  (`MigrationSource`, `<VERSION>_<DESCRIPTION>.sql`), `Transaction` (commit/rollback/auto-rollback),
  `Pool::begin`, `PoolOptions::acquire_timeout`, postgres types JSON table — fetched via Context7
  (`/websites/rs_sqlx_sqlx`).
- sqlx-cli README (github.com/launchbadge/sqlx, main): `sqlx database create`, `sqlx migrate
  add/run/revert/info`, `cargo sqlx prepare [--check]`, `SQLX_OFFLINE=true`.
- Repo: `/Users/mmpeal/Developer/personal/gameserver/migrations/versions/*`, `migrations/env.py`,
  `controlserver/models.py`, `saarctf_commons/db_utils.py`.
