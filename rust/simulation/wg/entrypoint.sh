#!/bin/sh
# Vulnbox wg client bring-up (SIM_VPN=1). Called by vulnbox/entrypoint.sh when
# /run/sim/keys/team$TEAM_ID.json exists. Needs NET_ADMIN.
# Env (all optional): WG_ADDRESS (default 10.32.<team>.2/32),
# SERVER_PUBLIC_KEY (default: public key from /run/sim/keys/server.json),
# SERVER_ENDPOINT (default controlserver:51820).
set -e
KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
WG_ADDRESS=${WG_ADDRESS:-10.32.${TEAM_ID}.2/32}
SERVER_ENDPOINT=${SERVER_ENDPOINT:-controlserver:51820}
SERVER_PUBLIC_KEY=${SERVER_PUBLIC_KEY:-$(python3 -c "import json;print(json.load(open('/run/sim/keys/server.json'))['public'])")}
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
