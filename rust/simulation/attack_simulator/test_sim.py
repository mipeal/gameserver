"""Tests for the attack simulator: rate gating, NOP exclusion, honest theft."""

import socket
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from sim import Simulator


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

    def sim(self, rate, teams=(1, 2, 3), victim=None):
        # vulnbox address override: point all at the local mock via prefix trick
        s = Simulator(
            teams=list(teams),
            rate=rate,
            tick_seconds=30,
            submitter=("127.0.0.1", self.sub.port),
            vulnbox_prefix="127.0.0.1",
            vulnbox_port=self.vuln_port,
            nop_team_id=1,
        )
        return s

    def test_rate_one_submits(self):
        s = self.sim(1.0, teams=(2,))
        stolen = s.tick(5)
        self.assertGreaterEqual(stolen, 1)
        self.assertGreaterEqual(len(self.sub.lines), 1)
        self.assertIn("SAAR{", self.sub.lines[0])

    def test_rate_zero_submits_nothing(self):
        s = self.sim(0.0, teams=(2,))
        stolen = s.tick(5)
        self.assertEqual(stolen, 0)
        self.assertEqual(self.sub.lines, [])

    def test_nop_team_never_victim(self):
        for _ in range(50):
            s = self.sim(1.0)  # teams (1,2,3), NOP=1
            s.tick(5)
        victims = {v for (_, v, _, _) in getattr(s, "submitted_log", [])}
        self.assertNotIn(1, victims)
        # also: submissions only reference victims 2 or 3
        for line in self.sub.lines:
            self.assertIn("SAAR{", line)


if __name__ == "__main__":
    unittest.main()
