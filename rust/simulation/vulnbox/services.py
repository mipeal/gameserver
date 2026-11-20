#!/usr/bin/env python3
"""Vulnbox service shims: one HTTP server per fixture service port.

Protocol (same for all three services):
  POST /   form-encoded team_id, service_id, tick, flag (multi-line) -> 200, stored
  GET /?team_id&service_id&tick -> 200, response text contains the stored flags
  GET /    (no params)          -> 200, integrity probe
Flags are keyed (service_id, tick) in memory — a vulnbox restart loses them.
"""

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

PORTS = [int(p) for p in os.environ.get("SIM_PORTS", "1337,8080,31337").split(",")]

# (service_id, tick) -> list[flag]
FLAGS = {}
LOCK = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # quiet
        pass

    def _send(self, code, body=b""):
        self.send_response(code)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parts = parse_qs(self.path.lstrip("/").removeprefix("?"))
        if not parts:
            self._send(200, b"ok")
            return
        key = (int(parts.get("service_id", ["0"])[0]), int(parts.get("tick", ["0"])[0]))
        with LOCK:
            flags = list(FLAGS.get(key, []))
        self._send(200, ("\n".join(flags) + ("\n" if flags else "")).encode())

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        form = parse_qs(self.rfile.read(length).decode())
        try:
            key = (int(form["service_id"][0]), int(form["tick"][0]))
            flags = [f for f in form["flag"][0].splitlines() if f.strip()]
        except (KeyError, IndexError, ValueError):
            self._send(400, b"bad request")
            return
        with LOCK:
            FLAGS.setdefault(key, []).extend(flags)
        self._send(200, json.dumps({"stored": len(flags)}).encode())


def main():
    servers = [ThreadingHTTPServer(("0.0.0.0", p), Handler) for p in PORTS]
    threads = [threading.Thread(target=s.serve_forever, daemon=True) for s in servers]
    for t in threads:
        t.start()
    print(f"vulnbox services up on {PORTS}", flush=True)
    from attack import start_from_env
    start_from_env(services_started=True)
    threads[0].join()


if __name__ == "__main__":
    main()
