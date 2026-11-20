#!/bin/sh
# Server-side wg bring-up (SIM_VPN=1), run inside the controlserver container
# (needs NET_ADMIN): wg0 at the vpn_host address, one peer per team.
# Env: TEAMS, KEYS_DIR (default /run/sim/keys), WG_IFACE (wg0),
# WG_ADDR (10.32.250.1/32 = config network.vpn_host), WG_PORT (51820).
set -e
TEAMS=${TEAMS:?TEAMS required}
KEYS_DIR=${KEYS_DIR:-/run/sim/keys}
WG_IFACE=${WG_IFACE:-wg0}
WG_ADDR=${WG_ADDR:-10.32.250.1/32}
WG_PORT=${WG_PORT:-51820}

PRIV=$(python3 -c "import json;print(json.load(open('$KEYS_DIR/server.json'))['private'])")
KEYFILE=$(mktemp)
trap 'rm -f "$KEYFILE"' EXIT
printf '%s' "$PRIV" > "$KEYFILE"

ip link add "$WG_IFACE" type wireguard 2>/dev/null || true
wg set "$WG_IFACE" listen-port "$WG_PORT" private-key "$KEYFILE"
ip addr add "$WG_ADDR" dev "$WG_IFACE" 2>/dev/null || true

t=1
while [ "$t" -le "$TEAMS" ]; do
    PUB=$(python3 -c "import json;print(json.load(open('$KEYS_DIR/team$t.json'))['public'])")
    wg set "$WG_IFACE" peer "$PUB" allowed-ips "10.32.$t.2/32"
    t=$((t + 1))
done

ip link set "$WG_IFACE" up
# route checker traffic (team.ip = 10.32.N.2) into the tunnel
ip route replace 10.32.0.0/16 dev "$WG_IFACE" 2>/dev/null || true
echo "wg up: $WG_IFACE $WG_ADDR port $WG_PORT, $TEAMS peers"
