use super::{
    mcp::Bridge,
    process::{self, Control, StreamProcess},
    *,
};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

struct GrokRuntime {
    id: String,
    binary: String,
    directory: Arc<tempfile::TempDir>,
    tools: Value,
    sink: Arc<dyn FixSink>,
    control: Arc<Control>,
}
pub(super) fn create(
    config: &GrokConnectionConfig,
    directory: &Path,
    tools: Value,
    sink: Arc<dyn FixSink>,
) -> Result<Arc<dyn FixRuntime>, Error> {
    std::fs::create_dir_all(directory)?;
    Ok(Arc::new(GrokRuntime {
        id: identity()?,
        binary: config.binary_path.clone().unwrap_or("grok".into()),
        directory: Arc::new(tempfile::tempdir_in(directory)?),
        tools,
        sink,
        control: Arc::new(Control::default()),
    }))
}
impl FixRuntime for GrokRuntime {
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
            .filter(|model| !model.trim().is_empty() && model.len() <= 256)
            .ok_or_else(|| Error::App("Select a Grok model before starting a fix.".into()))?
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
                let allowed_tools = tool_names(&tools);
                prepare_policy(directory.path(), &allowed_tools)?;
                verify_policy(
                    &binary,
                    directory.path(),
                    allowed_tools.len(),
                    &control.cancelled,
                )?;
                control.check()?;
                let bridge =
                    Bridge::start(tools, sink.clone(), session.clone(), turn_worker.clone())?;
                let mut command = crate::core::toolchain::command(&binary);
                command
                    .args([
                        "--trust",
                        "--tools",
                        "",
                        "--permission-mode",
                        "dontAsk",
                        "agent",
                        "--no-leader",
                        "stdio",
                    ])
                    .current_dir(directory.path())
                    .env("GROK_OAUTH2_REFERRER", "cvec")
                    .env("GROK_SUBAGENTS", "0");
                let mut rpc = StreamProcess::start(command, control.clone())?;
                let initialized = rpc.request("initialize", json!({"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"cvec","version":env!("CARGO_PKG_VERSION")}}), |_| {})?;
                if initialized
                    .pointer("/agentCapabilities/_meta/x.ai~1hooks/blockingEvents")
                    .and_then(Value::as_array)
                    .is_none_or(|events| !events.contains(&json!("pre_tool_use")))
                    || initialized["protocolVersion"] != 1
                    || initialized
                        .pointer("/agentCapabilities/mcpCapabilities/http")
                        .and_then(Value::as_bool)
                        != Some(true)
                {
                    return Err(Error::App("This Grok runtime does not support the required ACP HTTP MCP bridge. Update Grok before using fix mode.".into()));
                }
                let method = crate::core::grok_host::grok_auth_method(&initialized)?;
                rpc.request("authenticate", json!({"methodId":method}), |_| {})?;
                let created = rpc.request("session/new", json!({"cwd":directory.path(),"_meta":{"x.ai/hooks":{"pre_tool_use":[{"hookCallbackIds":["cvec-tools"],"timeout":5}]}},"mcpServers":[{"type":"http","name":"cvec","url":bridge.url,"headers":[{"name":"Authorization","value":bridge.authorization}]}]}), |_| {})?;
                let provider_session = created
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                    .ok_or_else(|| Error::App("Grok returned an invalid session.".into()))?;
                rpc.request(
                    "session/set_model",
                    json!({"sessionId":provider_session,"modelId":model}),
                    |_| {},
                )?;
                let hook_session = provider_session.to_string();
                rpc.set_request_handler(move |request| {
                    agent_response(request, &hook_session, &allowed_tools)
                });
                let mut text = String::new();
                let completed = rpc.request("session/prompt", json!({"sessionId":provider_session,"prompt":[{"type":"text","text":format!("{INSTRUCTIONS}\n\n{prompt}")}]}), |value| {
                    if value.pointer("/params/sessionId").and_then(Value::as_str) != Some(provider_session) { return; }
                    if value.pointer("/params/update/sessionUpdate").and_then(Value::as_str) == Some("agent_message_chunk") {
                        if let Some(delta) = value.pointer("/params/update/content/text").and_then(Value::as_str) {
                            if text.len() + delta.len() <= 32_000 { text.push_str(delta); }
                        }
                    }
                })?;
                bridge.revoke();
                if !text.is_empty() && !control.cancelled.load(Ordering::SeqCst) {
                    sink.event(FixEvent::Message {
                        session: session.clone(),
                        turn: turn_worker.clone(),
                        text,
                    });
                }
                if completed["stopReason"] != "end_turn" {
                    return Err(Error::App("Grok stopped without completing the fix turn. Review the retained changes.".into()));
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

fn verify_policy(
    binary: &str,
    directory: &Path,
    tool_count: usize,
    cancelled: &AtomicBool,
) -> Result<(), Error> {
    // Trust only CVEC's newly generated, empty runtime directory. Never trust
    // or launch Grok in the user's checkout/worktree. Without this explicit
    // scope Grok silently skips the project deny rules in headless mode.
    let directory = directory.canonicalize()?;
    let mut command = crate::core::toolchain::command(binary);
    command
        .args(["--trust", "inspect", "--json"])
        .current_dir(&directory);
    let output =
        bounded_process_output_cancellable(&mut command, Duration::from_secs(5), Some(cancelled))
            .map_err(|_| {
            Error::App("Grok's fix policy could not be inspected. Update Grok and retry.".into())
        })?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| Error::App("Grok returned an unsupported policy inventory.".into()))?;
    if !output.status.success() || !policy_is_isolated(&value, &directory, tool_count) {
        return Err(Error::App("Grok could not verify the isolated fix policy. This mode requires the generated permission rules and no inherited hooks, plugins or MCP servers. Review the Grok configuration or choose another connection.".into()));
    }
    Ok(())
}

fn policy_is_isolated(value: &Value, directory: &Path, tool_count: usize) -> bool {
    let source = format!("{} (config)", directory.join(".grok/config.toml").display());
    value["projectTrusted"] == true
        && value
            .pointer("/permissions/sources")
            .and_then(Value::as_array)
            .is_some_and(|sources| sources.contains(&json!(source)))
        && value
            .pointer("/permissions/loaded")
            .and_then(Value::as_u64)
            .is_some_and(|count| count >= (6 + tool_count) as u64)
        && ["/permissions/skipped", "/hooks", "/plugins", "/mcpServers"]
            .iter()
            .all(|path| {
                value
                    .pointer(path)
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
            })
}
impl Drop for GrokRuntime {
    fn drop(&mut self) {
        self.control.stop();
    }
}

fn tool_names(tools: &Value) -> Vec<String> {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(|name| format!("cvec__{name}")))
        .collect()
}

fn prepare_policy(directory: &Path, tools: &[String]) -> Result<(), Error> {
    std::fs::create_dir(directory.join(".grok"))?;
    std::fs::create_dir(directory.join(".claude"))?;
    // Rules are merged by severity. These deny rules therefore remain effective
    // even if the user's global configuration has broad allow rules.
    let allow = tools
        .iter()
        .map(|name| format!("MCPTool({name})"))
        .collect::<Vec<_>>();
    std::fs::write(directory.join(".grok/config.toml"), format!("[permission]\ndeny = [\"Bash\", \"Read\", \"Edit\", \"Grep\", \"WebFetch\", \"WebSearch\"]\nallow = {}\n", serde_json::to_string(&allow)?))?;
    std::fs::write(
        directory.join(".claude/settings.json"),
        json!({"permissions":{"defaultMode":"dontAsk"}}).to_string(),
    )?;
    Ok(())
}

fn agent_response(request: &Value, session: &str, allowed: &[String]) -> Value {
    let id = &request["id"];
    if request["method"] == "_x.ai/hooks/run" {
        let params = &request["params"];
        let permit = params["sessionId"] == session
            && params["hookCallbackId"] == "cvec-tools"
            && params["hookEventName"] == "pre_tool_use"
            && params["toolName"]
                .as_str()
                .is_some_and(|name| allowed.iter().any(|allowed| allowed == name));
        return json!({"jsonrpc":"2.0","id":id,"result":{"decision":if permit {"continue"} else {"deny"},"systemMessage":"Only the registered CVEC workspace tools are available."}});
    }
    if request["method"] == "session/request_permission" {
        return json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}});
    }
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Native tools and external actions are unavailable."}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_policy_requires_loaded_rules_and_no_inherited_integrations() {
        let directory = Path::new("/fixture");
        let safe = json!({"projectTrusted":true,"permissions":{"sources":["/fixture/.grok/config.toml (config)"],"loaded":13,"skipped":[]},"hooks":[],"plugins":[],"mcpServers":[]});
        assert!(policy_is_isolated(&safe, directory, 7));
        for (path, replacement) in [
            ("/projectTrusted", json!(false)),
            ("/permissions/loaded", json!(6)),
            (
                "/permissions/sources",
                json!(["/unrelated/config.toml (config)"]),
            ),
            ("/permissions/skipped", json!(["invalid rule"])),
            ("/hooks", json!([{"name":"inherited-command"}])),
            ("/plugins", json!([{"name":"inherited-plugin"}])),
            ("/mcpServers", json!([{"name":"external"}])),
        ] {
            let mut unsafe_policy = safe.clone();
            *unsafe_policy.pointer_mut(path).unwrap() = replacement;
            assert!(!policy_is_isolated(&unsafe_policy, directory, 7), "{path}");
        }
        assert!(!policy_is_isolated(&json!({}), directory, 7));
    }

    #[test]
    fn grok_hooks_bind_exact_tool_session_and_callback() {
        let allowed = vec!["cvec__cvec_workspace_read_file".into()];
        let valid = json!({"jsonrpc":"2.0","id":1,"method":"_x.ai/hooks/run","params":{"sessionId":"session","hookCallbackId":"cvec-tools","hookEventName":"pre_tool_use","toolName":allowed[0]}});
        assert_eq!(
            agent_response(&valid, "session", &allowed)["result"]["decision"],
            "continue"
        );
        for field in ["sessionId", "hookCallbackId", "hookEventName", "toolName"] {
            let mut invalid = valid.clone();
            invalid["params"][field] = json!("other");
            assert_eq!(
                agent_response(&invalid, "session", &allowed)["result"]["decision"],
                "deny"
            );
        }
        assert!(agent_response(
            &json!({"id":1,"method":"fs/read_text_file"}),
            "session",
            &allowed
        )
        .get("error")
        .is_some());
    }
}
