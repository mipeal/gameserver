//! Port of `vpn/build-openvpn-config-oneperteam.py`.
//! The `root` for the up/down hook scripts is `SAARCTF_PYTHON_DIR` (default
//! `..`)/vpn — the Python original derives it from `__file__`.

use saarctf_commons_rs::config::Config;
use std::path::Path;

const BASEPORT: i32 = 10000;

fn vpn_dir(cfg: &Config) -> std::path::PathBuf {
    saarctf_commons_rs::config::python_dir(cfg).join("vpn")
}

fn cfg_str(cfg: &Config, key: &str) -> String {
    cfg.config["network"][key]
        .as_str()
        .expect("INFALLIBLE: config.network key exists")
        .to_string()
}

fn build_team_config(
    cfg: &Config,
    team_id: i32,
    client_root: &Path,
    secret_root: &Path,
) -> Result<(), String> {
    let secret_file = secret_root.join(format!("team{team_id}.key"));

    // Create config
    let content = format!(
        "\n    topology p2p\n    remote {host} {port}\n    proto udp\n    dev game\n    dev-type tun\n    \n    ifconfig {peers_rev}\n    route {route}\n    \n    user nobody\n    group nogroup\n    nobind\n    persist-key\n    persist-tun\n    \n    # Do not change these two lines!\n    shaper 5000000\n    keepalive 10 60\n    connect-retry 5 30\n    \n    cipher AES-128-CBC\n    auth SHA256\n    \n    verb 3\n    \n    <secret>\n    {secret}\n    </secret>\n    ",
        host = cfg_str(cfg, "vpn_host"),
        port = BASEPORT + team_id,
        peers_rev = {
            let (a, b) = cfg.network.team_id_to_vpn_peers(team_id as i64);
            format!("{b} {a}")
        },
        route = saarctf_scripts_rs::vpnlib::network_to_mask(&cfg_str(cfg, "game"))?,
        secret = saarctf_scripts_rs::vpnlib::read(&secret_file)?,
    )
    .replace("\n    ", "\n");

    saarctf_scripts_rs::vpnlib::write(
        client_root.join(format!("client-team{team_id}.conf")),
        &content,
    )?;
    println!("[OK] Wrote team #{team_id} client configuration.");
    Ok(())
}

fn build_server_config(
    cfg: &Config,
    team_id: i32,
    team_name: &str,
    server_root: &Path,
    secret_root: &Path,
) -> Result<(), String> {
    let secret_file = secret_root.join(format!("team{team_id}.key"));

    // Create secret
    if !secret_file.exists() {
        let status = std::process::Command::new("openvpn")
            .args(["--genkey", "--secret"])
            .arg(&secret_file)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("CalledProcessError: openvpn --genkey".to_string());
        }
        println!("[OK] Created team #{team_id} secret key.");
    } else {
        println!("...  Team #{team_id} secret key present.");
    }

    // write config
    let root = vpn_dir(cfg);
    let mut serverconfig = format!(
        "\n    # OpenVPN Configuration for Team #{team_id} - \"{name}\"\n    topology p2p\n    ifconfig {peers}\n    proto udp\n    port {port}\n    dev tun{team_id}\n    dev-type tun\n    \n    # route game network there\n    route {route}\n    #push \"route {game_route}\"\n    #TODO avoid loops\n    \n    cipher AES-128-CBC\n    # ncp-ciphers AES-128-GCM\n    auth SHA256\n    keepalive 15 60\n    connect-retry 5 30\n    \n    # \"secure\" version\n    #user nobody\n    #group nogroup\n    #persist-tun\n    \n    # version with up/down callbacks\n    route-delay\n    up-restart\n    up-delay\n    \n    max-clients 1\n    persist-key\n    status /var/log/vpn/openvpn-status-team{team_id}.log\n\n    script-security 3\n    up \"{root}/on-connect.sh {team_id} teamhosted\"\n    down \"{root}/on-disconnect.sh {team_id} teamhosted\"\n    \n    verb 3\n    explicit-exit-notify 1\n    \n    <secret>\n    {secret}\n    </secret>\n    ",
        name = team_name,
        peers = {
            let (a, b) = cfg.network.team_id_to_vpn_peers(team_id as i64);
            format!("{a} {b}")
        },
        port = BASEPORT + team_id,
        route = saarctf_scripts_rs::vpnlib::network_to_mask(
            &cfg.network.team_id_to_network_range(team_id as i64)
        )?,
        game_route = saarctf_scripts_rs::vpnlib::network_to_mask(&cfg_str(cfg, "game"))?,
        root = root.display(),
        secret = saarctf_scripts_rs::vpnlib::read(&secret_file)?,
    )
    .replace("\n    ", "\n");
    if team_id == 0 {
        serverconfig = serverconfig
            .replace("\nup ", "\n# up ")
            .replace("\ndown ", "\n# down ")
            .replace("\ndev ", "\ndev orga0  # not ");
    }
    saarctf_scripts_rs::vpnlib::write(
        server_root.join(format!("team{team_id}.conf")),
        &serverconfig,
    )?;

    println!("[OK] Created team #{team_id} server config");
    Ok(())
}

fn build_systemd_file(cfg: &Config, teams: &[i32]) -> Result<(), String> {
    // NOTE: the Python prefix's dedent call targets '\n\t' and never matches —
    // the [Unit] block stays indented (kept bug-compatible)
    let prefix = "\n    [Unit]\n    After=network.target\n    After=vpn@team0.service\n    Requires=vpn@team0.service\n    ";
    let lines: Vec<String> = teams
        .iter()
        .map(|team_id| {
            format!("After=vpn@team{team_id}.service\nRequires=vpn@team{team_id}.service")
        })
        .collect();
    let suffix = "\n    \n    [Service]\n    Type=oneshot\n    RemainAfterExit=true\n    \n    [Install]\n    WantedBy=multi-user.target\n    ".replace("\n    ", "\n");
    let text = format!("{prefix}{}\n{suffix}", lines.join("\n"));
    saarctf_scripts_rs::vpnlib::write(cfg.vpn_base_dir.join("vpn.service"), &text)?;
    println!("[OK] Systemd service generated.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, db, _redis) = saarctf_scripts_rs::init("build-openvpn-config-oneperteam").await?;
    let server_root = cfg.vpn_base_dir.join("config-server"); // output: server config
    let client_root = cfg.vpn_base_dir.join("config-client"); // output: team config
    let secret_root = cfg.vpn_base_dir.join("secrets");
    std::fs::create_dir_all(&server_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&client_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&secret_root).map_err(|e| e.to_string())?;
    println!(
        "SETUP: rm -r /etc/openvpn/server && ln -s \"{}\" /etc/openvpn/server\n\n",
        server_root.display()
    );

    let mut teams: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM teams ORDER BY id")
        .fetch_all(&db)
        .await
        .map_err(|e| e.to_string())?;
    let mut prebuild: Vec<String> = std::env::args().skip(2).collect();
    if let Some(count) = prebuild.pop() {
        let count: i32 = count
            .trim()
            .parse()
            .map_err(|e| format!("ValueError: {e}"))?;
        let max_id = teams.iter().map(|(id, _)| *id).max().unwrap_or(0);
        for i in (max_id + 1)..=(max_id + count) {
            teams.push((i, format!("unnamed team #{i}")));
        }
    }
    for (team_id, name) in &teams {
        assert!(!name.contains('\n'), "team name contains newline");
        build_server_config(&cfg, *team_id, name, &server_root, &secret_root)?;
        build_team_config(&cfg, *team_id, &client_root, &secret_root)?;
    }
    if !teams.is_empty() {
        build_systemd_file(&cfg, &teams.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    } else {
        println!("No teams.");
    }
    // build a config for the orgas
    build_server_config(&cfg, 0, "orga", &server_root, &secret_root)?;
    build_team_config(&cfg, 0, &client_root, &secret_root)?;

    println!("[DONE]");
    Ok(())
}
