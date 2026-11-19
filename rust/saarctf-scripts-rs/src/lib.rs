//! Shared setup for the ported `scripts/*.py` CLIs: config + DB + redis init
//! (like `load_default_config() / config.set_script() / init_database()`) and
//! the `query_yes_no` prompt.

use controlserver_rs::load_config;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;

/// Script bootstrap: logging, config, `set_clientname('script-<name>')`,
/// DB pool, redis connection.
pub async fn init(name: &str) -> Result<(Config, PgPool, redis::aio::ConnectionManager), String> {
    setup_logging();
    let cfg = load_config()?;
    saarctf_commons_rs::redis::set_clientname(&format!("script-{name}"), true);
    let db = controlserver_rs::connect_db(&cfg)
        .await
        .map_err(|e| format!("db connect: {e}"))?;
    let redis = saarctf_commons_rs::redis::get_redis_connection(&cfg.redis)
        .await
        .map_err(|e| format!("redis connect: {e}"))?;
    Ok((cfg, db, redis))
}

fn setup_logging() {
    // tracing_subscriber default fmt; init failure means it's already set up
    let _ = tracing_subscriber_fmt();
}

fn tracing_subscriber_fmt() -> Result<(), String> {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer as _;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
                    "%Y-%m-%d %H:%M:%S".to_string(),
                ))
                .with_filter(LevelFilter::INFO),
        )
        .try_init()
        .map_err(|e| e.to_string())
}

/// `query_yes_no` — reads stdin, `default` is the empty-input answer.
pub fn query_yes_no(question: &str, default: &str) -> bool {
    let prompt = match default {
        "yes" => " [Y/n] ",
        "no" => " [y/N] ",
        _ => " [y/n] ",
    };
    let valid = ["yes", "y", "ye", "no", "n"];
    loop {
        print!("{question}{prompt}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            // EOF behaves like an empty answer
            return default == "yes";
        }
        let choice = line.trim().to_lowercase();
        if choice.is_empty() {
            return default == "yes";
        }
        if valid.contains(&choice.as_str()) {
            return choice.starts_with('y');
        }
        println!("Please respond with 'yes' or 'no' (or 'y' or 'n').");
    }
}

/// `Timer.count_master_timer` — PUBSUB NUMSUB `timing:master` count.
pub async fn count_master_timer(
    redis: &mut redis::aio::ConnectionManager,
) -> Result<i64, redis::RedisError> {
    let sub: (String, i64) = redis::cmd("PUBSUB")
        .arg("NUMSUB")
        .arg("timing:master")
        .query_async(redis)
        .await?;
    Ok(sub.1)
}

/// Exit with the standard script error shape.
pub fn die(msg: &str) -> ! {
    eprintln!("[ERROR] {msg}");
    std::process::exit(1);
}
