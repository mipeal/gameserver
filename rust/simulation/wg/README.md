# Simulation WireGuard plumbing

SIM_VPN=1 gives every vulnbox container a real WireGuard tunnel to the
controlserver container. The checker traffic to `team.ip` (10.32.<team>.2)
then flows through the tunnel — `wg-quick down` (or a killed vulnbox) makes
the team genuinely offline. SIM_VPN=0 skips all of this; vulnboxes are plain
bridge peers.

## Why not wireguard-sync's sync loop

`wireguard-sync` fetches interfaces/peers from the **controlserver API**
(`/api/router/...`), which the Rust controlserver does not implement yet
(later wave). The sim therefore uses wireguard-sync only for **key
pregeneration** (`generate -a N`), and sets both wg endpoints up by hand
(`wg/server-setup.sh`, `wg/entrypoint.sh`).

## Key flow

1. **sim-init** (one-shot, Task 6): `wireguard-sync generate -a $((TEAMS + 1))`
   with `KEYSTORE_PATH=/run/sim/wg/keystore.json`. The keystore JSON is
   `{"<id>": ["<public>", "<private>"], ...}`; ids 1..TEAMS are the teams and
   **TEAMS+1 is reserved as the server keypair** (no interface-id key is
   needed because the sync loop never runs).
2. `split-keystore.py` splits it into `/run/sim/keys/team<id>.json`
   (`{"public", "private"}`) and `/run/sim/keys/server.json`, on the volume
   shared with the vulnboxes and the controlserver.
3. **controlserver** (SIM_VPN=1): `server-setup.sh` creates `wg0`
   (address `10.32.250.1` = `network.vpn_host`, listen port 51820), adds one
   peer per team with `allowed-ips 10.32.<team>.2/32`, and routes
   `10.32.0.0/16` into `wg0`.
4. **vulnbox**: `vulnbox/entrypoint.sh` sources `wg/entrypoint.sh` when its
   key file exists → `wg0` with address `10.32.<team>.2/32`, peer =
   server public key, `Endpoint = controlserver:51820`,
   `AllowedIPs = 10.32.0.0/16`, `PersistentKeepalive = 15`.

Privileges: controlserver container needs `NET_ADMIN` + `/dev/net/tun`;
vulnboxes need `NET_ADMIN` (SIM_VPN=1 only).

## Verifying a handshake

```sh
docker exec <vulnbox-team-2> ping -c1 10.32.250.1     # through the tunnel
docker exec <controlserver> wg show wg0               # latest handshake per peer
```

Down/up cycle: `wg-quick down wg0 && wg-quick up wg0` on a vulnbox, or just
restart the container — the next checker round flags the team offline, then
online again.
