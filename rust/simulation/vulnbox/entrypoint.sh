#!/bin/sh
# Vulnbox entrypoint: wg bring-up (SIM_VPN=1), then the service shims.
set -e

# sim-init writes the keys shortly after bring-up; if we start before it
# finishes, wait (with SIM_VPN=0 the file never appears and we skip).
KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
if [ ! -f "$KEY_FILE" ]; then
	n=0
	until [ -f "$KEY_FILE" ] || [ "$n" -ge 12 ]; do
		sleep 5
		n=$((n + 1))
	done
fi
if [ -f "$KEY_FILE" ]; then
	. /sim/wg/entrypoint.sh
fi

exec python3 services.py
