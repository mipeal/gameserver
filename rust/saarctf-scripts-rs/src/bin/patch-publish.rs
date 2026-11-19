//! `scripts/patch_publish.py` — publish patch files and print the ansible
//! commands. ARGUMENTS: <patch-script> [<additional-file> ...]

use saarctf_scripts_rs::patch_utils::PatchUtils;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let (cfg, db, _redis) = saarctf_scripts_rs::init("patch_publish.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let args: Vec<String> = std::env::args().collect();
    let patchfile = args
        .get(1)
        .unwrap_or_else(|| {
            saarctf_scripts_rs::die("usage: patch_publish <patch-script> [<additional-file> ...]")
        })
        .clone();
    let p = PatchUtils::new(
        &cfg,
        PathBuf::from(patchfile),
        args[2..].iter().map(PathBuf::from).collect(),
    );

    match p.create_ansible_hosts_template(&db).await {
        Ok(tmpl) => println!("[*] Created hosts template:   {}", tmpl.display()),
        Err(e) => saarctf_scripts_rs::die(&e),
    }
    let urls = match p.publish_patch_files() {
        Ok(urls) => urls,
        Err(e) => saarctf_scripts_rs::die(&e),
    };
    println!("[*] Published patch files. Download URLs:");
    for url in urls {
        println!("- {url}");
    }
    println!();
    let ansible_file = p.ansible_filename();
    let dir = ansible_file
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let name = ansible_file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    println!("To test this patch against NOP team:");
    println!(r#">   cd "{dir}""#);
    println!(r#">   ansible-playbook {name} -i hosts_nop.yaml"#);
    println!();
    println!("To deploy this patch:");
    println!(r#">   cd "{dir}""#);
    println!(r#">   ansible-playbook {name} -i hosts.yaml"#);
}
