//! `wireguard_sync/__main__.py` — `generate` pregenerates key pairs, the
//! default command initializes all interfaces and runs the sync loop.

use wireguard_sync_rs::{generate_key_pair, InterfaceManager, KeyStore, Settings};

/// `generate_key_pairs` — teams 1..=amount, skipping existing entries.
fn generate_key_pairs(amount: i64, key_store: &mut KeyStore) {
    for i in 1..=amount {
        if key_store.contains(i) {
            println!("Skipping team{i}");
            continue;
        }
        let key_pair = generate_key_pair();
        if let Err(e) = key_store.put_keys(i, &key_pair) {
            eprintln!("team{i}: {e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut command: Option<String> = None;
    let mut amount: i64 = 400;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "generate" => command = Some(arg),
            "-a" | "--amount" => {
                amount = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    eprintln!("wireguard-sync: --amount needs a number");
                    std::process::exit(2);
                });
            }
            other => {
                eprintln!("wireguard-sync: unknown argument {other:?} (subcommands: generate)");
                std::process::exit(2);
            }
        }
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
            "%Y-%m-%d %H:%M:%S".to_string(),
        ))
        .init();

    let settings = match Settings::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wireguard-sync: {e}");
            std::process::exit(1);
        }
    };
    let mut manager = match InterfaceManager::new(settings) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("wireguard-sync: {e}");
            std::process::exit(1);
        }
    };

    match command.as_deref() {
        Some("generate") => generate_key_pairs(amount, &mut manager.keystore),
        _ => {
            if let Err(e) = manager.initialize_all().await {
                eprintln!("wireguard-sync: {e}");
                std::process::exit(1);
            }
            manager.sync_loop().await;
        }
    }
}
