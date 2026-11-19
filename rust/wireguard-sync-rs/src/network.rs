//! `wireguard_sync/network_lib.py` — wireguard interface and peer management.
//!
//! pyroute2 (netlink) is replaced by the standard `ip` and `wg` tools:
//! - create interface:  `ip link add tun{id} type wireguard` + addr + up
//! - listen port:       `wg set tun{id} listen-port {port}`
//! - private key:       piped via stdin (`wg set tun{id} private-key /dev/stdin`)
//! - peers:             `wg show tun{id} allowed-ips` / `wg set ... peer ...`
//!
//! Linux only; errors map like netlink's (missing device → InterfaceDoesNotExist).

use crate::api::Interface;
use rand_core::OsRng;
use std::process::Stdio;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPair {
    pub public_key: String,
    pub private_key: String,
}

/// base64 of 32 raw bytes, like Python's `base64.b64encode`.
fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn decode_key(s: &str) -> Result<[u8; 32], String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("ValueError: not a base64 key: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "ValueError: key is not 32 bytes".to_string())
}

/// `generate_key_pair`.
pub fn generate_key_pair() -> KeyPair {
    let private = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&private);
    KeyPair {
        public_key: b64(public.as_bytes()),
        private_key: b64(private.as_bytes()),
    }
}

/// The public key a private key derives (used to compare against the running
/// interface, which `wg` will not hand back the private key for).
pub fn public_key_from_private(private_key: &str) -> Result<String, String> {
    let bytes: [u8; 32] = decode_key(private_key)?;
    Ok(b64(PublicKey::from(&StaticSecret::from(bytes)).as_bytes()))
}

pub fn get_interface_name(interface: &Interface) -> String {
    format!("tun{}", interface.id)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Peer {
    pub public_key: String,
    pub cidr: String,
}

impl Peer {
    /// `Peer.from_api`.
    pub fn from_api(api: &crate::api::ApiPeer) -> Peer {
        Peer {
            public_key: api.key_slot.public_key.clone(),
            cidr: api.cidr.clone(),
        }
    }
}

async fn run(cmd: &mut tokio::process::Command) -> Result<std::process::Output, String> {
    cmd.output().await.map_err(|e| e.to_string())
}

async fn run_ok(mut cmd: tokio::process::Command, what: &str) -> Result<(), String> {
    let out = run(&mut cmd).await?;
    if !out.status.success() {
        return Err(format!(
            "OSError: {what} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

async fn run_stdout(mut cmd: tokio::process::Command, what: &str) -> Result<String, String> {
    let out = run(&mut cmd).await?;
    if !out.status.success() {
        return Err(format!(
            "OSError: {what} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `remove_interface`.
pub async fn remove_interface(ifname: &str) -> Result<(), String> {
    let mut c = tokio::process::Command::new("ip");
    c.args(["link", "del", ifname]);
    run_ok(c, "ip link del").await
}

/// `initialize_interface` — idempotent: create the interface with its address
/// and listen port if it does not exist, then set the private key if changed.
/// Returns the interface name.
pub async fn initialize_interface(
    interface: &Interface,
    private_key: &str,
) -> Result<String, String> {
    let ifname = get_interface_name(interface);
    let exists = tokio::process::Command::new("ip")
        .args(["link", "show", &ifname])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        tracing::info!("Creating wireguard interface {ifname}");
        let mut c = tokio::process::Command::new("ip");
        c.args(["link", "add", &ifname, "type", "wireguard"]);
        run_ok(c, "ip link add").await?;
        let mut c = tokio::process::Command::new("ip");
        c.args(["address", "add", &interface.cidr, "dev", &ifname]);
        run_ok(c, "ip address add").await?;
        let mut c = tokio::process::Command::new("ip");
        c.args(["link", "set", &ifname, "up"]);
        run_ok(c, "ip link set up").await?;
        let mut c = tokio::process::Command::new("wg");
        c.args(["set", &ifname, "listen-port", &interface.port.to_string()]);
        run_ok(c, "wg set listen-port").await?;
    }
    // private key: compare the derived public key (wg does not expose privkeys)
    let current_pub = run_stdout(
        {
            let mut c = tokio::process::Command::new("wg");
            c.args(["show", &ifname, "public-key"]);
            c
        },
        "wg show public-key",
    )
    .await;
    let want_pub = public_key_from_private(private_key)?;
    if current_pub.as_deref() != Ok(want_pub.as_str()) {
        tracing::info!("Setting private key for {ifname}");
        let mut c = tokio::process::Command::new("wg");
        c.args(["set", &ifname, "private-key", "/dev/stdin"])
            .stdin(Stdio::piped());
        let mut child = c.spawn().map_err(|e| e.to_string())?;
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().ok_or("OSError: no stdin")?;
        stdin
            .write_all(private_key.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.write_all(b"\n").await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        drop(stdin);
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("OSError: wg set private-key failed".to_string());
        }
    }
    Ok(ifname)
}

/// `get_peers` — `wg show <if> allowed-ips`: one line per peer, tab-separated
/// `pubkey\tcidr cidr ...`; the first cidr is the peer's cidr (Python takes
/// `allowed_ips[0]["addr"]`).
pub async fn get_peers(wg_interface: &str) -> Result<std::collections::HashSet<Peer>, String> {
    let mut c = tokio::process::Command::new("wg");
    c.args(["show", wg_interface, "allowed-ips"]);
    let out = run_stdout(c, "wg show allowed-ips").await?;
    let mut peers = std::collections::HashSet::new();
    for line in out.lines() {
        let mut parts = line.split('\t').filter(|p| !p.is_empty());
        let (Some(public_key), Some(cidr)) = (parts.next(), parts.next()) else {
            tracing::warn!("Invalid peer for {wg_interface}. line={line:?}");
            continue;
        };
        peers.insert(Peer {
            public_key: public_key.to_string(),
            cidr: cidr.to_string(),
        });
    }
    Ok(peers)
}

/// `add_peer`.
async fn add_peer(ifname: &str, peer: &Peer) {
    let mut c = tokio::process::Command::new("wg");
    c.args([
        "set",
        ifname,
        "peer",
        &peer.public_key,
        "allowed-ips",
        &peer.cidr,
    ]);
    if let Err(e) = run_ok(c, "wg set peer").await {
        tracing::error!(
            "Failed to set wg peer {} {}: {e}",
            peer.public_key,
            peer.cidr
        );
    }
}

/// `remove_peer`.
async fn remove_peer(ifname: &str, peer: &Peer) {
    let mut c = tokio::process::Command::new("wg");
    c.args(["set", ifname, "peer", &peer.public_key, "remove"]);
    if let Err(e) = run_ok(c, "wg set peer remove").await {
        tracing::error!("Failed to remove wg peer {}: {e}", peer.public_key);
    }
}

/// `sync_peers` — remove-then-add the peer difference, verify afterwards.
async fn sync_peers(ifname: &str, interface: &Interface) -> Result<(), String> {
    let requested: std::collections::HashSet<Peer> =
        interface.peers.iter().map(Peer::from_api).collect();
    let existing = get_peers(ifname).await?;

    // First remove...
    for peer in existing.difference(&requested) {
        remove_peer(ifname, peer).await;
    }
    // then add, otherwise there might be conflicting addresses!
    for peer in requested.difference(&existing) {
        add_peer(ifname, peer).await;
    }

    let new_peers = get_peers(ifname).await?;
    if new_peers != requested {
        tracing::error!("Peer mismatch after sync on {ifname}");
    }
    Ok(())
}

/// `sync` — a missing device maps to `InterfaceDoesNotExist: <ifname>`.
pub async fn sync(interface: &Interface) -> Result<(), String> {
    let ifname = get_interface_name(interface);
    match sync_peers(&ifname, interface).await {
        Ok(()) => Ok(()),
        Err(e) if e.contains("No such device") || e.contains("does not exist") => {
            Err(format!("{}{}", crate::ERR_INTERFACE_DOES_NOT_EXIST, ifname))
        }
        Err(e) => {
            tracing::error!("Failed to sync peers for {ifname}");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_key_derives_public_key() {
        let kp = generate_key_pair();
        assert_eq!(
            public_key_from_private(&kp.private_key).unwrap(),
            kp.public_key
        );
        assert!(public_key_from_private("not base64").is_err());
    }
}
