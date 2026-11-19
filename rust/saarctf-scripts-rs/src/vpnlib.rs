//! Port of `vpn/vpnlib.py` — easyrsa/openvpn/wg key helpers.

use std::path::Path;
use std::process::Command;

pub const EASYRSA_BINARY: &str = "/usr/share/easy-rsa/easyrsa";
pub const REUSE_DH: bool = true;

pub fn read(fname: impl AsRef<Path>) -> Result<String, String> {
    std::fs::read_to_string(fname).map_err(|e| e.to_string())
}

pub fn write(fname: impl AsRef<Path>, content: &str) -> Result<(), String> {
    std::fs::write(fname, content).map_err(|e| e.to_string())
}

/// `"1.2.3.4/16"` => `"1.2.3.4 255.255.0.0"`
pub fn network_to_mask(network: &str) -> Result<String, String> {
    let (ip, netrange) = network
        .split_once('/')
        .ok_or_else(|| format!("no '/' in {network}"))?;
    let bits: u32 = netrange
        .parse()
        .map_err(|_| format!("bad netrange {netrange}"))?;
    let mask_int = u32::from_str_radix(
        &format!(
            "{}{}",
            "1".repeat(bits as usize),
            "0".repeat((32 - bits) as usize)
        ),
        2,
    )
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "{ip} {}.{}.{}.{}",
        mask_int >> 24,
        (mask_int >> 16) & 0xff,
        (mask_int >> 8) & 0xff,
        mask_int & 0xff
    ))
}

fn run_easyrsa(args: &[&str], directory: &Path) -> Result<(), String> {
    let status = Command::new(EASYRSA_BINARY)
        .args(args)
        .env("EASYRSA_BATCH", "1")
        .current_dir(directory)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "CalledProcessError: easyrsa {} exited {status:?}",
            args.join(" ")
        ));
    }
    Ok(())
}

/// `generate_vpn_keys` — idempotent (skips when `pki/ta.key` exists).
pub fn generate_vpn_keys(directory: &Path) -> Result<(), String> {
    let path = directory.join("pki");
    if path.join("ta.key").exists() {
        println!("[.] VPN keys already present");
        return Ok(());
    }
    for args in [
        vec!["init-pki"],
        vec!["build-ca", "nopass"],
        vec!["gen-req", "server", "nopass"],
        vec!["gen-req", "TeamMember", "nopass"],
        vec!["sign-req", "server", "server"],
        vec!["sign-req", "client", "TeamMember"],
    ] {
        run_easyrsa(&args, directory)?;
    }
    if REUSE_DH && std::path::Path::new("/tmp/dh.pem").exists() {
        std::fs::copy("/tmp/dh.pem", path.join("dh.pem")).map_err(|e| e.to_string())?;
    } else {
        run_easyrsa(&["gen-dh"], directory)?;
        if REUSE_DH {
            std::fs::copy(path.join("dh.pem"), "/tmp/dh.pem").map_err(|e| e.to_string())?;
        }
    }
    let status = Command::new("openvpn")
        .args(["--genkey", "--secret", "ta.key"])
        .current_dir(&path)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("CalledProcessError: openvpn --genkey".to_string());
    }
    println!("[*] VPN keys have been generated.");
    Ok(())
}

/// `generate_additional_vpn_keys`.
pub fn generate_additional_vpn_keys(directory: &Path, name: &str) -> Result<(), String> {
    let path = directory.join("pki");
    if path.join("issued").join(format!("{name}.crt")).exists() {
        return Ok(());
    }
    run_easyrsa(&["gen-req", name, "nopass"], directory)?;
    run_easyrsa(&["sign-req", "client", name], directory)
}

fn format_keys(_directory: &Path, files: &[(&str, &Path)]) -> Result<String, String> {
    let mut config = String::new();
    for (name, fname) in files {
        config.push_str(&format!("\n<{name}>\n"));
        config.push_str(&read(fname)?);
        config.push_str(&format!("\n</{name}>\n"));
    }
    Ok(config)
}

/// `format_vpn_keys` — TeamMember client key bundle.
pub fn format_vpn_keys(directory: &Path) -> Result<String, String> {
    let pki = directory.join("pki");
    format_keys(
        directory,
        &[
            ("ca", &pki.join("ca.crt")),
            ("cert", &pki.join("issued").join("TeamMember.crt")),
            ("key", &pki.join("private").join("TeamMember.key")),
            ("tls-auth", &pki.join("ta.key")),
        ],
    )
}

/// `format_vpn_server_keys`.
pub fn format_vpn_server_keys(directory: &Path) -> Result<String, String> {
    let pki = directory.join("pki");
    format_keys(
        directory,
        &[
            ("ca", &pki.join("ca.crt")),
            ("cert", &pki.join("issued").join("server.crt")),
            ("key", &pki.join("private").join("server.key")),
            ("dh", &pki.join("dh.pem")),
            ("tls-auth", &pki.join("ta.key")),
        ],
    )
}

/// `gen_wg_keypair` — (private, public).
pub fn gen_wg_keypair() -> Result<(String, String), String> {
    let out = Command::new("wg")
        .arg("genkey")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("CalledProcessError: wg genkey".to_string());
    }
    let privkey = String::from_utf8_lossy(&out.stdout).trim().to_string();
    use std::io::Write;
    let mut child = Command::new("wg")
        .arg("pubkey")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .as_mut()
        .expect("INFALLIBLE: stdin piped")
        .write_all(privkey.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("CalledProcessError: wg pubkey".to_string());
    }
    Ok((
        privkey,
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// The hook scripts' error path: append to /tmp/connect-error.log, then
/// re-raise (caller prints to stderr and exits non-zero).
pub fn log_hook_error(header: &str, argv: &[String], err: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/connect-error.log")
    {
        let _ = writeln!(f, "{header}{argv:?}\n{err}\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        assert_eq!(
            network_to_mask("1.2.3.4/16").unwrap(),
            "1.2.3.4 255.255.0.0"
        );
        assert_eq!(
            network_to_mask("10.32.11.0/24").unwrap(),
            "10.32.11.0 255.255.255.0"
        );
        assert_eq!(network_to_mask("10.0.0.0/8").unwrap(), "10.0.0.0 255.0.0.0");
        assert!(network_to_mask("1.2.3.4").is_err());
    }
}
