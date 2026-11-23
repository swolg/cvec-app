use crate::core::timestamps;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

use super::companion::{CompanionAuthKind, CompanionModel};
use super::grok_host::{discover_grok_models, GrokDiscoveredModel};
use super::opencode_runtime::OpenCodeRuntime;

pub const PROVIDER_CONNECTION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_CODEX_CONNECTION_ID: &str = "codex_default";
pub const DEFAULT_GROK_CONNECTION_ID: &str = "grok_default";
pub const DEFAULT_OPENCODE_CONNECTION_ID: &str = "opencode_default";
pub const MINIMUM_OPENCODE_VERSION: &str = "1.0.0";

pub fn is_builtin_connection_id(id: &str) -> bool {
    matches!(
        id,
        DEFAULT_CODEX_CONNECTION_ID | DEFAULT_GROK_CONNECTION_ID | DEFAULT_OPENCODE_CONNECTION_ID
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ProviderConnectionId(pub String);

impl ProviderConnectionId {
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.chars().enumerate().all(|(index, ch)| {
                ch.is_ascii_alphanumeric() || (index > 0 && matches!(ch, '_' | '-'))
            })
            || !value
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
        {
            return Err(Error::App(
                "connection id must start with a letter and contain only letters, digits, '-' or '_'"
                    .into(),
            ));
        }
        Ok(Self(value))
    }
}

impl AsRef<str> for ProviderConnectionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDriverKind {
    Codex,
    Grok,
    OpenCode,
}

impl ProviderDriverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "codex" => Ok(Self::Codex),
            "grok" => Ok(Self::Grok),
            "opencode" => Ok(Self::OpenCode),
            _ => Err(Error::App("unsupported AI connection driver".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthState {
    Unchecked,
    Checking,
    Ready,
    SetupRequired,
    Unavailable,
    Incompatible,
    Disabled,
    Stale,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceBoundary {
    VerifiedLocal,
    External,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderUsageScope {
    Turn,
    Thread,
    Account,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorCode {
    NotInstalled,
    Incompatible,
    AuthenticationRequired,
    ConnectionRefused,
    ModelUnavailable,
    HistoryUnavailable,
    PolicyDenied,
    ProviderError,
    ProcessExited,
    ProtocolError,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderErrorInfo {
    pub code: ProviderErrorCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodexConnectionConfig {
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default)]
    pub home_path: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub launch_args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrokConnectionConfig {
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default)]
    pub custom_models: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeConnectionMode {
    #[default]
    ManagedLocal,
    ExistingLoopback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenCodeConnectionConfig {
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default)]
    pub mode: OpenCodeConnectionMode,
    #[serde(default)]
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderConnectionConfig {
    Codex(CodexConnectionConfig),
    Grok(GrokConnectionConfig),
    OpenCode(OpenCodeConnectionConfig),
}

impl ProviderConnectionConfig {
    pub fn driver(&self) -> ProviderDriverKind {
        match self {
            Self::Codex(_) => ProviderDriverKind::Codex,
            Self::Grok(_) => ProviderDriverKind::Grok,
            Self::OpenCode(_) => ProviderDriverKind::OpenCode,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Codex(config) => {
                validate_optional_path(config.binary_path.as_deref(), "Codex binary path")?;
                validate_optional_path(config.home_path.as_deref(), "Codex home path")?;
                validate_optional_text(config.profile.as_deref(), "Codex profile", 128)?;
                validate_launch_args(config.launch_args.as_deref())
            }
            Self::Grok(config) => {
                validate_optional_path(config.binary_path.as_deref(), "Grok binary path")?;
                if config.custom_models.len() > 64 {
                    return Err(Error::App("Grok custom model list is too large".into()));
                }
                for model in &config.custom_models {
                    let model = model.trim();
                    if model.is_empty() || model.len() > 256 || model.contains(['\n', '\r', '\0']) {
                        return Err(Error::App("Grok custom model id is invalid".into()));
                    }
                }
                Ok(())
            }
            Self::OpenCode(config) => {
                validate_optional_path(config.binary_path.as_deref(), "OpenCode binary path")?;
                match config.mode {
                    OpenCodeConnectionMode::ManagedLocal => {
                        if config
                            .server_url
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                        {
                            return Err(Error::App(
                                "managed OpenCode connections cannot set a server URL".into(),
                            ));
                        }
                        Ok(())
                    }
                    OpenCodeConnectionMode::ExistingLoopback => validate_loopback_url(
                        config
                            .server_url
                            .as_deref()
                            .ok_or_else(|| Error::App("OpenCode server URL is required".into()))?,
                    ),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnection {
    pub schema_version: u32,
    pub id: ProviderConnectionId,
    pub driver: ProviderDriverKind,
    pub display_name: String,
    pub accent_color: Option<String>,
    pub enabled: bool,
    pub built_in: bool,
    pub config: ProviderConnectionConfig,
    pub has_server_password: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderConnection {
    pub fn default_codex(timestamp: String) -> Self {
        Self {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: ProviderConnectionId(DEFAULT_CODEX_CONNECTION_ID.into()),
            driver: ProviderDriverKind::Codex,
            display_name: "Codex".into(),
            accent_color: None,
            enabled: true,
            built_in: true,
            config: ProviderConnectionConfig::Codex(CodexConnectionConfig::default()),
            has_server_password: false,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    pub fn default_grok(timestamp: String) -> Self {
        Self {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: ProviderConnectionId(DEFAULT_GROK_CONNECTION_ID.into()),
            driver: ProviderDriverKind::Grok,
            display_name: "Grok".into(),
            accent_color: None,
            enabled: true,
            built_in: true,
            config: ProviderConnectionConfig::Grok(GrokConnectionConfig::default()),
            has_server_password: false,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    pub fn default_opencode(timestamp: String) -> Self {
        Self {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: ProviderConnectionId(DEFAULT_OPENCODE_CONNECTION_ID.into()),
            driver: ProviderDriverKind::OpenCode,
            display_name: "OpenCode".into(),
            accent_color: None,
            enabled: true,
            built_in: true,
            config: ProviderConnectionConfig::OpenCode(OpenCodeConnectionConfig::default()),
            has_server_password: false,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    pub fn builtin_defaults(timestamp: String) -> [Self; 3] {
        [
            Self::default_codex(timestamp.clone()),
            Self::default_grok(timestamp.clone()),
            Self::default_opencode(timestamp),
        ]
    }

    pub fn validate(&self) -> Result<(), Error> {
        ProviderConnectionId::parse(self.id.0.clone())?;
        validate_display_name(&self.display_name)?;
        if self.driver != self.config.driver() {
            return Err(Error::App(
                "connection driver does not match its configuration".into(),
            ));
        }
        if let Some(color) = self.accent_color.as_deref() {
            if color.len() != 7
                || !color.starts_with('#')
                || !color[1..].chars().all(|value| value.is_ascii_hexdigit())
            {
                return Err(Error::App("connection accent must be a hex color".into()));
            }
        }
        self.config.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionDraft {
    pub id: String,
    pub driver: ProviderDriverKind,
    pub display_name: String,
    pub accent_color: Option<String>,
    pub enabled: bool,
    pub config: ProviderConnectionConfig,
    #[serde(default)]
    pub server_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub create_thread: bool,
    pub resume_thread: bool,
    pub interrupt: bool,
    pub ordered_streaming: bool,
    pub read_only_policy: bool,
    pub account_status: bool,
    pub rate_limits: bool,
    pub token_usage: bool,
    pub reasoning_effort: bool,
    pub attachments: bool,
}

impl ProviderCapabilities {
    pub fn codex() -> Self {
        Self {
            create_thread: true,
            resume_thread: true,
            interrupt: true,
            ordered_streaming: true,
            read_only_policy: true,
            account_status: true,
            rate_limits: true,
            token_usage: true,
            reasoning_effort: true,
            attachments: true,
        }
    }

    pub fn opencode() -> Self {
        Self {
            create_thread: true,
            resume_thread: true,
            interrupt: true,
            ordered_streaming: true,
            read_only_policy: true,
            account_status: false,
            rate_limits: false,
            token_usage: true,
            reasoning_effort: true,
            attachments: false,
        }
    }

    pub fn grok() -> Self {
        Self {
            create_thread: true,
            resume_thread: true,
            interrupt: true,
            ordered_streaming: true,
            read_only_policy: true,
            account_status: false,
            rate_limits: false,
            token_usage: true,
            reasoning_effort: false,
            attachments: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUpstream {
    pub id: String,
    pub display_name: String,
    pub connected: bool,
    pub inference_boundary: InferenceBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionModel {
    pub connection_id: ProviderConnectionId,
    pub driver: ProviderDriverKind,
    pub upstream_provider_id: Option<String>,
    pub inference_boundary: InferenceBoundary,
    #[serde(flatten)]
    pub model: CompanionModel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionSnapshot {
    pub schema_version: u32,
    pub connection: ProviderConnection,
    pub health: ProviderHealthState,
    pub installed_version: Option<String>,
    pub auth_kind: CompanionAuthKind,
    pub account_label: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub upstream_providers: Vec<ProviderUpstream>,
    pub models: Vec<ProviderConnectionModel>,
    pub checked_at: Option<String>,
    pub message: Option<String>,
    pub error: Option<ProviderErrorInfo>,
}

impl ProviderConnectionSnapshot {
    pub fn unchecked(connection: ProviderConnection) -> Self {
        let capabilities = match connection.driver {
            ProviderDriverKind::Codex => ProviderCapabilities::codex(),
            ProviderDriverKind::Grok => ProviderCapabilities::grok(),
            ProviderDriverKind::OpenCode => ProviderCapabilities::opencode(),
        };
        let health = if connection.enabled {
            ProviderHealthState::Unchecked
        } else {
            ProviderHealthState::Disabled
        };
        Self {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            connection,
            health,
            installed_version: None,
            auth_kind: CompanionAuthKind::None,
            account_label: None,
            capabilities,
            upstream_providers: Vec::new(),
            models: Vec::new(),
            checked_at: None,
            message: None,
            error: None,
        }
    }
}

pub fn probe_grok(connection: &ProviderConnection) -> ProviderConnectionSnapshot {
    let mut snapshot = ProviderConnectionSnapshot::unchecked(connection.clone());
    snapshot.checked_at = Some(timestamps::now());
    if !connection.enabled {
        return snapshot;
    }
    let ProviderConnectionConfig::Grok(config) = &connection.config else {
        snapshot.health = ProviderHealthState::Unavailable;
        snapshot.message = Some("Grok configuration is invalid.".into());
        return snapshot;
    };
    let binary = config.binary_path.as_deref().unwrap_or("grok");
    let output = match bounded_command_output(binary, &["--version"], Duration::from_secs(4)) {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::NotInstalled,
                message: safe_process_failure("Grok CLI could not be started", &output.stderr),
                retryable: true,
            });
            return snapshot;
        }
        Err(ProcessProbeError::Spawn) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::NotInstalled,
                message: "Grok CLI is not installed or is not on PATH.".into(),
                retryable: true,
            });
            return snapshot;
        }
        Err(cause @ (ProcessProbeError::Timeout | ProcessProbeError::Cancelled)) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::ProcessExited,
                message: if cause == ProcessProbeError::Cancelled {
                    "Grok CLI version check was cancelled."
                } else {
                    "Grok CLI timed out while checking its version."
                }
                .into(),
                retryable: true,
            });
            return snapshot;
        }
    };
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    snapshot.installed_version = (!version.is_empty()).then_some(version);
    let fallback = vec![GrokDiscoveredModel {
        id: "grok-build".into(),
        display_name: "Grok Build".into(),
        is_default: true,
    }];
    let mut discovered = match discover_grok_models(binary) {
        Ok(models) => models,
        Err(cause) => {
            let message = cause.to_string();
            let normalized = message.to_ascii_lowercase();
            let authentication = normalized.contains("auth")
                || normalized.contains("cached_token")
                || normalized.contains("xai_api_key");
            snapshot.health = if authentication {
                ProviderHealthState::SetupRequired
            } else {
                ProviderHealthState::Unavailable
            };
            snapshot.models = grok_connection_models(connection, fallback, &config.custom_models);
            snapshot.upstream_providers = vec![ProviderUpstream {
                id: "xai".into(),
                display_name: "xAI".into(),
                connected: false,
                inference_boundary: InferenceBoundary::External,
            }];
            snapshot.error = Some(ProviderErrorInfo {
                code: if authentication {
                    ProviderErrorCode::AuthenticationRequired
                } else {
                    ProviderErrorCode::ProtocolError
                },
                message,
                retryable: authentication,
            });
            return snapshot;
        }
    };
    for model in &config.custom_models {
        let id = model.trim();
        if !discovered.iter().any(|known| known.id == id) {
            discovered.push(GrokDiscoveredModel {
                id: id.into(),
                display_name: id.into(),
                is_default: false,
            });
        }
    }
    snapshot.models = grok_connection_models(connection, discovered, &[]);
    snapshot.auth_kind = if std::env::var("XAI_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        CompanionAuthKind::ApiKey
    } else {
        CompanionAuthKind::None
    };
    snapshot.upstream_providers = vec![ProviderUpstream {
        id: "xai".into(),
        display_name: "xAI".into(),
        connected: true,
        inference_boundary: InferenceBoundary::External,
    }];
    snapshot.health = ProviderHealthState::Ready;
    snapshot.message = Some("Grok ACP is ready and its model inventory was refreshed.".into());
    snapshot
}

fn grok_connection_models(
    connection: &ProviderConnection,
    mut discovered: Vec<GrokDiscoveredModel>,
    custom_models: &[String],
) -> Vec<ProviderConnectionModel> {
    for model in custom_models {
        let id = model.trim();
        if !discovered.iter().any(|known| known.id == id) {
            discovered.push(GrokDiscoveredModel {
                id: id.into(),
                display_name: id.into(),
                is_default: false,
            });
        }
    }
    discovered
        .into_iter()
        .map(|model| ProviderConnectionModel {
            connection_id: connection.id.clone(),
            driver: ProviderDriverKind::Grok,
            upstream_provider_id: Some("xai".into()),
            inference_boundary: InferenceBoundary::External,
            model: CompanionModel {
                display_name: model.display_name,
                id: model.id,
                description: None,
                is_default: model.is_default,
                hidden: false,
                reasoning_efforts: Vec::new(),
                input_modalities: vec!["text".into()],
                context_window: None,
            },
        })
        .collect()
}

pub fn probe_opencode(connection: &ProviderConnection) -> ProviderConnectionSnapshot {
    let mut snapshot = ProviderConnectionSnapshot::unchecked(connection.clone());
    snapshot.checked_at = Some(timestamps::now());
    if !connection.enabled {
        return snapshot;
    }
    let ProviderConnectionConfig::OpenCode(config) = &connection.config else {
        snapshot.health = ProviderHealthState::Unavailable;
        snapshot.message = Some("OpenCode configuration is invalid.".into());
        return snapshot;
    };
    let binary = config.binary_path.as_deref().unwrap_or("opencode");
    let version = match bounded_command_output(binary, &["--version"], Duration::from_secs(4)) {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(output) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::NotInstalled,
                message: safe_process_failure("OpenCode could not be started", &output.stderr),
                retryable: true,
            });
            return snapshot;
        }
        Err(cause) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: if cause == ProcessProbeError::Timeout {
                    ProviderErrorCode::ProcessExited
                } else {
                    ProviderErrorCode::NotInstalled
                },
                message: if cause == ProcessProbeError::Timeout {
                    "OpenCode version check timed out. Try again."
                } else {
                    "OpenCode is not installed or is not on PATH."
                }
                .into(),
                retryable: true,
            });
            return snapshot;
        }
    };
    snapshot.installed_version = Some(version.clone());
    if !version_at_least(&version, MINIMUM_OPENCODE_VERSION) {
        snapshot.health = ProviderHealthState::Incompatible;
        snapshot.error = Some(ProviderErrorInfo {
            code: ProviderErrorCode::Incompatible,
            message: format!(
                "OpenCode {MINIMUM_OPENCODE_VERSION} or newer is required. Update OpenCode, then refresh this connection."
            ),
            retryable: true,
        });
        return snapshot;
    }

    let v2 = version_at_least(&version, "2.0.0");
    if v2 && config.mode == OpenCodeConnectionMode::ExistingLoopback {
        snapshot.health = ProviderHealthState::Incompatible;
        snapshot.error = Some(ProviderErrorInfo {
            code: ProviderErrorCode::Incompatible,
            message: "OpenCode V2 requires Managed local mode in CVEC to enforce its read-only policy. Change the connection mode, then refresh.".into(),
            retryable: true,
        });
        return snapshot;
    }
    let inventory = if v2 {
        (|| {
            let directory = tempfile::tempdir()?;
            let runtime = OpenCodeRuntime::start(binary, directory.path())?;
            runtime.models(binary, directory.path(), None)
        })()
    } else {
        bounded_command_output(binary, &["models", "--verbose"], Duration::from_secs(20))
            .map_err(|_| Error::App("OpenCode model discovery failed or timed out.".into()))
    };
    let output = match inventory {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            snapshot.health = ProviderHealthState::SetupRequired;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::AuthenticationRequired,
                message: safe_process_failure(
                    "OpenCode is available but no model inventory could be loaded",
                    &output.stderr,
                ),
                retryable: true,
            });
            return snapshot;
        }
        Err(cause) => {
            snapshot.health = ProviderHealthState::Unavailable;
            snapshot.error = Some(ProviderErrorInfo {
                code: ProviderErrorCode::ProviderError,
                message: cause.to_string(),
                retryable: true,
            });
            return snapshot;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let models = parse_opencode_models(&stdout, &connection.id);
    if models.is_empty() {
        snapshot.health = ProviderHealthState::SetupRequired;
        snapshot.message = Some(
            "OpenCode is installed, but it did not report any connected models. Configure a provider in OpenCode, then refresh."
                .into(),
        );
        return snapshot;
    }
    let mut providers = BTreeMap::new();
    for model in &models {
        if let Some(provider) = model.upstream_provider_id.as_deref() {
            providers
                .entry(provider.to_string())
                .or_insert_with(|| ProviderUpstream {
                    id: provider.to_string(),
                    display_name: upstream_display_name(provider),
                    connected: true,
                    inference_boundary: model.inference_boundary,
                });
        }
    }
    snapshot.health = ProviderHealthState::Ready;
    snapshot.auth_kind = CompanionAuthKind::None;
    snapshot.upstream_providers = providers.into_values().collect();
    snapshot.models = models;
    snapshot.message = Some(format!(
        "{} upstream provider{} connected through OpenCode.",
        snapshot.upstream_providers.len(),
        if snapshot.upstream_providers.len() == 1 {
            ""
        } else {
            "s"
        }
    ));
    snapshot
}

pub fn parse_opencode_models(
    output: &str,
    connection_id: &ProviderConnectionId,
) -> Vec<ProviderConnectionModel> {
    let mut models = Vec::new();
    let lines = output.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let slug = lines[index].split_whitespace().next().unwrap_or_default();
        let Some((provider, model_id)) = slug.split_once('/') else {
            index += 1;
            continue;
        };
        if provider.is_empty() || model_id.is_empty() || slug.len() > 256 {
            index += 1;
            continue;
        }
        let mut metadata = None;
        if lines
            .get(index + 1)
            .is_some_and(|line| line.trim_start().starts_with('{'))
        {
            let mut json = String::new();
            let mut depth = 0_i32;
            let mut cursor = index + 1;
            while cursor < lines.len() {
                let line = lines[cursor];
                depth += line.chars().filter(|ch| *ch == '{').count() as i32;
                depth -= line.chars().filter(|ch| *ch == '}').count() as i32;
                json.push_str(line);
                json.push('\n');
                cursor += 1;
                if depth == 0 {
                    break;
                }
            }
            metadata = serde_json::from_str::<Value>(&json).ok();
            index = cursor;
        } else {
            index += 1;
        }
        let endpoint = metadata
            .as_ref()
            .and_then(|value| value.pointer("/api/url"))
            .and_then(Value::as_str);
        let boundary = classify_upstream(provider, endpoint);
        let display_name = metadata
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(model_id)
            .to_string();
        let context_window = metadata
            .as_ref()
            .and_then(|value| value.pointer("/limit/context"))
            .and_then(Value::as_u64);
        let supports_reasoning = metadata
            .as_ref()
            .and_then(|value| value.pointer("/capabilities/reasoning"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        models.push(ProviderConnectionModel {
            connection_id: connection_id.clone(),
            driver: ProviderDriverKind::OpenCode,
            upstream_provider_id: Some(provider.into()),
            inference_boundary: boundary,
            model: CompanionModel {
                id: slug.into(),
                display_name,
                description: None,
                is_default: models.is_empty(),
                hidden: false,
                reasoning_efforts: if supports_reasoning {
                    vec!["provider_default".into()]
                } else {
                    Vec::new()
                },
                input_modalities: vec!["text".into()],
                context_window,
            },
        });
    }
    models.sort_by(|left, right| left.model.id.cmp(&right.model.id));
    models.dedup_by(|left, right| left.model.id == right.model.id);
    models
}

pub fn classify_upstream(provider: &str, endpoint: Option<&str>) -> InferenceBoundary {
    if endpoint.is_some_and(|value| validate_loopback_url(value).is_ok()) {
        return InferenceBoundary::VerifiedLocal;
    }
    let provider = provider.to_ascii_lowercase();
    if matches!(
        provider.as_str(),
        "openai"
            | "anthropic"
            | "openrouter"
            | "google"
            | "groq"
            | "xai"
            | "github-copilot"
            | "ollama-cloud"
    ) {
        InferenceBoundary::External
    } else {
        InferenceBoundary::Unknown
    }
}

pub(crate) fn version_at_least(actual: &str, minimum: &str) -> bool {
    fn parts(value: &str) -> Option<[u64; 3]> {
        let candidate = value.split_whitespace().find(|part| {
            part.trim_start_matches('v')
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        })?;
        let values = candidate
            .trim_start_matches('v')
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .take(3)
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        Some([
            *values.first().unwrap_or(&0),
            *values.get(1).unwrap_or(&0),
            *values.get(2).unwrap_or(&0),
        ])
    }
    parts(actual)
        .zip(parts(minimum))
        .is_some_and(|(actual, minimum)| actual >= minimum)
}

pub fn validate_loopback_url(value: &str) -> Result<(), Error> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| Error::App("OpenCode server URL is malformed".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::App(
            "OpenCode server URL must be a credential-free loopback HTTP(S) URL".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::App("OpenCode server URL must include a host".into()))?;
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    if host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(80);
    if (host, port)
        .to_socket_addrs()
        .map(|addresses| addresses.into_iter().all(|value| value.ip().is_loopback()))
        .unwrap_or(false)
    {
        return Ok(());
    }
    Err(Error::App(
        "OpenCode existing-server mode accepts loopback addresses only".into(),
    ))
}

pub fn config_from_json(
    driver: ProviderDriverKind,
    value: &str,
) -> Result<ProviderConnectionConfig, Error> {
    let config: ProviderConnectionConfig = serde_json::from_str(value)?;
    if config.driver() != driver {
        return Err(Error::App(
            "stored connection driver/config mismatch".into(),
        ));
    }
    config.validate()?;
    Ok(config)
}

pub fn config_to_json(config: &ProviderConnectionConfig) -> Result<String, Error> {
    config.validate()?;
    Ok(serde_json::to_string(config)?)
}

pub fn now_timestamp() -> String {
    timestamps::now()
}

fn validate_display_name(value: &str) -> Result<(), Error> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() || normalized.len() > 80 {
        return Err(Error::App(
            "connection display name must contain 1 to 80 characters".into(),
        ));
    }
    Ok(())
}

fn validate_optional_path(value: Option<&str>, label: &str) -> Result<(), Error> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if value.len() > 4096 || value.contains('\0') {
        return Err(Error::App(format!("{label} is invalid")));
    }
    let path = Path::new(value);
    if path.components().count() == 0 {
        return Err(Error::App(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_optional_text(value: Option<&str>, label: &str, max: usize) -> Result<(), Error> {
    if value.is_some_and(|value| value.len() > max || value.contains(['\n', '\r', '\0'])) {
        return Err(Error::App(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_launch_args(value: Option<&str>) -> Result<(), Error> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if value.len() > 2048 || value.contains(['\n', '\r', '\0']) {
        return Err(Error::App("Codex launch arguments are invalid".into()));
    }
    let lower = value.to_ascii_lowercase();
    let disallowed_substrings = [
        "dangerously-bypass-approvals-and-sandbox",
        "danger-full-access",
        "workspace-write",
        "full-auto",
        "approval_policy",
        "sandbox_mode",
        "ask-for-approval",
    ];
    let has_disallowed_token = lower.split_ascii_whitespace().any(|raw| {
        let token = raw.trim_matches(['"', '\'']);
        matches!(token, "--yolo" | "--sandbox" | "-s" | "-a")
            || token.starts_with("--sandbox=")
            || token.starts_with("-s=")
            || token.starts_with("-a=")
    });
    if disallowed_substrings
        .iter()
        .any(|needle| lower.contains(needle))
        || has_disallowed_token
    {
        return Err(Error::App(
            "Codex launch arguments cannot weaken cvec read-only policy".into(),
        ));
    }
    Ok(())
}

fn upstream_display_name(provider: &str) -> String {
    match provider {
        "ollama" => "Ollama".into(),
        "lmstudio" => "LM Studio".into(),
        "llama.cpp" => "llama.cpp".into(),
        "openai" => "OpenAI".into(),
        "anthropic" => "Anthropic".into(),
        "openrouter" => "OpenRouter".into(),
        _ => provider.replace(['-', '_'], " "),
    }
}

fn safe_process_failure(prefix: &str, stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.lines().next().unwrap_or_default().trim();
    if detail.is_empty() {
        prefix.into()
    } else {
        format!(
            "{prefix}: {}",
            redact(detail).chars().take(300).collect::<String>()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessProbeError {
    Spawn,
    Timeout,
    Cancelled,
}

pub(crate) fn bounded_command_output(
    binary: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, ProcessProbeError> {
    bounded_process_output(super::toolchain::command(binary).args(args), timeout)
}

pub(crate) fn bounded_process_output(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, ProcessProbeError> {
    bounded_process_output_cancellable(command, timeout, None)
}

pub(crate) fn bounded_process_output_cancellable(
    command: &mut Command,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
) -> Result<Output, ProcessProbeError> {
    let stopped = || cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst));
    if stopped() {
        return Err(ProcessProbeError::Cancelled);
    }
    // Files avoid pipe backpressure for large model inventories and inherited
    // pipe handles in provider descendants. Only a bounded amount is read back.
    let mut stdout = tempfile::tempfile().map_err(|_| ProcessProbeError::Spawn)?;
    let mut stderr = tempfile::tempfile().map_err(|_| ProcessProbeError::Spawn)?;
    let mut child = super::process::spawn_owned(
        command
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().map_err(|_| ProcessProbeError::Spawn)?)
            .stderr(stderr.try_clone().map_err(|_| ProcessProbeError::Spawn)?),
    )
    .map_err(|_| ProcessProbeError::Spawn)?;
    let deadline = Instant::now() + timeout;
    loop {
        if stopped() {
            return Err(ProcessProbeError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let read = |file: &mut std::fs::File| {
                    file.seek(SeekFrom::Start(0))?;
                    let mut bytes = Vec::new();
                    file.take(8 * 1024 * 1024).read_to_end(&mut bytes)?;
                    Ok::<_, std::io::Error>(bytes)
                };
                return Ok(Output {
                    status,
                    stdout: read(&mut stdout).map_err(|_| ProcessProbeError::Spawn)?,
                    stderr: read(&mut stderr).map_err(|_| ProcessProbeError::Spawn)?,
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                return Err(ProcessProbeError::Timeout);
            }
        }
    }
}

pub(crate) fn sanitize_provider_diagnostic(value: &str, runtime: &str) -> String {
    let first = value.lines().next().unwrap_or_default();
    let normalized = first.to_ascii_lowercase();
    let authentication = [
        "authorization",
        "bearer ",
        "basic ",
        "api key",
        "apikey",
        "api_key",
        "api-key",
        "password",
        "access_token",
        "refresh_token",
        "token=",
        "secret=",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
        || (first.contains("://") && first.contains('@'));
    let redacted = if authentication {
        format!("{runtime} authentication failed; provider diagnostics were redacted.")
    } else {
        redact(first)
    };
    redacted.chars().take(512).collect()
}

pub fn redact(value: &str) -> String {
    let mut output = value.to_string();
    for (marker, case_insensitive) in [
        ("bearer ", true),
        ("basic ", true),
        ("sk-or-", false),
        ("sk-", false),
        ("xai-", false),
        ("ghp_", false),
        ("gho_", false),
        ("ghu_", false),
        ("ghs_", false),
        ("ghr_", false),
        ("github_pat_", false),
        ("glpat-", false),
        ("gldt-", false),
        ("glrt-", false),
        ("AIza", false),
    ] {
        let mut cursor = 0;
        loop {
            let relative = if case_insensitive {
                output[cursor..].to_ascii_lowercase().find(marker)
            } else {
                output[cursor..].find(marker)
            };
            let Some(relative) = relative else {
                break;
            };
            let index = cursor + relative;
            let start = index + marker.len();
            let end = output[start..]
                .char_indices()
                .find(|(_, ch)| {
                    ch.is_whitespace() || matches!(ch, '\'' | '"' | ',' | '}' | ']' | ')' | ';')
                })
                .map(|(offset, _)| start + offset)
                .unwrap_or(output.len());
            if end == start {
                cursor = start;
                continue;
            }
            output.replace_range(start..end, "[REDACTED]");
            cursor = start + "[REDACTED]".len();
        }
    }
    output
}

pub fn boundary_fingerprint(
    connection: &ProviderConnectionId,
    driver: ProviderDriverKind,
    upstream: Option<&str>,
    boundary: InferenceBoundary,
    disclosure_version: u32,
) -> String {
    format!(
        "v{disclosure_version}:{}:{}:{}:{}",
        connection.as_ref(),
        driver.as_str(),
        upstream.unwrap_or("unknown"),
        match boundary {
            InferenceBoundary::VerifiedLocal => "verified_local",
            InferenceBoundary::External => "external",
            InferenceBoundary::Unknown => "unknown",
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_connection_ids_and_driver_config() {
        assert!(ProviderConnectionId::parse("opencode_local").is_ok());
        assert!(ProviderConnectionId::parse("1bad").is_err());
        assert!(
            ProviderConnectionConfig::OpenCode(OpenCodeConnectionConfig {
                mode: OpenCodeConnectionMode::ExistingLoopback,
                server_url: Some("http://127.0.0.1:4096".into()),
                ..Default::default()
            })
            .validate()
            .is_ok()
        );
        assert!(validate_loopback_url("https://example.com").is_err());
        assert!(validate_loopback_url("http://user:secret@localhost:4096").is_err());
    }

    #[test]
    fn recognizes_every_stable_builtin_connection() {
        assert!(is_builtin_connection_id(DEFAULT_CODEX_CONNECTION_ID));
        assert!(is_builtin_connection_id(DEFAULT_GROK_CONNECTION_ID));
        assert!(is_builtin_connection_id(DEFAULT_OPENCODE_CONNECTION_ID));
        assert!(!is_builtin_connection_id("grok_work"));

        let defaults = ProviderConnection::builtin_defaults("2026-08-20T00:00:00Z".into());
        assert_eq!(defaults.len(), 3);
        assert!(defaults
            .iter()
            .all(|connection| connection.built_in && connection.enabled));
    }

    #[test]
    fn strict_config_rejects_unknown_and_unsafe_values() {
        let unknown = r#"{"kind":"codex","binaryPath":null,"unknown":true}"#;
        assert!(serde_json::from_str::<ProviderConnectionConfig>(unknown).is_err());
        for unsafe_args in [
            "--dangerously-bypass-approvals-and-sandbox",
            "--DANGEROUSLY-BYPASS-APPROVALS-AND-SANDBOX",
            "--full-auto",
            "--yolo",
            "--ask-for-approval never",
            "-a never",
            "--sandbox danger-full-access",
            "-s workspace-write",
            "-c approval_policy = \"never\"",
            "-c sandbox_mode=workspace-write",
        ] {
            assert!(
                ProviderConnectionConfig::Codex(CodexConnectionConfig {
                    launch_args: Some(unsafe_args.into()),
                    ..Default::default()
                })
                .validate()
                .is_err(),
                "expected unsafe Codex launch_args to be rejected: {unsafe_args}"
            );
        }
        assert!(ProviderConnectionConfig::Codex(CodexConnectionConfig {
            launch_args: Some("--profile review -c model_reasoning_effort=medium".into()),
            ..Default::default()
        })
        .validate()
        .is_ok());
        assert!(ProviderConnectionConfig::Grok(GrokConnectionConfig {
            binary_path: None,
            custom_models: vec!["bad\nmodel".into()],
        })
        .validate()
        .is_err());
    }

    #[test]
    fn parses_provider_qualified_models_and_boundaries() {
        let id = ProviderConnectionId("opencode_local".into());
        let models = parse_opencode_models("ollama/qwen3-coder\nopenai/gpt-5\n", &id);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model.id, "ollama/qwen3-coder");
        assert_eq!(models[1].inference_boundary, InferenceBoundary::External);
        assert_eq!(
            classify_upstream("ollama", None),
            InferenceBoundary::Unknown
        );
        assert_eq!(
            classify_upstream("ollama", Some("http://127.0.0.1:11434/v1")),
            InferenceBoundary::VerifiedLocal
        );
    }

    #[test]
    fn parses_verbose_model_metadata_and_proves_loopback_boundary() {
        let id = ProviderConnectionId("local_models".into());
        let models = parse_opencode_models(
            "ollama/qwen3\n{\n  \"name\": \"Qwen 3\",\n  \"api\": { \"url\": \"http://127.0.0.1:11434/v1\" },\n  \"limit\": { \"context\": 32768 },\n  \"capabilities\": { \"reasoning\": true }\n}\n",
            &id,
        );
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].inference_boundary,
            InferenceBoundary::VerifiedLocal
        );
        assert_eq!(models[0].model.display_name, "Qwen 3");
        assert_eq!(models[0].model.context_window, Some(32768));
        assert_eq!(models[0].model.reasoning_efforts, vec!["provider_default"]);
    }

    #[test]
    fn redacts_common_secret_shapes() {
        let xai = format!("xai-{}", "a".repeat(24));
        let gho = format!("gho_{}", "b".repeat(24));
        let google = format!("AIza{}", "C".repeat(35));
        let sample = format!("Authorization: bearer abc123, sk-secret, {xai}), {gho}; {google}");
        let value = redact(&sample);
        assert!(!value.contains("abc123"));
        assert!(!value.contains("secret"));
        assert!(!value.contains(&"a".repeat(24)));
        assert!(!value.contains(&"b".repeat(24)));
        assert!(!value.contains(&"C".repeat(35)));
        let grok_diag =
            sanitize_provider_diagnostic(&format!("Request rejected for {xai}"), "Grok");
        assert!(!grok_diag.contains(&"a".repeat(24)));
        assert!(grok_diag.contains("[REDACTED]"));
    }

    #[test]
    #[ignore = "opt-in live OpenCode inventory; no inference request"]
    fn live_opencode_inventory_after_initialization() {
        let mut connection = ProviderConnection::builtin_defaults("now".into())
            .into_iter()
            .find(|connection| connection.driver == ProviderDriverKind::OpenCode)
            .unwrap();
        if let ProviderConnectionConfig::OpenCode(config) = &mut connection.config {
            config.binary_path = std::env::var("CVEC_LIVE_OPENCODE_BINARY").ok();
        }
        let snapshot = probe_opencode(&connection);
        println!(
            "health={:?}, models={}, error={:?}",
            snapshot.health,
            snapshot.models.len(),
            snapshot.error
        );
        assert_eq!(snapshot.health, ProviderHealthState::Ready);
        assert!(!snapshot.models.is_empty());
    }

    #[test]
    fn opencode_v2_probe_uses_supported_flags_and_distinguishes_missing_models() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("opencode");
        let mut connection = ProviderConnection::builtin_defaults("now".into())
            .into_iter()
            .find(|connection| connection.driver == ProviderDriverKind::OpenCode)
            .unwrap();
        if let ProviderConnectionConfig::OpenCode(config) = &mut connection.config {
            config.binary_path = Some(binary.display().to_string());
        }
        for inventory in ["openai/test", ""] {
            std::fs::write(&binary, format!(r#"#!/usr/bin/env python3
import sys, pathlib
if '--version' in sys.argv: print('opencode v2.0.25')
elif 'serve' in sys.argv:
 print('{{"url":"http://127.0.0.1:4096"}}',flush=True)
 sys.stdin.read()
elif 'api' in sys.argv:
 marker=pathlib.Path(__file__).with_suffix('.ready')
 if marker.exists(): print('{{"data":[{{"id":"cvec","permissions":[{{"action":"*","resource":"*","effect":"deny"}}]}}]}}')
 else:
  marker.touch(); print('{{"data":[]}}')
elif 'models' in sys.argv:
 assert '--server' in sys.argv and '--verbose' not in sys.argv
 print('{inventory}')
else: sys.exit(2)
"#)).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            let snapshot = probe_opencode(&connection);
            assert_eq!(
                snapshot.health,
                if inventory.is_empty() {
                    ProviderHealthState::SetupRequired
                } else {
                    ProviderHealthState::Ready
                }
            );
            assert_eq!(snapshot.models.len(), usize::from(!inventory.is_empty()));
        }
        if let ProviderConnectionConfig::OpenCode(config) = &mut connection.config {
            config.mode = OpenCodeConnectionMode::ExistingLoopback;
        }
        let snapshot = probe_opencode(&connection);
        assert_eq!(snapshot.health, ProviderHealthState::Incompatible);
        assert!(snapshot.error.unwrap().message.contains("Managed local"));
    }

    #[test]
    fn enforces_minimum_opencode_version() {
        assert!(version_at_least("opencode 1.18.18", "1.0.0"));
        assert!(version_at_least("opencode v2.0.25", "2.0.0"));
        assert!(version_at_least("v1.18.18", "1.0.0"));
        assert!(version_at_least("1.0.0", "1.0.0"));
        assert!(!version_at_least("0.9.9", "1.0.0"));
        assert!(!version_at_least("unknown", "1.0.0"));
    }

    #[test]
    fn grok_probe_fails_cleanly_for_a_missing_binary() {
        let timestamp = timestamps::now();
        let connection = ProviderConnection {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: ProviderConnectionId("grok_missing".into()),
            driver: ProviderDriverKind::Grok,
            display_name: "Missing Grok".into(),
            accent_color: None,
            enabled: true,
            built_in: false,
            config: ProviderConnectionConfig::Grok(GrokConnectionConfig {
                binary_path: Some("/definitely/not/a/cvec-grok-binary".into()),
                custom_models: Vec::new(),
            }),
            has_server_password: false,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        let snapshot = probe_grok(&connection);
        assert_eq!(snapshot.health, ProviderHealthState::Unavailable);
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code),
            Some(ProviderErrorCode::NotInstalled)
        );
    }

    #[test]
    fn provider_process_probe_has_a_hard_timeout() {
        let result = bounded_command_output(
            "python3",
            &["-c", "import time; time.sleep(0.2)"],
            Duration::from_millis(25),
        );
        assert!(matches!(result, Err(ProcessProbeError::Timeout)));
    }
}
