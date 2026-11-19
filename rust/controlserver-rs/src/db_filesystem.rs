//! Port of `controlserver/db_filesystem.py` — folders packed into
//! content-addressed "packages" in the DB (`checker_filesystem` /
//! `checker_files`, md5-identified). The package id is
//! `md5(json.dumps([[path, kind], ...]))` with Python's json.dumps separators.

use saarctf_commons_rs::metrics::py_json_str;
use sqlx::PgPool;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The `ignore_patterns` — same set of anchored regexes, hand-rolled.
pub fn is_ignored(name: &str) -> bool {
    name == "__pycache__"
        || name.ends_with(".pyc")
        || name == ".idea"
        || name.starts_with(".git")
        || name == ".mypy_cache"
        || name == "gamelib"
}

fn md5_hex(data: &[u8]) -> String {
    use md5::Digest;
    let mut h = md5::Md5::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

/// `json.dumps([[path, kind], ...])` byte-exact: `", "` between items, `", "`
/// inside each pair, `py_json_str` escaping.
fn py_json_pairs(infos: &[(String, String)]) -> String {
    let items: Vec<String> = infos
        .iter()
        .map(|(p, k)| format!("[{}, {}]", py_json_str(p), py_json_str(k)))
        .collect();
    format!("[{}]", items.join(", "))
}

/// Recursive `os.walk(top, followlinks=True)` — yields (root, subdirs, files)
/// with the ignored names filtered out of `subdirs` (so they're not descended).
fn walk(top: &Path, out: &mut Vec<(PathBuf, Vec<PathBuf>, Vec<PathBuf>)>) {
    let Ok(entries) = std::fs::read_dir(top) else { return };
    let mut subdirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() || (ft.is_symlink() && entry.path().is_dir()) {
            if !is_ignored(&name) {
                subdirs.push(entry.path());
            }
        } else if !is_ignored(&name) {
            files.push(entry.path());
        }
    }
    out.push((top.to_path_buf(), subdirs.clone(), files));
    for dir in subdirs {
        walk(&dir, out);
    }
}

/// `move_folder_to_package` — returns (package, is_new).
pub async fn move_folder_to_package(db: &PgPool, folder: &Path) -> Result<(String, bool), String> {
    let mut filesystem: Vec<(String, Option<String>)> = Vec::new(); // (path, file_hash)
    let mut file_information: Vec<(String, String)> = Vec::new(); // (path, 'dir' | hash)
    let mut walked = Vec::new();
    walk(folder, &mut walked);
    let folder_str = folder.to_string_lossy();
    for (root, subdirs, files) in walked {
        let root_str = root.to_string_lossy();
        for dir in subdirs {
            let dirname = dir.file_name().unwrap().to_string_lossy().into_owned();
            let path = if root_str == folder_str {
                dirname
            } else {
                format!("{}/{}", &root_str[folder_str.len() + 1..], dirname)
            };
            filesystem.push((path.clone(), None));
            file_information.push((path, "dir".into()));
        }
        for file in files {
            let fname = file.to_string_lossy().into_owned();
            let basename = file.file_name().unwrap().to_string_lossy().into_owned();
            let path = if root_str == folder_str {
                basename
            } else {
                format!("{}/{}", &root_str[folder_str.len() + 1..], basename)
            };
            let hash = store_file_in_database(db, &fname).await?;
            filesystem.push((path.clone(), Some(hash.clone())));
            file_information.push((path, hash));
        }
    }
    finish_package(db, filesystem, file_information).await
}

/// `move_single_files_to_package` — files in the package root, no folders.
pub async fn move_single_files_to_package(
    db: &PgPool,
    fnames: &[PathBuf],
) -> Result<(String, bool), String> {
    let mut filesystem: Vec<(String, Option<String>)> = Vec::new();
    let mut file_information: Vec<(String, String)> = Vec::new();
    for fname in fnames {
        let path = fname
            .file_name()
            .ok_or("ValueError: no filename")?
            .to_string_lossy()
            .into_owned();
        let hash = store_file_in_database(db, &fname.to_string_lossy()).await?;
        filesystem.push((path.clone(), Some(hash.clone())));
        file_information.push((path, hash));
    }
    finish_package(db, filesystem, file_information).await
}

async fn finish_package(
    db: &PgPool,
    filesystem: Vec<(String, Option<String>)>,
    mut file_information: Vec<(String, String)>,
) -> Result<(String, bool), String> {
    file_information.sort_by(|a, b| a.0.cmp(&b.0));
    let package = md5_hex(py_json_pairs(&file_information).as_bytes());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM checker_filesystem WHERE package = $1")
        .bind(&package)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok((package, false));
    }
    for (path, file_hash) in filesystem {
        sqlx::query("INSERT INTO checker_filesystem (package, path, file_hash) VALUES ($1, $2, $3)")
            .bind(&package)
            .bind(&path)
            .bind(file_hash)
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok((package, true))
}

/// `store_file_in_database` — md5-identified blob insert (deduplicated).
pub async fn store_file_in_database(db: &PgPool, fname: &str) -> Result<String, String> {
    let data = std::fs::read(fname).map_err(|e| format!("{fname}: {e}"))?;
    let hash = md5_hex(&data);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM checker_files WHERE file_hash = $1")
        .bind(&hash)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;
    if count == 0 {
        sqlx::query("INSERT INTO checker_files (file_hash, content) VALUES ($1, $2)")
            .bind(&hash)
            .bind(&data)
            .execute(db)
            .await
            .map_err(|e| e.to_string())?;
        println!("Stored {fname} as {hash} in db");
    }
    Ok(hash)
}

/// `load_package_to_folder` — returns Ok(true) if the package was loaded,
/// Ok(false) if the target folder already existed. LFS: files > 500000 bytes
/// go to `lfs_path` and are symlinked in (0o400).
pub async fn load_package_to_folder(
    db: &PgPool,
    package: &str,
    folder: &Path,
    lfs_path: Option<&Path>,
) -> Result<bool, String> {
    if folder.exists() {
        return Ok(false);
    }
    let base = PathBuf::from(format!("{}.tmp", folder.to_string_lossy()));
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    if let Some(lfs) = lfs_path {
        std::fs::create_dir_all(lfs).map_err(|e| e.to_string())?;
    }
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT path, file_hash FROM checker_filesystem WHERE package = $1 ORDER BY path")
            .bind(package)
            .fetch_all(db)
            .await
            .map_err(|e| e.to_string())?;
    for (path, file_hash) in rows {
        let target = base.join(&path);
        match file_hash {
            Some(hash) => {
                let lfs_file = lfs_path.map(|l| l.join(&hash)).filter(|l| l.exists());
                if let Some(lfs) = lfs_file {
                    std::os::unix::fs::symlink(&lfs, &target).map_err(|e| e.to_string())?;
                } else {
                    let (content,): (Vec<u8>,) =
                        sqlx::query_as("SELECT content FROM checker_files WHERE file_hash = $1")
                            .bind(&hash)
                            .fetch_one(db)
                            .await
                            .map_err(|e| e.to_string())?;
                    if let Some(lfs_path) = lfs_path.filter(|_| content.len() > 500_000) {
                        let lfs = lfs_path.join(&hash);
                        let mut f = std::fs::File::create(&lfs).map_err(|e| e.to_string())?;
                        f.write_all(&content).map_err(|e| e.to_string())?;
                        std::os::unix::fs::symlink(&lfs, &target).map_err(|e| e.to_string())?;
                        std::fs::set_permissions(
                            &lfs,
                            std::os::unix::fs::PermissionsExt::from_mode(0o400),
                        )
                        .map_err(|e| e.to_string())?;
                    } else {
                        std::fs::write(&target, &content).map_err(|e| e.to_string())?;
                    }
                }
            }
            None => {
                std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            }
        }
    }
    std::fs::rename(&base, folder).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_patterns_match_python_regexes() {
        assert!(is_ignored("__pycache__"));
        assert!(is_ignored("x.pyc"));
        assert!(is_ignored(".idea"));
        assert!(is_ignored(".github"));
        assert!(is_ignored(".mypy_cache"));
        assert!(is_ignored("gamelib"));
        assert!(!is_ignored("checker.py"));
        assert!(!is_ignored("game_library"));
    }

    #[test]
    fn py_json_pairs_matches_json_dumps_separators() {
        // Python: json.dumps([["a", "dir"], ["b/x", "abc"]]) == '[["a", "dir"], ["b/x", "abc"]]'
        let infos = vec![
            ("a".to_string(), "dir".to_string()),
            ("b/x".to_string(), "abc".to_string()),
        ];
        assert_eq!(py_json_pairs(&infos), r#"[["a", "dir"], ["b/x", "abc"]]"#);
    }
}
