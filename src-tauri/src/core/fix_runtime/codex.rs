use super::{process::Control, *};
use crate::core::companion_host::FixSessionHandler;
use std::sync::{atomic::Ordering, Mutex};

struct CodexRuntime {
    host: CompanionHost,
    id: String,
    _handler: Arc<Handler>,
    control: Control,
    turn: Mutex<Option<String>>,
}
struct Handler(Arc<dyn FixSink>);
impl FixSessionHandler for Handler {
    fn call(
        &self,
        tool: &str,
        arguments: &Value,
        thread: &str,
        turn: &str,
    ) -> Result<Value, Error> {
        self.0.call(tool, arguments, thread, turn)
    }
    fn notify(&self, method: &str, params: &Value) {
        if let Some(event) = event(method, params) {
            self.0.event(event);
        }
    }
}
pub(crate) fn event(method: &str, params: &Value) -> Option<FixEvent> {
    let session = params.get("threadId")?.as_str()?.to_owned();
    if method == "cvec/interrupted" {
        return Some(FixEvent::Disconnected { session });
    }
    let turn = params
        .get("turnId")
        .or_else(|| params.pointer("/turn/id"))?
        .as_str()?
        .to_owned();
    match method {
        "turn/started" => Some(FixEvent::Started { session, turn }),
        "turn/completed" => Some(FixEvent::Finished {
            session,
            turn,
            status: params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("failed")
                .into(),
            error: params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
                .map(|message| {
                    crate::core::provider_connections::sanitize_provider_diagnostic(
                        message, "Codex",
                    )
                }),
        }),
        "item/completed"
            if params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage") =>
        {
            Some(FixEvent::Message {
                session,
                turn,
                text: params.pointer("/item/text")?.as_str()?.into(),
            })
        }
        _ => None,
    }
}
pub(super) fn create(
    host: CompanionHost,
    tools: Value,
    sink: Arc<dyn FixSink>,
    model: Option<&str>,
) -> Result<Arc<dyn FixRuntime>, Error> {
    let handler = Arc::new(Handler(sink));
    let id = host
        .create_fix_session(tools, handler.clone(), model)
        .map_err(sanitize_rpc_error)?;
    Ok(Arc::new(CodexRuntime {
        host,
        id,
        _handler: handler,
        control: Control::default(),
        turn: Mutex::new(None),
    }))
}

fn sanitize_rpc_error(error: Error) -> Error {
    Error::App(
        crate::core::provider_connections::sanitize_provider_diagnostic(
            &error.to_string(),
            "Codex",
        ),
    )
}

impl FixRuntime for CodexRuntime {
    fn id(&self) -> &str {
        &self.id
    }
    fn start(
        &self,
        prompt: String,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<String, Error> {
        self.control.begin()?;
        let turn = self
            .host
            .start_fix_turn(&self.id, prompt, model, effort)
            .map_err(sanitize_rpc_error)?;
        *self.turn.lock().expect("Codex fix turn") = Some(turn.clone());
        if self.control.cancelled.load(Ordering::SeqCst) {
            self.stop(Some(&turn))?;
            return Err(Error::App(
                "Fix stopped while Codex was starting its turn.".into(),
            ));
        }
        Ok(turn)
    }
    fn stop(&self, turn: Option<&str>) -> Result<(), Error> {
        self.control.stop();
        let known = self.turn.lock().expect("Codex fix turn").take();
        if let Some(turn) = turn.or(known.as_deref()) {
            self.host
                .interrupt_turn(&self.id, turn)
                .map_err(sanitize_rpc_error)?;
        }
        Ok(())
    }
}
impl Drop for CodexRuntime {
    fn drop(&mut self) {
        self.control.stop();
        self.host.forget_fix_session(&self.id);
    }
}
