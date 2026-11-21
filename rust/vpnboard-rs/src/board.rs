//! Port of `vpnboard/vpn_board.py` — renders the static board files into
//! `config.vpnboard_path`. jinja2 + htmlmin replaced by inline HTML generation
//! (same markup, not minified); JSON files identical; static files copied from
//! the Python tree (`CTF_PYTHON_DIR`, default `..`).

use crate::VpnStatus;
use chrono::{DateTime, TimeZone, Utc};
use controlserver_rs::models::Team;
use ctf_commons_rs::config::Config;
use std::collections::HashSet;
use std::path::Path;

pub struct VpnBoard;

impl VpnBoard {
    pub fn new(cfg: &Config) -> Result<Self, String> {
        let path = Path::new(&cfg.vpnboard_path);
        if !path.exists() {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
        }
        if !path.join("index.css").exists() {
            Self::copy_static_files(cfg, path)?;
        }
        Ok(VpnBoard)
    }

    fn copy_static_files(cfg: &Config, path: &Path) -> Result<(), String> {
        let static_dir = ctf_commons_rs::config::python_dir(cfg)
            .join("controlserver")
            .join("static");
        std::fs::copy(
            static_dir.join("css").join("vpnboard.css"),
            path.join("index.css"),
        )
        .map_err(|e| e.to_string())?;
        std::fs::copy(
            static_dir.join("img").join("favicon.png"),
            path.join("favicon.png"),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn write_file(path: &Path, filename: &str, content: &str) -> Result<(), String> {
        std::fs::write(path.join(filename), content.as_bytes()).map_err(|e| e.to_string())
    }

    /// `build_vpn_json`.
    pub fn build_vpn_json(cfg: &Config, path: &Path, teams: &[&Team]) -> Result<(), String> {
        let entry = |team: &Team| {
            serde_json::json!({
                "id": team.id,
                "name": team.name,
                "ip": cfg.network.team_id_to_vulnbox_ip(team.id as i64),
                "online": team.vpn_connected || team.vpn2_connected || team.wg_boxes_connected,
                "ever_online": team.vpn_last_connect.is_some(),
            })
        };
        let all =
            serde_json::json!({ "teams": teams.iter().map(|t| entry(t)).collect::<Vec<_>>() });
        Self::write_file(path, "all_teams.json", &all.to_string())?;
        let available = serde_json::json!({
            "teams": teams
                .iter()
                .filter(|t| {
                    t.vpn_connected
                        || t.vpn2_connected
                        || t.wg_boxes_connected
                        || t.vpn_last_connect.is_some()
                })
                .map(|t| entry(t))
                .collect::<Vec<_>>()
        });
        Self::write_file(path, "available_teams.json", &available.to_string())
    }

    /// `build_vpn_board`.
    pub fn build_vpn_board(
        cfg: &Config,
        path: &Path,
        states: &[VpnStatus],
        banned_teams: &HashSet<i32>,
        check_vulnboxes: bool,
        start: DateTime<Utc>,
    ) -> Result<(), String> {
        let wireguard = states.iter().any(|s| s.wg.is_some());
        let html = render_vpn_html(cfg, states, banned_teams, check_vulnboxes, wireguard, start);
        Self::write_file(path, "vpn.html", &html)?;
        let teams: Vec<&Team> = states.iter().map(|s| &s.team).collect();
        Self::build_vpn_json(cfg, path, &teams)
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// jinja2's `'%d. %b %Y %H:%M:%S %Z'` (UTC).
fn fmt_dt(dt: DateTime<Utc>) -> String {
    dt.format("%d. %b %Y %H:%M:%S %Z").to_string()
}

fn ping_cell(ping: Option<f64>) -> String {
    match ping {
        Some(v) => format!(
            "<td><span class=\"label label-success\">ok</span> {:.1} ms</td>",
            v
        ),
        None => "<td><span class=\"label label-danger\">unreachable</span></td>".into(),
    }
}

fn dash_cell() -> String {
    "<td><span class=\"label label-default\">-</span></td>".into()
}

#[allow(clippy::too_many_arguments)]
fn render_vpn_html(
    cfg: &Config,
    states: &[VpnStatus],
    banned_teams: &HashSet<i32>,
    check_vulnboxes: bool,
    wireguard: bool,
    start: DateTime<Utc>,
) -> String {
    let mut out = String::with_capacity(4096 + states.len() * 512);
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n\t<meta charset=\"UTF-8\">\n\t<meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">\n\t<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\n\t<link rel=\"stylesheet\" href=\"index.css\">\n\t<link rel=\"icon\" type=\"image/png\" href=\"favicon.png\">\n\t<link rel=\"shortcut icon\" type=\"image/png\" href=\"favicon.png\">\n\n    <style>\n        table th { vertical-align: top !important; }\n        table th small { font-weight: normal }\n    </style>\n\n\t<title>Team VPN Status - CTF</title>\n\t<meta http-equiv=\"refresh\" content=\"60\">\n</head>\n<body>\n<div class=\"bg-primary-theme\">\n\t<div class=\"container-fluid\">\n\t\t<h1 class=\"page-header\">CTF Team VPN Status</h1>\n\t</div>\n\t<br/>\n</div>\n\n<br/>\n\n<div class=\"container\">\n\n\t<p>\n\t\t<a href=\"all_teams.json\">&rArr; all teams (as JSON)</a><br/>\n\t\t<a href=\"available_teams.json\">&rArr; teams that connected to the VPN once (as JSON)</a>\n\t\t<span class=\"text-muted\"> &mdash; Cloud-hosting teams will not be included until the competition starts.</span>\n\t</p>\n\n\t<p class=\"text-muted\">Last update: ");
    out.push_str(&esc(&fmt_dt(start)));
    out.push_str("</p>\n\n\t<table class=\"table table-bordered\">\n\t\t<thead>\n\t\t<tr>\n\t\t\t<th>#</th>\n\t\t\t<th>Team</th>\n\t\t\t<th>\n                VPN Status\n");
    if wireguard {
        out.push_str(
            "                    <br/>\n                    <small>for vuln/testbox</small>\n",
        );
    }
    out.push_str("            </th>\n\t\t\t<th>\n                Last VPN Connection\n");
    if wireguard {
        out.push_str("                    <br/>\n                    <small>to vulnbox</small>\n");
    }
    out.push_str("            </th>\n\t\t\t<th>Ping to Router</th>\n\t\t\t<th>Ping to Testbox</th>\n\t\t\t<th>Testbox Check</th>\n");
    if check_vulnboxes {
        out.push_str("\t\t\t\t<th>Ping to Vulnbox</th>\n");
    }
    out.push_str("\t\t</tr>\n\t\t</thead>\n\t\t<tbody>\n");

    for state in states {
        let team = &state.team;
        let banned = banned_teams.contains(&team.id);
        out.push_str("\t\t\t<tr>\n\t\t\t\t<th>");
        out.push_str(&team.id.to_string());
        out.push_str("</th>\n\t\t\t\t<td>\n");
        if banned {
            out.push_str(&format!(
                "                        <s class=\"text-danger\">{}</s>\n",
                esc(&team.name)
            ));
        } else {
            out.push_str(&format!("                        {}\n", esc(&team.name)));
        }
        out.push_str(&format!(
            "                    <br/><small>(vulnbox&nbsp;{ip})</small>\n",
            ip = esc(&cfg.network.team_id_to_vulnbox_ip(team.id as i64))
        ));
        out.push_str("\t\t\t\t</td>\n\t\t\t\t<td class=\"more-line-height\">\n");
        // VPN status label
        if team.vpn_connected {
            out.push_str(
                "\t\t\t\t\t\t<span class=\"label label-success\">connected (self-hosted)</span>\n",
            );
        } else if team.vpn2_connected {
            out.push_str(
                "\t\t\t\t\t\t<span class=\"label label-success\">connected (cloud-hosted)</span>\n",
            );
        } else if team.wg_boxes_connected {
            if team.wg_vulnbox_connected {
                out.push_str("\t\t\t\t\t\t<span class=\"label label-success\" title=\"VPN peer with route to vulnbox is connected\">connected (v)</span>\n");
            } else {
                out.push_str("\t\t\t\t\t\t<span class=\"label label-success\" title=\"VPN connection with at least one IP from .1-.3 active\">connected</span>\n");
            }
        } else {
            out.push_str("\t\t\t\t\t\t<span class=\"label label-default\">disconnected</span>\n");
        }
        if team.vpn_connection_count > 0 {
            let count = team.vpn_connection_count.to_string();
            if wireguard {
                out.push_str(&format!("\t\t\t\t\t\t<br/>\n\t\t\t\t\t\t<span class=\"label label-success\" title=\"Connected wireguard peers\">{count} peers</span>\n"));
            } else {
                out.push_str(&format!("\t\t\t\t\t\t<br/>\n\t\t\t\t\t\t<span class=\"label label-success\" title=\"Connected team members\">{count} players</span>\n"));
            }
        }
        if team.vpn_connected && team.vpn2_connected {
            out.push_str("\t\t\t\t\t\t<br />\n\t\t\t\t\t\t<span class=\"label label-danger\" title=\"Team-hosted VPN must not be active while the cloud-hosted vulnbox is used. Use the other provided VPN config instead.\">Multiple VPN</span>\n");
        }
        if team.vpn_connected && team.vpn_connection_count > 0 {
            out.push_str("\t\t\t\t\t\t<br />\n\t\t\t\t\t\t<span class=\"label label-danger\" title=\"Team-hosted VPN and cloud-hosted VPN must not be used at the same time.\">Multiple VPN</span>\n");
        }
        if banned {
            out.push_str("\t\t\t\t\t\t<br/>\n\t\t\t\t\t\t<span class=\"label label-danger\" title=\"Network traffic is currently blocked. Please contact the organizers in IRC!\">! blocked !</span>\n");
        }
        out.push_str("\t\t\t\t</td>\n\t\t\t\t<td>\n");
        match team.vpn_last_connect {
            Some(dt) => out.push_str(&format!("					{}\n", esc(&fmt_dt(dt)))),
            None => out.push_str("					<span class=\"text-danger\">never connected</span>\n"),
        }
        out.push_str("\t\t\t\t</td>\n");
        if state.connected() {
            out.push_str(&ping_cell(state.router_ping_ms));
            out.push('\n');
            out.push_str(&ping_cell(state.testbox_ping_ms));
            out.push('\n');
            out.push_str("\t\t\t\t<td>\n");
            match state.testbox_err.as_deref() {
                _ if state.testbox_ok => {
                    out.push_str("\t\t\t\t\t<span class=\"label label-success\">ok</span>\n")
                }
                Some("unreachable") => out
                    .push_str("\t\t\t\t\t<span class=\"label label-danger\">unreachable</span>\n"),
                Some(err) => out.push_str(&format!(
                    "\t\t\t\t\t<span class=\"label label-warning\">{}</span>\n",
                    esc(err)
                )),
                None => out.push_str("\t\t\t\t\t<span class=\"label label-warning\">None</span>\n"),
            }
            out.push_str("\t\t\t\t</td>\n");
            if check_vulnboxes {
                out.push_str(&ping_cell(state.vulnbox_ping_ms));
                out.push('\n');
            }
        } else {
            out.push_str(&dash_cell());
            out.push('\n');
            out.push_str(&dash_cell());
            out.push('\n');
            out.push_str(&dash_cell());
            out.push('\n');
            if check_vulnboxes {
                out.push_str(&dash_cell());
                out.push('\n');
            }
        }
        out.push_str("\t\t\t</tr>\n");
    }

    out.push_str("\n\t\t</tbody>\n\t</table>\n\n</div>\n</body>\n</html>\n");
    out
}

/// The `ts` timestamp used by the metrics handler (seconds since epoch).
pub fn ts_secs(t: std::time::SystemTime) -> f64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn ts_datetime(t: std::time::SystemTime) -> DateTime<Utc> {
    let secs = ts_secs(t) as i64;
    Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now)
}
