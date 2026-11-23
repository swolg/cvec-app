//! Read process pipes concurrently with explicit byte and time limits. A timeout
//! or output limit kills and reaps the child before returning to the caller.
use crate::Error;
use std::{
    process::{Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{watch, Notify},
};

static SHUTDOWN: OnceLock<watch::Sender<bool>> = OnceLock::new();
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static IDLE: Notify = Notify::const_new();
fn shutdown_signal() -> &'static watch::Sender<bool> {
    SHUTDOWN.get_or_init(|| watch::channel(false).0)
}
struct ActiveProcess;
impl Drop for ActiveProcess {
    fn drop(&mut self) {
        if ACTIVE.fetch_sub(1, Ordering::SeqCst) == 1 {
            IDLE.notify_one();
        }
    }
}
/// Stop only commands created through this module and let their owners reap
/// them. No process names or retained PIDs are used to discover children.
pub(crate) async fn shutdown() -> Result<(), Error> {
    shutdown_signal().send_replace(true);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let notified = IDLE.notified();
            if ACTIVE.load(Ordering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
    })
    .await
    .map_err(|_| Error::Scanner("Scanner shutdown exceeded its cleanup deadline".into()))
}

#[cfg(unix)]
struct ProcessGroup {
    pid: i32,
    armed: bool,
}
#[cfg(unix)]
impl ProcessGroup {
    /// The caller must create this child's group with `process_group(0)`.
    fn for_child(pid: u32) -> Self {
        Self {
            pid: pid as i32,
            armed: true,
        }
    }

    fn stop(&mut self) {
        if self.armed && self.pid > 1 {
            unsafe extern "C" {
                fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
            }
            // This group was created for this child, never inherited from cvec.
            unsafe {
                kill(-self.pid, 9);
            }
            self.armed = false;
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A synchronous command and its isolated Unix process group share one lifetime.
/// Helpers are stopped even when the direct child has already exited, while
/// processes outside this newly created group are never signalled.
pub(crate) struct OwnedChild {
    child: std::process::Child,
    #[cfg(unix)]
    group: ProcessGroup,
}

pub(crate) fn spawn_owned(command: &mut std::process::Command) -> std::io::Result<OwnedChild> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    Ok(OwnedChild {
        #[cfg(unix)]
        group: ProcessGroup::for_child(child.id()),
        child,
    })
}

impl OwnedChild {
    /// Keep the child's cached exit status available, but stop helpers now:
    /// inherited stdout handles must not delay the caller's protocol EOF.
    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = self.child.try_wait()?;
        #[cfg(unix)]
        if status.is_some() {
            self.group.stop();
        }
        Ok(status)
    }
}

impl std::ops::Deref for OwnedChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.group.stop();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const MAX_STDERR: usize = 256 * 1024;
async fn read_pipe(pipe: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    pipe.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        return Err(Error::Scanner(
            "Scanner output exceeded the supported size".into(),
        ));
    }
    Ok(bytes)
}
pub(crate) async fn output(
    command: Command,
    input: Option<&[u8]>,
    deadline: Duration,
    stdout_limit: usize,
) -> Result<Output, Error> {
    output_with_shutdown(
        command,
        input,
        deadline,
        stdout_limit,
        shutdown_signal().subscribe(),
    )
    .await
}
async fn output_with_shutdown(
    command: Command,
    input: Option<&[u8]>,
    deadline: Duration,
    stdout_limit: usize,
    shutdown: watch::Receiver<bool>,
) -> Result<Output, Error> {
    output_with_cancellation(command, input, deadline, stdout_limit, shutdown, None).await
}

pub(crate) async fn output_cancellable(
    command: Command,
    deadline: Duration,
    stdout_limit: usize,
    cancellation: Arc<AtomicBool>,
) -> Result<Output, Error> {
    output_with_cancellation(
        command,
        None,
        deadline,
        stdout_limit,
        shutdown_signal().subscribe(),
        Some(cancellation),
    )
    .await
}

async fn output_with_cancellation(
    mut command: Command,
    input: Option<&[u8]>,
    deadline: Duration,
    stdout_limit: usize,
    mut shutdown: watch::Receiver<bool>,
    cancellation: Option<Arc<AtomicBool>>,
) -> Result<Output, Error> {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _active = ActiveProcess;
    if *shutdown.borrow() {
        return Err(Error::Scanner(
            "Scanner command interrupted during app shutdown".into(),
        ));
    }
    if cancellation
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
    {
        return Err(Error::App(
            "Workspace command stopped before execution.".into(),
        ));
    }
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let mut group = ProcessGroup::for_child(child.id().unwrap_or(0));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Scanner("Scanner stdout is unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Scanner("Scanner stderr is unavailable".into()))?;
    let mut stdin = child.stdin.take();
    let work = tokio::time::timeout(deadline, async {
        let write = async {
            if let (Some(pipe), Some(bytes)) = (stdin.as_mut(), input) {
                pipe.write_all(bytes).await?;
            }
            drop(stdin);
            Ok::<_, Error>(())
        };
        let wait = async {
            let status = child.wait().await?;
            // The direct command has finished. Helpers retaining its stdout or
            // stderr must not keep the pipe readers pending until the deadline.
            #[cfg(unix)]
            group.stop();
            Ok::<_, Error>(status)
        };
        let ((), stdout, stderr, status) = tokio::try_join!(
            write,
            read_pipe(stdout, stdout_limit),
            read_pipe(stderr, MAX_STDERR),
            wait
        )?;
        Ok::<_, Error>(Output {
            status,
            stdout,
            stderr,
        })
    });
    let result = tokio::select! {
        result = work => result,
        _ = shutdown.changed() => Ok(Err(Error::Scanner("Scanner command interrupted during app shutdown".into()))),
        _ = async {
            loop {
                if cancellation.as_ref().is_some_and(|flag| flag.load(Ordering::SeqCst)) { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }, if cancellation.is_some() => Ok(Err(Error::App("Workspace command stopped. Partial changes are preserved.".into()))),
    };
    match result {
        Ok(Ok(output)) => Ok(output),
        other => {
            #[cfg(unix)]
            group.stop();
            let _ = child.start_kill();
            let _ = child.wait().await;
            match other {
                Ok(Err(error)) => Err(error),
                _ => Err(Error::Scanner(format!(
                    "Scanner command exceeded its {} second deadline",
                    deadline.as_secs()
                ))),
            }
        }
    }
}
pub(crate) fn output_sync(
    command: &mut std::process::Command,
    deadline: Duration,
    stdout_limit: usize,
) -> Result<Output, Error> {
    let command = std::mem::replace(command, std::process::Command::new("unused"));
    // A dedicated executor also makes this helper safe for CLI and async callers.
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(output(Command::from(command), None, deadline, stdout_limit))
    })
    .join()
    .map_err(|_| Error::Scanner("Scanner command worker failed".into()))?
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn assert_exited_parent_does_not_wait_on_inherited_pipes(cancellable: bool) {
        for exit_code in [0, 9] {
            let mut command = Command::new("python3");
            command.args([
                "-c",
                r#"import subprocess, sys
helper = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
sys.stdout.buffer.write(b'o' * 100000)
sys.stdout.flush()
sys.stderr.buffer.write(b'e' * 50000)
sys.stderr.flush()
sys.exit(int(sys.argv[1]))
"#,
                &exit_code.to_string(),
            ]);
            let output = if cancellable {
                output_cancellable(
                    command,
                    Duration::from_secs(2),
                    200_000,
                    Arc::new(AtomicBool::new(false)),
                )
                .await
            } else {
                output(command, None, Duration::from_secs(2), 200_000).await
            }
            .expect("an exited command must not wait for inherited helper pipes");
            assert_eq!(output.status.code(), Some(exit_code));
            assert_eq!(output.stdout, vec![b'o'; 100_000]);
            assert_eq!(output.stderr, vec![b'e'; 50_000]);
        }
    }

    #[tokio::test]
    async fn exited_scanner_parent_does_not_wait_for_inherited_pipes() {
        assert_exited_parent_does_not_wait_on_inherited_pipes(false).await;
    }

    #[tokio::test]
    async fn exited_workspace_command_does_not_wait_for_inherited_pipes() {
        assert_exited_parent_does_not_wait_on_inherited_pipes(true).await;
    }

    #[tokio::test]
    async fn output_is_bounded_and_waits_for_process_cleanup() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do printf '1234567890'; done"]);
        let start = std::time::Instant::now();
        assert!(output(command, None, Duration::from_secs(2), 1024)
            .await
            .unwrap_err()
            .to_string()
            .contains("size"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 5"]);
        assert!(output(command, None, Duration::from_millis(30), 1024)
            .await
            .unwrap_err()
            .to_string()
            .contains("deadline"));
    }
    #[tokio::test]
    async fn stdin_and_both_pipes_are_drained_concurrently() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "cat; printf diagnostic >&2"]);
        let input = vec![b'x'; 100_000];
        let result = output(command, Some(&input), Duration::from_secs(2), 200_000)
            .await
            .unwrap();
        assert_eq!(result.stdout, input);
        assert_eq!(result.stderr, b"diagnostic");
    }

    #[tokio::test]
    async fn cancellation_timeout_and_dropped_future_stop_descendants() {
        for mode in ["shutdown", "timeout", "dropped"] {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("leaked-child");
            let ready = directory.path().join("started");
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "(sleep 0.3; printf leaked > \"$1\") & printf ready > \"$2\"; wait",
                    "cvec-test",
                ])
                .arg(&marker)
                .arg(&ready);
            let (sender, receiver) = watch::channel(false);
            let deadline = if mode == "timeout" {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(3)
            };
            let task = tokio::spawn(async move {
                output_with_shutdown(command, None, deadline, 1024, receiver).await
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            if mode == "shutdown" {
                sender.send_replace(true);
            }
            if mode == "dropped" {
                task.abort();
            }
            let result = task.await;
            if mode == "dropped" {
                assert!(result.unwrap_err().is_cancelled());
            } else {
                let error = result.unwrap().unwrap_err().to_string();
                assert!(error.contains(if mode == "shutdown" {
                    "interrupted"
                } else {
                    "deadline"
                }));
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
            assert!(!marker.exists(), "descendant survived {mode}");
        }
    }
}
