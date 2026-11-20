#!/bin/sh
# Vulnbox wg client bring-up (SIM_VPN=1). Called by vulnbox/entrypoint.sh when
# /run/sim/keys/team$TEAM_ID.json exists. Needs NET_ADMIN.
# Env: TEAM_ID, WG_ADDRESS (this vulnbox's tunnel IP, 10.32.<team>.2/32),
# SERVER_PUBLIC_KEY, SERVER_ENDPOINT (controlserver:51820).
set -e
KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
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
