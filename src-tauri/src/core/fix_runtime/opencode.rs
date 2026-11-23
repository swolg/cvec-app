use super::{
    mcp::Bridge,
    process::{self, Control, StreamProcess},
    *,
};
use crate::core::opencode_runtime::OpenCodeRuntime;
use serde_json::json;
use std::time::{Duration, Instant};

struct OpenCodeFixRuntime {
    id: String,
    binary: String,
    directory: Arc<tempfile::TempDir>,
    tools: Value,
    sink: Arc<dyn FixSink>,
    control: Arc<Control>,
}
pub(super) fn create(
    config: &OpenCodeConnectionConfig,
    directory: &Path,
    tools: Value,
    sink: Arc<dyn FixSink>,
) -> Result<Arc<dyn FixRuntime>, Error> {
    std::fs::create_dir_all(directory)?;
    Ok(Arc::new(OpenCodeFixRuntime {
        id: identity()?,
        binary: config.binary_path.clone().unwrap_or("opencode".into()),
        directory: Arc::new(tempfile::tempdir_in(directory)?),
        tools,
        sink,
        control: Arc::new(Control::default()),
    }))
}
fn configuration(bridge: &Bridge, tools: &Value) -> (Value, Value) {
    let mut permissions = vec![json!({"action":"*","resource":"*","effect":"deny"})];
    let mut policies = vec![json!({"action":"tool.use","resource":"*","effect":"deny"})];
    for tool in tools.as_array().into_iter().flatten() {
        if let Some(name) = tool["name"].as_str() {
            permissions
                .push(json!({"action":format!("cvec_{name}"),"resource":"*","effect":"allow"}));
            policies.push(
                json!({"action":"tool.use","resource":format!("cvec_{name}:*"),"effect":"allow"}),
            );
        }
    }
    policies.extend([
        json!({"action":"integration.use","resource":"*","effect":"deny"}),
        json!({"action":"integration.use","resource":"mcp:cvec","effect":"allow"}),
        json!({"action":"mcp.use","resource":"*","effect":"deny"}),
        json!({"action":"mcp.use","resource":"cvec","effect":"allow"}),
        json!({"action":"skill.use","resource":"*","effect":"deny"}),
        json!({"action":"plugin.use","resource":"*","effect":"deny"}),
    ]);
    let permissions = json!(permissions);
    let config = json!({
        "permissions":permissions,
        "agents":{"cvec-fix":{"description":"CVEC bounded workspace fixes","mode":"primary","permissions":permissions}},
        "mcp":{"servers":{"cvec":{"type":"remote","url":bridge.url,"oauth":false,"codemode":false,"headers":{"Authorization":bridge.authorization},"timeout":{"execution":600000}}}},
        "experimental":{"policies":policies}
    });
    (config, permissions)
}

fn confirm_completed_turn(
    runtime: &OpenCodeRuntime,
    binary: &str,
    directory: &Path,
    session: &str,
    control: &Control,
) -> Result<bool, Error> {
    let mut command = crate::core::toolchain::command(binary);
    command
        .args([
            "api",
            "session.message.list",
            "--param",
            &format!("sessionID={session}"),
            "--param",
            "limit=2",
            "--param",
            "order=desc",
        ])
        .current_dir(directory);
    runtime.configure_client(&mut command);
    let output = bounded_process_output_cancellable(
        &mut command,
        Duration::from_secs(4),
        Some(&control.cancelled),
    )
    .map_err(|_| Error::App("OpenCode's final fix status could not be verified.".into()))?;
    if !output.status.success() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let Some(messages) = value.get("data").and_then(Value::as_array) else {
        return Ok(false);
    };
    // OpenCode 2.0.32 can reconcile the last assistant text after its live
    // step_finish event was missed. Its persisted idle marker and final
    // assistant finish establish completion without trusting exit code alone.
    Ok(messages.len() == 2
        && messages.iter().all(|message| {
            message.get("sessionID").is_none_or(|id| id == session)
                && message.get("error").is_none_or(Value::is_null)
        })
        && messages[0]["type"] == "idle"
        && messages[0]["outcome"] == "succeeded"
        && messages[1]["type"] == "assistant"
        && messages[1]["finish"] == "stop")
}
impl FixRuntime for OpenCodeFixRuntime {
    fn id(&self) -> &str {
        &self.id
    }
    fn start(
        &self,
        prompt: String,
        model: Option<&str>,
        _effort: Option<&str>,
    ) -> Result<String, Error> {
        let model = model
            .filter(|model| model.contains('/') && model.len() <= 256)
            .ok_or_else(|| {
                Error::App("Select an OpenCode provider/model before starting a fix.".into())
            })?
            .to_string();
        self.control.begin()?;
        let turn = identity()?;
        let (session, turn_worker, binary, directory, tools, sink, control) = (
            self.id.clone(),
            turn.clone(),
            self.binary.clone(),
            self.directory.clone(),
            self.tools.clone(),
            self.sink.clone(),
            self.control.clone(),
        );
        std::thread::spawn(move || {
            sink.event(FixEvent::Started {
                session: session.clone(),
                turn: turn_worker.clone(),
            });
            let result = (|| {
                control.check()?;
                let version = bounded_process_output_cancellable(
                    crate::core::toolchain::command(&binary).arg("--version"),
                    Duration::from_secs(4),
                    Some(&control.cancelled),
                )
                .map_err(|_| Error::App("OpenCode version could not be checked.".into()))?;
                if !version.status.success()
                    || !version_at_least(&String::from_utf8_lossy(&version.stdout), "2.0.0")
                {
                    return Err(Error::App(
                        "OpenCode fixes require managed OpenCode V2. Update the runtime first."
                            .into(),
                    ));
                }
                control.check()?;
                let bridge = Bridge::start(
                    tools.clone(),
                    sink.clone(),
                    session.clone(),
                    turn_worker.clone(),
                )?;
                let (config, permissions) = configuration(&bridge, &tools);
                let runtime = OpenCodeRuntime::start_with_policy(
                    &binary,
                    directory.path(),
                    config.to_string(),
                    "cvec-fix",
                    &permissions,
                    Some(&control.cancelled),
                )?;
                let inventory = runtime.models_cancellable(
                    &binary,
                    directory.path(),
                    Some(&model),
                    &control.cancelled,
                )?;
                control.check()?;
                if !inventory.status.success()
                    || !String::from_utf8_lossy(&inventory.stdout)
                        .lines()
                        .any(|line| line.trim() == model)
                {
                    return Err(Error::App(
                        "The selected OpenCode model is unavailable. Refresh the connection."
                            .into(),
                    ));
                }
                let mut command = crate::core::toolchain::command(&binary);
                command
                    .args([
                        "run", "--format", "json", "--model", &model, "--agent", "cvec-fix",
                    ])
                    .current_dir(directory.path())
                    .env("PWD", directory.path());
                runtime.configure_client(&mut command);
                command.arg("--").arg(format!("{INSTRUCTIONS}\n\n{prompt}"));
                let stream = StreamProcess::start_output(command, control.clone())?;
                let deadline = Instant::now() + Duration::from_secs(30 * 60);
                let mut completed = false;
                let mut native_session = None;
                while let Some(value) = stream.next(deadline)? {
                    if let Some(identity) = value.get("sessionID") {
                        let id = identity.as_str().ok_or_else(|| {
                            Error::App("OpenCode returned an invalid fix session identity.".into())
                        })?;
                        if id.is_empty()
                            || id.len() > 256
                            || native_session
                                .as_deref()
                                .is_some_and(|current| current != id)
                        {
                            return Err(Error::App(
                                "OpenCode returned inconsistent fix session identities.".into(),
                            ));
                        }
                        native_session = Some(id.to_owned());
                    }
                    if value["type"] == "error" {
                        let detail = crate::core::opencode_host::sanitize_diagnostic(
                            value
                                .pointer("/error/message")
                                .and_then(Value::as_str)
                                .unwrap_or("Check its login and selected model."),
                        );
                        return Err(Error::App(format!(
                            "OpenCode could not complete this fix: {detail}"
                        )));
                    }
                    if value["type"] == "text" {
                        if let Some(text) = value
                            .pointer("/part/text")
                            .or_else(|| value.get("text"))
                            .and_then(Value::as_str)
                        {
                            sink.event(FixEvent::Message {
                                session: session.clone(),
                                turn: turn_worker.clone(),
                                text: text.into(),
                            });
                        }
                    }
                    if value["type"] == "step_finish" {
                        completed = value
                            .pointer("/part/reason")
                            .or_else(|| value.get("reason"))
                            .and_then(Value::as_str)
                            == Some("stop");
                    }
                }
                bridge.revoke();
                stream.wait_success()?;
                let native_session = native_session.as_deref().ok_or_else(|| Error::App(
                    "OpenCode exited without a valid fix session identity. Review retained changes.".into(),
                ))?;
                if !completed
                    && !confirm_completed_turn(
                        &runtime,
                        &binary,
                        directory.path(),
                        native_session,
                        &control,
                    )?
                {
                    return Err(Error::App(
                        "OpenCode exited without a completed fix turn. Review retained changes."
                            .into(),
                    ));
                }
                Ok(())
            })();
            process::finish(&*sink, &control, &session, &turn_worker, result);
        });
        Ok(turn)
    }
    fn stop(&self, _turn: Option<&str>) -> Result<(), Error> {
        self.control.stop();
        Ok(())
    }
}
impl Drop for OpenCodeFixRuntime {
    fn drop(&mut self) {
        self.control.stop();
    }
}
