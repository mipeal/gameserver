"""Tests for the per-vulnbox attack loop: rate gating, NOP exclusion,
honest theft, tick sync from the scoreboard."""

import socket
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from attack import AttackThread


class MockVulnbox(BaseHTTPRequestHandler):
    stored = {"1": "SAAR{AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA}\n"}

    def log_message(self, *a):
        pass

    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write("".join(MockVulnbox.stored.values()).encode())


class SubmitterCapture(threading.Thread):
    """Captures lines sent to a TCP port, replies [OK]."""

    def __init__(self):
        super().__init__(daemon=True)
        self.lines = []
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(5)
        self.port = srv.getsockname()[1]
        self.srv = srv
        self.start()

    def run(self):
        while True:
            try:
                conn, _ = self.srv.accept()
            except OSError:
                return
            with conn:
                buf = b""
                while True:
                    data = conn.recv(4096)
                    if not data:
                        break
                    buf += data
                    while b"\n" in buf:
                        line, buf = buf.split(b"\n", 1)
                        self.lines.append(line.decode())
                        conn.sendall(b"[OK]\n")


class Test(unittest.TestCase):
    def setUp(self):
        self.sub = SubmitterCapture()
        self.vuln = HTTPServer(("127.0.0.1", 0), MockVulnbox)
        threading.Thread(target=self.vuln.serve_forever, daemon=True).start()
        self.vuln_port = self.vuln.server_address[1]

    def attack(self, team_id, rate, teams=(1, 2, 3)):
        # vulnbox address override: point all at the local mock via prefix trick
        return AttackThread(
            team_id=team_id,
            teams=list(teams),
            rate=rate,
            tick_seconds=30,
            submitter=("127.0.0.1", self.sub.port),
            vulnbox_prefix="127.0.0.1",
            vulnbox_port=self.vuln_port,
            nop_team_id=1,
        )

    def test_steal_targets_team_ip(self):
        # vulnbox IP is 10.32.<team>.2 — the .2 host suffix must be there
        import attack
        captured = {}

        class FakeConn:
            def __init__(self, host, port, timeout=None):
                captured["host"] = host

            def request(self, *a):
                pass

            def getresponse(self):
                class R:
                    status = 200

                    def read(self):
                        return b""

                return R()

            def close(self):
                pass

        orig = attack.http.client.HTTPConnection
        attack.http.client.HTTPConnection = FakeConn
        try:
            a = self.attack(2, 1.0)
            a.vulnbox_port = None
            a.vulnbox_prefix = "10.32"
            a._steal(3, 1, 1337, 9)
        finally:
            attack.http.client.HTTPConnection = orig
        self.assertEqual(captured["host"], "10.32.3.2")

    def test_rate_one_submits(self):
        a = self.attack(2, 1.0)  # opponents = (2, 3) with the default teams
        stolen = a.tick(5)
        self.assertGreaterEqual(stolen, 1)
        self.assertGreaterEqual(len(self.sub.lines), 1)
        self.assertIn("SAAR{", self.sub.lines[0])

    def test_rate_zero_submits_nothing(self):
        a = self.attack(2, 0.0, teams=(2,))
        stolen = a.tick(5)
        self.assertEqual(stolen, 0)
        self.assertEqual(self.sub.lines, [])

    def test_nop_team_never_victim(self):
        for _ in range(50):
            a = self.attack(2, 1.0)  # opponents = (2, 3), NOP=1 excluded
            a.tick(5)
            victims = {v for (_, v, _, _) in a.stolen_log}
            self.assertNotIn(1, victims)
        for line in self.sub.lines:
            self.assertIn("SAAR{", line)

    def test_empty_tick_falls_back_to_older_tick(self):
        # tick-1 store can be empty at bring-up (scoreboard tick lags,
        # vulnbox restart wiped old flags) — must try tick-2 before giving up
        class TickVulnbox(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                if "tick=8" in self.path:
                    body = b"SAAR{BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB}\n"
                else:
                    body = b""
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)

        vuln = HTTPServer(("127.0.0.1", 0), TickVulnbox)
        threading.Thread(target=vuln.serve_forever, daemon=True).start()
        a = self.attack(2, 1.0)
        a.vuln_port = vuln.server_address[1]
        a.tick(10)  # tick-1=9 empty, tick-2=8 has flags
        self.assertGreaterEqual(len(self.sub.lines), 1)

    def test_current_tick_synced_from_scoreboard(self):
        class FakeScoreboard(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                body = b'{"current_tick": 89}'
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        sb = HTTPServer(("127.0.0.1", 0), FakeScoreboard)
        threading.Thread(target=sb.serve_forever, daemon=True).start()
        a = self.attack(2, 1.0)
        a.scoreboard_addr = ("127.0.0.1", sb.server_address[1])
        self.assertEqual(a.current_tick(fallback=5), 89)

    def test_current_tick_falls_back_when_unreachable(self):
        a = self.attack(2, 1.0)
        a.scoreboard_addr = ("127.0.0.1", 1)  # nothing listens here
        self.assertEqual(a.current_tick(fallback=5), 5)

    def test_submit_and_tick_bind_team_ip(self):
        # SIM_VPN=1: the vulnbox's bridge IP (10.33.N.2) attributes to no team
        # — submissions and scoreboard reads must be sent from the team's wg
        # IP (10.32.<team>.2) so the submitter's getTeamIdFromIp sees the team
        captured = {}

        def fake_create_connection(addr, timeout=None, source_address=None):
            captured["submit_src"] = source_address
            raise OSError("stop here")

        orig_cc = socket.create_connection
        socket.create_connection = fake_create_connection
        try:
            a = self.attack(2, 1.0)
            a.source_ip = "10.32.2.2"
            a._submit(["SAAR{X}"])
            self.assertEqual(captured["submit_src"], ("10.32.2.2", 0))
        finally:
            socket.create_connection = orig_cc

        import attack

        captured2 = {}
        orig_http = attack.http.client.HTTPConnection

        class FakeHTTPConnection:
            def __init__(self, *a, source_address=None, **kw):
                captured2["tick_src"] = source_address

            def request(self, *a):
                pass

            def getresponse(self):
                raise OSError("stop here")

            def close(self):
                pass

        attack.http.client.HTTPConnection = FakeHTTPConnection
        try:
            a = self.attack(2, 1.0)
            a.source_ip = "10.32.2.2"
            a.current_tick(fallback=5)
            self.assertEqual(captured2["tick_src"], ("10.32.2.2", 0))
        finally:
            attack.http.client.HTTPConnection = orig_http

    def test_no_source_ip_binds_nothing(self):
        # default (no source_ip): unchanged behavior — no source_address arg
        captured = {}

        def fake_create_connection(addr, timeout=None, source_address=None):
            captured["src"] = source_address
            raise OSError("stop here")

        orig = socket.create_connection
        socket.create_connection = fake_create_connection
        try:
            a = self.attack(2, 1.0)  # source_ip unset
            a._submit(["SAAR{X}"])
            self.assertIsNone(captured["src"])
        finally:
            socket.create_connection = orig


if __name__ == "__main__":
    unittest.main()
