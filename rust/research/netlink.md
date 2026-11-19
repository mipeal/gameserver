# pyroute2 → Rust: what the repo actually needs

## 1. Where pyroute2 is used

Only two components import it. `vpn/` itself does **not** (its network work is
iptables subprocess + eBPF maps).

| file | what it does |
|---|---|
| `wireguard-sync/wireguard_sync/network_lib.py:12-13` | imports `NDB`, `WireGuard`, `NetlinkError` |
| `wireguard-sync/wireguard_sync/test_network_lib.py:7` | same in tests |
| `vpnboard/wg_status.py:7-8` | `WireGuard`, `NetlinkError` |
| `vpnboard/vpn_status_daemon.py:126-127` | `WireGuard` (asyncio daemon) |
| `wireguard-sync/pyproject.toml:14` | pins `pyroute2 = "^0.7.12"`; also in top-level `requirements.txt:19`, `pyproject.toml:58-59` |

## 2. Exact netlink operations used

From `wireguard-sync/wireguard_sync/network_lib.py`:

- **Create WireGuard interface** — `ndb.interfaces.create(kind="wireguard", ifname=...)`,
  `link.add_ip(cidr)`, `link.set(state="up")` (network_lib.py:69-71). Idempotent: skipped
  if ifname already in `ndb.interfaces` (:67).
- **Delete interface** — `ndb.interfaces[ifname]` → `link.remove()` (:89-91).
- **WireGuard device config** — `wg.set(ifname, listen_port=...)` (:73) and
  `wg.set(ifname, private_key=...)` (:78), guarded by reading
  `wg.info(ifname)[0]["WGDEVICE_A_PRIVATE_KEY"]` (:75).
- **Add / remove peer** — `wg.set(ifname, peer={"public_key", "allowed_ips", ...})`
  (:104) and `peer={"public_key", "remove": True}` (:118).
- **Read peers** — `wg.info(ifname)` walking `WGDEVICE_A_PEERS` → `WGPEER_A_PUBLIC_KEY`,
  `WGPEER_A_ALLOWEDIPS[0]["addr"]` (:133-149).
- **Error mapping** — `NetlinkError` with `errno.ENODEV` → "interface does not exist" (:162-164).

From `vpnboard/wg_status.py:49-77` (read-only status):

- `wg.info(ifname)` → per-peer `WGPEER_A_ALLOWEDIPS[0]["addr"]`,
  `WGPEER_A_LAST_HANDSHAKE_TIME.tv_sec`, `WGPEER_A_ENDPOINT.addr/port` (also tx/rx
  bytes exist in the payload, see demo dict at wg_status.py:81+). This is the
  team-up/down liveness check for the dashboard.

**Not used via pyroute2:** routes (no `ndb.routes` anywhere), tc/qdisc
(`vpn/manage-trafficstats.py` reads eBPF maps `/sys/fs/bpf/tc/globals/*` via ctypes,
manage-trafficstats.py:156-164 — no pyroute2), nftables. **iptables is subprocess
only**: `vpn/manage-iptables.py:41-62` shells out to `iptables -A/-I/-D/-C/-F`;
`vpn/iptables.sh` likewise. So the Rust netlink surface is: link create/delete,
addr add, link up, and WireGuard genl get/set. That is the whole list.

## 3. Rust replacements

### Recommended stack (one netlink family)

| pyroute2 piece | Rust crate | notes |
|---|---|---|
| `NDB` link create/remove/up, add_ip | `rtnetlink` 0.24.0 | `RouteNetlinkHandle`/`LinkAddRequest` with `LinkUnspec` + `.link_info(InfoKind::Wireguard, ...)`; `AddressAddRequest`; `LinkSetRequest` up/down. Covers link+addr; no `NDB`-style cache needed for this app. |
| `WireGuard` genl get/set | `netlink-packet-wireguard` 0.5.1 + `genetlink` 0.3.1 (or rtnetlink's genl support) | typed `WgDeviceMsg` with `WGDEVICE_A_*` / `WGPEER_A_*` attrs — 1:1 with the constants the Python code already matches on (network_lib.py:75,133-149; wg_status.py:57-70). |
| underlying socket/transport | `netlink-sys` 0.9.0 / `netlink-proto` 0.14.1 | pulled in by rtnetlink; all rust-netlink crates were released together 2026-11-04/15 (crates.io). |
| `cryptography` X25519 keygen (network_lib.py:26-33) | `x25519-dalek` (2.0) + `base64` | trivial; not netlink. |
| async runtime | `tokio` | rtnetlink is async-first (tokio); there is no maintained sync blocking wrapper — wrap calls in `tokio::runtime::Handle::block_on` or make the port async. |

### Alternative for the WireGuard-only parts

- **`wireguard-uapi` 3.0.1** (updated 2025-05-10 per crates.io; docs.rs shows 3.0.1):
  purpose-built get/set of WireGuard devices+peers, kernel backend via genl (depends
  on `neli`) plus an `xplatform` userspace client (docs.rs/wireguard-uapi). If the
  Rust port keeps the same split (netlink for links/addrs, WG config separate), this
  is the least code — but it drags in a *second* netlink stack (`neli`) alongside
  rtnetlink.
- **`wgctrl` 0.1.0** (2026-01-11, 4.8k downloads): young niche crate, not enough
  adoption — skip.
- **`boringtun` 0.7.1**: WireGuard *protocol* (userspace tunnel), not interface management — irrelevant. **`rtnl` 1.0.0** (2024-02, ~2k downloads): thin sync wrapper, stale, skip both.

### iptables

Keep shelling out (`std::process::Command`) — the Python code's only interface to
iptables is subprocess (`manage-iptables.py:41-62`), and `iptables_exists` semantics
(`-C` exit-code check, `manage-iptables.py:56-58`) port 1:1. The `nftables` crate
0.6.3 (2025-08-15, 3.2M downloads) is a maintained option only if the project later
moves to native nftables JSON; it is *not* needed to reach feature parity.

## 4. Semantic traps

1. **Permissions**: every operation here (link create, addr, WG genl set *and get*)
   needs `CAP_NET_ADMIN` on the target netns — pyroute2 fails with `NetlinkError
   EPERM` just like the Rust crates will. The Dockerfile already runs as root
   (`wireguard-sync/docker/Dockerfile:12` installs wireguard-tools/iproute2 into a
   privileged container); keep that, and expect wireguard-uapi's docs/tests to note
   the same requirement (docs.rs/wireguard-uapi).
2. **wg.info() parsing quirk is load-bearing**: the Python code indexes
   `allowed_ips[0]["addr"]` and treats a peer as "the team subnet" — one allowed-IP
   per peer, first wins (network_lib.py:145; wg_status.py:57). Any Rust peer model
   that round-trips *all* allowed-IPs will disagree with the existing REST API state
   (`Peer.from_api`, network_lib.py:53). Reproduce the first-allowed-IP semantics or
   migrate both sides.
3. **NetlinkError → ENODEV mapping** (network_lib.py:162-164) becomes
   `errno::ErrorKind` / `NetlinkError` code match in netlink-packet-*; keep the
   `InterfaceDoesNotExist` distinction, the sync loop depends on it.
4. **Netns**: pyroute2's `NDB()` operates in the process's current netns; nothing in
   the repo creates/enters namespaces. rtnetlink defaults to the same. If someone
   later adds `setns`-based isolation, rtnetlink supports it via `NetlinkRouteProtocol`
   on an explicit socket fd, but nothing today requires it.
5. **Async mismatch**: `wireguard_sync/__main__.py` is asyncio; rtnetlink is
   tokio-native. Interop is easy (block_on per op, as network_lib.py does one-shot
   NDB/WireGuard context managers today), but don't hold a rtnetlink connection
   across a long-lived daemon without reconnect-on-error — pyroute2's NDB caches
   state, rtnetlink does not, so `if ifname not in ndb.interfaces` (:67) becomes an
   actual `link.get()` + match on NotFound.
6. **wireguard-uapi double-stack**: if you pick wireguard-uapi for WG genl you get
   neli as a second netlink implementation in the binary; harmless but larger, and
   its last release (2025-05) is older than the rust-netlink family (2026-11).

## 5. Sources

- Repo usage: paths/lines in sections 1-2 (grep + read, 2026-11-19).
- `rtnetlink` 0.24.0, `netlink-packet-wireguard` 0.5.1, `genetlink` 0.3.1,
  `netlink-sys` 0.9.0, `netlink-proto` 0.14.1, `wireguard-uapi` 3.0.1, `wgctrl`
  0.1.0, `boringtun` 0.7.1, `nftables` 0.6.3, `rtnl` 1.0.0 — crates.io API
  (`crates.io/api/v1/crates/<name>`: max_stable_version, updated_at), fetched
  2026-11-19.
- wireguard-uapi backend/architecture: docs.rs/wireguard-uapi (neli dependency,
  get/set modules, xplatform), fetched 2026-11-19.
