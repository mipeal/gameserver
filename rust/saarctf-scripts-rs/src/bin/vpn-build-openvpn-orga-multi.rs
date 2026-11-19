//! Port of `vpn/build-openvpn-orga-multi.py`.

use saarctf_commons_rs::config::Config;

fn cfg_str(cfg: &Config, key: &str) -> String {
    cfg.config["network"][key]
        .as_str()
        .expect("INFALLIBLE: config.network key exists")
        .to_string()
}

async fn run(cfg: &Config) -> Result<(), String> {
    let server_config_file = cfg
        .vpn_base_dir
        .join("config-server")
        .join("orga-multi.conf");
    let client_config_file = cfg
        .vpn_base_dir
        .join("config-client")
        .join("orga-multi.conf");
    let orga_secrets_dir = cfg.vpn_base_dir.join("orga-secrets");
    std::fs::create_dir_all(&orga_secrets_dir).map_err(|e| e.to_string())?;

    saarctf_scripts_rs::vpnlib::generate_vpn_keys(&orga_secrets_dir)?;
    let server_config = format!(
        "\n    port 1194\n    proto udp\n    dev orga1\n    dev-type tun\n\n    server {server_ip} 255.255.255.128\n    keepalive 10 120\n\n    push \"route 10.32.0.0 255.255.0.0\"\n    push \"route 10.33.0.0 255.255.0.0\"\n\n    ca {dir}/pki/ca.crt\n    cert {dir}/pki/issued/server.crt\n    key {dir}/pki/private/server.key\n    dh {dir}/pki/dh.pem\n    tls-auth {dir}/pki/ta.key\n    duplicate-cn\n    cipher AES-128-GCM\n\n    user nobody\n    group nogroup\n    persist-key\n    persist-tun\n    status /var/log/vpn/openvpn-status-orga-multi.log\n    verb 3\n    explicit-exit-notify 1\n    ",
        server_ip = {
            let gw = cfg.network.team_id_to_gateway_ip(0);
            format!("{}128", &gw[..gw.len() - 1])
        },
        dir = orga_secrets_dir.display(),
    )
    .replace("\n    ", "\n");
    saarctf_scripts_rs::vpnlib::write(&server_config_file, &server_config)?;

    let mut client_config = format!(
        "\n    remote {host} 1194\n    client\n    dev tun\n    proto udp\n    nobind\n\n    remote-cert-tls server\n    cipher AES-128-GCM\n\n    user nobody\n    group nogroup\n    persist-key\n    persist-tun\n    ",
        host = cfg_str(cfg, "vpn_host"),
    )
    .replace("\n    ", "\n");
    client_config += &saarctf_scripts_rs::vpnlib::format_vpn_keys(&orga_secrets_dir)?;
    saarctf_scripts_rs::vpnlib::write(&client_config_file, &client_config)?;

    println!("Configurations generated:");
    println!("  server: {}", server_config_file.display());
    println!("  client: {}", client_config_file.display());
    println!("Activation: systemctl start vpn@orga-multi && systemctl enable vpn@orga-multi");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, _db, _redis) = saarctf_scripts_rs::init("build-openvpn-orga-multi").await?;
    run(&cfg).await
}
