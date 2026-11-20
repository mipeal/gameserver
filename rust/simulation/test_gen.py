"""Tests for gen_compose.py: TEAMS expansion, static-IP math, VPN gating."""
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, __file__.rsplit("/", 1)[0])
import gen_compose


def run_gen(tmp, teams, sim_vpn):
    env = tmp / "e.env"
    env.write_text(f"TEAMS={teams}\nSIM_VPN={int(sim_vpn)}\n")
    buf = io.StringIO()
    with redirect_stdout(buf):
        rc = gen_compose.main(["--env", str(env), "--check"])
    assert rc == 0
    return buf.getvalue()


class Test(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.gen = lambda teams, vpn: run_gen(self.tmp, teams, vpn)

    def test_teams_3_expands_all_including_nop(self):
        out = self.gen(3, 0)
        self.assertIn("vulnbox-1:", out)   # NOP team still gets a vulnbox
        self.assertIn("vulnbox-3:", out)
        self.assertNotIn("vulnbox-4:", out)
        self.assertIn("ipv4_address: 10.32.3.2", out)
        self.assertIn("subnet: 10.32.0.0/16", out)

    def test_vpn_off_no_privilege(self):
        out = self.gen(3, 0)
        self.assertNotIn("NET_ADMIN", out)
        self.assertNotIn("sim-init", out)
        self.assertNotIn("/dev/net/tun", out)

    def test_vpn_on_adds_wg_services(self):
        out = self.gen(3, 1)
        self.assertIn("NET_ADMIN", out)
        self.assertIn("sim-init", out)
        self.assertIn("wireguard-sync generate -a $((TEAMS + 1))", out)  # TEAMS + 1 (server key), resolved in-container
        self.assertIn("vpn-status-daemon", out)
        self.assertIn("server-setup.sh", out)

    def test_vpn_on_routes_runners_via_controlserver(self):
        # vpnboard reads one wg interface per team (tt-<id>) on the server
        # side; the controlserver is the sim's router, so checker runners
        # need a route to 10.32.0.0/16 via its static bridge IP
        out = self.gen(3, 1)
        runner = out.split("checker_runner:")[1]
        self.assertIn("NET_ADMIN", runner)
        self.assertIn("10.32.0.0/16 via 10.33.0.250", runner)
        self.assertIn("ipv4_address: 10.33.0.250", out)

    def test_vpn_on_host_route_for_team_ip_replies(self):
        # team IPs (10.32.N.2) live in the tunnels; simnet services that reply
        # to them (submitter, scoreboard) send via the default gw, so the
        # docker host needs a route to the controlserver's static bridge IP —
        # one privileged one-shot covers every container
        out = self.gen(3, 1)
        self.assertIn("host-route:", out)
        self.assertIn("network_mode: host", out)
        self.assertIn('"10.32.0.0/16", "via", "10.33.0.250"', out)

    def test_sim_init_keygen_idempotent(self):
        # sim-init runs on every `up -d`; regenerating keys would break the
        # tunnels of a non-recreated controlserver (peer keys no longer match)
        out = self.gen(3, 1)
        self.assertIn("[ ! -f /run/sim/keys/server.json ]", out)

    def test_vpnboard_needs_net_admin_for_wg_show(self):
        # vpnboard shares the controlserver's netns and runs `wg show tt-<id>`;
        # without NET_ADMIN that fails with "Operation not permitted" and no
        # team ever reports connected
        out = self.gen(3, 1)
        vpnboard = out.split("vpnboard:")[1]
        self.assertIn("NET_ADMIN", vpnboard)

    def test_written_files(self):
        env = self.tmp / "e.env"
        env.write_text("TEAMS=2\nSIM_VPN=1\n")
        rc = gen_compose.main(["--env", str(env), "--out-dir", str(self.tmp)])
        self.assertEqual(rc, 0)
        self.assertTrue((self.tmp / "docker-compose.vulnboxes.yml").exists())
        self.assertTrue((self.tmp / "docker-compose.sim-vpn.yml").exists())


if __name__ == "__main__":
    unittest.main()
