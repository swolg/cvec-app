//! Ordered, bounded conversation persistence. UI events leave this worker only
//! after their deduplication keys, text and lifecycle state are durable.
use super::{
    companion::{CompanionEventEnvelope, CompanionEventKind, COMPANION_EVENT_NAME},
    db::Database,
};
use crate::core::timestamps;
use crate::Error;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

type Sink = Arc<dyn Fn(CompanionEventEnvelope) + Send + Sync>;
type Failure = Arc<dyn Fn(&str, &[CompanionEventEnvelope]) + Send + Sync>;
const QUEUE_CAPACITY: usize = 32;
const MAX_BATCH_EVENTS: usize = 128;
const FRAME: Duration = Duration::from_millis(50);
struct Message {
    bytes: usize,
    barrier: bool,
    event: CompanionEventEnvelope,
    done: Option<mpsc::SyncSender<Result<(), String>>>,
}
struct Inner {
    sender: mpsc::SyncSender<Message>,
    batches: Arc<AtomicU64>,
    events: Arc<AtomicU64>,
    epoch: String,
}
#[derive(Clone)]
pub(crate) struct EventWriter(Arc<Inner>);
impl EventWriter {
    pub(crate) fn new(db: Database, forward: Sink, failure: Failure) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::sync_channel::<Message>(QUEUE_CAPACITY);
        let batches = Arc::new(AtomicU64::new(0));
        let events = Arc::new(AtomicU64::new(0));
        let batch_count = batches.clone();
        let event_count = events.clone();
        std::thread::Builder::new().name("cvec-history-writer".into()).spawn(move || {
            while let Ok(first) = receiver.recv() {
                let deadline = Instant::now() + FRAME;
                let mut bytes = first.bytes;
                let mut messages = vec![first];
                while messages.len() < MAX_BATCH_EVENTS && bytes < 256 * 1024
                    && messages.last().is_some_and(|message| message.event.kind == CompanionEventKind::ItemDelta) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() { break; }
                    match receiver.recv_timeout(remaining) {
                        Ok(message) => { bytes = bytes.saturating_add(message.bytes); messages.push(message); }
                        Err(_) => break,
                    }
                }
                let mut acknowledgements = Vec::new();
                let mut input = Vec::with_capacity(messages.len());
                for message in messages {
                    if let Some(done) = message.done { acknowledgements.push(done); }
                    if !message.barrier {input.push(message.event);}
                }
                let started = Instant::now();
                let result = db.cache_companion_events(&input);
                batch_count.fetch_add(1, Ordering::Relaxed);
                event_count.fetch_add(input.len() as u64, Ordering::Relaxed);
                let result = match result {
                    Ok(accepted) => { for event in coalesce(accepted) { forward(event); } Ok(()) }
                    Err(_) => {
                        let message = "Conversation history could not be saved locally. Refresh the conversation before continuing.";
                        failure(message, &input);
                        Err(message.to_string())
                    }
                };
                if std::env::var_os("CVEC_PERF_TRACE").is_some() {
                    eprintln!("cvec-perf history events={} write_ms={:.3}", input.len(), started.elapsed().as_secs_f64() * 1000.0);
                }
                for done in acknowledgements { let _ = done.send(result.clone()); }
            }
        })?;
        Ok(Self(Arc::new(Inner {
            sender,
            batches,
            events,
            epoch: format!("{}-{}", std::process::id(), timestamps::unix_nanos()),
        })))
    }
    pub(crate) fn push(&self, mut event: CompanionEventEnvelope) -> Result<(), String> {
        if std::env::var_os("CVEC_PERF_TRACE").is_some() {
            let received_at = timestamps::unix_millis();
            mark_stream_received(&mut event, received_at);
            let kind = match event.kind {
                CompanionEventKind::TurnStarted => Some(1),
                CompanionEventKind::ItemDelta => Some(2),
                CompanionEventKind::TurnCompleted => Some(3),
                _ => None,
            };
            if let Some(kind) = kind {
                let text_bytes = event
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .map_or(0, str::len);
                eprintln!("cvec-perf stream kind={kind} epoch_ms={received_at} sequence={} text_bytes={text_bytes}", event.sequence);
            }
        }
        if event.cache_epoch.is_none() {
            event.cache_epoch = Some(self.0.epoch.clone());
        }
        let bytes = serde_json::to_vec(&event.payload)
            .map_err(|_| "Conversation event could not be encoded".to_string())?
            .len();
        if bytes > 4 * 1024 * 1024 {
            return Err("Conversation event exceeds the supported size".into());
        }
        let terminal = event.kind != CompanionEventKind::ItemDelta;
        let (done, result) = if terminal {
            let (sender, receiver) = mpsc::sync_channel(1);
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        self.0
            .sender
            .send(Message {
                bytes,
                barrier: false,
                event,
                done,
            })
            .map_err(|_| "Conversation history writer is unavailable".to_string())?;
        if let Some(result) = result {
            result.recv_timeout(Duration::from_secs(10)).map_err(|_| {
                "Conversation history write did not finish within its deadline".to_string()
            })??;
        }
        if terminal && std::env::var_os("CVEC_PERF_TRACE").is_some() {
            let (batches, events) = self.statistics();
            eprintln!("cvec-perf history total_batches={batches} total_events={events}");
        }
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        let (done, result) = mpsc::sync_channel(1);
        let event = CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: super::companion::COMPANION_SCHEMA_VERSION,
            sequence: 0,
            repository_id: None,
            thread_id: None,
            provider_thread_id: None,
            connection_id: None,
            driver: None,
            model_id: None,
            turn_id: None,
            kind: CompanionEventKind::RuntimeStatus,
            payload: serde_json::Value::Null,
        };
        self.0
            .sender
            .send(Message {
                bytes: 0,
                barrier: true,
                event,
                done: Some(done),
            })
            .map_err(|_| "Conversation history writer is unavailable".to_string())?;
        result
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Conversation history flush timed out".to_string())?
    }
    fn statistics(&self) -> (u64, u64) {
        (
            self.0.batches.load(Ordering::Relaxed),
            self.0.events.load(Ordering::Relaxed),
        )
    }
}
fn same_delta_route(a: &CompanionEventEnvelope, b: &CompanionEventEnvelope) -> bool {
    a.kind == CompanionEventKind::ItemDelta
        && b.kind == CompanionEventKind::ItemDelta
        && a.repository_id == b.repository_id
        && a.thread_id == b.thread_id
        && a.connection_id == b.connection_id
        && a.provider_thread_id == b.provider_thread_id
        && a.provider_segment_id == b.provider_segment_id
        && a.cache_epoch == b.cache_epoch
        && a.driver == b.driver
        && a.model_id == b.model_id
        && a.turn_id == b.turn_id
        && a.payload.get("itemId") == b.payload.get("itemId")
}
// Opt-in timestamps start before persistence/coalescing, so live measurements
// include the same durable writer path as replay. Never include message content
// in numeric trace output.
fn mark_stream_received(event: &mut CompanionEventEnvelope, received_at: i64) {
    if event.kind == CompanionEventKind::ItemDelta {
        if let Some(payload) = event.payload.as_object_mut() {
            payload.insert("perfReceivedAt".into(), serde_json::json!(received_at));
        }
    }
}

fn coalesce(events: Vec<CompanionEventEnvelope>) -> Vec<CompanionEventEnvelope> {
    let mut output = Vec::<CompanionEventEnvelope>::new();
    for event in events {
        if let Some(previous) = output
            .last_mut()
            .filter(|previous| same_delta_route(previous, &event))
        {
            if let (Some(serde_json::Value::String(text)), Some(delta)) = (
                previous.payload.get_mut("text"),
                event
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str),
            ) {
                text.push_str(delta);
                previous.sequence = event.sequence;
                continue;
            }
        }
        output.push(event);
    }
    output
}
// All provider hosts for a database share this single writer and FIFO.
static WRITERS: OnceLock<Mutex<HashMap<PathBuf, Weak<Inner>>>> = OnceLock::new();
pub(crate) fn flush_database(path: &std::path::Path) -> Result<(), String> {
    let writer = WRITERS.get().and_then(|registry| {
        registry
            .lock()
            .ok()
            .and_then(|registry| registry.get(path).and_then(Weak::upgrade))
    });
    if let Some(writer) = writer {
        EventWriter(writer).flush()
    } else {
        Ok(())
    }
}

pub(crate) fn sink(app: AppHandle, db: Database) -> Result<Sink, Error> {
    let mut registry = WRITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| Error::App("Conversation writer registry is unavailable".into()))?;
    registry.retain(|_, writer| writer.strong_count() > 0);
    let writer = if let Some(existing) = registry.get(db.path()).and_then(Weak::upgrade) {
        EventWriter(existing)
    } else {
        let emit_app = app.clone();
        let error_app = app.clone();
        let writer = EventWriter::new(
            db.clone(),
            Arc::new(move |event| {
                let _ = emit_app.emit(COMPANION_EVENT_NAME, event);
            }),
            Arc::new(move |message, input| {
                let _ = error_app.emit("cvec://error", message);
                let mut routes = HashMap::new();
                for event in input.iter().filter(|event| event.thread_id.is_some()) {
                    routes.insert(
                        (event.connection_id.clone(), event.thread_id),
                        event.clone(),
                    );
                }
                let mut failures = routes.into_values().collect::<Vec<_>>();
                failures.sort_by_key(|event| event.sequence);
                for mut event in failures {
                    event.kind = CompanionEventKind::Error;
                    event.payload = serde_json::json!({"code":"local_history_unavailable","message":message,"retryable":true});
                    let _ = error_app.emit(COMPANION_EVENT_NAME, event);
                }
            }),
        )?;
        registry.insert(db.path().to_path_buf(), Arc::downgrade(&writer.0));
        writer
    };
    Ok(Arc::new(move |event| {
        if let Err(message) = writer.push(event) {
            let _ = app.emit("cvec://error", message);
        }
    }))
}
#[cfg(test)]
mod tests {
    use super::super::{companion::COMPANION_SCHEMA_VERSION, repo_discovery::DiscoveredRepo};
    use super::*;
    use serde_json::json;
    fn event(
        thread: i64,
        sequence: u64,
        kind: CompanionEventKind,
        payload: serde_json::Value,
    ) -> CompanionEventEnvelope {
        CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: COMPANION_SCHEMA_VERSION,
            sequence,
            repository_id: Some(thread),
            thread_id: Some(thread),
            provider_thread_id: Some(format!("provider-{thread}")),
            connection_id: Some("codex_default".into()),
            driver: Some("codex".into()),
            model_id: None,
            turn_id: Some(format!("turn-{thread}")),
            kind,
            payload,
        }
    }
    #[test]
    fn stream_timing_keeps_original_text_and_first_batch_timestamp() {
        let mut first = event(
            1,
            1,
            CompanionEventKind::ItemDelta,
            json!({"itemId":"answer","text":"synthetic "}),
        );
        let mut second = event(
            1,
            2,
            CompanionEventKind::ItemDelta,
            json!({"itemId":"answer","text":"evidence"}),
        );
        mark_stream_received(&mut first, 100);
        mark_stream_received(&mut second, 120);
        let output = coalesce(vec![first, second]);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].payload["text"], "synthetic evidence");
        assert_eq!(output[0].payload["perfReceivedAt"], 100);
        let mut completed = event(
            1,
            3,
            CompanionEventKind::TurnCompleted,
            json!({"status":"completed"}),
        );
        mark_stream_received(&mut completed, 130);
        assert!(completed.payload.get("perfReceivedAt").is_none());
    }

    #[test]
    fn burst_writer_is_durable_batched_and_keeps_interleaved_routes_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.db");
        let db = Database::new(path.clone()).unwrap();
        for id in 1..=2 {
            db.sync_repositories_with_completeness(
                &[DiscoveredRepo {
                    name: format!("repo-{id}"),
                    path: temp.path().join(format!("repo-{id}")),
                    remote_url: None,
                    remote_host: None,
                    provider: None,
                }],
                false,
            )
            .unwrap();
            db.create_companion_thread(id, &format!("provider-{id}"), "Conversation", None)
                .unwrap();
        }
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let capture = forwarded.clone();
        let writer = EventWriter::new(
            db.clone(),
            Arc::new(move |event| capture.lock().unwrap().push(event)),
            Arc::new(|error, _| panic!("{error}")),
        )
        .unwrap();
        for sequence in 1..=1000 {
            let thread = if sequence % 2 == 0 { 1 } else { 2 };
            writer
                .push(event(
                    thread,
                    sequence,
                    CompanionEventKind::ItemDelta,
                    json!({"itemId":"answer","text":"x"}),
                ))
                .unwrap();
        }
        writer
            .push(event(
                1,
                1001,
                CompanionEventKind::TurnCompleted,
                json!({"status":"completed"}),
            ))
            .unwrap();
        writer
            .push(event(
                2,
                1002,
                CompanionEventKind::TurnCompleted,
                json!({"status":"completed"}),
            ))
            .unwrap();
        let (batches, events) = writer.statistics();
        assert_eq!(events, 1002);
        assert!(batches < 50, "{batches} writes");
        let restarted = Database::new(path).unwrap();
        for thread in 1..=2 {
            let snapshot = restarted.cached_companion_thread_snapshot(thread).unwrap();
            assert_eq!(snapshot.items.len(), 1);
            assert_eq!(snapshot.items[0].text, "x".repeat(500));
            assert_eq!(
                snapshot.items[0].status,
                super::super::companion::CompanionTimelineItemStatus::Completed
            );
        }
        let events = forwarded.lock().unwrap();
        assert!(events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        let a = event(
            1,
            1,
            CompanionEventKind::ItemDelta,
            json!({"itemId":"answer","text":"a"}),
        );
        let mut b = a.clone();
        b.sequence = 2;
        b.payload["text"] = json!("b");
        let combined = coalesce(vec![a, b]);
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].payload["text"], "ab");
        assert_eq!(combined[0].sequence, 2);
    }
}
