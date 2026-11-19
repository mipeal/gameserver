"""Golden generator for saarctf-commons-rs (ADR-014).

Captures the Python original's behavior as JSON goldens that
`tests/golden.rs` replays against the Rust port:
- Config.load_from_file(...).to_dict() per fixture, with a clean environment
  and once with environment overrides
- postgres/celery URL builders
- NetworkConfig team math and network_ip_to_id probes
- Influx line-protocol formatting (values, attributes, timestamps)
- validate() results

Run from the repository root:  python3 rust/saarctf-commons-rs/tests/generate_goldens.py
"""

import io
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

FIXTURES = Path(__file__).resolve().parent / "fixtures"
OUT = Path(__file__).resolve().parent / "goldens"

from saarctf_commons.config import Config  # noqa: E402
from saarctf_commons.metric_utils import (  # noqa: E402
    TelegrafBufferMetricsRecorder,
)

FIXTURE_FILES = [
    "config.sample.yaml",
    "config.sample.json",
    "config_test.yaml",
    "config_norabbit.yaml",
]

ENV_OVERRIDE = {
    "POSTGRES_SERVER": "override-db.example",
    "REDIS_PASSWORD": "override-pw",
    "REDIS_DB": "7",
    "METRICS_PORT": "1234",
    "SECRET_FLAG_KEY": "deadbeef" * 4,
    "RABBITMQ_HOST": "rabbit.example",
}

TEAM_IDS = [0, 1, 2, 3, 5, 10, 31, 40, 63, 64, 100, 127, 128, 255, 256, 257, 1000, 1024, 4095, 65534, 65535, 70000, -1]

INFLUX_CASES = [
    # (values, attributes, ts)
    ({"x": 42}, {}, 1700000000),
    ({"x": -7}, {}, 1700000000),
    ({"b": True}, {}, 1700000000),
    ({"b": False}, {}, 1700000000),
    ({"s": 'hello "world"\n\tü微😀'}, {}, 1700000000),
    ({"s": "plain"}, {}, 1700000000),
    ({"f": 1.5}, {}, 1700000000),
    ({"f": 15.0}, {}, 1700000000),
    ({"f": 1e16}, {}, 1700000000),
    ({"f": 1.5e16}, {}, 1700000000),
    ({"f": 1e-5}, {}, 1700000000),
    ({"f": -0.0}, {}, 1700000000),
    ({"f": 0.1}, {}, 1700000000),
    ({"f": 1 / 3}, {}, 1700000000),
    ({"f": float("nan")}, {}, 1700000000),
    ({"f": float("inf")}, {}, 1700000000),
    ({"f": float("-inf")}, {}, 1700000000),
    ({"f": 3.0, "i": 3}, {}, 1700000000),
    ({"v": 1}, {"team": 5, "service": "name with space"}, 1700000000),
    ({"v": 2.5}, {"ok": True}, 1700000000),
    ({"v": 1}, {}, 1700000000.123456789),
    ({"v": 1}, {}, 0),
    ({"v": 1}, {}, -5),
    ({"v": 1}, {}, datetime.fromtimestamp(1700000000, tz=timezone.utc)),
    ({"v": 1}, {}, datetime.fromtimestamp(1700000000.5, tz=timezone.utc)),
]

# ip probes for network_ip_to_id: one IP inside each probed team network plus
# out-of-range and malformed ones
IP_PROBES = ["10.0.0.0", "10.0.0.1", "10.0.39.255", "10.1.5.7", "10.10.10.10",
             "10.39.200.3", "10.40.0.1", "127.0.0.1", "8.8.8.8", "1.2.3.4"]


def capture_error(f) -> str:
    try:
        return f()
    except (ValueError, KeyError) as e:
        return f"{type(e).__name__}: {e}"


def capture_influx() -> dict:
    buf = io.BytesIO()
    recorder = TelegrafBufferMetricsRecorder(buf)
    lines = []
    for values, attrs, ts in INFLUX_CASES:
        recorder.record_many("test_metric", values, ts, **attrs)
        lines.append(buf.getvalue().decode("utf-8"))
        buf.seek(0)
        buf.truncate(0)
    return {"lines": lines}


def capture_fixture(name: str, env: dict) -> dict:
    keys = list(env) + list(ENV_OVERRIDE) + ["SAARCTF_POSTGRES_USE_SOCKET"]
    saved = {k: os.environ.get(k) for k in keys}
    for k in keys:
        os.environ.pop(k, None)
    os.environ.update(env)
    try:
        cfg = Config.load_from_file(FIXTURES / name)
        result = {
            "to_dict": cfg.to_dict(),
            "urls": {
                "postgres_sqlalchemy": cfg.postgres_sqlalchemy(),
                "postgres_psycopg2": cfg.postgres_psycopg2(),
                "celery_redis_url": cfg.celery_redis_url(),
                "celery_url": capture_error(lambda: cfg.celery_url()),
            },
            "network": capture_network(cfg.NETWORK),
        }
        try:
            cfg.validate()
            result["validate"] = "ok"
        except ValueError as e:
            result["validate"] = str(e)
        # RABBITMQ url only where rabbitmq exists (celery_rabbitmq_url raises otherwise)
        result["urls"]["celery_rabbitmq_url"] = capture_error(cfg.celery_rabbitmq_url)
        return result
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def capture_network(net) -> dict:
    out = {}
    for tid in TEAM_IDS:
        out[str(tid)] = {
            "vulnbox": net.team_id_to_vulnbox_ip(tid),
            "gateway": net.team_id_to_gateway_ip(tid),
            "testbox": net.team_id_to_testbox_ip(tid),
            "network_range": net.team_id_to_network_range(tid),
            "vpn_peers": list(net.team_id_to_vpn_peers(tid)),
        }
    out["ip_to_id"] = {ip: net.network_ip_to_id(ip) for ip in IP_PROBES}
    # one IP from inside each probed team's range
    for tid in TEAM_IDS:
        ip = net.team_id_to_vulnbox_ip(tid)
        out["ip_to_id"][f"vulnbox({tid})"] = net.network_ip_to_id(ip)
    return out


def main() -> None:
    OUT.mkdir(exist_ok=True)
    goldens = {"influx": capture_influx(), "fixtures": {}}
    for name in FIXTURE_FILES:
        goldens["fixtures"][name] = capture_fixture(name, env={})
        goldens["fixtures"][name + "+env"] = capture_fixture(name, env=ENV_OVERRIDE)
    (OUT / "goldens.json").write_text(json.dumps(goldens, indent=1, sort_keys=True))
    print(f"wrote {OUT / 'goldens.json'}")


if __name__ == "__main__":
    main()
