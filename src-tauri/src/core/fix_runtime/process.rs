use super::*;
use crate::core::process::{spawn_owned, OwnedChild};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Control {
    pub cancelled: AtomicBool,
    pub started: AtomicBool,
    child: Mutex<Option<OwnedChild>>,
}
impl Control {
    pub fn check(&self) -> Result<(), Error> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(Error::App("Fix stopped.".into()));
        }
        Ok(())
    }
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.reap();
    }
    pub fn reap(&self) {
        let child = self.child.lock().expect("fix process").take();
        drop(child);
    }
    pub fn begin(&self) -> Result<(), Error> {
        if self.cancelled.load(Ordering::SeqCst) || self.started.swap(true, Ordering::SeqCst) {
            return Err(Error::App(
                "This fix session has already started or stopped.".into(),
            ));
        }
        Ok(())
    }

    fn poll_exit(&self) -> Result<(), Error> {
        if let Some(child) = self.child.lock().expect("fix process").as_mut() {
            let _ = child.try_wait()?;
        }
        Ok(())
    }

    fn wait_success(&self) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(Error::App("Fix stopped.".into()));
            }
            let mut lock = self.child.lock().expect("fix process");
            let child = lock
                .as_mut()
                .ok_or_else(|| Error::App("Fix runtime exited unexpectedly.".into()))?;
            if let Some(status) = child.try_wait()? {
                let child = lock.take();
                drop(lock);
                drop(child);
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::App("The fix runtime exited unsuccessfully. Review retained changes before retrying.".into()))
                };
            }
            drop(lock);
            if Instant::now() >= deadline {
                return Err(Error::App(
                    "The fix runtime closed its output without exiting.".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop();
    }
}

type AgentRequestHandler = Box<dyn Fn(&Value) -> Value + Send>;

struct WriteTask {
    bytes: Vec<u8>,
    done: SyncSender<std::io::Result<()>>,
}

pub(super) struct StreamProcess {
    control: Arc<Control>,
    input: Option<SyncSender<WriteTask>>,
    messages: Receiver<Result<Value, String>>,
    next_id: u64,
    request_handler: Option<AgentRequestHandler>,
    deadline: Instant,
}
impl StreamProcess {
    pub fn start(command: Command, control: Arc<Control>) -> Result<Self, Error> {
        Self::start_with_stdin(command, control, Stdio::piped())
    }

    /// One-shot CLI commands receive their task in argv and must see stdin EOF.
    pub fn start_output(command: Command, control: Arc<Control>) -> Result<Self, Error> {
        Self::start_with_stdin(command, control, Stdio::null())
    }

    fn start_with_stdin(
        mut command: Command,
        control: Arc<Control>,
        stdin: Stdio,
    ) -> Result<Self, Error> {
        let mut lock = control.child.lock().expect("fix process");
        if control.cancelled.load(Ordering::SeqCst) {
            return Err(Error::App("Fix stopped.".into()));
        }
        let mut child = spawn_owned(
            command
                .stdin(stdin)
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .map_err(|_| {
            Error::App(
                "The selected fix runtime could not start. Check its installation and login."
                    .into(),
            )
        })?;
        let writer = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::App("Runtime output unavailable.".into()))?;
        *lock = Some(child);
        drop(lock);
        let input = writer.map(|mut writer| {
            let (input, writes) = mpsc::sync_channel::<WriteTask>(1);
            std::thread::spawn(move || {
                while let Ok(task) = writes.recv() {
                    let result = writer.write_all(&task.bytes).and_then(|_| writer.flush());
                    let failed = result.is_err();
                    if task.done.send(result).is_err() || failed {
                        break;
                    }
                }
            });
            input
        });
        let (sender, messages) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match Read::by_ref(&mut reader)
                    .take(1_048_577)
                    .read_line(&mut line)
                {
                    Ok(0) => break,
                    Ok(_) if line.len() <= 1_048_576 => {
                        let value = serde_json::from_str(&line)
                            .map_err(|_| "Runtime returned malformed output.".into());
                        let invalid = value.is_err();
                        if sender.send(value).is_err() || invalid {
                            break;
                        }
                    }
                    _ => {
                        let _ =
                            sender.send(Err("Runtime output exceeded its limit or failed.".into()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            control,
            input,
            messages,
            next_id: 1,
            request_handler: None,
            deadline: Instant::now() + Duration::from_secs(30 * 60),
        })
    }
    pub fn set_request_handler(&mut self, handler: impl Fn(&Value) -> Value + Send + 'static) {
        self.request_handler = Some(Box::new(handler));
    }

    pub fn wait_success(&self) -> Result<(), Error> {
        self.control.wait_success()
    }

    pub fn send(&mut self, value: &Value) -> Result<(), Error> {
        let deadline = self.deadline.min(Instant::now() + Duration::from_secs(30));
        self.input_ready(deadline)?;
        let input = self.input.as_ref().ok_or_else(|| {
            Error::App("This runtime command does not accept protocol input.".into())
        })?;
        let mut bytes = serde_json::to_vec(value)?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Error::App(
                "The fix runtime input exceeded its size limit.".into(),
            ));
        }
        bytes.push(b'\n');
        let (done, completion) = mpsc::sync_channel(1);
        self.input_ready(deadline)?;
        // Never block the protocol worker on a pipe that the agent stopped
        // reading. Killing/reaping the owned child also releases the writer.
        if input.try_send(WriteTask { bytes, done }).is_err() {
            self.control.reap();
            return Err(Error::App("The fix runtime input is unavailable.".into()));
        }
        loop {
            self.input_ready(deadline)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            match completion.recv_timeout(remaining.min(Duration::from_millis(100))) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    self.control.reap();
                    return Err(error.into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.control.reap();
                    return Err(Error::App("The fix runtime input disconnected.".into()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn input_ready(&self, deadline: Instant) -> Result<(), Error> {
        if self.control.cancelled.load(Ordering::SeqCst) {
            return Err(Error::App("Fix stopped.".into()));
        }
        if Instant::now() >= deadline {
            self.control.reap();
            return Err(Error::App("The fix runtime input timed out.".into()));
        }
        Ok(())
    }
    pub fn next(&self, deadline: Instant) -> Result<Option<Value>, Error> {
        loop {
            if self.control.cancelled.load(Ordering::SeqCst) {
                return Err(Error::App("Fix stopped.".into()));
            }
            if Instant::now() >= deadline.min(self.deadline) {
                return Err(Error::App("The fix runtime timed out.".into()));
            }
            self.control.poll_exit()?;
            match self.messages.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(value)) => return Ok(Some(value)),
                Ok(Err(error)) => return Err(Error::App(error)),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        mut update: impl FnMut(&Value),
    ) -> Result<Value, Error> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let timeout = if method == "session/prompt" {
            30 * 60
        } else {
            30
        };
        let deadline = Instant::now() + Duration::from_secs(timeout);
        loop {
            let value = self
                .next(deadline)?
                .ok_or_else(|| Error::App("The fix runtime disconnected.".into()))?;
            if value.get("method").is_some() && value.get("id").is_some() {
                // Never approve native tools, permissions, terminal or filesystem RPC.
                let response = if let Some(handler) = &self.request_handler {
                    handler(&value)
                } else if value["method"] == "session/request_permission" {
                    json!({"jsonrpc":"2.0","id":value["id"],"result":{"outcome":{"outcome":"cancelled"}}})
                } else {
                    json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Only the CVEC workspace MCP tools are available."}})
                };
                self.send(&response)?;
            } else if value.get("id").and_then(Value::as_u64) == Some(id) {
                if value.get("error").is_some() {
                    return Err(Error::App(format!("The fix runtime rejected {method}. Check its login, model and required capabilities.")));
                }
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| Error::App("Runtime returned an invalid response.".into()));
            } else {
                update(&value);
            }
        }
    }
}
impl Drop for StreamProcess {
    fn drop(&mut self) {
        self.control.reap();
    }
}

pub(super) fn finish(
    sink: &dyn FixSink,
    control: &Control,
    session: &str,
    turn: &str,
    result: Result<(), Error>,
) {
    control.reap();
    let cancelled = control.cancelled.load(Ordering::SeqCst);
    sink.event(FixEvent::Finished {
        session: session.into(),
        turn: turn.into(),
        status: if cancelled {
            "interrupted"
        } else if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into(),
        error: result.err().map(|error| error.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn assert_provider_descendants_reaped(mode: &str) {
        let control = Arc::new(Control::default());
        let mut command = Command::new("python3");
        command.args([
            "-c",
            r#"import json, subprocess, sys, time
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
print(json.dumps({'childPid': child.pid}), flush=True)
if sys.argv[1] != 'success':
    time.sleep(120)
"#,
            mode,
        ]);
        let stream = StreamProcess::start(command, control.clone()).unwrap();
        let ready = stream
            .next(Instant::now() + Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let pid = ready["childPid"].as_u64().unwrap().to_string();
        let alive = || {
            // A terminated orphan can briefly remain a zombie on Unix. Its
            // retained PID does not mean the helper can still execute.
            Command::new("/bin/ps")
                .args(["-p", &pid, "-o", "stat="])
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|output| {
                    let state = String::from_utf8_lossy(&output.stdout);
                    output.status.success()
                        && !state.trim().is_empty()
                        && !state.trim().starts_with('Z')
                })
        };
        let mut unrelated = Command::new("sleep").arg("120").spawn().unwrap();
        match mode {
            "stop" => control.stop(),
            "drop" => drop(stream),
            "success" => stream.wait_success().unwrap(),
            _ => unreachable!(),
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let descendant_survived = alive();
        let unrelated_survived = unrelated.try_wait().unwrap().is_none();
        // Always clean up the synthetic fixtures, including on the old failing
        // implementation. Never search for or signal another provider process.
        if descendant_survived {
            let _ = Command::new("/bin/kill").args(["-KILL", &pid]).status();
        }
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        assert!(unrelated_survived, "unrelated process was stopped");
        assert!(control.child.lock().unwrap().is_none());
        assert!(!descendant_survived, "owned descendant survived {mode}");
    }

    #[cfg(unix)]
    #[test]
    fn provider_stop_reaps_owned_descendants() {
        assert_provider_descendants_reaped("stop");
    }

    #[cfg(unix)]
    #[test]
    fn provider_stream_drop_reaps_owned_descendants() {
        assert_provider_descendants_reaped("drop");
    }

    #[cfg(unix)]
    #[test]
    fn provider_success_reaps_owned_descendants() {
        assert_provider_descendants_reaped("success");
    }

    #[cfg(unix)]
    fn assert_stream_finishes_when_parent_exits(exit_code: i32) {
        let control = Arc::new(Control::default());
        let mut command = Command::new("python3");
        command.args([
            "-c",
            r#"import json, subprocess, sys
helper = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
for sequence in range(64):
    print(json.dumps({'sequence': sequence}), flush=True)
sys.exit(int(sys.argv[1]))
"#,
            &exit_code.to_string(),
        ]);
        let stream = StreamProcess::start_output(command, control.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        let ended = loop {
            match stream.next(deadline) {
                Ok(Some(value)) => received.push(value["sequence"].as_u64().unwrap()),
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        // Preserve the actual exit outcome, and clean up the fixture even when
        // the old stream loop timed out waiting for the helper's inherited pipe.
        let outcome = stream.wait_success();
        drop(stream);
        control.stop();
        assert!(ended.is_ok(), "stream waited after parent exit: {ended:?}");
        assert_eq!(received, (0..64).collect::<Vec<_>>());
        if exit_code == 0 {
            assert!(outcome.is_ok(), "successful parent: {outcome:?}");
        } else {
            assert!(outcome
                .unwrap_err()
                .to_string()
                .contains("exited unsuccessfully"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn stream_drains_after_successful_parent_exit_with_inherited_stdout() {
        assert_stream_finishes_when_parent_exits(0);
    }

    #[cfg(unix)]
    #[test]
    fn stream_drains_after_failed_parent_exit_with_inherited_stdout() {
        assert_stream_finishes_when_parent_exits(9);
    }

    #[test]
    fn nonreading_provider_cannot_block_input_past_the_session_deadline() {
        let control = Arc::new(Control::default());
        let mut command = Command::new("python3");
        command.args(["-c", "import time; time.sleep(120)"]);
        let mut stream = StreamProcess::start(command, control.clone()).unwrap();
        stream.deadline = Instant::now() + Duration::from_millis(150);
        // A watchdog makes this regression fail instead of hanging on the old
        // blocking pipe implementation. It never starts a provider or model.
        let (done, finished) = mpsc::sync_channel::<()>(1);
        let watchdog_control = control.clone();
        let watchdog = std::thread::spawn(move || {
            if finished.recv_timeout(Duration::from_secs(2)).is_err() {
                watchdog_control.stop();
            }
        });
        let started = Instant::now();
        let error = stream.send(&json!({"jsonrpc":"2.0","id":1,"method":"session/prompt","params":{"text":"x".repeat(512 * 1024)}})).unwrap_err();
        let elapsed = started.elapsed();
        let _ = done.send(());
        watchdog.join().unwrap();
        assert!(error.to_string().contains("input timed out"), "{error}");
        assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
        assert!(control.child.lock().unwrap().is_none());
    }
}
