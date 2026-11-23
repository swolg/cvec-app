//! A private V2 server keeps location initialization alive between inventory
//! checks and a turn. V2's one-shot `models --standalone` can return before the
//! provider plugins have loaded, so an empty first response is not authoritative.
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::process::{spawn_owned, OwnedChild};
use super::provider_connections::{bounded_process_output_cancellable, ProcessProbeError};
use crate::Error;

pub(crate) struct OpenCodeRuntime {
    child: OwnedChild,
    pub url: String,
    password: String,
}

impl Drop for OpenCodeRuntime {
    fn drop(&mut self) {
        // EOF releases OpenCode's lease; the owned-child guard then stops its
        // process group and reaps the server without touching other runtimes.
        self.child.stdin.take();
    }
}

impl OpenCodeRuntime {
    pub fn start(binary: &str, directory: &Path) -> Result<Self, Error> {
        Self::start_with_policy(
            binary,
            directory,
            read_only_config(),
            "cvec",
            &json!([{"action":"*","resource":"*","effect":"deny"}]),
            None,
        )
    }

    pub(crate) fn start_with_policy(
        binary: &str,
        directory: &Path,
        config: String,
        agent: &str,
        permissions: &Value,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Self, Error> {
        check_cancelled(cancelled)?;
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).map_err(|_| {
            Error::App("Could not create a private OpenCode server credential.".into())
        })?;
        let password = secret
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut output = tempfile::tempfile()?;
        let mut command = super::toolchain::command(binary);
        let child = spawn_owned(
            command
                .args(["serve", "--stdio", "--hostname", "127.0.0.1", "--port", "0"])
                .current_dir(directory)
                .env("PWD", directory)
                .env("OPENCODE_PASSWORD", &password)
                .env("OPENCODE_CONFIG_CONTENT", config)
                .stdin(Stdio::piped())
                .stdout(output.try_clone()?)
                .stderr(Stdio::null()),
        )
        .map_err(|_| Error::App("OpenCode V2 private server could not start.".into()))?;
        let mut runtime = Self {
            child,
            url: String::new(),
            password,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            check_cancelled(cancelled)?;
            output.seek(SeekFrom::Start(0))?;
            let mut text = String::new();
            (&mut output).take(64 * 1024).read_to_string(&mut text)?;
            if let Some(url) = text
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find_map(|value| value.get("url").and_then(Value::as_str).map(str::to_owned))
            {
                let parsed = reqwest::Url::parse(&url).map_err(|_| {
                    Error::App("OpenCode returned an invalid private server URL.".into())
                })?;
                if parsed.scheme() != "http"
                    || parsed.host_str() != Some("127.0.0.1")
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || parsed.query().is_some()
                    || parsed.fragment().is_some()
                {
                    return Err(Error::App(
                        "OpenCode private server must use loopback HTTP.".into(),
                    ));
                }
                runtime.url = url;
                runtime.wait_for_agent(binary, directory, agent, permissions, cancelled)?;
                return Ok(runtime);
            }
            if runtime.child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(Error::App(
            "OpenCode V2 private server startup failed or timed out.".into(),
        ))
    }

    pub fn configure_client(&self, command: &mut Command) {
        command
            .arg("--server")
            .arg(&self.url)
            .env("OPENCODE_PASSWORD", &self.password);
    }

    fn wait_for_agent(
        &self,
        binary: &str,
        directory: &Path,
        agent: &str,
        permissions: &Value,
        cancelled: Option<&AtomicBool>,
    ) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            check_cancelled(cancelled)?;
            let mut command = super::toolchain::command(binary);
            command.args(["api", "agent.list"]).current_dir(directory);
            self.configure_client(&mut command);
            let output = bounded_process_output_cancellable(
                &mut command,
                deadline.saturating_duration_since(Instant::now()),
                cancelled,
            )
            .map_err(|_| Error::App("OpenCode V2 agent initialization timed out.".into()))?;
            if output.status.success() {
                if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
                    if has_agent_policy(&value, agent, permissions) {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(Error::App("OpenCode V2 did not initialize CVEC's required agent permissions. Check the OpenCode configuration.".into()))
    }

    pub fn models(
        &self,
        binary: &str,
        directory: &Path,
        required_model: Option<&str>,
    ) -> Result<std::process::Output, Error> {
        let deadline = Instant::now() + Duration::from_secs(8);
        self.models_until(binary, directory, required_model, deadline, None)
    }

    pub fn models_cancellable(
        &self,
        binary: &str,
        directory: &Path,
        required_model: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<std::process::Output, Error> {
        self.models_until(
            binary,
            directory,
            required_model,
            Instant::now() + Duration::from_secs(8),
            Some(cancelled),
        )
    }

    fn models_until(
        &self,
        binary: &str,
        directory: &Path,
        required_model: Option<&str>,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
    ) -> Result<std::process::Output, Error> {
        let mut last_inventory = None;
        loop {
            check_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return last_inventory
                    .ok_or_else(|| Error::App("OpenCode V2 model discovery timed out.".into()));
            }
            let mut command = super::toolchain::command(binary);
            command.arg("models").current_dir(directory);
            self.configure_client(&mut command);
            let output =
                match bounded_process_output_cancellable(&mut command, remaining, cancelled) {
                    Ok(output) => output,
                    // Initialization can remain empty until the deadline. A final
                    // retry exhausting that budget must not erase a valid inventory.
                    Err(ProcessProbeError::Timeout)
                        if Instant::now() >= deadline && last_inventory.is_some() =>
                    {
                        return Ok(last_inventory.unwrap());
                    }
                    Err(ProcessProbeError::Cancelled) => {
                        return Err(Error::App("Fix stopped.".into()))
                    }
                    Err(_) => {
                        return Err(Error::App("OpenCode V2 model discovery timed out.".into()))
                    }
                };
            let inventory = String::from_utf8_lossy(&output.stdout);
            let available = required_model.map_or_else(
                || !inventory.trim().is_empty(),
                |model| inventory.lines().any(|line| line.trim() == model),
            );
            if !output.status.success()
                || available
                || Instant::now() + Duration::from_millis(100) >= deadline
            {
                return Ok(output);
            }
            last_inventory = Some(output);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), Error> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        return Err(Error::App("Fix stopped.".into()));
    }
    Ok(())
}

#[cfg(test)]
fn has_read_only_agent(value: &Value) -> bool {
    has_agent_policy(
        value,
        "cvec",
        &json!([{"action":"*","resource":"*","effect":"deny"}]),
    )
}

pub(crate) fn has_agent_policy(value: &Value, agent_id: &str, expected: &Value) -> bool {
    let Some(expected) = expected.as_array().filter(|rules| !rules.is_empty()) else {
        return false;
    };
    value
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|agents| {
            agents.iter().any(|agent| {
                if agent.get("id").and_then(Value::as_str) != Some(agent_id) {
                    return false;
                }
                let Some(rules) = agent.get("permissions").and_then(Value::as_array) else {
                    return false;
                };
                rules
                    .windows(expected.len())
                    .rposition(|window| window == expected)
                    .is_some_and(|index| {
                        rules[index + expected.len()..]
                            .iter()
                            .all(|rule| rule.get("effect").and_then(Value::as_str) == Some("deny"))
                    })
            })
        })
}

fn read_only_config() -> String {
    let deny = json!([{"action": "*", "resource": "*", "effect": "deny"}]);
    json!({
        "permissions": deny,
        "agents": {"cvec": {
            "description": "CVEC bounded read-only security chat", "mode": "primary", "permissions": deny
        }},
        "experimental": {"policies": [
            {"action": "tool.use", "resource": "*", "effect": "deny"},
            {"action": "integration.use", "resource": "*", "effect": "deny"}
        ]}
    }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn model_discovery_keeps_the_last_inventory_when_a_final_retry_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("models-fixture");
        std::fs::write(
            &binary,
            r#"#!/usr/bin/env python3
import pathlib, time
marker = pathlib.Path(__file__).with_suffix('.queried')
if marker.exists():
    time.sleep(2)
marker.touch()
print('')
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = OpenCodeRuntime {
            child: spawn_owned(
                Command::new("cat")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null()),
            )
            .unwrap(),
            url: "http://127.0.0.1:4096".into(),
            password: "fixture".into(),
        };
        let output = runtime
            .models_until(
                binary.to_str().unwrap(),
                directory.path(),
                None,
                Instant::now() + Duration::from_millis(800),
                None,
            )
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
        // With no successful response, the same timeout is a real failure.
        assert!(runtime
            .models_until(
                binary.to_str().unwrap(),
                directory.path(),
                None,
                Instant::now() + Duration::from_millis(100),
                None,
            )
            .is_err());
    }

    #[test]
    fn readiness_requires_effective_final_deny_rule() {
        assert!(!has_read_only_agent(&json!({"data": []})));
        assert!(!has_read_only_agent(
            &json!({"data": [{"id":"cvec","permissions":[]}]})
        ));
        let deny = json!({"action":"*", "resource":"*", "effect":"deny"});
        assert!(has_read_only_agent(
            &json!({"data":[{"id":"cvec","permissions":[deny]}]})
        ));
        assert!(!has_read_only_agent(
            &json!({"data":[{"id":"cvec","permissions":[deny,{"action":"shell","resource":"*","effect":"allow"}]}]})
        ));
        assert!(has_read_only_agent(
            &json!({"data":[{"id":"cvec","permissions":[deny,{"action":"browser","resource":"*","effect":"deny"}]}]})
        ));
    }
}
