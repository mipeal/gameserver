//! Port of `vpnboard/wg_status.py`. pyroute2's `WireGuard.info(ifname)` is
//! replaced by `wg show <ifname> dump` (tab-separated, first line is the device,
//! one line per peer). A missing interface exits non-zero → Err, mapping the
//! NetlinkError → ValueError the daemon catches.

use chrono::{DateTime, TimeZone, Utc};

// INTERFACE_NAME = 'tt-'  # for ctfroute
pub const INTERFACE_NAME: &str = "tt-";

#[derive(Debug, Clone)]
pub struct WgPeerStatus {
    pub address: String, // CIDR
    pub connected: bool,
    pub last_handshake: Option<DateTime<Utc>>,
    pub remote_addr: Option<String>,
    pub remote_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct WgStatus {
    pub team_id: i32,
    pub peers: Vec<WgPeerStatus>,
}

impl WgStatus {
    pub fn connection_count(&self) -> i32 {
        self.peers.iter().filter(|p| p.connected).count() as i32
    }

    /// `get_peer_for` — IPv4Address in IPv4Network(cidr, strict=False).
    pub fn get_peer_for(&self, ip: &str) -> Option<&WgPeerStatus> {
        self.peers
            .iter()
            .find(|peer| ipv4_in_cidr(ip, &peer.address))
    }
}

/// Minimal IPv4 CIDR containment (strict=False: host bits may be set).
fn ipv4_in_cidr(ip: &str, cidr: &str) -> bool {
    fn parse_ip4(s: &str) -> Option<u32> {
        let mut parts = s.split('.');
        let mut v = 0u32;
        for _ in 0..4 {
            let o: u8 = parts.next()?.parse().ok()?;
            v = (v << 8) | o as u32;
        }
        parts.next().is_none().then_some(v)
    }
    let Some(ip) = parse_ip4(ip) else {
        return false;
    };
    let (net, mask) = match cidr.split_once('/') {
        Some((n, m)) => {
            let Ok(bits) = m.parse::<u32>() else {
                return false;
            };
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            (parse_ip4(n), mask)
        }
        None => (parse_ip4(cidr), u32::MAX),
    };
    match net {
        Some(net) => (ip & mask) == (net & mask),
        None => false,
    }
}

/// `get_wireguard_status` for `tt-{team_id}`.
pub async fn get_wireguard_status(team_id: i32) -> Result<WgStatus, String> {
    let ifname = format!("{INTERFACE_NAME}{team_id}");
    let out = tokio::process::Command::new("wg")
        .args(["show", &ifname, "dump"])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        // netlink: NetlinkError → ValueError(e)
        return Err(format!(
            "ValueError: wg show {} failed: {}",
            ifname,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut result = WgStatus {
        team_id,
        peers: Vec::new(),
    };
    let now = Utc::now();
    for line in String::from_utf8_lossy(&out.stdout).lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        // KeyError/IndexError peers are skipped with an eprint
        let (Some(handshake), Some(allowed)) = (f.get(2), f.get(7)) else {
            eprintln!("short wg dump line for {ifname}: {line:?}");
            continue;
        };
        let ts: i64 = handshake.parse().unwrap_or(0);
        let last_handshake = if ts > 0 {
            Utc.timestamp_opt(ts, 0).single()
        } else {
            None
        };
        // handshake happens every 120sec roughly. We consider a team offline iff it misses two handshakes
        let connected = last_handshake
            .map(|h| h > now - chrono::Duration::seconds(245))
            .unwrap_or(false);
        let (remote_addr, remote_port) = f
            .get(6)
            .filter(|e| **e != "(none)")
            .and_then(|e| e.rsplit_once(':'))
            .map(|(a, p)| {
                (
                    Some(a.trim_matches(['[', ']']).to_string()),
                    p.parse::<u16>().ok(),
                )
            })
            .unwrap_or((None, None));
        result.peers.push(WgPeerStatus {
            // first allowed-ips entry (Python: allowed_ips[0]["addr"])
            address: allowed.split_whitespace().next().unwrap_or("").to_string(),
            connected,
            last_handshake,
            remote_addr,
            remote_port,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_containment() {
        assert!(ipv4_in_cidr("10.32.11.136", "10.32.11.136/32"));
        assert!(ipv4_in_cidr("10.32.11.136", "10.32.11.0/24"));
        assert!(!ipv4_in_cidr("10.32.12.1", "10.32.11.0/24"));
        assert!(ipv4_in_cidr("10.32.11.136", "10.32.11.136"));
        assert!(!ipv4_in_cidr("bad", "10.32.11.0/24"));
    }

    #[test]
    fn peer_lookup() {
        let ws = WgStatus {
            team_id: 1,
            peers: vec![WgPeerStatus {
                address: "10.32.11.0/24".into(),
                connected: true,
                last_handshake: None,
                remote_addr: None,
                remote_port: None,
            }],
        };
        assert!(ws.get_peer_for("10.32.11.5").is_some());
        assert!(ws.get_peer_for("10.32.12.5").is_none());
        assert_eq!(ws.connection_count(), 1);
    }
}
