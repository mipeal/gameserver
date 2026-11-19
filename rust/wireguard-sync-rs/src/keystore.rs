//! `wireguard_sync/__init__.py` KeyStore — team_id → (public, private) in a
//! JSON file, written atomically (tmp + fsync + rename).

use crate::network::KeyPair;
use crate::settings::Settings;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug)]
pub struct KeyStore {
    path: PathBuf,
    keys: BTreeMap<String, [String; 2]>,
}

impl KeyStore {
    pub fn new(settings: &Settings) -> Result<KeyStore, String> {
        let path = settings.keystore_path.clone();
        let keys = if path.is_file() {
            let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            serde_json::from_str(&data).map_err(|e| e.to_string())?
        } else if path.is_dir() {
            return Err(format!(
                "ConfigurationError: Key store path: {} is a directory",
                path.to_string_lossy()
            ));
        } else if path.exists() {
            return Err(format!(
                "ConfigurationError: Key store path: {} is neither dir nor file?",
                path.to_string_lossy()
            ));
        } else {
            BTreeMap::new()
        };
        Ok(KeyStore { path, keys })
    }

    pub fn contains(&self, team_id: i64) -> bool {
        self.keys.contains_key(&team_id.to_string())
    }

    pub fn get_keys(&self, team_id: i64) -> Option<KeyPair> {
        self.keys.get(&team_id.to_string()).map(|kv| KeyPair {
            public_key: kv[0].clone(),
            private_key: kv[1].clone(),
        })
    }

    pub fn put_keys(&mut self, team_id: i64, key_pair: &KeyPair) -> Result<(), String> {
        self.keys.insert(
            team_id.to_string(),
            [key_pair.public_key.clone(), key_pair.private_key.clone()],
        );
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut f, &self.keys).map_err(|e| e.to_string())?;
            f.flush().map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        // os.replace is atomic as long as both files are on the same FS
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_settings(tag: &str) -> Settings {
        Settings {
            keystore_path: std::env::temp_dir().join(format!(
                "wg-keystore-test-{tag}-{}.json",
                std::process::id()
            )),
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip_and_atomic_replace() {
        let s = tmp_settings("roundtrip");
        let _ = std::fs::remove_file(&s.keystore_path);
        let mut ks = KeyStore::new(&s).unwrap();
        assert!(!ks.contains(1));
        ks.put_keys(
            1,
            &KeyPair {
                public_key: "PUB".into(),
                private_key: "PRIV".into(),
            },
        )
        .unwrap();
        // re-open from disk
        let ks = KeyStore::new(&s).unwrap();
        assert!(ks.contains(1));
        let kp = ks.get_keys(1).unwrap();
        assert_eq!(kp.public_key, "PUB");
        assert_eq!(kp.private_key, "PRIV");
        let _ = std::fs::remove_file(&s.keystore_path);
    }

    #[test]
    fn directory_path_is_configuration_error() {
        let mut s = tmp_settings("dir");
        s.keystore_path = std::env::temp_dir();
        let err = KeyStore::new(&s).unwrap_err();
        assert!(err.starts_with("ConfigurationError:"), "{err}");
    }
}
