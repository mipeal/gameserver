# Redis: Rust client choice + Python byte-compatibility

Question: which Rust Redis client + pool replaces the Python `redis` package, and
what data-format traps exist during a staged cutover?

## How the Python side uses Redis (repo survey)
- `saarctf_commons/redis.py`: one global `redis.ConnectionPool` built from
  `**config.REDIS` (`{host, port, db, password}`, see `saarctf_commons/config.py`
  lines ~230/278-297). A custom `Connection.on_connect` runs `CLIENT SETNAME`
  (name with spaces → `_`). `get_redis_connection()` returns `StrictRedis` per
  call over the shared pool.
- Config comment (`config.py` ~505-513): gameserver data lives in `db`, Celery
  broker/results in `db + 1` (`celery_redis_url()`). flushdb both on reset
  (`scripts/reset_ctf.py`).
- **No TTL anywhere**: no `expire`/`setex`/`pexpire` call sites in the codebase
  (only the word "expires" for DB columns / celery result_expires). Keys live
  until `flushdb`.
- Key/value encodings observed (all consumers read raw bytes back and decode
  manually — Python client is *not* in `decode_responses` mode):
  - ints stored as strings: `timing:currentRound`, `round:{tick}:start/end/time`
    (`redis.set(key, str(value))` in `timer.py:redis_set_and_publish`), read
    with `int(redis.get(...))` (`endpoints/pages.py:398`, `runners/eno.py:103`).
  - JSON: `dispatcher:order:{tick}` = `json.dumps(combination_ids)`
    (`dispatcher.py:68`), read with `json.loads` (same file). Only internal
    producer/consumer, but see trap below.
  - sets: `network:banned`, `network:permissions` — team ids as strings
    (`sadd`/`smembers`/`srem`, `vpncontrol.py`), decoded via `int(b.decode())`
    (`scoring/scoreboard.py:375`).
  - sentinel-ish values: `network:bannedteam:{id}` = `until_tick or ""`
    (`vpncontrol.py:76`) — **empty string** means "no end tick".
  - `None` sentinel: `redis_set_and_publish` (`timer.py:40-49`) maps
    `None → b"None"` — the 4 ASCII bytes `None`, not nil. Consumers read it
    back as bytes and compare.
  - raw strings: `network:state`, `dispatcher:ref:{tick}`, flag ids
    `custom_flag_ids:{service}:{tick}:{team}:{idx}` (str values,
    `runners/eno.py:117-130`), `components:worker` (read-only liveness probe,
    `checker_runner/runner.py:39`).
- Pub/sub: `publish` + `pubsub().subscribe(...)` + `listen()` on channels
  `timing:*` (state, currentRound, roundStart/End/Time, stopAfterRound,
  startAt, openVulnboxAccessAt, master, scoreboard_tick), `network:state`,
  `network:ban/unban/add_permission/remove_permission`. Payloads are the same
  string/int values as the keys (`timer.py:47-49` publishes the key name as
  channel and value as payload). `pubsub_numsub("timing:master")` used to count
  live master timers (`timer.py:208`).
- Other commands: `incr("runner:eno:task_id")` (`runners/eno.py:110`),
  `keys(pattern)` incl. bytes pattern `b"services:*"`
  (`scripts/reset_ctf_to_round.py:57`), `mget`, `delete`, `client_list`
  (`endpoints/pages.py:140`), `flushdb`.

## Recommendation: `redis` crate + `deadpool-redis`

Use `redis` (crates.io, current major 1.x, e.g. 1.5) with the `tokio-comp` /
`aio` features, pooled via `deadpool-redis` (0.23).

Why this over `fred`:
- `redis` is the de-facto standard, API surface maps 1:1 onto what this repo
  needs (`cmd()`/AsyncCommands, `get_async_pubsub`, `incr`, `sadd`, `keys`).
  `fred` is a fine alternative (built-in pool, reconnection semantics) but is a
  larger, opinionated API; nothing in this codebase needs what `fred` adds
  (cluster/sentinel/centralized client tracking). Ponytail rule: pick the
  smaller thing that covers the list above.
- Pooling: `deadpool-redis::Pool` wraps `MultiplexedConnection`s, get-per-unit-
  of-work, recycled on drop — direct analogue of the Python
  `ConnectionPool` + `StrictRedis(connection_pool=...)` pattern
  ([docs.rs/deadpool-redis](https://docs.rs/deadpool-redis/latest/deadpool_redis/)).

Setup: `deadpool_redis::Config::from_url("redis://host:port/db")` →
`create_pool(Some(Runtime::Tokio1))` → `pool.get().await` per unit of work;
values as strings (`set("timing:currentRound", "12")`).

CLIENT SETNAME equivalent: deadpool's `Hooks::on_get` (or first command after
`pool.get()`) — a multiplexed connection carries the name for its lifetime, so
set it once per pooled connection.

Pub/sub: do **not** use pool connections (Python's `pubsub()` is also a
dedicated connection). Use `client.get_async_pubsub()`, `subscribe(&[...])`,
then `into_on_message()` stream; `msg.get_payload::<Vec<u8>>()` gives bytes.
`pubsub_numsub` → `redis::cmd("PUBSUB").arg("NUMSUB").arg("timing:master")`.

## Byte-identical value formats (staged cutover rules)

Both sides will run against the same Redis DB during cutover, so values must be
byte-identical or at least parse-identical on both ends.

1. **Everything is bytes.** Python client returns `bytes`; consumers call
   `.decode()` / `int(...)` themselves (`dispatcher.py:106`, `eno.py:104`). In
   Rust use `Vec<u8>`/`&[u8]` at the boundary (`FromRedisValue for Vec<u8>`),
   strict `String::from_utf8` where Python used `.decode()` (default UTF-8).
   Ints stay strings on the wire: `set(key, format!("{}", tick))`,
   read via `str::parse`.
2. **`json.dumps` default separators.** Python `json.dumps(x)` (no kwargs, as in
   `dispatcher.py:68`) emits `", "` and `": "` separators: `[1, 2, 3]`,
   `{"a": 1}`. `serde_json::to_string` emits compact `[1,2,3]`. Both round-trip
   through `json.loads`/`serde_json`, so for `dispatcher:order:*` byte equality
   is *not* required — accept the difference consciously. If any value is ever
   compared or hashed as a string (grep says none are), compact vs spaced JSON
   would break it.
3. **`None` → the string `"None"`** (`timer.py:43-44`), not nil and not `""`.
   Rust must write `b"None"` for the Python-None case. Conversely `nil`
   (key absent) is Python `None` / Rust `Option::None` — different from the
   stored `"None"` sentinel; do not conflate when porting `timer.py` readers.
4. **Empty string is a real value**: `network:bannedteam:{id}` may be `""`
   (`vpncontrol.py:76`) — Rust sees `Some(vec![])`, not nil.
5. **Ints as strings, never redis integers**: only `runner:eno:task_id` uses
   `INCR`; everything else is GET/SET of decimal strings. Python `int()` accepts
   whitespace (`int(b" 12 ")`); `i64::from_str` does not — no current value has
   whitespace, but keep parse errors loud.
6. **Set members are strings**: `SADD network:banned team_id` stores the decimal
   string; Rust must `sadd(key, team_id.to_string())` and parse on read
   (`scoreboard.py:375` does `int(b.decode())`).
7. **Key names are plain UTF-8**, f-string formatted; `keys(b"services:*")`
   (`reset_ctf_to_round.py:57`) works as `keys("*")` string arg.

## Semantic traps

- **TTL precision**: no TTLs exist in this codebase — nothing to port. If added
  later, Python `set(..., ex=n)` is second-granularity; redis-rs
  `set_ex(Duration)` matches.
- **Pub/sub message types**: Python `listen()` yields the `subscribe`
  confirmation messages (`type: 'subscribe'`) before the first real event;
  redis-rs `on_message()` also yields subscribe confirmations as `Msg` with no
  payload — filter or ignore empty payloads like the Python code implicitly does.
- **Resubscription after reconnect**: redis-py pubsub auto-resubscribes on
  reconnect; redis-rs plain `get_async_pubsub` does not. Either rebuild the
  subscription on stream error, or use `ConnectionManager` with
  `set_automatic_resubscription()` + push sender
  ([redis-rs connection-manager docs](https://docs.rs/redis/latest/redis/aio/struct.ConnectionManager.html)).
  For the CTF timer, losing one pubsub message stalls the round.
- **Bytes vs UTF-8**: Python `.decode()` raises on non-UTF8; use strict
  `String::from_utf8` + error to match, not `from_utf8_lossy`.
- **CLIENT SETNAME**: name normalization `' ' → '_'` is app-side
  (`saarctf_commons/redis.py:5-9`); replicate so `client_list` diagnostics
  (`pages.py:140`) show the same names.
- **DB separation**: gameserver db = `config.REDIS["db"]`, Celery db = db+1
  (`config.py:509-513`). The Rust port never touches db+1. Password goes in the
  URL: `redis://:password@host:port/db`.

## Sources

- Repo files cited inline, all under /Users/mmpeal/Developer/personal/gameserver/:
  `saarctf_commons/redis.py`, `saarctf_commons/config.py`,
  `controlserver/{timer,dispatcher,vpncontrol,flag_id_file}.py`,
  `controlserver/endpoints/pages.py`, `controlserver/scoring/{scoreboard,scoreboard_process}.py`,
  `checker_runner/runner.py`, `checker_runner/runners/eno.py`, `scripts/*.py`.
- [docs.rs/redis](https://docs.rs/redis) (via ctx7 `/redis-rs/redis-rs`):
  `get_async_pubsub`, `PubSub::subscribe/on_message/into_on_message`,
  `ConnectionManagerConfig::set_automatic_resubscription`.
- [docs.rs/deadpool-redis](https://docs.rs/deadpool-redis/latest/deadpool_redis/):
  `Config::from_url`, `create_pool(Some(Runtime::Tokio1))`, pooled `Connection`
  = `MultiplexedConnection` wrapper, `PoolConfig::max_size`, `Timeouts`.
