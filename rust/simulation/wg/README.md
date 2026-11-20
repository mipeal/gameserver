# Simulation WireGuard plumbing

SIM_VPN=1 gives every vulnbox container a real WireGuard tunnel to the
controlserver container, and the controlserver becomes the sim's router.
Checker traffic to `team.ip` (10.32.<team>.2) then genuinely flows through
the tunnels — `wg-quick down` (or a killed vulnbox) makes the team offline
on the scoreboard and stops dispatching to it (dispatcher and the
scoreboard's online flag read the live wg state that vpnboard writes).
SIM_VPN=0 skips all of this; vulnboxes are plain bridge peers.

## Traffic model (verified live, TEAMS=3)

- **simnet bridge is 10.33.0.0/16** (SIM_VPN=1); the tunnels own
  10.32.0.0/16. Vulnbox bridge IPs move to 10.33.<team>.2.
- **vulnbox wg0** (`wg/entrypoint.sh`): `Table = off` with
  `AllowedIPs = 10.32.0.0/16, 10.33.0.0/16`, then a manual
  `ip route add 10.32.0.0/16 dev wg0`. The dual AllowedIPs make the
  cryptokey accept packets sourced by checkers, the submitter and the
  scoreboard (10.33.x.x) without touching routes; wg-quick itself would
  fail with "RTNETLINK answers: File exists" if 10.33.0.0/16 got a wg
  route (it must keep its connected eth0 route for DNS/endpoint traffic).
  **No SNAT anywhere** — SNATing checker sources to 10.32.<team>.1 breaks
  the submitter/scoreboard asymmetric return path (responses get rewritten
  and clients RST).
- **checker runners**: `ip route add 10.32.0.0/16 via 10.33.0.250` (the
  controlserver's static bridge IP); the controlserver forwards between the
  bridge and the tunnels.
- **submissions / scoreboard reads from a vulnbox** (attack loop): bound to
  the team IP `10.32.<team>.2` (`source_address`) — the submitter's
  `Config::getTeamIdFromIp` only maps `team_range` 10.32.<team>.x/24, so a
  bridge-sourced submission attributes to nobody. Replies go back over the
  host route: the SIM_VPN overlay adds a one-shot
  `ip route replace 10.32.0.0/16 via 10.33.0.250` on the **docker host**
  (`host-route` service, privileged `network_mode: host`), covering every
  simnet container's default-gw path. Re-applied on each `simctl up`; after
  a docker VM restart re-run `simctl up` once.

## Why one interface per team

`vpnboard` (vpn-status-daemon `--check-wg`) models the production deployment:
one wg interface per router, `tt-<team_id>`, one daemon reading
`wg show tt-<id> dump` per team. The sim therefore creates `tt-1..tt-N` on
the controlserver (which plays the routers), not a single `wg0` — otherwise
vpnboard can never see a peer and every team would stay "online" forever.
vpnboard shares the controlserver's netns and needs `NET_ADMIN` for
`wg show`. Dispatcher + scoreboard only treat a team as online when vpnboard
has written `wg_vulnbox_connected` (seed sets all `vpn_connected` FALSE —
live wg state is the single source of truth).

## Key flow

1. **sim-init** (one-shot, idempotent): `wireguard-sync generate -a
   $((TEAMS + 1))` only when `/run/sim/keys/server.json` is absent —
   regenerating on every `up -d` would rotate all keys and break the tunnels
   of a non-recreated controlserver. The keystore JSON is
   `{"<id>": ["<public>", "<private>"], ...}`; ids 1..TEAMS are the teams and
   **TEAMS+1 is reserved as the server keypair** (the sim never runs
   wireguard-sync's sync loop, so no interface-id key is needed).
2. `split-keystore.py` splits it into `/run/sim/keys/team<id>.json`
   (`{"public", "private"}`) and `/run/sim/keys/server.json`, on the volume
   shared with the vulnboxes and the controlserver.
3. **controlserver** (SIM_VPN=1, privileged): `server-setup.sh` creates one
   `tt-<id>` per team (address `10.32.<id>.1/32`, listen port `51820+<id>`,
   one peer = that team's public key with `allowed-ips 10.32.<id>.2/32`),
   turns on `net.ipv4.ip_forward`, and routes `10.32.<id>.2/32` into
   `tt-<id>`.
4. **vulnbox**: `vulnbox/entrypoint.sh` waits up to 60 s for its key file,
   sources `wg/entrypoint.sh`, then starts the services.

Privileges: controlserver is `privileged` (ip_forward is not namespaced);
vpnboard and vulnboxes and runners need `NET_ADMIN` (SIM_VPN=1 only).

## Verifying a handshake

```sh
docker exec <vulnbox-team-2> ping -c1 10.32.3.1        # through the tunnel(s)
docker exec <controlserver> wg show                    # tt-<id> + recent handshakes
docker exec <postgres> psql -U saarctf -d saarctf -tAc \
  "SELECT id, wg_vulnbox_connected FROM teams"         # vpnboard's verdicts
```

Down/up cycle: `wg-quick down wg0 && wg-quick up wg0` on a vulnbox, or just
restart the container — vpnboard marks the team offline after two missed
~120 s handshakes (measured: **~4 min** down → `wg_vulnbox_connected=f`),
and back online quickly after reconnect (measured: **~35 s** — the 15 s
keepalive rekeys the tunnel immediately, then the next vpnboard cycle
flips the flag).

## Ops gotchas (recreating a running controlserver)

`docker compose ... up -d --force-recreate vpnboard` (or any recreation of
the controlserver) restarts the shared netns — wg sessions reset and the
vulnboxes' neighbor caches go stale, so tunnels can sit black-holed. Fix:
`ip neigh flush 10.33.0.250` inside the vulnboxes; keepalives (15 s)
re-handshake within seconds. Normal `simctl up` teardowns bring everything
back together and don't hit this.
