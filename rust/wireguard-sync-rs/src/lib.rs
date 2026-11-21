//! Port of `wireguard-sync/` — the daemon that keeps wireguard interfaces in
//! sync with the vpnboard REST API (`/api/router/`).
//!
//! The pyroute2 netlink calls are replaced with the standard `ip`/`wg` tools
//! (Linux only, same semantics: create interface, add address, set key and
//! peers). Everything else (REST client, keystore, key generation, loop) is a
//! straight port.

pub mod api;
pub mod keystore;
pub mod network;
pub mod settings;

pub use keystore::KeyStore;
pub use settings::Settings;

use api::ApiClient;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// `wireguard_sync.exceptions` as error strings. `InterfaceDoesNotExist` is
/// matched by prefix so `fetch_and_sync` can initialize the interface.
pub const ERR_INTERFACE_DOES_NOT_EXIST: &str = "InterfaceDoesNotExist: ";
pub const ERR_API: &str = "ApiError: ";

pub fn is_api_error(e: &str) -> bool {
    e.starts_with(ERR_API)
}

pub fn is_interface_does_not_exist(e: &str) -> bool {
    e.starts_with(ERR_INTERFACE_DOES_NOT_EXIST)
}

/// `load_default_config()` — CONFIG env var or the default config file.
pub fn load_config() -> Result<ctf_commons_rs::config::Config, String> {
    let filename = match std::env::var("CONFIG") {
        Ok(f) => std::path::PathBuf::from(f),
        Err(_) => std::path::PathBuf::from("../config/config.yaml"),
    };
    ctf_commons_rs::config::Config::load_from_file(&filename, true)
}

/// `force_schema`.
pub fn force_schema(url_or_hostname: &str) -> String {
    if url_or_hostname.starts_with("http://") || url_or_hostname.starts_with("https://") {
        url_or_hostname.to_string()
    } else {
        format!("https://{url_or_hostname}")
    }
}

/// `generate_key_pair` — x25519, raw 32-byte keys, base64 (RFC 4648 with
/// padding, like Python's b64encode).
pub fn generate_key_pair() -> network::KeyPair {
    network::generate_key_pair()
}

/// `run_interface_up_hooks` — shell scripts with `dev`/`team` in the env.
async fn run_interface_up_hooks(settings: &Settings, ifname: &str, team_id: i64) {
    for hook in &settings.interface_up_hooks {
        let out = tokio::process::Command::new("sh")
            .arg(hook.to_string_lossy().as_ref())
            .env("dev", ifname)
            .env("team", team_id.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;
        match out {
            Ok(out) if out.status.success() => {
                tracing::info!(
                    "Ran hook={:?} stdout={:?} stderr={:?}",
                    hook,
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Ok(out) => tracing::error!(
                "Failed to run hook={:?} stdout={:?} stderr={:?}",
                hook,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(e) => tracing::error!("Failed to run hook={hook:?}: {e}"),
        }
    }
}

/// `InterfaceManager` — retry with exponential backoff, `initialize_all`, the
/// 1-second sync loop, and `fetch_and_sync`.
pub struct InterfaceManager {
    client: ApiClient,
    pub keystore: KeyStore,
    pub settings: Settings,
}

impl InterfaceManager {
    pub fn new(settings: Settings) -> Result<InterfaceManager, String> {
        let client = ApiClient::new(
            settings.clone(),
            Arc::new(Semaphore::new(settings.api_concurrency.max(1) as usize)),
        );
        let keystore = KeyStore::new(&settings)?;
        Ok(InterfaceManager {
            client,
            keystore,
            settings,
        })
    }

    /// `retry` — up to 3 retries with `2^error_count` second backoff on
    /// ApiError; other errors propagate immediately.
    async fn retry<T, F, Fut>(&self, mut f: F) -> Result<T, String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        const RETRY_LIMIT: u32 = 3;
        let mut error_count = 0;
        loop {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if !is_api_error(&e) || error_count >= RETRY_LIMIT {
                        return Err(e);
                    }
                    let sleep_time = 1u64 << error_count;
                    tracing::error!("api call failed ({e}), retrying in {sleep_time} seconds");
                    error_count += 1;
                    tokio::time::sleep(std::time::Duration::from_secs(sleep_time)).await;
                }
            }
        }
    }

    /// `initialize_all` — full pass including interfaces already on the API.
    pub async fn initialize_all(&mut self) -> Result<(), String> {
        tracing::info!("Initializing all interfaces");
        let need_init = self
            .retry(|| async { self.client.get_interfaces_to_update(true).await })
            .await?;
        for interface in need_init {
            if let Err(e) = self.fetch_and_sync(interface.id).await {
                tracing::error!("fetch_and_sync {} failed: {e}", interface.id);
            }
        }
        Ok(())
    }

    /// `fetch_and_sync` — ensure the keypair, push the pubkey, sync peers,
    /// create the interface if the system does not have it, ack.
    pub async fn fetch_and_sync(&mut self, interface_id: i64) -> Result<(), String> {
        let mut interface = self
            .retry(|| async { self.client.get_interface(interface_id).await })
            .await?;
        let team_id = interface.id;

        let key_pair = self.keystore.get_keys(team_id);
        let api_pub_key = interface.public_key.clone().filter(|k| !k.is_empty());

        if let (Some(kp), Some(api)) = (&key_pair, &api_pub_key) {
            if kp.public_key != *api {
                return Err(format!(
                    "AssertionError: Public key mismatch team_id={team_id} api_pub_key={api} key_pair.public_key={}",
                    kp.public_key
                ));
            }
        }
        if let (None, Some(api)) = (&key_pair, &api_pub_key) {
            return Err(format!(
                "ValueError: Missing private key for team_id={team_id} api_pub_key={api}"
            ));
        }

        let key_pair = match key_pair {
            Some(kp) => kp,
            None => {
                tracing::info!("Creating keypair for team_id={team_id}");
                let kp = generate_key_pair();
                self.keystore.put_keys(team_id, &kp)?;
                kp
            }
        };

        if api_pub_key.is_none() {
            self.retry(|| async {
                self.client
                    .set_pubk(interface.id, &key_pair.public_key)
                    .await
            })
            .await?;
            interface.public_key = Some(key_pair.public_key.clone());
        }

        if let Err(e) = network::sync(&interface).await {
            if !is_interface_does_not_exist(&e) {
                return Err(e);
            }
            tracing::warn!("Interface {} does not exist on the system", interface.id);
            let ifname = network::initialize_interface(&interface, &key_pair.private_key).await?;
            run_interface_up_hooks(&self.settings, &ifname, team_id).await;
            network::sync(&interface).await?;
        }

        tracing::info!("Interface {} synced", interface.id);
        self.retry(|| async {
            self.client
                .ack_interface(interface.id, interface.last_modified.clone())
                .await
        })
        .await?;
        Ok(())
    }

    /// `loop` — poll for interfaces needing sync, forever.
    pub async fn sync_loop(&mut self) {
        loop {
            match self.client.get_interfaces_to_update(false).await {
                Ok(need_init) => {
                    for interface in need_init {
                        if let Err(e) = self.fetch_and_sync(interface.id).await {
                            tracing::error!("fetch_and_sync {} failed: {e}", interface.id);
                        }
                    }
                }
                Err(e) => tracing::error!("API Error: {e}"),
            }
            tracing::info!("Finished loop, sleeping");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_schema_matches_python() {
        assert_eq!(force_schema("http://x"), "http://x");
        assert_eq!(force_schema("https://x"), "https://x");
        assert_eq!(force_schema("ctf.localhost"), "https://ctf.localhost");
    }

    #[test]
    fn key_pair_is_base64_of_32_bytes() {
        let kp = generate_key_pair();
        for (name, s) in [("pub", &kp.public_key), ("priv", &kp.private_key)] {
            let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
                .unwrap_or_else(|e| panic!("{name} not base64: {e}"));
            assert_eq!(decoded.len(), 32, "{name} not 32 bytes");
        }
        // determinism: private key derives its own public key
        let derived = network::public_key_from_private(&kp.private_key).unwrap();
        assert_eq!(derived, kp.public_key);
    }
}
