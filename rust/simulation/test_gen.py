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
        self.assertIn("vpn_status_daemon", out)
        self.assertIn("server-setup.sh", out)

    def test_written_files(self):
        env = self.tmp / "e.env"
        env.write_text("TEAMS=2\nSIM_VPN=1\n")
        rc = gen_compose.main(["--env", str(env), "--out-dir", str(self.tmp)])
        self.assertEqual(rc, 0)
        self.assertTrue((self.tmp / "docker-compose.vulnboxes.yml").exists())
        self.assertTrue((self.tmp / "docker-compose.sim-vpn.yml").exists())


if __name__ == "__main__":
    unittest.main()
