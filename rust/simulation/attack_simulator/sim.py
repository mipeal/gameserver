#!/usr/bin/env python3
"""Attack simulator sidecar: steals real flags from vulnboxes and submits
them to the flag submission server.

Flags are only ever taken from vulnbox read responses (checker-stored, i.e.
validly signed) — never fabricated, so submitted state is honest by
construction. The submitter validates HMAC server-side; the simulator needs
no key.

Env: SIM_ATTACK_RATE (0.0-1.0, default 0.25; 0 disables submissions),
TEAMS (default 8), TICK_SECONDS (default 30), SUBMITTER_ADDR (host:port,
default submitter:31337), VULNBOX_IP_PREFIX (default 10.32), NOP_TEAM_ID
(default 1, never attacked).
"""

import http.client
import os
import random
import socket
import sys
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


class Simulator:
    def __init__(self, teams, rate, tick_seconds, submitter, vulnbox_prefix,
                 vulnbox_port=None, nop_team_id=1):
        self.teams = [t for t in teams if t != nop_team_id]
        self.rate = rate
        self.tick_seconds = tick_seconds
        self.submitter = submitter  # (host, port)
        self.vulnbox_prefix = vulnbox_prefix
        self.vulnbox_port = vulnbox_port  # None -> per-service port
        self.nop_team_id = nop_team_id
        self.submitted_log = []  # (service_id, victim, tick, flag)

    def _steal(self, victim, service_id, port, tick):
        """Read stored flags from a vulnbox — the same path checkers use."""
        host = f"{self.vulnbox_prefix}.{victim}" if self.vulnbox_port is None else self.vulnbox_prefix
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

    def _submit(self, attacker, flags):
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
        """One tick of attacks: rate-proportioned theft from random victims."""
        submitted = 0
        for service_id, port in SERVICES:
            victims = [t for t in self.teams if random.random() < self.rate]
            for victim in victims:
                flags = self._steal(victim, service_id, port, tick)
                if not flags:
                    continue
                # attribute to a random attacker team (any team incl. the
                # victim itself would be odd; pick another, falling back)
                attackers = [t for t in self.teams if t != victim] or [victim]
                attacker = random.choice(attackers)
                ok = self._submit(("127.0.0.1", 0) if False else self.submitter, flags)
                submitted += ok
                self.submitted_log.append((service_id, victim, tick, flags[0]))
                print(f"stole {len(flags)} flags from team {victim} service "
                      f"{service_id}, submitted {ok} as team {attacker}", flush=True)
        return submitted

    def run(self):
        tick = env_int("SIM_START_TICK", 0)
        while True:
            time.sleep(self.tick_seconds)
            tick += 1
            self.tick(tick)


def main():
    rate = env_float("SIM_ATTACK_RATE", 0.25)
    teams = list(range(1, env_int("TEAMS", 8) + 1))
    addr = os.environ.get("SUBMITTER_ADDR", "submitter:31337")
    host, _, port = addr.rpartition(":")
    submitter = (host or "submitter", int(port or 31337))
    print(f"attack simulator: rate={rate} teams={teams} submitter={submitter}", flush=True)
    Simulator(
        teams=teams,
        rate=rate,
        tick_seconds=env_int("TICK_SECONDS", 30),
        submitter=submitter,
        vulnbox_prefix=os.environ.get("VULNBOX_IP_PREFIX", "10.32"),
        nop_team_id=env_int("NOP_TEAM_ID", 1),
    ).run()


if __name__ == "__main__":
    main()
