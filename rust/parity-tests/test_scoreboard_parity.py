"""Wire-format parity: the Rust scoreboard generator's demo output vs. the live
saarCTF scoreboard fixtures (rust/parity-tests/fixtures/live/).

Schema equality = same key sets at every object level and the same JSON type
for every value (arrays checked against their first element). Values themselves
may differ — the demo runs different data than the CTF.

Run:  pytest rust/parity-tests/test_scoreboard_parity.py -v
"""

import json
import re
from pathlib import Path

import pytest

HERE = Path(__file__).parent
LIVE = HERE / "fixtures" / "live"
DEMO = Path("/tmp/saarctf-demo/scoreboard/api")


def shape(v):
    """Recursive schema fingerprint: dict of key->shape, or JSON type name;
    lists use the shape of their first element."""
    if isinstance(v, dict):
        return {k: shape(val) for k, val in v.items()}
    if isinstance(v, list):
        return [shape(v[0])] if v else "empty-list"
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, (int, float)):
        return "number"
    return "string"


def schema_matches(a, b, path="$"):
    """(message, path) of first difference, or None."""
    if isinstance(a, dict) and isinstance(b, dict):
        # id-keyed maps (scoreboard_teams.json) may hold different team sets;
        # compare entry shapes only
        if all(k.isdigit() for k in a.keys() | b.keys()) and a and b:
            return schema_matches(a[next(iter(a))], b[next(iter(b))], f"{path}.<entry>")
        for k in a.keys() | b.keys():
            if k not in a:
                return (f"live has extra key", f"{path}.{k}")
            if k not in b:
                return (f"demo has extra key", f"{path}.{k}")
        for k in a.keys() & b.keys():
            d = schema_matches(a[k], b[k], f"{path}.{k}")
            if d:
                return d
        return None
    ta, tb = type(a), type(b)
    if isinstance(a, list) and isinstance(b, list):
        if a and b:
            return schema_matches(a[0], b[0], f"{path}[0]")
        return None
    if ta is bool or tb is bool:
        ok = ta is tb
    elif ta in (int, float) and tb in (int, float):
        ok = True  # int vs float is fine across generators
    else:
        ok = (ta is tb) or (ta is dict and tb is dict)
    if not ok:
        return (f"live={ta.__name__} demo={tb.__name__}", path)
    if isinstance(a, dict) and isinstance(b, dict):
        return None
    return None


ENDPOINTS = {
    "current": "scoreboard_current.json",
    "teams": "scoreboard_teams.json",
    "service_stats": "scoreboard_service_stats.json",
}


def load_pair(rel):
    live_path = LIVE / rel
    demo_path = DEMO / rel
    if not live_path.exists():
        pytest.skip(f"no live fixture {rel}")
    if not demo_path.exists():
        pytest.skip(f"demo output missing (daemon not running): {demo_path}")
    return json.loads(live_path.read_text()), json.loads(demo_path.read_text())


@pytest.mark.parametrize("name,rel", sorted(ENDPOINTS.items()))
def test_endpoint_schema_parity(name, rel):
    live, demo = load_pair(rel)
    assert schema_matches(live, demo) is None


def test_round_schema_parity():
    live_rounds = sorted(
        LIVE.glob("scoreboard_round_*.json"),
        key=lambda p: int(p.stem.rsplit("_", 1)[1]),
    )
    demo_rounds = sorted(
        DEMO.glob("scoreboard_round_*.json"),
        key=lambda p: int(p.stem.rsplit("_", 1)[1]),
    )
    assert demo_rounds, "demo daemon produced no rounds"
    # newest ticks on both sides (skip round_-1 placeholders)
    live, demo = (
        json.loads(p.read_text()) for p in (live_rounds[-1], demo_rounds[-1])
    )
    assert schema_matches(live, demo) is None


def test_team_schema_parity():
    live, demo = load_pair("scoreboard_team_1.json")
    assert schema_matches(live, demo) is None


def _demo_api_missing():
    return not DEMO.exists()


requires_demo = pytest.mark.skipif(_demo_api_missing(), reason="demo api/ missing")


@requires_demo
def test_firstblood_confirmed_shape():
    """first_blood entries carry the same fields live and in the demo."""
    live, demo = load_pair(ENDPOINTS["current"])  # ensure both exist
    def fb_counts(base):
        p = sorted(
            Path(base).glob("scoreboard_round_*.json"),
            key=lambda x: int(x.stem.rsplit("_", 1)[1]),
        )[-1]
        data = json.loads(p.read_text())
        return [
            sorted(fb.keys())
            for svc in data["services"]
            for fb in svc.get("first_blood", [])
        ]
    live_fbs = fb_counts(LIVE)
    demo_fbs = fb_counts(DEMO)
    if live_fbs and demo_fbs:
        assert live_fbs[0] == demo_fbs[0]
    # "?" frozen attackers is a valid value on both sides
    for svc in json.loads(p_live.read_text())["services"] if (p_live := sorted(LIVE.glob("scoreboard_round_*.json"), key=lambda x: int(x.stem.rsplit("_", 1)[1]))[-1]) else []:
        assert isinstance(svc["attackers"], (int, str))


LIVE_ROUND_RE = re.compile(r"scoreboard_round_(-?\d+)\.json$")


@pytest.mark.parametrize(
    "path", [LIVE / "scoreboard_current.json"], ids=["current"]
)
def test_live_current_parses(path):
    data = json.loads(path.read_text())
    assert data["current_tick"] >= -1
    assert data["state"] in (1, 2, 3, "STOPPED", "SUSPENDED", "RUNNING")


def test_all_live_fixtures_parse():
    files = sorted(LIVE.glob("*.json"))
    assert len(files) >= 500, f"expected the full fixture capture, found {len(files)}"
    for p in files:
        assert json.loads(p.read_text()) is not None


@requires_demo
def test_demo_tick_consistency():
    """The Rust generator's round files agree with its current-state tick."""
    cur = json.loads((DEMO / "scoreboard_current.json").read_text())
    tick = cur["scoreboard_tick"]
    rnd = json.loads((DEMO / f"scoreboard_round_{tick}.json").read_text())
    assert rnd["tick"] == tick
