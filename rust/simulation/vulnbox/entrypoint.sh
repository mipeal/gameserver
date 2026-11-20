#!/bin/sh
# Vulnbox entrypoint: optional wg bring-up (SIM_VPN=1), then the service shims.
set -e

KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
if [ -f "$KEY_FILE" ]; then
	# {"private": "...", "public": "..."} — written by sim-init (Task 6)
	. /sim/wg/entrypoint.sh
fi

exec python3 services.py
