use tokio::time::Duration;

use super::models::ScheduleInterval;

pub fn duration_for(interval: ScheduleInterval) -> Option<Duration> {
    interval
        .duration()
        .and_then(|duration| Duration::try_from(duration).ok())
}

// Changing a timer must not cancel a scan that has already acquired the scan guard.
// Dropping this waiter detaches the worker; the worker still saves its results
// and runs its own error/next-schedule handling.
pub async fn finish_started_job<F>(job: F) -> Result<(), tokio::task::JoinError>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(job).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn changing_the_timer_does_not_cancel_a_started_scan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = crate::core::db::Database::new(temp.path().join("cvec.db")).expect("database");
        db.sync_repositories(&[crate::core::repo_discovery::DiscoveredRepo {
            name: "scheduled".into(),
            path: temp.path().join("repo"),
            remote_url: None,
            remote_host: None,
            provider: None,
        }])
        .expect("repository");
        let repo = db.list_active_repositories().expect("repositories")[0].id;
        let worker_db = db.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (saved_tx, saved_rx) = tokio::sync::oneshot::channel();
        let timer = tokio::spawn(finish_started_job(async move {
            let run = worker_db
                .insert_scan_start(
                    repo,
                    &crate::core::models::ScannerVersions {
                        syft: "test".into(),
                        grype: "test".into(),
                    },
                )
                .expect("scan started");
            started_tx.send(()).expect("started");
            release_rx.await.expect("release scan");
            worker_db
                .complete_scan(
                    repo,
                    run,
                    &[],
                    &[],
                    &crate::core::models::DeltaSummary::default(),
                )
                .expect("persist results");
            saved_tx
                .send(())
                .expect("save results after timer cancellation");
        }));
        started_rx.await.expect("scan started");
        timer.abort();
        assert!(timer.await.expect_err("timer aborted").is_cancelled());
        release_tx.send(()).expect("worker still running");
        tokio::time::timeout(Duration::from_secs(1), saved_rx)
            .await
            .expect("scan completes")
            .expect("results saved");
        let scan = db
            .repo_detail(repo)
            .expect("repository detail")
            .latest_scan
            .expect("completed scan");
        assert_eq!(scan.status, crate::core::models::ScanRunStatus::Completed);
        assert!(scan.finished_at.is_some());
    }
}
