#!/bin/sh
# Vulnbox wg client bring-up (SIM_VPN=1). Called by vulnbox/entrypoint.sh when
# /run/sim/keys/team$TEAM_ID.json exists. Needs NET_ADMIN.
# Env (all optional): WG_ADDRESS (default 10.32.<team>.2/32),
# SERVER_PUBLIC_KEY (default: public key from /run/sim/keys/server.json),
# SERVER_ENDPOINT (default controlserver:51820+<team> — the server runs one
# wg interface per team, tt-<id>, each on its own listen port).
#
# Table = off + manual team route: AllowedIPs must include 10.33.0.0/16 (the
# simnet bridge) so the tunnel accepts packets sourced by checkers, the
# submitter and the scoreboard — but that prefix must KEEP its connected
# route on eth0 (wg-quick would otherwise fail with "File exists" and break
# DNS/endpoint routing). Simnet-bound egress stays on eth0; services that
# reply to the team IP are reached via the host route added by the sim-vpn
# compose's host-route one-shot.
set -e
KEY_FILE="/run/sim/keys/team${TEAM_ID}.json"
WG_ADDRESS=${WG_ADDRESS:-10.32.${TEAM_ID}.2/32}
SERVER_ENDPOINT=${SERVER_ENDPOINT:-controlserver:$((51820 + TEAM_ID))}
SERVER_PUBLIC_KEY=${SERVER_PUBLIC_KEY:-$(python3 -c "import json;print(json.load(open('/run/sim/keys/server.json'))['public'])")}
PRIV=$(python3 -c "import json;print(json.load(open('$KEY_FILE'))['private'])")
cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address = ${WG_ADDRESS}
PrivateKey = ${PRIV}
Table = off

[Peer]
PublicKey = ${SERVER_PUBLIC_KEY}
Endpoint = ${SERVER_ENDPOINT}
AllowedIPs = 10.32.0.0/16, 10.33.0.0/16
PersistentKeepalive = 15
EOF
wg-quick up wg0
ip route add 10.32.0.0/16 dev wg0
