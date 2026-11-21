//! `sim_boot` gating + sim config sanity (plan task 3).

use std::path::PathBuf;

use controlserver_rs::sim_boot;
use ctf_commons_rs::config::Config;

#[test]
fn gating_off_by_default() {
    assert!(!sim_boot::simulation_enabled_with(Some("")));
    assert!(!sim_boot::simulation_enabled_with(None));
}

#[test]
fn gating_on() {
    assert!(sim_boot::simulation_enabled_with(Some("1")));
}

#[test]
fn sim_config_targets_10_32_network() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../simulation/config.sim.yaml");
    let cfg = Config::load_from_file(&path, true).expect("config.sim.yaml");
    assert_eq!(cfg.network.team_id_to_vulnbox_ip(2), "10.32.2.2");
    assert!(cfg.dispatcher_check_vpn_status);
    assert_eq!(cfg.services_path, "/opt/services");
    assert_eq!(cfg.tick_duration_default, 30);
}
