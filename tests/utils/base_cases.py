import os
import unittest
from pathlib import Path

from saarctf_commons.config import config, load_default_config_file

config_basis = {
    "scoreboard_path": "/dev/shm/scoreboard",
    "vpnboard_path": "/dev/shm/vpnboard",
    "checker_packages_path": "/dev/shm/packages",
    "logo_input_path": "/dev/shm/logo_input_path",
    "flower_url": "http://localhost:5555/",
    "secret_flags": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "external_timer": False,
    "scoring": {
        "flags_rounds_valid": 10,
        "nop_team_id": 1,
    },
    "network": {
        "game": "127.0.0.0/16",
        "__ip_syntax": ["number", "or list", ["a", "b", "c"], "= ((team_id / a) mod b) + c"],
        "vulnbox_ip": [127, [200, 256, 32], [1, 200, 0], 2],
        "testbox_ip": [127, [200, 256, 32], [1, 200, 0], 3],
        "gateway_ip": [127, [200, 256, 32], [1, 200, 0], 1],
        "__range_syntax": ["number", "or list", ["a", "b", "c"], "= ((team_id / a) mod b) + c", "/range"],
        "team_range": [127, [200, 256, 32], [1, 200, 0], 0, 24],
        "gameserver_ip": "10.13.0.2",
        "vpn_host": "10.13.0.1",
        "vpn_peer_ips": [127, [200, 256, 48], [1, 200, 0], 1],
        "gameserver_range": "10.32.250.0/24",
    },
}
config_file: Path = Path(__file__).absolute().parent.parent.parent / "config.test.yaml"
os.environ["CTF_CONFIG"] = str(config_file)


class TestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        super().setUpClass()
        load_default_config_file(config_file, config_basis)
        assert config.CONFIG_FILE.name == "config.test.yaml"
