use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const COMPANION_SCHEMA_VERSION: u32 = 1;
pub const REPOSITORY_CHAT_CONTEXT_VERSION: u32 = 1;
pub const COMPANION_DISCLOSURE_VERSION: u32 = 1;
pub const COMPANION_EVENT_NAME: &str = "cvec://companion";
pub const MAX_CONTEXT_BYTES: usize = 32 * 1024;
pub const MAX_SOURCE_BYTES: usize = 24 * 1024;
pub const MAX_CONTEXT_SOURCES: usize = 24;
pub const MAX_CHAT_DRAFT_UTF16_UNITS: usize = 65_536;

pub(crate) fn chat_text_fits(text: &str) -> bool {
    // UTF-8 uses at most three bytes per UTF-16 code unit.
    text.len() <= 3 * MAX_CHAT_DRAFT_UTF16_UNITS
        && text.encode_utf16().count() <= MAX_CHAT_DRAFT_UTF16_UNITS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionRuntimePhase {
    Disabled,
    Unavailable,
    Discovering,
    Starting,
    Initializing,
    SignedOut,
    Ready,
    ActiveTurn,
    Interrupted,
    Incompatible,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionRuntimeStatus {
    pub schema_version: u32,
    pub phase: CompanionRuntimePhase,
    pub codex_path: Option<String>,
    pub codex_version: Option<String>,
    pub message: Option<String>,
    pub started_at: Option<String>,
    pub active_turns: u32,
}

impl CompanionRuntimeStatus {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            schema_version: COMPANION_SCHEMA_VERSION,
            phase: CompanionRuntimePhase::Unavailable,
            codex_path: None,
            codex_version: None,
            message: Some(message.into()),
            started_at: None,
            active_turns: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAuthKind {
    Chatgpt,
    ApiKey,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionAccountStatus {
    pub schema_version: u32,
    pub signed_in: bool,
    pub auth_kind: CompanionAuthKind,
    pub plan_type: Option<String>,
    pub account_label: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionLoginKind {
    Chatgpt,
    ChatgptDeviceCode,
    ApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionLoginStart {
    pub kind: CompanionLoginKind,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionModel {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub hidden: bool,
    pub reasoning_efforts: Vec<String>,
    pub input_modalities: Vec<String>,
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompanionTokenUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub context_used: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionRateLimitWindow {
    pub used_percent: f64,
    pub window_duration_minutes: Option<u64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompanionUsageStatus {
    pub thread: Option<CompanionTokenUsage>,
    pub primary: Option<CompanionRateLimitWindow>,
    pub secondary: Option<CompanionRateLimitWindow>,
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub supported: bool,
    pub unavailable_reason: Option<String>,
    pub fetched_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionThreadStatus {
    NotLoaded,
    Ready,
    Active,
    Interrupted,
    HistoryUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionThread {
    pub id: i64,
    pub repository_id: i64,
    pub provider: String,
    pub provider_thread_id: String,
    pub title: String,
    pub status: CompanionThreadStatus,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: String,
    pub archived_at: Option<String>,
    pub context_version: u32,
    pub last_model_id: Option<String>,
    pub provider_connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionProviderSegmentStatus {
    Ready,
    Active,
    Interrupted,
    HistoryUnavailable,
}

impl CompanionProviderSegmentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Interrupted => "interrupted",
            Self::HistoryUnavailable => "history_unavailable",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "interrupted" => Self::Interrupted,
            "history_unavailable" => Self::HistoryUnavailable,
            _ => Self::Ready,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionProviderSegment {
    pub id: i64,
    pub thread_id: i64,
    pub ordinal: u32,
    pub provider: String,
    pub connection_id: String,
    pub provider_thread_id: String,
    pub initial_model_id: Option<String>,
    pub last_model_id: Option<String>,
    pub status: CompanionProviderSegmentStatus,
    pub created_at: String,
    pub updated_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionConversationIndex {
    pub schema_version: u32,
    pub threads: Vec<CompanionThread>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionComposerSelection {
    pub repository_id: i64,
    pub connection_id: String,
    pub model_id: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionTimelineItemKind {
    UserMessage,
    AssistantMessage,
    Activity,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionTimelineItemStatus {
    Streaming,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionActivityKind {
    Search,
    Read,
    Context,
    Runtime,
    Reasoning,
    ProviderTransition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionActivity {
    pub kind: CompanionActivityKind,
    pub label: String,
    pub detail: Option<String>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSourceKind {
    Repository,
    Scan,
    Finding,
    Package,
    File,
    Changes,
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionCitation {
    pub source_id: String,
    pub kind: ContextSourceKind,
    pub label: String,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionErrorInfo {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionTimelineItem {
    pub id: String,
    pub kind: CompanionTimelineItemKind,
    pub status: CompanionTimelineItemStatus,
    pub text: String,
    pub activity: Option<CompanionActivity>,
    pub citations: Vec<CompanionCitation>,
    pub error: Option<CompanionErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub created_at: String,
}

impl CompanionTimelineItem {
    pub(crate) fn preserve_cached_model_id(&mut self, cached: &Self) {
        if self.kind != cached.kind {
            return;
        }
        if cached.model_id.is_some()
            && (self.kind == CompanionTimelineItemKind::UserMessage || self.model_id.is_none())
        {
            self.model_id.clone_from(&cached.model_id);
        }
    }

    pub(crate) fn preserve_cached_citations(&mut self, cached: &Self) {
        if self.kind != cached.kind {
            return;
        }
        for citation in &mut self.citations {
            let fallback_label: String = citation.source_id.chars().take(256).collect();
            if citation.stale && citation.label == fallback_label {
                if let Some(previous) = cached
                    .citations
                    .iter()
                    .find(|previous| previous.source_id == citation.source_id)
                {
                    *citation = previous.clone();
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionThreadSnapshot {
    pub thread: CompanionThread,
    pub items: Vec<CompanionTimelineItem>,
    pub usage: Option<CompanionTokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionCachedThreadSnapshot {
    #[serde(default)]
    pub older_cursor: Option<String>,
    #[serde(default)]
    pub event_watermarks: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub streaming_item_ids: Vec<String>,
    #[serde(default)]
    pub provider_ordinals: std::collections::BTreeMap<String, u32>,
    pub schema_version: u32,
    pub thread: CompanionThread,
    pub segments: Vec<CompanionProviderSegment>,
    pub items: Vec<CompanionTimelineItem>,
    pub provider_history_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionEventKind {
    RuntimeStatus,
    AccountStatus,
    ThreadSnapshot,
    ThreadUpdated,
    TurnStarted,
    ItemStarted,
    ItemDelta,
    ItemCompleted,
    Activity,
    Usage,
    TurnCompleted,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionEventEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_segment_id: Option<i64>,
    pub schema_version: u32,
    pub repository_id: Option<i64>,
    pub thread_id: Option<i64>,
    pub provider_thread_id: Option<String>,
    pub connection_id: Option<String>,
    pub driver: Option<String>,
    pub model_id: Option<String>,
    pub turn_id: Option<String>,
    pub sequence: u64,
    pub kind: CompanionEventKind,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextAttachmentKind {
    Repository,
    Scan,
    Finding,
    Package,
    File,
    Changes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextAttachment {
    pub id: String,
    pub kind: ContextAttachmentKind,
    pub label: String,
    pub optional: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextFreshnessState {
    Matched,
    Mismatched,
    Dirty,
    Unknown,
    NewerScan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextFreshness {
    pub state: ContextFreshnessState,
    pub source_revision: Option<String>,
    pub scan_revision: Option<String>,
    pub source_dirty: Option<bool>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextRepository {
    pub id: i64,
    pub name: String,
    pub source_type: String,
    pub source_mode: String,
    pub approved_root: Option<String>,
    pub branch: Option<String>,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextScan {
    pub id: Option<i64>,
    pub status: Option<String>,
    pub scanned_at: Option<String>,
    pub revision: Option<String>,
    pub evidence_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryChatScanStatus {
    pub repository_id: i64,
    pub latest_completed: ContextScan,
    pub latest_attempt: ContextScan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextSource {
    pub id: String,
    pub kind: ContextSourceKind,
    pub label: String,
    pub repository_relative_path: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub scan_id: Option<i64>,
    pub finding_fingerprint: Option<String>,
    pub package_identity: Option<String>,
    pub revision: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextOmission {
    pub category: String,
    pub reason: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UntrustedEvidence {
    pub source_id: String,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryChatContext {
    pub schema_version: u32,
    pub repository: ContextRepository,
    pub scan: ContextScan,
    pub freshness: ContextFreshness,
    pub attachments: Vec<ContextAttachment>,
    pub sources: Vec<ContextSource>,
    pub omissions: Vec<ContextOmission>,
    pub untrusted_evidence: Vec<UntrustedEvidence>,
    pub instructions: Vec<String>,
    pub estimated_bytes: usize,
    pub bounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextAttachmentRequest {
    pub kind: ContextAttachmentKind,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPreviewRequest {
    pub repository_id: i64,
    pub query: String,
    pub attachments: Vec<ContextAttachmentRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionTurnRequest {
    pub repository_id: i64,
    pub thread_id: i64,
    #[serde(default = "default_provider_connection_id")]
    pub connection_id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context: RepositoryChatContext,
    pub client_turn_id: String,
}

fn default_provider_connection_id() -> String {
    super::provider_connections::DEFAULT_CODEX_CONNECTION_ID.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionTurnStart {
    pub turn_id: String,
    pub provider_thread_id: String,
    pub accepted_client_turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionMention {
    pub id: String,
    pub kind: ContextAttachmentKind,
    pub label: String,
    pub detail: String,
    pub available: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompanionSourceDetail {
    pub source: ContextSource,
    pub content: Option<String>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyConsentStatus {
    pub provider: String,
    pub disclosure_version: u32,
    pub accepted_at: Option<String>,
    pub revoked_at: Option<String>,
    pub valid: bool,
}

pub(crate) fn generated_event_item_id(event: &CompanionEventEnvelope, kind: &str) -> String {
    if kind == "error" {
        if let Some(turn) = &event.turn_id {
            return format!("error:{turn}");
        }
    }
    if let Some(epoch) = &event.cache_epoch {
        format!("{kind}:{epoch}:{}", event.sequence)
    } else {
        format!("{kind}:{}", event.sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_contract_serializes_with_camel_case_and_version() {
        let context = RepositoryChatContext {
            schema_version: REPOSITORY_CHAT_CONTEXT_VERSION,
            repository: ContextRepository {
                id: 1,
                name: "demo".into(),
                source_type: "local".into(),
                source_mode: "local_source".into(),
                approved_root: Some("/tmp/demo".into()),
                branch: Some("main".into()),
                revision: Some("abc".into()),
            },
            scan: ContextScan {
                id: Some(4),
                status: Some("completed".into()),
                scanned_at: Some("2026-08-19T00:00:00Z".into()),
                revision: Some("abc".into()),
                evidence_complete: true,
            },
            freshness: ContextFreshness {
                state: ContextFreshnessState::Matched,
                source_revision: Some("abc".into()),
                scan_revision: Some("abc".into()),
                source_dirty: Some(false),
                message: "Scan and source are aligned.".into(),
            },
            attachments: vec![],
            sources: vec![],
            omissions: vec![],
            untrusted_evidence: vec![],
            instructions: vec!["Cite source ids.".into()],
            estimated_bytes: 128,
            bounded: true,
        };
        let value = serde_json::to_value(context).expect("serialize");
        assert_eq!(value["schemaVersion"], REPOSITORY_CHAT_CONTEXT_VERSION);
        assert_eq!(value["freshness"]["sourceRevision"], "abc");
        assert!(value.get("untrustedEvidence").is_some());
        assert!(value.to_string().len() < MAX_CONTEXT_BYTES);
    }

    #[test]
    fn runtime_status_never_contains_a_secret_field() {
        let value = serde_json::to_value(CompanionRuntimeStatus::unavailable("missing"))
            .expect("serialize");
        let serialized = value.to_string().to_lowercase();
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("credential"));
    }
}
