//! Rust port of `saarctf_commons/` (ADR-014): config + network math + metrics
//! formatting verified by goldens (`tests/golden.rs`); redis/retry ported
//! directly and exercised by the wave-1 service replays.

pub mod config;
pub mod metrics;
pub mod network;
pub mod patch_utils;
pub mod redis;
pub mod retry;

use std::path::{Path, PathBuf};

/// `opt_path`: `Path(s) if s else None`.
pub fn opt_path(s: Option<&str>) -> Option<PathBuf> {
    s.filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// `load_default_config_file(filename, additional)` — loads and returns the
/// config (the Python side assigns it to a global; Rust callers hold it).
pub fn load_default_config_file(
    filename: &Path,
    interpolate_env: bool,
) -> Result<config::Config, String> {
    config::Config::load_from_file(filename, interpolate_env)
}
