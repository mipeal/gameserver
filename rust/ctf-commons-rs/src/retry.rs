//! Port of `saarctf_commons/db_utils.py` `retry_on_sql_error` — async, with the
//! retryable-error predicate injected (the sqlx impl arrives with wave 1).

use std::future::Future;
use std::time::Duration;

/// Retry `f` while `is_retryable` says the error is a SQL/connection error.
/// `attempts` total tries, `sleeptime` between them; the last error is returned.
pub async fn retry_on_sql_error<T, E, F, Fut>(
    mut f: F,
    is_retryable: fn(&E) -> bool,
    attempts: u32,
    sleeptime: Duration,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut failures = 0u32;
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if !is_retryable(&e) {
                    return Err(e);
                }
                failures += 1;
                if failures < attempts {
                    tokio::time::sleep(sleeptime).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
}
