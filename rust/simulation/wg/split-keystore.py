#!/usr/bin/env python3
"""Split wireguard-sync's keystore.json into per-team key files + server key.

Input:  /run/sim/wg/keystore.json = {"<id>": ["<public>", "<private>"], ...}
        produced by `wireguard-sync generate -a $((TEAMS + 1))` — ids 1..TEAMS
        are teams, TEAMS+1 is reserved as the server keypair (the sim never
        runs wireguard-sync's sync loop, so no interface-id key is needed).
Output: /run/sim/keys/team<id>.json = {"public": ..., "private": ...}
        /run/sim/keys/server.json = {"public": ..., "private": ...}
"""
import json
import os
import sys

keystore_path = sys.argv[1] if len(sys.argv) > 1 else "/run/sim/wg/keystore.json"
teams = int(os.environ["TEAMS"])
keys_dir = sys.argv[2] if len(sys.argv) > 2 else "/run/sim/keys"
os.makedirs(keys_dir, exist_ok=True)

keystore = json.load(open(keystore_path))
for team_id in range(1, teams + 1):
    pub, priv = keystore[str(team_id)]
    with open(f"{keys_dir}/team{team_id}.json", "w") as f:
        json.dump({"public": pub, "private": priv}, f)
pub, priv = keystore[str(teams + 1)]
with open(f"{keys_dir}/server.json", "w") as f:
    json.dump({"public": pub, "private": priv}, f)
print(f"split: {teams} team keys + server key -> {keys_dir}")
