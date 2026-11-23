//! Bounded execution of blocking desktop work. The permit belongs to the actual
//! worker, so cancelling an IPC response cannot release capacity prematurely.
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tokio::sync::Semaphore;

const BLOCKING_CONCURRENCY: usize = 2;
static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static IO_WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static RESCAN_WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub(crate) async fn run<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_with(&WORKERS, "core", operation).await
}

// Slow filesystem/process/provider operations have their own bounded lane. A
// stalled provider cannot consume both slots needed by persisted UI reads.
pub(crate) async fn run_io<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_with(&IO_WORKERS, "io", operation).await
}

// A manual rescan drives an async pipeline from a blocking worker. That pipeline
// uses both core and I/O lanes, so its orchestrator must hold neither permit.
pub(crate) async fn run_rescan<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_with(&RESCAN_WORKERS, "rescan", operation).await
}
async fn run_with<T, F>(
    workers: &'static OnceLock<Arc<Semaphore>>,
    class: &'static str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let queued = Instant::now();
    let permit = workers
        .get_or_init(|| Arc::new(Semaphore::new(BLOCKING_CONCURRENCY)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Desktop worker queue is unavailable".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let started = Instant::now();
        let value = operation();
        if std::env::var_os("CVEC_PERF_TRACE").is_some() {
            eprintln!(
                "cvec-perf worker class={class} queue_ms={:.3} work_ms={:.3}",
                started.duration_since(queued).as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        value
    })
    .await
    .map_err(|cause| format!("Desktop operation could not complete: {cause}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn blocking_operations_are_bounded_and_do_not_block_the_async_caller() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let requests = (0..8).map(|_| {
            let active = active.clone();
            let maximum = maximum.clone();
            run(move || {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(count, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(15));
                active.fetch_sub(1, Ordering::SeqCst);
            })
        });
        let work = futures::future::join_all(requests);
        let heartbeat = async {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            assert!(active.load(Ordering::SeqCst) > 0);
        };
        let (results, _) = tokio::join!(work, heartbeat);
        assert!(results.into_iter().all(|result| result.is_ok()));
        assert!(maximum.load(Ordering::SeqCst) <= BLOCKING_CONCURRENCY);
    }
}
