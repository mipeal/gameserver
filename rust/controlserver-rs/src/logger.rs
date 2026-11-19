//! Port of `controlserver/logger.py` (central DB log) plus the useful part of
//! `saarctf_commons/logging_utils.py` (tracing setup).

use crate::models::{LOG_DEBUG, LOG_ERROR, LOG_INFO, LOG_NOTIFICATION, LOG_WARNING};
use chrono::Utc;
use sqlx::PgPool;
use tracing::Level;

/// `LogMessage.level_to_python` → tracing level.
fn level_to_tracing(level: i16) -> Level {
    if level <= LOG_DEBUG {
        Level::DEBUG
    } else if level <= LOG_NOTIFICATION {
        Level::INFO
    } else if level <= LOG_WARNING {
        Level::WARN
    } else {
        Level::ERROR
    }
}

/// `log(component, title, text, level)` — also emits to the tracing logger.
pub async fn log(
    db: &PgPool,
    component: &str,
    title: &str,
    text: &str,
    level: i16,
) -> Result<(), sqlx::Error> {
    let msg = if text.is_empty() {
        title.to_string()
    } else {
        format!("{title}\n{text}")
    };
    match level_to_tracing(level) {
        Level::DEBUG => tracing::debug!(target: "log_messages", "{msg}"),
        Level::INFO => tracing::info!(target: "log_messages", "{msg}"),
        Level::WARN => tracing::warn!(target: "log_messages", "{msg}"),
        _ => tracing::error!(target: "log_messages", "{msg}"),
    }
    log_with_retry(db, component, title, text, level).await
}

/// `retry_on_sql_error(attempts=2)` wrapper.
async fn log_with_retry(
    db: &PgPool,
    component: &str,
    title: &str,
    text: &str,
    level: i16,
) -> Result<(), sqlx::Error> {
    for attempt in 0..2 {
        match sqlx::query(
            "INSERT INTO logmessages (component, level, title, text, created) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(component)
        .bind(level)
        .bind(title)
        .bind(text)
        .bind(Utc::now())
        .execute(db)
        .await
        {
            Ok(_) => return Ok(()),
            Err(e) if attempt == 0 && is_retryable(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn is_retryable(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Database(_) | sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut
    )
}

/// `log_exception` — message is "TypeName: message", stacktrace as text body.
pub async fn log_exception(db: &PgPool, component: &str, e: &dyn std::error::Error) {
    let mut msg = e.to_string();
    if let Some(src) = e.source() {
        msg = format!("{msg}: {src}");
    }
    let stacktrace = format!("{e:?}");
    let _ = log(db, component, &msg, &stacktrace, LOG_ERROR).await;
}

/// `log_result_of_execution` — run `f`, log success (with elapsed) or the error.
pub async fn log_result_of_execution<T, E, F, Fut>(
    db: &PgPool,
    component: &str,
    f: F,
    success: Option<&str>,
    error: Option<&str>,
) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display + std::error::Error,
{
    let t = std::time::Instant::now();
    match f().await {
        Ok(v) => {
            if let Some(success) = success {
                let _ = log(
                    db,
                    component,
                    &success.replace("{}", &format!("{:.3}", t.elapsed().as_secs_f64())),
                    "",
                    LOG_INFO,
                )
                .await;
            }
            Ok(v)
        }
        Err(e) => {
            if let Some(error) = error {
                let _ = log(
                    db,
                    component,
                    &error.replace("{}", &format!("{e}")),
                    &format!("{e:?}"),
                    LOG_ERROR,
                )
                .await;
            }
            Err(e)
        }
    }
}

/// `setup_script_logging` — INFO to stdout, optional file, optional ECS (json) file.
pub fn setup_script_logging(logfile: Option<&str>) {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    let info = LevelFilter::INFO;
    let open = |path: &str| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("logfile openable")
    };
    let stdout = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(std::io::stdout)
        .with_filter(info)
        .boxed();
    let extra: Option<Box<dyn tracing_subscriber::Layer<_> + Send + Sync>> =
        match (logfile, std::env::var("ECS_LOGFILE").ok()) {
            // add_ecs_logging: ECS_LOGFILE gets a JSON-formatted copy
            (_, Some(path)) => Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::sync::Mutex::new(open(&path)))
                    .with_filter(info)
                    .boxed(),
            ),
            (Some(path), None) => Some(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::sync::Mutex::new(open(path)))
                    .with_filter(info)
                    .boxed(),
            ),
            (None, None) => None,
        };
    match extra {
        Some(l) => tracing_subscriber::registry()
            .with(stdout)
            .with(l)
            .try_init()
            .ok(),
        None => tracing_subscriber::registry().with(stdout).try_init().ok(),
    };
}
