//! `scripts/patch_prepare.py` — create the ansible playbook + inventory for a
//! patch script. ARGUMENTS: <patch-script> [<additional-file> ...]

use ctf_scripts_rs::patch_utils::PatchUtils;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = ctf_scripts_rs::init("patch_prepare.rs")
        .await
        .unwrap_or_else(|e| ctf_scripts_rs::die(&e));
    let args: Vec<String> = std::env::args().collect();
    let patchfile = args
        .get(1)
        .unwrap_or_else(|| {
            ctf_scripts_rs::die("usage: patch_prepare <patch-script> [<additional-file> ...]")
        })
        .clone();
    let p = PatchUtils::new(
        &cfg,
        PathBuf::from(patchfile),
        args[2..].iter().map(PathBuf::from).collect(),
    );

    match p.create_ansible_template() {
        Ok(tmpl) => println!("[*] Created ansible template: {}", tmpl.display()),
        Err(e) => ctf_scripts_rs::die(&e),
    }
    match p.create_ansible_hosts_template(&db).await {
        Ok(tmpl) => println!("[*] Created hosts template:   {}", tmpl.display()),
        Err(e) => ctf_scripts_rs::die(&e),
    }
    println!("    Use patch_publish.rs when you're done editing.");
}
