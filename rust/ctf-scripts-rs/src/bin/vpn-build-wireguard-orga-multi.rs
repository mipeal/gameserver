//! Port of `vpn/build-wireguard-orga-multi.py`.

use ctf_commons_rs::config::Config;

fn cfg_str(cfg: &Config, key: &str) -> String {
    cfg.config["network"][key]
        .as_str()
        .expect("INFALLIBLE: config.network key exists")
        .to_string()
}

fn run(cfg: &Config, orga_dir: &std::path::Path) -> Result<(), String> {
    let (server_sk, server_pk) = ctf_scripts_rs::vpnlib::gen_wg_keypair()?;
    let server_ip = cfg.network.team_id_to_gateway_ip(0);
    let server_ip_base = server_ip
        .rsplit_once('.')
        .map(|(base, _)| base.to_string())
        .unwrap_or(server_ip.clone());
    let mut server_config = format!(
        "\n[Interface]\nAddress = {server_ip}/24\nSaveConfig = true\nListenPort = 51821\nPrivateKey = {server_sk}\n"
    );

    for i in 10..64 {
        let (client_sk, client_pk) = ctf_scripts_rs::vpnlib::gen_wg_keypair()?;
        server_config +=
            &format!("\n[Peer]\nPublicKey = {client_pk}\nAllowedIPs = {server_ip_base}.{i}/32\n");
        let client_config = format!(
            "\n[Interface]\nPrivateKey = {client_sk}\nAddress = {server_ip_base}.{i}/32\n\n[Peer]\nPublicKey = {server_pk}\nAllowedIPs = 10.32.0.0/15\nEndpoint = {host}:51821\nPersistentKeepalive = 20\n",
            host = cfg_str(cfg, "vpn_host"),
        );
        std::fs::write(orga_dir.join(format!("client-{i:02}.conf")), client_config)
            .map_err(|e| e.to_string())?;
    }

    std::fs::write(orga_dir.join("orga.conf"), server_config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let (cfg, _db, _redis) = ctf_scripts_rs::init("build-wireguard-orga-multi").await?;
    let orga_dir = cfg.vpn_base_dir.join("orga");
    std::fs::create_dir_all(&orga_dir).map_err(|e| e.to_string())?;

    run(&cfg, &orga_dir)?;
    println!("Configurations generated:");
    println!("  server: {}", orga_dir.join("orga.conf").display());
    println!("  client: {}", orga_dir.join("client-XY.conf").display());
    println!(
        "Installation: ln -s {}/orga.conf /etc/wireguard/",
        orga_dir.display()
    );
    println!("Activation:   systemctl start wg-quick@orga && systemctl enable wg-quick@orga");
    Ok(())
}
