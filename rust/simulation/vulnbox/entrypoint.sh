#!/bin/sh
# Vulnbox entrypoint: optional wg bring-up (SIM_VPN=1), then the service shims.
set -e

KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
if [ -f "$KEY_FILE" ]; then
	# {"private": "...", "public": "..."} — written by sim-init (Task 6)
	PRIV=$(python3 -c "import json;print(json.load(open('$KEY_FILE'))['private'])")
	cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address = ${WG_ADDRESS}
PrivateKey = ${PRIV}

[Peer]
PublicKey = ${SERVER_PUBLIC_KEY}
Endpoint = ${SERVER_ENDPOINT}
AllowedIPs = 10.32.0.0/16
PersistentKeepalive = 15
EOF
	wg-quick up wg0
fi

exec python3 services.py
