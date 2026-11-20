#!/usr/bin/env python3
"""Per-vulnbox attack loop: this team steals flags from every opponent
vulnbox and submits them to the flag submission server. Submissions carry
the vulnbox's own source IP, so the submitter attributes them to this team
(Config::getTeamIdFromIp) — the real CTF attack model.

Flags are only ever taken from vulnbox read responses (checker-stored, i.e.
validly signed) — never fabricated, so submitted state is honest by
construction. The submitter validates HMAC server-side; we need no key.

Env: SIM_ATTACK_RATE (0.0-1.0, default 0.25; 0 disables), TEAM_ID (this
team), TEAMS (opponents are 1..TEAMS except self and NOP), NOP_TEAM_ID
(default 1, never attacked and never attacks), TICK_SECONDS (default 30),
SUBMITTER_ADDR (host:port, default submitter:31337), SCOREBOARD_ADDR
(default scoreboard:80), VULNBOX_IP_PREFIX (default 10.32).
"""

import http.client
import json
import os
import random
import socket
import sys
import threading
import time

SERVICES = [(1, 1337), (2, 8080), (3, 31337)]
FLAG_MARKERS = ("SAAR{",)


def env_float(name, default):
    try:
        return float(os.environ.get(name, default))
    except ValueError:
        return default


def env_int(name, default):
    try:
        return int(os.environ.get(name, default))
    except ValueError:
        return default


def parse_addr(value, default_port):
    host, _, port = value.rpartition(":")
    return (host, int(port or default_port))


class AttackThread(threading.Thread):
    daemon = True

    def __init__(self, team_id, teams, rate, tick_seconds=30,
                 submitter=("submitter", 31337), vulnbox_prefix="10.32",
                 vulnbox_port=None, nop_team_id=1, scoreboard_addr=("scoreboard", 80)):
        super().__init__(name=f"attack-{team_id}")
        self.team_id = team_id
        self.rate = rate
        self.tick_seconds = tick_seconds
        self.submitter = submitter  # (host, port)
        self.vulnbox_prefix = vulnbox_prefix
        self.vulnbox_port = vulnbox_port  # None -> per-service port
        self.nop_team_id = nop_team_id
        self.scoreboard_addr = scoreboard_addr
        # NOP never attacks; nobody attacks the NOP team
        self.opponents = [t for t in teams if t not in (team_id, nop_team_id)]
        self.stolen_log = []  # (service_id, victim, tick, flag)

    def current_tick(self, fallback):
        """The CTF's real tick (scoreboard JSON), so theft targets ticks that
        actually have flags; fallback when the scoreboard is unreachable."""
        if self.scoreboard_addr is None:
            return fallback
        try:
            conn = http.client.HTTPConnection(*self.scoreboard_addr, timeout=3)
            conn.request("GET", "/api/scoreboard_current.json")
            body = conn.getresponse().read()
            conn.close()
            return int(json.loads(body)["current_tick"])
        except (OSError, ValueError, KeyError):
            return fallback

    def _steal(self, victim, service_id, port, tick):
        """Read stored flags from a vulnbox — the same path checkers use."""
        host = f"{self.vulnbox_prefix}.{victim}.2" if self.vulnbox_port is None else self.vulnbox_prefix
        port = self.vulnbox_port or port
        try:
            conn = http.client.HTTPConnection(host, port, timeout=5)
            conn.request("GET", f"/?team_id={victim}&service_id={service_id}&tick={tick}")
            resp = conn.getresponse()
            body = resp.read().decode()
            conn.close()
        except OSError:
            return []
        if resp.status != 200:
            return []
        return [f.strip() for f in body.splitlines()
                if any(m in f for m in FLAG_MARKERS)]

    def _submit(self, flags):
        ok = 0
        try:
            with socket.create_connection(self.submitter, timeout=5) as s:
                for flag in flags:
                    s.sendall((flag + "\n").encode())
                    data = s.recv(64)
                    if b"OK" in data:
                        ok += 1
        except OSError as e:
            print(f"submitter unreachable: {e}", file=sys.stderr, flush=True)
        return ok

    def tick(self, tick):
        """One tick of attacks: rate-proportioned theft from opponents."""
        submitted = 0
        # previous ticks' flags: the current tick's store only fills as
        # checkers run, and the scoreboard's tick view can lag at bring-up —
        # try the two most recent ticks (flags stay valid 10 rounds)
        for service_id, port in SERVICES:
            for victim in self.opponents:
                if random.random() >= self.rate:
                    continue
                for steal_tick in (tick - 1, tick - 2):
                    flags = self._steal(victim, service_id, port, steal_tick)
                    if not flags:
                        continue
                    ok = self._submit(flags)
                    submitted += ok
                    self.stolen_log.append((service_id, victim, steal_tick, flags[0]))
                    print(f"team {self.team_id} stole {len(flags)} flags from team "
                          f"{victim} service {service_id} (tick {steal_tick}), "
                          f"submitted {ok}", flush=True)
                    break
        return submitted

    def run(self):
        tick = env_int("SIM_START_TICK", 0)
        while True:
            time.sleep(self.tick_seconds)
            tick = self.current_tick(fallback=tick + 1)
            self.tick(tick)


def start_from_env(services_started):
    """Spawn the attack thread if enabled (rate > 0, this team is not NOP)."""
    rate = env_float("SIM_ATTACK_RATE", 0.25)
    team_id = env_int("TEAM_ID", 0)
    nop = env_int("NOP_TEAM_ID", 1)
    if rate <= 0 or not team_id or team_id == nop:
        return None
    thread = AttackThread(
        team_id=team_id,
        teams=list(range(1, env_int("TEAMS", 8) + 1)),
        rate=rate,
        tick_seconds=env_int("TICK_SECONDS", 30),
        submitter=parse_addr(os.environ.get("SUBMITTER_ADDR", "submitter:31337"), 31337),
        vulnbox_prefix=os.environ.get("VULNBOX_IP_PREFIX", "10.32"),
        nop_team_id=nop,
        scoreboard_addr=parse_addr(os.environ.get("SCOREBOARD_ADDR", "scoreboard:80"), 80),
    )
    if services_started:
        thread.start()
    print(f"attack loop: team {team_id} rate={rate} opponents={thread.opponents}", flush=True)
    return thread
