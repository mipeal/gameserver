#!/bin/sh
# Server-side wg bring-up (SIM_VPN=1), run inside the controlserver container
# (needs NET_ADMIN): one wg interface per team — tt-<id>, listen port
# 51820+<id>, address 10.32.<id>.1, one peer = that team's vulnbox
# (10.32.<id>.2). This is the shape vpnboard reads (`wg show tt-<id> dump`,
# the production per-router model). The controlserver is the sim's router:
# ip_forward moves checker/attack traffic between the simnet bridge and the
# tunnels (runners get `ip route add 10.32.0.0/16 via 10.33.0.250`).
# Env: TEAMS, KEYS_DIR (default /run/sim/keys), WG_PORT (51820).
set -e
TEAMS=${TEAMS:?TEAMS required}
KEYS_DIR=${KEYS_DIR:-/run/sim/keys}
WG_PORT=${WG_PORT:-51820}

sysctl -w net.ipv4.ip_forward=1 >/dev/null
PRIV=$(python3 -c "import json;print(json.load(open('$KEYS_DIR/server.json'))['private'])")
KEYFILE=$(mktemp)
printf '%s' "$PRIV" > "$KEYFILE"

t=1
while [ "$t" -le "$TEAMS" ]; do
    PUB=$(python3 -c "import json;print(json.load(open('$KEYS_DIR/team$t.json'))['public'])")
    IF=tt-$t
    ip link add "$IF" type wireguard 2>/dev/null || true
    wg set "$IF" listen-port $((WG_PORT + t)) private-key "$KEYFILE" \
        peer "$PUB" allowed-ips 10.32.$t.2/32
    ip addr add 10.32.$t.1/32 dev "$IF" 2>/dev/null || true
    ip link set "$IF" up
    ip route replace 10.32.$t.2/32 dev "$IF" 2>/dev/null || true
    t=$((t + 1))
done
rm -f "$KEYFILE"
echo "wg up: tt-1..tt-$TEAMS, ports $((WG_PORT + 1))..$((WG_PORT + TEAMS))"
