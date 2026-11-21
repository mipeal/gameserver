//! Port of `vpn/build-openvpn-config-cloud.py`.
//! Hook script paths point at `CTF_PYTHON_DIR` (default `..`)/vpn, the
//! Python original's `__file__` directory.

use ctf_commons_rs::config::Config;
use std::path::Path;

const BASEPORT: i32 = 12000;
const BASEPORT2: i32 = 14000;

fn vpn_dir(cfg: &Config) -> std::path::PathBuf {
    ctf_commons_rs::config::python_dir(cfg).join("vpn")
}

fn cfg_str(cfg: &Config, key: &str) -> String {
    cfg.config["network"][key]
        .as_str()
        .expect("INFALLIBLE: config.network key exists")
        .to_string()
}

/// Python's `gateway_ip[:-1] + "128"`
fn gw_128(cfg: &Config, team_id: i64) -> String {
    let gw = cfg.network.team_id_to_gateway_ip(team_id);
    format!("{}128", &gw[..gw.len() - 1])
}

fn build_cloud_server_config(
    cfg: &Config,
    team_id: i32,
    team_name: &str,
    server_root: &Path,
    secret_root: &Path,
) -> Result<(), String> {
    // Create secret
    let secret_path = secret_root.join(format!("team{team_id}"));
    std::fs::create_dir_all(&secret_path).map_err(|e| e.to_string())?;
    ctf_scripts_rs::vpnlib::generate_vpn_keys(&secret_path)?;

    // write config
    let vpnroot = vpn_dir(cfg);
    let run_sh = "/bin/gspython";
    let mut serverconfig = format!(
        "\n    # OpenVPN Cloud Configuration for Team #{team_id} - \"{name}\"\n    proto udp\n    port {port}\n    dev tun{dev}\n    dev-type tun\n\n    push \"route 10.32.0.0 255.255.0.0\"\n    push \"route 10.33.0.0 255.255.0.0\"\n\n    server {server_ip} 255.255.255.128\n\n    duplicate-cn\n    client-to-client\n    cipher AES-128-GCM\n    keepalive 15 60\n    connect-retry 5 30\n\n    # \"secure\" version\n    #user nobody\n    #group nogroup\n    #persist-tun\n\n    # version with up/down callbacks\n    up-restart\n\n    max-clients 63\n    persist-key\n    status /var/log/vpn/openvpn-cloud-status-team{team_id}.log\n\n    script-security 3\n    up \"{vpnroot}/on-device-up.sh {team_id}\"\n    down \"{run_sh} {vpnroot}/on-cloud-down.py {team_id}\"\n    client-connect \"{run_sh} {vpnroot}/on-cloud-connect.py {team_id}\"\n    client-disconnect \"{run_sh} {vpnroot}/on-cloud-disconnect.py {team_id}\"\n\n    verb 3\n    explicit-exit-notify 1\n    ",
        name = team_name,
        port = BASEPORT + team_id,
        dev = 1000 + team_id,
        server_ip = gw_128(cfg, team_id as i64),
        vpnroot = vpnroot.display(),
        run_sh = run_sh,
    )
    .replace("\n    ", "\n");
    serverconfig += &ctf_scripts_rs::vpnlib::format_vpn_server_keys(&secret_path)?;
    ctf_scripts_rs::vpnlib::write(
        server_root.join(format!("team{team_id}-cloud.conf")),
        &serverconfig,
    )?;

    println!("[OK] Created team #{team_id} server config");
    Ok(())
}

fn build_cloud_team_config(
    cfg: &Config,
    team_id: i32,
    client_root: &Path,
    secret_root: &Path,
) -> Result<(), String> {
    let secret_path = secret_root.join(format!("team{team_id}"));

    // Create config
    let mut content = format!(
        "\n    client\n    remote {host} {port}\n    proto udp\n    dev game\n    dev-type tun\n\n    user nobody\n    group nogroup\n    nobind\n    persist-key\n    persist-tun\n\n    remote-cert-tls server\n    cipher AES-128-GCM\n\n    # Do not change these two lines!\n    shaper 5000000\n    keepalive 10 60\n    connect-retry 5 30\n\n    verb 3\n\t",
        host = cfg_str(cfg, "vpn_host"),
        port = BASEPORT + team_id,
    )
    .replace("\n    ", "\n");
    content += &ctf_scripts_rs::vpnlib::format_vpn_keys(&secret_path)?;

    ctf_scripts_rs::vpnlib::write(
        client_root.join(format!("client-cloud-team{team_id}.conf")),
        &content,
    )?;
    println!("[OK] Wrote team #{team_id} client configuration.");
    Ok(())
}

fn build_vulnbox_server_config(
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
    let serverconfig = format!(
        "\n\t# OpenVPN Configuration for vulnbox of Team #{team_id} - \"{name}\"\n\ttopology p2p\n\tifconfig {gw} {vb}\n\tproto udp\n\tport {port}\n\tdev tun{dev}\n\tdev-type tun\n\n\t# route game network there\n\t#route {route}\n\t#push \"route {game_route}\"\n\t#TODO avoid loops\n\n\tcipher AES-128-CBC\n\t# ncp-ciphers AES-128-GCM\n\tauth SHA256\n\tkeepalive 15 60\n\tconnect-retry 5 30\n\n\t# \"secure\" version\n\t#user nobody\n\t#group nogroup\n\t#persist-tun\n\n\t# version with up/down callbacks\n\troute-delay\n\tup-restart\n\tup-delay\n\n\tmax-clients 1\n\tpersist-key\n\tstatus /var/log/vpn/openvpn-status-team{team_id}.log\n\n\tscript-security 3\n\tup \"{root}/on-connect.sh {team_id} cloudhosted\"\n\tdown \"{root}/on-disconnect.sh {team_id} cloudhosted\"\n\n\tverb 3\n\texplicit-exit-notify 1\n\n\t<secret>\n\t{secret}\n\t</secret>\n\t",
        name = team_name,
        gw = cfg.network.team_id_to_gateway_ip(team_id as i64),
        vb = cfg.network.team_id_to_vulnbox_ip(team_id as i64),
        port = BASEPORT2 + team_id,
        dev = 2000 + team_id,
        route = ctf_scripts_rs::vpnlib::network_to_mask(
            &cfg.network.team_id_to_network_range(team_id as i64)
        )?,
        game_route = ctf_scripts_rs::vpnlib::network_to_mask(&cfg_str(cfg, "game"))?,
        root = root.display(),
        secret = ctf_scripts_rs::vpnlib::read(&secret_file)?,
    )
    .replace("\n\t", "\n");
    ctf_scripts_rs::vpnlib::write(
        server_root.join(format!("team{team_id}-vulnbox.conf")),
        &serverconfig,
    )?;

    println!("[OK] Created team #{team_id} server config");
    Ok(())
}

fn build_vulnbox_team_config(
    cfg: &Config,
    team_id: i32,
    client_root: &Path,
    secret_root: &Path,
) -> Result<(), String> {
    let secret_file = secret_root.join(format!("team{team_id}.key"));

    // Create config
    let content = format!(
        "\n\ttopology p2p\n\tremote {host} {port}\n\tproto udp\n\tdev game\n\tdev-type tun\n\n\tifconfig {vb} {gw}\n\troute {game_route}\n\n\tuser nobody\n\tgroup nogroup\n\tnobind\n\tpersist-key\n\tpersist-tun\n\n\t# Do not change these two lines!\n\tshaper 5000000\n\tkeepalive 10 60\n\tconnect-retry 5 30\n\n\tcipher AES-128-CBC\n\tauth SHA256\n\n\tverb 3\n\n\t<secret>\n\t{secret}\n\t</secret>\n\t",
        host = cfg_str(cfg, "vpn_host"),
        port = BASEPORT2 + team_id,
        vb = cfg.network.team_id_to_vulnbox_ip(team_id as i64),
        gw = cfg.network.team_id_to_gateway_ip(team_id as i64),
        game_route = ctf_scripts_rs::vpnlib::network_to_mask(&cfg_str(cfg, "game"))?,
        secret = ctf_scripts_rs::vpnlib::read(&secret_file)?,
    )
    .replace("\n\t", "\n");

    ctf_scripts_rs::vpnlib::write(
        client_root.join(format!("client-team{team_id}-vulnbox.conf")),
        &content,
    )?;
    println!("[OK] Wrote team #{team_id} client configuration.");
    Ok(())
}

fn build_systemd_file(cfg: &Config, teams: &[i32]) -> Result<(), String> {
    let prefix = "\n[Unit]\nAfter=network.target\n";
    let mut lines: Vec<String> = teams
        .iter()
        .map(|team_id| {
            format!(
                "After=vpn2@team{team_id}-cloud.service\nRequires=vpn2@team{team_id}-cloud.service"
            )
        })
        .collect();
    lines.extend(teams.iter().map(|team_id| {
        format!(
            "After=vpn@team{team_id}-vulnbox.service\nRequires=vpn@team{team_id}-vulnbox.service"
        )
    }));
    let suffix = "\n\n[Service]\nType=oneshot\nRemainAfterExit=true\n\n[Install]\nWantedBy=multi-user.target\n";
    let text = format!("{prefix}{}\n{suffix}", lines.join("\n"));
    ctf_scripts_rs::vpnlib::write(cfg.vpn_base_dir.join("vpncloud.service"), &text)?;
    println!("[OK] Systemd service generated.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, db, _redis) = ctf_scripts_rs::init("build-openvpn-config-cloud").await?;
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
        build_cloud_server_config(&cfg, *team_id, name, &server_root, &secret_root)?;
        build_cloud_team_config(&cfg, *team_id, &client_root, &secret_root)?;
        build_vulnbox_server_config(&cfg, *team_id, name, &server_root, &secret_root)?;
        build_vulnbox_team_config(&cfg, *team_id, &client_root, &secret_root)?;
    }
    if !teams.is_empty() {
        build_systemd_file(&cfg, &teams.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    } else {
        println!("No teams.");
    }

    println!("[DONE]");
    Ok(())
}
