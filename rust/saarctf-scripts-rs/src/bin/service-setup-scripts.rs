//! `scripts/service_setup_scripts.py` — run the setup scripts from all
//! services. ARGUMENTS: [service names ...]

use controlserver_rs::db_filesystem::load_package_to_folder;
use controlserver_rs::models::Service;
use std::path::Path;
use std::process::Command;

async fn run_service_setup_scripts(db: &sqlx::PgPool, filters: &[String]) -> usize {
    let services: Vec<Service> = if filters.is_empty() {
        sqlx::query_as("SELECT * FROM services WHERE setup_package IS NOT NULL")
            .fetch_all(db)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")))
    } else {
        let mut q = sqlx::QueryBuilder::new(
            "SELECT * FROM services WHERE setup_package IS NOT NULL AND name IN (",
        );
        {
            let mut sep = q.separated(", ");
            for f in filters {
                sep.push_bind(f);
            }
        }
        q.push(")")
            .build_query_as::<Service>()
            .fetch_all(db)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")))
    };
    let mut errors = 0;
    for service in &services {
        let Some(setup_package) = &service.setup_package else {
            continue;
        };
        println!(
            "[{}] Extracting script from package {setup_package} ...",
            service.name
        );
        let path = Path::new("/tmp/setup_packages").join(setup_package);
        load_package_to_folder(
            db,
            setup_package,
            &path,
            Some(Path::new("/tmp/setup_packages/lfs")),
        )
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("[{}] {e}", service.name)));

        let dependencies_script = path.join("dependencies.sh");
        if !dependencies_script.exists() {
            println!(
                "[{}] Script not found: \"{}\"!",
                service.name,
                dependencies_script.to_string_lossy()
            );
            errors += 1;
        } else {
            println!(
                "[{}] Running script \"{}\" ...",
                service.name,
                dependencies_script.to_string_lossy()
            );
            if let Err(e) = Command::new("chmod")
                .arg("+x")
                .arg(&dependencies_script)
                .status()
            {
                saarctf_scripts_rs::die(&format!("[{}] chmod: {e}", service.name));
            }
            match Command::new(&dependencies_script).status() {
                Ok(status) if !status.success() => {
                    errors += 1;
                    println!(
                        "[{}] Script failed with code {:?}.",
                        service.name,
                        status.code()
                    );
                }
                Ok(_) => println!("[{}] Script succeeded.", service.name),
                Err(e) => {
                    errors += 1;
                    println!("[{}] Script failed to run: {e}.", service.name);
                }
            }
        }

        let requirements_file = path.join("checker-requirements.txt");
        if !requirements_file.exists() {
            println!(
                "[{}] Requirements file not found: \"{}\"!",
                service.name,
                requirements_file.to_string_lossy()
            );
            errors += 1;
        } else {
            println!(
                "[{}] Installing from \"{}\" ...",
                service.name,
                requirements_file.to_string_lossy()
            );
            match Command::new("pip")
                .args(["install", "-r"])
                .arg(&requirements_file)
                .status()
            {
                Ok(status) if !status.success() => {
                    errors += 1;
                    println!(
                        "[{}] Pip failed with code {:?}.",
                        service.name,
                        status.code()
                    );
                }
                Ok(_) => println!("[{}] Pip succeeded.", service.name),
                Err(e) => {
                    errors += 1;
                    println!("[{}] Pip failed to run: {e}.", service.name);
                }
            }
        }
    }
    errors
}

#[tokio::main]
async fn main() {
    let (_cfg, db, _redis) = saarctf_scripts_rs::init("service_setup_scripts.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let filters: Vec<String> = std::env::args().skip(1).collect();
    let errors = run_service_setup_scripts(&db, &filters).await;
    if errors > 0 {
        println!("[ERROR] {errors} errors reported.");
        std::process::exit(1);
    }
}
