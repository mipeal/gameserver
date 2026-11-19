//! Port of `vpnboard/` — the VPN status daemon (wg status, pings, board).
//! pyroute2 (netlink) is replaced by the `wg` CLI (see wg_status.rs), like
//! wireguard-sync-rs. The static board HTML is rendered inline (no jinja2/htmlmin).

pub mod board;
pub mod checks;
pub mod wg_status;

use controlserver_rs::models::Team;
use wg_status::WgStatus;

/// `VpnStatus` (vpnboard/__init__.py).
#[derive(Debug, Clone)]
pub struct VpnStatus {
    pub team: Team,
    pub wg: Option<WgStatus>,
    pub router_ping_ms: Option<f64>,
    pub testbox_ping_ms: Option<f64>,
    pub testbox_ok: bool,
    pub testbox_err: Option<String>,
    pub vulnbox_ping_ms: Option<f64>, // only filled if explicitly selected
}

impl VpnStatus {
    pub fn new(team: Team) -> Self {
        VpnStatus {
            team,
            wg: None,
            router_ping_ms: None,
            testbox_ping_ms: None,
            testbox_ok: false,
            testbox_err: None,
            vulnbox_ping_ms: None,
        }
    }

    pub fn connected(&self) -> bool {
        self.team.vpn_connected || self.team.vpn2_connected || self.team.wg_boxes_connected
    }
}
