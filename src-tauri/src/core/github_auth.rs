use crate::core::timestamps;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use keyring_core::{Entry, Error as KeyringError};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::Error;

use super::{
    db::Database,
    github_api::{GithubApiClient, GithubRateLimit},
};

const KEYCHAIN_SERVICE: &str = "dev.cvec.app.github";
const KEYCHAIN_USER: &str = "github-app-user-token";
const APP_SETTING: &str = "github_app_configuration";
#[cfg(debug_assertions)]
const QA_KEYCHAIN_SERVICE_ENV: &str = "CVEC_GITHUB_KEYCHAIN_SERVICE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GithubAuthSnapshot {
    #[serde(default)]
    pub checking: bool,
    #[serde(default)]
    pub deferred: bool,
    pub configured: bool,
    pub client_id: Option<String>,
    #[serde(default)]
    pub config_revision: u64,
    #[serde(default)]
    pub session_revision: u64,
    #[serde(default)]
    pub can_configure: bool,
    pub signed_in: bool,
    pub login: Option<String>,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    pub expires_at: Option<String>,
    pub rate_limit_remaining: Option<u32>,
    pub rate_limit_limit: Option<u32>,
    pub rate_limit_reset_at: Option<String>,
    pub install_url: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GithubLoginStart {
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubLoginPollStatus {
    Pending,
    SlowDown,
    Success,
    Expired,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GithubLoginPollResult {
    pub status: GithubLoginPollStatus,
    pub message: Option<String>,
    pub interval_seconds: u64,
    pub auth: GithubAuthSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepositoryOption {
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
    pub default_branch: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubAuthConfig {
    client_id: Option<String>,
    install_url: Option<String>,
    #[serde(skip)]
    revision: u64,
}

impl GithubAuthConfig {
    fn from_input(client_id: Option<String>, install_url: Option<String>) -> Result<Self, Error> {
        let Some(client_id) = client_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return Ok(Self::default());
        };
        if client_id.len() > 128
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(Error::App(
                "Enter the Client ID from your GitHub App settings.".into(),
            ));
        }
        let install_url = install_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                Url::parse(value)
                    .ok()
                    .filter(|url| {
                        url.scheme() == "https"
                            && url.host_str() == Some("github.com")
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.port().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                    })
                    .map(|url| url.to_string())
                    .ok_or_else(|| {
                        Error::App(
                            "Enter a GitHub installation link starting with https://github.com/."
                                .into(),
                        )
                    })
            })
            .transpose()?;
        Ok(Self {
            client_id: Some(client_id),
            install_url,
            revision: 0,
        })
    }
}

#[derive(Debug, Clone)]
struct PendingGithubLogin {
    login_id: String,
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_at: OffsetDateTime,
    interval_seconds: u64,
}

#[derive(Debug, Default)]
struct GithubLoginFlow {
    sequence: u64,
    login: Option<PendingGithubLogin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredGithubAuth {
    #[serde(default)]
    client_id: Option<String>,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<String>,
    refresh_expires_at: Option<String>,
    login: String,
    avatar_url: Option<String>,
    profile_url: Option<String>,
}

#[derive(Debug)]
pub struct GithubAuthState {
    isolated: bool,
    keychain_service: String,
    config: RwLock<GithubAuthConfig>,
    pending: Mutex<GithubLoginFlow>,
    cached_auth: Mutex<CachedGithubAuth>,
    session_revision: AtomicU64,
    refresh: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Default)]
enum CachedGithubAuth {
    #[default]
    Uninitialized,
    Loaded(Option<StoredGithubAuth>),
    Unavailable(String),
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token_expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
    avatar_url: Option<String>,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstallationsResponse {
    installations: Vec<GithubInstallation>,
}

#[derive(Debug, Deserialize)]
struct GithubInstallation {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct InstallationRepositoriesResponse {
    repositories: Vec<InstallationRepository>,
}

#[derive(Debug, Deserialize)]
struct InstallationRepository {
    name: String,
    full_name: String,
    private: bool,
    html_url: String,
    default_branch: String,
    owner: InstallationRepositoryOwner,
}

#[derive(Debug, Deserialize)]
struct InstallationRepositoryOwner {
    login: String,
}

impl GithubAuthState {
    pub fn new() -> Self {
        let client_id = optional_environment("CVEC_GITHUB_APP_CLIENT_ID");
        let install_url = client_id
            .as_ref()
            .and_then(|_| optional_environment("CVEC_GITHUB_APP_INSTALL_URL"));
        Self {
            isolated: false,
            keychain_service: keychain_service(),
            config: RwLock::new(GithubAuthConfig {
                client_id,
                install_url,
                revision: 0,
            }),
            pending: Mutex::new(GithubLoginFlow::default()),
            cached_auth: Mutex::new(CachedGithubAuth::Uninitialized),
            session_revision: AtomicU64::new(0),
            refresh: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn from_settings(db: &Database) -> Result<Self, Error> {
        let mut state = Self::new();
        if let Some(value) = db.get_setting(APP_SETTING)? {
            let persisted: GithubAuthConfig = serde_json::from_str(&value).unwrap_or_default();
            state.config = RwLock::new(
                GithubAuthConfig::from_input(persisted.client_id, persisted.install_url)
                    .unwrap_or_default(),
            );
        }
        Ok(state)
    }

    pub fn isolated() -> Self {
        let mut state = Self::new();
        state.isolated = true;
        state.keychain_service = "dev.cvec.performance-qa.github".into();
        state.config = RwLock::new(GithubAuthConfig::default());
        state.cached_auth = Mutex::new(CachedGithubAuth::Loaded(None));
        state
    }

    pub(crate) fn configure_app(
        &self,
        db: &Database,
        client_id: Option<String>,
        install_url: Option<String>,
    ) -> Result<GithubAuthSnapshot, Error> {
        if self.isolated {
            return Err(Error::App(
                "GitHub configuration is unavailable in this isolated test profile.".into(),
            ));
        }
        let mut next = GithubAuthConfig::from_input(client_id, install_url)?;
        {
            // Device polls, credential writes, and configuration changes use this lock order.
            let mut flow = self.pending.lock().expect("github auth lock poisoned");
            let mut cached = self.cached_auth.lock().expect("github auth lock poisoned");
            db.set_setting(APP_SETTING, serde_json::to_string(&next)?)?;
            let mut config = self
                .config
                .write()
                .expect("github configuration lock poisoned");
            flow.sequence += 1;
            flow.login = None;
            if config.client_id != next.client_id {
                *cached = CachedGithubAuth::Uninitialized;
                self.session_revision.fetch_add(1, Ordering::Relaxed);
            }
            next.revision = config.revision + 1;
            *config = next;
        }
        Ok(self.cached_snapshot())
    }

    /// Persisted security views do not wait for an OS credential prompt.
    pub fn cached_snapshot(&self) -> GithubAuthSnapshot {
        let state = self.cached_auth.try_lock().ok();
        let config = self
            .config
            .read()
            .expect("github configuration lock poisoned");
        let auth = state.as_ref().and_then(|state| {
            if let CachedGithubAuth::Loaded(auth) = &**state {
                auth.clone()
            } else {
                None
            }
        });
        let configured = config.client_id.is_some();
        let checking = state.is_none();
        let deferred = configured
            && state
                .as_ref()
                .is_some_and(|state| matches!(&**state, CachedGithubAuth::Uninitialized));
        let message = if checking {
            Some("Checking GitHub sign-in status…".into())
        } else if self.isolated {
            Some("GitHub sign-in is unavailable in this isolated test profile.".into())
        } else {
            state.as_ref().and_then(|state| {
                if let CachedGithubAuth::Unavailable(message) = &**state {
                    Some(message.clone())
                } else {
                    None
                }
            })
        };
        GithubAuthSnapshot {
            checking,
            deferred,
            configured,
            client_id: config.client_id.clone(),
            config_revision: config.revision,
            session_revision: self.session_revision.load(Ordering::Relaxed),
            can_configure: !self.isolated,
            signed_in: auth.is_some(),
            login: auth.as_ref().map(|auth| auth.login.clone()),
            avatar_url: auth.as_ref().and_then(|auth| auth.avatar_url.clone()),
            profile_url: auth.as_ref().and_then(|auth| auth.profile_url.clone()),
            expires_at: auth.as_ref().and_then(|auth| auth.expires_at.clone()),
            rate_limit_remaining: None,
            rate_limit_limit: None,
            rate_limit_reset_at: None,
            install_url: config.install_url.clone(),
            message,
        }
    }

    pub fn snapshot(&self) -> GithubAuthSnapshot {
        self.snapshot_with_rate_limit(None)
    }

    /// Only an explicit account action may read saved credentials. Snapshots,
    /// scans and optional enrichment consume the process cache without unlocking.
    pub fn load_saved_auth(&self) -> GithubAuthSnapshot {
        self.load_saved_auth_using(|| self.load_auth_from_keychain())
    }

    fn load_saved_auth_using(
        &self,
        read: impl FnOnce() -> Result<Option<StoredGithubAuth>, Error>,
    ) -> GithubAuthSnapshot {
        let mut cached = match self.cached_auth.try_lock() {
            Ok(cached) => cached,
            Err(std::sync::TryLockError::WouldBlock) => return self.cached_snapshot(),
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("github auth lock poisoned"),
        };
        let Ok(client_id) = self.client_id() else {
            drop(cached);
            return self.cached_snapshot();
        };
        *cached = match read() {
            Ok(auth) => CachedGithubAuth::Loaded(
                auth.filter(|auth| auth.matches_client_id(Some(&client_id))),
            ),
            Err(cause) => CachedGithubAuth::Unavailable(cause.to_string()),
        };
        self.session_revision.fetch_add(1, Ordering::Relaxed);
        drop(cached);
        self.snapshot()
    }

    pub async fn snapshot_with_live_rate_limit(&self) -> GithubAuthSnapshot {
        if self.isolated {
            return self.cached_snapshot();
        }
        let token = match self.usable_token().await {
            Ok(token) => token,
            Err(cause) => {
                let mut snapshot = self.snapshot();
                snapshot.message = Some(cause.to_string());
                return snapshot;
            }
        };
        let rate_limit = match GithubApiClient::new(token) {
            Ok(client) => client.rate_limit().await.ok(),
            Err(_) => None,
        };
        self.snapshot_with_rate_limit(rate_limit)
    }

    pub async fn advisory_token(&self) -> Result<Option<String>, Error> {
        if self.cached_snapshot().checking {
            return Ok(None);
        }
        self.usable_token().await
    }

    pub async fn usable_token(&self) -> Result<Option<String>, Error> {
        // GitHub rotates the refresh token. Wait here so concurrent callers
        // read the replacement credentials before deciding whether to refresh.
        let _refresh = self.refresh.lock().await;
        let Some(auth) = self.load_auth()? else {
            return Ok(None);
        };

        if !auth.access_token_expiring_soon() {
            return Ok(Some(auth.access_token));
        }

        let Some(refresh_token) = auth.refresh_token.as_deref() else {
            return Ok(Some(auth.access_token));
        };

        let refreshed = self.refresh_token(refresh_token, &auth).await?;
        self.store_refreshed_auth(&auth, refreshed, |replacement| {
            self.write_auth_to_keychain(replacement)
        })
    }

    pub async fn start_login(&self) -> Result<GithubLoginStart, Error> {
        let (client_id, sequence) = {
            let mut flow = self.pending.lock().expect("github auth lock poisoned");
            let client_id = self.client_id()?;
            flow.sequence += 1;
            flow.login = None;
            (client_id, flow.sequence)
        };
        let client = auth_http_client()?;
        let response = client
            .post("https://github.com/login/device/code")
            .header("Accept", "application/json")
            .form(&[("client_id", client_id.as_str())])
            .send()
            .await
            .map_err(|cause| Error::App(format!("failed to start GitHub login: {cause}")))?
            .error_for_status()
            .map_err(|cause| Error::App(format!("GitHub login failed to start: {cause}")))?
            .json::<DeviceCodeResponse>()
            .await
            .map_err(|cause| {
                Error::App(format!("failed to parse GitHub login response: {cause}"))
            })?;

        let expires_at = OffsetDateTime::now_utc() + Duration::seconds(response.expires_in as i64);
        let login_id = format!("github-login-{sequence}");
        let pending = PendingGithubLogin {
            login_id: login_id.clone(),
            device_code: response.device_code,
            user_code: response.user_code,
            verification_uri: response.verification_uri,
            expires_at,
            interval_seconds: response.interval.unwrap_or(5),
        };
        let start = GithubLoginStart {
            login_id,
            user_code: pending.user_code.clone(),
            verification_uri: pending.verification_uri.clone(),
            expires_at: format_time(expires_at),
            interval_seconds: pending.interval_seconds,
        };

        let mut flow = self.pending.lock().expect("github auth lock poisoned");
        if flow.sequence != sequence {
            return Err(Error::App("GitHub sign-in was superseded.".into()));
        }
        flow.login = Some(pending);
        Ok(start)
    }

    pub async fn poll_login(&self, login_id: &str) -> Result<GithubLoginPollResult, Error> {
        let (pending, client_id) = {
            let flow = self.pending.lock().expect("github auth lock poisoned");
            let Some(pending) = flow
                .login
                .as_ref()
                .filter(|pending| pending.login_id == login_id)
                .cloned()
            else {
                return Ok(self.inactive_login());
            };
            (pending, self.client_id()?)
        };

        if OffsetDateTime::now_utc() >= pending.expires_at {
            if !self.consume_pending_login(login_id, || Ok(()))? {
                return Ok(self.inactive_login());
            }
            return Ok(GithubLoginPollResult {
                status: GithubLoginPollStatus::Expired,
                message: Some("GitHub login code expired.".into()),
                interval_seconds: 0,
                auth: self.snapshot(),
            });
        }

        let client = auth_http_client()?;
        let response = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id.as_str()),
                ("device_code", pending.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|cause| Error::App(format!("failed to poll GitHub login: {cause}")))?
            .error_for_status()
            .map_err(|cause| Error::App(format!("GitHub login polling failed: {cause}")))?
            .json::<TokenResponse>()
            .await
            .map_err(|cause| Error::App(format!("failed to parse GitHub login token: {cause}")))?;

        if let Some(error) = response.error.as_deref() {
            return self.handle_login_poll_error(login_id, error, response.error_description);
        }

        let access_token = response
            .access_token
            .ok_or_else(|| Error::App("GitHub login did not return an access token.".into()))?;
        let user = self.fetch_user(&access_token).await?;
        let auth = StoredGithubAuth {
            client_id: Some(client_id),
            access_token,
            refresh_token: response.refresh_token,
            expires_at: response
                .expires_in
                .map(|seconds| format_time(OffsetDateTime::now_utc() + Duration::seconds(seconds))),
            refresh_expires_at: response
                .refresh_token_expires_in
                .map(|seconds| format_time(OffsetDateTime::now_utc() + Duration::seconds(seconds))),
            login: user.login,
            avatar_url: user.avatar_url,
            profile_url: user.html_url,
        };
        if !self.consume_pending_login(login_id, || self.store_auth(&auth))? {
            return Ok(self.inactive_login());
        }

        Ok(GithubLoginPollResult {
            status: GithubLoginPollStatus::Success,
            message: None,
            interval_seconds: 0,
            auth: self.snapshot_with_live_rate_limit().await,
        })
    }

    fn inactive_login(&self) -> GithubLoginPollResult {
        GithubLoginPollResult {
            status: GithubLoginPollStatus::Expired,
            message: Some("GitHub login is no longer pending.".into()),
            interval_seconds: 0,
            auth: self.snapshot(),
        }
    }

    fn consume_pending_login(
        &self,
        login_id: &str,
        complete: impl FnOnce() -> Result<(), Error>,
    ) -> Result<bool, Error> {
        let mut flow = self.pending.lock().expect("github auth lock poisoned");
        if !flow
            .login
            .as_ref()
            .is_some_and(|login| login.login_id == login_id)
        {
            return Ok(false);
        }
        complete()?;
        flow.login = None;
        Ok(true)
    }

    fn handle_login_poll_error(
        &self,
        login_id: &str,
        error: &str,
        message: Option<String>,
    ) -> Result<GithubLoginPollResult, Error> {
        let mut flow = self.pending.lock().expect("github auth lock poisoned");
        let Some(login) = flow
            .login
            .as_mut()
            .filter(|login| login.login_id == login_id)
        else {
            return Ok(self.inactive_login());
        };
        let (status, interval_seconds) = match error {
            "authorization_pending" => (GithubLoginPollStatus::Pending, login.interval_seconds),
            "slow_down" => {
                login.interval_seconds = login.interval_seconds.saturating_add(5);
                (GithubLoginPollStatus::SlowDown, login.interval_seconds)
            }
            "expired_token" => (GithubLoginPollStatus::Expired, 0),
            "access_denied" => (GithubLoginPollStatus::Denied, 0),
            _ => {
                return Err(Error::App(format!(
                    "GitHub login failed: {}",
                    message.unwrap_or_else(|| error.to_string())
                )))
            }
        };
        if matches!(
            status,
            GithubLoginPollStatus::Expired | GithubLoginPollStatus::Denied
        ) {
            flow.login = None;
        }
        Ok(GithubLoginPollResult {
            status,
            message,
            interval_seconds,
            auth: self.snapshot(),
        })
    }

    pub fn logout(&self) -> Result<(), Error> {
        let mut flow = self.pending.lock().expect("github auth lock poisoned");
        let mut cached = self.cached_auth.lock().expect("github auth lock poisoned");
        if matches!(&*cached, CachedGithubAuth::Loaded(Some(_))) && self.client_id().is_ok() {
            match keychain_entry(&self.keychain_service)?.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => {}
                Err(cause) => {
                    return Err(Error::App(format!(
                        "failed to remove GitHub token from Keychain: {cause}"
                    )))
                }
            }
        }
        flow.sequence += 1;
        flow.login = None;
        *cached = CachedGithubAuth::Loaded(None);
        self.session_revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn list_repositories(&self) -> Result<Vec<GithubRepositoryOption>, Error> {
        let token = self.usable_token().await?.ok_or_else(|| {
            Error::App("Sign in with GitHub before listing installable repositories.".into())
        })?;
        let client = GithubApiClient::new(Some(token))?;
        let mut installation_urls =
            Some("https://api.github.com/user/installations?per_page=100".to_string());

        let mut repos = Vec::new();
        while let Some(url) = installation_urls {
            let (installations, next_url) =
                client.get_json_page::<InstallationsResponse>(&url).await?;
            installation_urls = next_url;

            for installation in installations.installations {
                let mut repository_urls = Some(format!(
                    "https://api.github.com/user/installations/{}/repositories?per_page=100",
                    installation.id
                ));
                while let Some(url) = repository_urls {
                    let (response, next_url) = client
                        .get_json_page::<InstallationRepositoriesResponse>(&url)
                        .await?;
                    repository_urls = next_url;
                    repos.extend(response.repositories.into_iter().map(|repo| {
                        GithubRepositoryOption {
                            owner: repo.owner.login,
                            name: repo.name,
                            full_name: repo.full_name,
                            private: repo.private,
                            html_url: repo.html_url,
                            default_branch: repo.default_branch,
                        }
                    }));
                }
            }
        }
        repos.sort_by(|left, right| left.full_name.cmp(&right.full_name));
        repos.dedup_by(|left, right| left.full_name == right.full_name);
        Ok(repos)
    }

    fn snapshot_with_rate_limit(&self, rate_limit: Option<GithubRateLimit>) -> GithubAuthSnapshot {
        let mut snapshot = self.cached_snapshot();
        snapshot.rate_limit_remaining = rate_limit.as_ref().map(|rate| rate.remaining);
        snapshot.rate_limit_limit = rate_limit.as_ref().map(|rate| rate.limit);
        snapshot.rate_limit_reset_at = rate_limit.and_then(|rate| rate.reset_at);
        snapshot
    }

    async fn refresh_token(
        &self,
        refresh_token: &str,
        current: &StoredGithubAuth,
    ) -> Result<StoredGithubAuth, Error> {
        let client_id = current
            .client_id
            .as_deref()
            .ok_or_else(|| Error::App("Saved GitHub credentials have no app Client ID.".into()))?;
        let client = auth_http_client()?;
        let response = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(|cause| Error::App(format!("failed to refresh GitHub token: {cause}")))?
            .error_for_status()
            .map_err(|cause| Error::App(format!("GitHub token refresh failed: {cause}")))?
            .json::<TokenResponse>()
            .await
            .map_err(|cause| {
                Error::App(format!("failed to parse GitHub token refresh: {cause}"))
            })?;

        if let Some(error) = response.error {
            return Err(Error::App(format!(
                "GitHub token refresh failed: {}",
                response.error_description.unwrap_or(error)
            )));
        }

        let access_token = response
            .access_token
            .ok_or_else(|| Error::App("GitHub token refresh did not return a token.".into()))?;
        let user = self.fetch_user(&access_token).await.ok();
        Ok(StoredGithubAuth {
            client_id: current.client_id.clone(),
            access_token,
            refresh_token: response
                .refresh_token
                .or_else(|| current.refresh_token.clone()),
            expires_at: response
                .expires_in
                .map(|seconds| format_time(OffsetDateTime::now_utc() + Duration::seconds(seconds)))
                .or_else(|| current.expires_at.clone()),
            refresh_expires_at: response
                .refresh_token_expires_in
                .map(|seconds| format_time(OffsetDateTime::now_utc() + Duration::seconds(seconds)))
                .or_else(|| current.refresh_expires_at.clone()),
            login: user
                .as_ref()
                .map(|user| user.login.clone())
                .unwrap_or_else(|| current.login.clone()),
            avatar_url: user
                .as_ref()
                .and_then(|user| user.avatar_url.clone())
                .or_else(|| current.avatar_url.clone()),
            profile_url: user
                .and_then(|user| user.html_url)
                .or_else(|| current.profile_url.clone()),
        })
    }

    async fn fetch_user(&self, access_token: &str) -> Result<GithubUser, Error> {
        GithubApiClient::new(Some(access_token.to_string()))?
            .get_json("https://api.github.com/user")
            .await
    }

    fn load_auth(&self) -> Result<Option<StoredGithubAuth>, Error> {
        let Ok(cached) = self.cached_auth.try_lock() else {
            return Ok(None);
        };
        match &*cached {
            CachedGithubAuth::Loaded(auth) => Ok(auth.clone()),
            CachedGithubAuth::Unavailable(message) => Err(Error::App(message.clone())),
            CachedGithubAuth::Uninitialized => Ok(None),
        }
    }

    fn store_auth(&self, auth: &StoredGithubAuth) -> Result<(), Error> {
        let mut cached = self.cached_auth.lock().expect("github auth lock poisoned");
        self.write_auth_to_keychain(auth)?;
        *cached = CachedGithubAuth::Loaded(Some(auth.clone()));
        self.session_revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn store_refreshed_auth(
        &self,
        previous: &StoredGithubAuth,
        refreshed: StoredGithubAuth,
        write: impl FnOnce(&StoredGithubAuth) -> Result<(), Error>,
    ) -> Result<Option<String>, Error> {
        let mut cached = self.cached_auth.lock().expect("github auth lock poisoned");
        match &*cached {
            CachedGithubAuth::Loaded(Some(current)) if current == previous => {}
            // An explicit sign-in or sign-out took effect while the request was in flight.
            CachedGithubAuth::Loaded(current) => {
                return Ok(current.as_ref().map(|auth| auth.access_token.clone()));
            }
            CachedGithubAuth::Uninitialized => return Ok(None),
            CachedGithubAuth::Unavailable(message) => return Err(Error::App(message.clone())),
        }
        write(&refreshed)?;
        let token = refreshed.access_token.clone();
        *cached = CachedGithubAuth::Loaded(Some(refreshed));
        Ok(Some(token))
    }

    fn write_auth_to_keychain(&self, auth: &StoredGithubAuth) -> Result<(), Error> {
        if self.isolated {
            return Err(Error::App(
                "Credential storage is unavailable in this isolated test profile.".into(),
            ));
        }
        let value = serde_json::to_string(auth)?;
        keychain_entry(&self.keychain_service)?
            .set_password(&value)
            .map_err(|cause| {
                Error::App(format!("failed to store GitHub token in Keychain: {cause}"))
            })
    }

    fn load_auth_from_keychain(&self) -> Result<Option<StoredGithubAuth>, Error> {
        if self.isolated {
            return Ok(None);
        }
        let password = match keychain_entry(&self.keychain_service)?.get_password() {
            Ok(password) => password,
            Err(KeyringError::NoEntry) => return Ok(None),
            Err(cause) => {
                return Err(Error::App(format!(
                    "failed to read GitHub token from Keychain: {cause}"
                )))
            }
        };
        let auth = serde_json::from_str::<StoredGithubAuth>(&password).map_err(|cause| {
            Error::App(format!("failed to decode stored GitHub token: {cause}"))
        })?;
        Ok(Some(auth))
    }

    fn client_id(&self) -> Result<String, Error> {
        self.config
            .read()
            .expect("github configuration lock poisoned")
            .client_id
            .clone()
            .ok_or_else(|| {
                Error::App("Set a GitHub App Client ID in Settings before signing in.".into())
            })
    }
}

impl Default for GithubAuthState {
    fn default() -> Self {
        Self::new()
    }
}

impl StoredGithubAuth {
    fn matches_client_id(&self, expected: Option<&str>) -> bool {
        self.client_id.as_deref() == expected
    }

    fn access_token_expiring_soon(&self) -> bool {
        let Some(expires_at) = self.expires_at.as_deref().and_then(parse_time) else {
            return false;
        };
        expires_at <= OffsetDateTime::now_utc() + Duration::minutes(5)
    }
}

fn keychain_entry(service: &str) -> Result<Entry, Error> {
    use_keychain_store()?;
    Entry::new(service, KEYCHAIN_USER)
        .map_err(|cause| Error::App(format!("failed to open GitHub Keychain entry: {cause}")))
}

fn keychain_service() -> String {
    #[cfg(debug_assertions)]
    let override_value = std::env::var(QA_KEYCHAIN_SERVICE_ENV).ok();
    #[cfg(not(debug_assertions))]
    let override_value: Option<String> = None;

    resolve_keychain_service(override_value.as_deref())
}

fn resolve_keychain_service(override_value: Option<&str>) -> String {
    override_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(KEYCHAIN_SERVICE)
        .to_string()
}

fn use_keychain_store() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        let config = std::collections::HashMap::new();
        let store = apple_native_keyring_store::keychain::Store::new_with_configuration(&config)
            .map_err(|cause| {
                Error::App(format!(
                    "failed to initialize macOS Keychain store: {cause}"
                ))
            })?;
        keyring_core::set_default_store(store);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::App(
            "GitHub token storage requires macOS Keychain in this build.".into(),
        ))
    }
}

fn auth_http_client() -> Result<Client, Error> {
    Client::builder()
        .user_agent(format!("cvec/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|cause| Error::App(format!("failed to build GitHub auth client: {cause}")))
}

fn optional_environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn format_time(value: OffsetDateTime) -> String {
    timestamps::format_seconds(value)
}

fn parse_time(value: &str) -> Option<OffsetDateTime> {
    timestamps::parse(value).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_keychain_service, GithubAuthConfig, GithubAuthState, StoredGithubAuth,
        KEYCHAIN_SERVICE,
    };

    fn configured_state() -> GithubAuthState {
        GithubAuthState {
            config: super::RwLock::new(GithubAuthConfig {
                client_id: Some("fixture-client".into()),
                ..GithubAuthConfig::default()
            }),
            ..GithubAuthState::new()
        }
    }

    fn stored_auth(client_id: Option<&str>) -> StoredGithubAuth {
        StoredGithubAuth {
            client_id: client_id.map(str::to_string),
            access_token: "token".into(),
            refresh_token: None,
            expires_at: None,
            refresh_expires_at: None,
            login: "user".into(),
            avatar_url: None,
            profile_url: None,
        }
    }

    #[test]
    fn cached_status_never_waits_for_a_keychain_read() {
        let state = configured_state();
        let locked = state.cached_auth.lock().unwrap();
        let start = std::time::Instant::now();
        let snapshot = state.cached_snapshot();
        assert!(snapshot.checking);
        assert!(state.load_auth().unwrap().is_none());
        assert!(state.snapshot().checking);
        assert!(start.elapsed() < std::time::Duration::from_millis(20));
        drop(locked);
        assert!(!state.cached_snapshot().checking);
        assert!(state.cached_snapshot().deferred);
        let isolated = GithubAuthState::isolated();
        assert!(!isolated.cached_snapshot().checking);
        assert!(!isolated.cached_snapshot().signed_in);
        assert_eq!(isolated.keychain_service, "dev.cvec.performance-qa.github");
    }

    #[test]
    fn deferred_snapshots_and_optional_tokens_never_load_credentials() {
        use super::*;
        let state = configured_state();
        assert!(state.snapshot().deferred);
        assert!(!state.snapshot().checking);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(runtime.block_on(state.usable_token()).unwrap().is_none());
        assert!(runtime.block_on(state.advisory_token()).unwrap().is_none());
        assert!(matches!(
            *state.cached_auth.lock().unwrap(),
            CachedGithubAuth::Uninitialized
        ));
    }

    #[test]
    fn explicit_saved_login_can_retry_a_failed_read_and_preserves_the_process_cache() {
        use super::*;
        let state = configured_state();
        let failed =
            state.load_saved_auth_using(|| Err(Error::App("Synthetic Keychain denial".into())));
        assert!(!failed.signed_in);
        assert!(failed
            .message
            .unwrap()
            .contains("Synthetic Keychain denial"));
        let snapshot =
            state.load_saved_auth_using(|| Ok(Some(stored_auth(Some("fixture-client")))));
        assert!(snapshot.signed_in);
        assert_eq!(snapshot.config_revision, failed.config_revision);
        assert!(snapshot.session_revision > failed.session_revision);
        assert!(!snapshot.deferred);
        assert!(state.cached_snapshot().signed_in);
        let isolated = GithubAuthState::isolated();
        assert!(
            !isolated
                .load_saved_auth_using(|| panic!(
                    "isolated profile must never invoke credential storage"
                ))
                .signed_in
        );
        assert!(!isolated.cached_snapshot().configured);
        assert!(isolated.store_auth(&stored_auth(None)).is_err());
        isolated.logout().unwrap();
    }

    #[test]
    fn concurrent_saved_login_reuses_the_pending_read_without_waiting_or_reading_again() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        let state = Arc::new(configured_state());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_state = state.clone();
        let first = std::thread::spawn(move || {
            first_state.load_saved_auth_using(|| {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(Some(stored_auth(Some("fixture-client"))))
            })
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        let second_state = state.clone();
        let second = std::thread::spawn(move || {
            let snapshot = second_state
                .load_saved_auth_using(|| panic!("a second credential read must not be started"));
            result_tx.send(snapshot).unwrap();
        });
        let pending = result_rx.recv_timeout(Duration::from_millis(500));
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        let pending =
            pending.expect("the duplicate request must return while the first read is pending");
        assert!(pending.checking);
        assert!(state.cached_snapshot().signed_in);
    }

    #[test]
    fn stored_auth_only_matches_the_issuing_client() {
        assert!(stored_auth(Some("new-client")).matches_client_id(Some("new-client")));
        assert!(!stored_auth(Some("old-client")).matches_client_id(Some("new-client")));
        assert!(!stored_auth(None).matches_client_id(Some("new-client")));
    }

    #[test]
    fn sign_out_or_new_login_discards_a_background_refresh() {
        use super::*;
        let state = GithubAuthState::isolated();
        let previous = stored_auth(Some("fixture-client"));
        let mut replacement = previous.clone();
        replacement.access_token = "rotated-token".into();
        *state.cached_auth.lock().unwrap() = CachedGithubAuth::Loaded(Some(previous.clone()));

        state.logout().unwrap();
        let result = state
            .store_refreshed_auth(&previous, replacement.clone(), |_| {
                panic!("stale refresh attempted to persist credentials after sign-out")
            })
            .unwrap();
        assert_eq!(result, None);

        let mut new_login = previous.clone();
        new_login.access_token = "new-login".into();
        *state.cached_auth.lock().unwrap() = CachedGithubAuth::Loaded(Some(new_login));
        let result = state
            .store_refreshed_auth(&previous, replacement, |_| {
                panic!("stale refresh attempted to replace another login")
            })
            .unwrap();
        assert_eq!(result.as_deref(), Some("new-login"));
    }

    #[tokio::test]
    async fn queued_token_read_uses_the_credentials_present_after_the_refresh() {
        use super::*;
        let state = configured_state();
        let mut auth = stored_auth(Some("fixture-client"));
        let rotation = state.refresh.lock().await;
        let pending = state.usable_token();
        futures::pin_mut!(pending);
        assert!(futures::poll!(&mut pending).is_pending());

        auth.access_token = "rotated-token".into();
        *state.cached_auth.lock().unwrap() = CachedGithubAuth::Loaded(Some(auth));
        drop(rotation);
        assert_eq!(pending.await.unwrap().as_deref(), Some("rotated-token"));
    }

    #[test]
    fn stale_device_poll_preserves_the_active_login_and_sign_out_cancels_it() {
        use super::*;
        let state = GithubAuthState::isolated();
        let login = PendingGithubLogin {
            login_id: "current".into(),
            device_code: "synthetic-device-code".into(),
            user_code: "synthetic-user-code".into(),
            verification_uri: "https://github.com/login/device".into(),
            expires_at: OffsetDateTime::now_utc() + Duration::minutes(1),
            interval_seconds: 5,
        };
        state.pending.lock().unwrap().login = Some(login);

        let stale = state
            .handle_login_poll_error("older", "slow_down", None)
            .unwrap();
        assert_eq!(stale.interval_seconds, 0);
        assert_eq!(
            state
                .pending
                .lock()
                .unwrap()
                .login
                .as_ref()
                .unwrap()
                .interval_seconds,
            5
        );
        assert!(!state
            .consume_pending_login("older", || panic!("stale login attempted storage"))
            .unwrap());
        state.logout().unwrap();
        assert!(!state
            .consume_pending_login("current", || panic!("signed-out login attempted storage"))
            .unwrap());
    }

    #[tokio::test]
    async fn unconfigured_login_leaves_saved_credentials_alone() {
        let state = GithubAuthState {
            config: super::RwLock::new(GithubAuthConfig::default()),
            ..GithubAuthState::new()
        };
        let snapshot = state.load_saved_auth_using(|| panic!("unexpected Keychain read"));
        assert!(!snapshot.configured);
        assert!(!snapshot.deferred);
        assert!(!snapshot.signed_in);
        assert!(snapshot.install_url.is_none());
        assert!(state.advisory_token().await.unwrap().is_none());
        assert!(state
            .start_login()
            .await
            .unwrap_err()
            .to_string()
            .contains("Client ID in Settings"));
    }

    #[test]
    fn own_app_settings_survive_restart_and_reuse_only_matching_credentials() {
        use super::*;
        let fixture = tempfile::tempdir().unwrap();
        let database_path = fixture.path().join("cvec.db");
        let db = Database::new(database_path.clone()).unwrap();
        let state = configured_state();
        let allowed = state
            .configure_app(
                &db,
                Some("  fixture-client  ".into()),
                Some("https://github.com/apps/fixture/installations/new".into()),
            )
            .unwrap();
        assert_eq!(allowed.client_id.as_deref(), Some("fixture-client"));
        assert!(allowed.deferred);
        assert!(allowed.can_configure);
        assert!(
            state
                .load_saved_auth_using(|| Ok(Some(stored_auth(Some("fixture-client")))))
                .signed_in
        );
        assert!(
            state
                .configure_app(&db, Some("fixture-client".into()), None)
                .unwrap()
                .signed_in
        );
        assert!(
            state
                .configure_app(&db, Some("another-client".into()), None)
                .unwrap()
                .deferred
        );
        let restarted =
            GithubAuthState::from_settings(&Database::new(database_path).unwrap()).unwrap();
        assert_eq!(
            restarted.snapshot().client_id.as_deref(),
            Some("another-client")
        );
        assert!(restarted.snapshot().deferred);
        assert!(
            !restarted
                .load_saved_auth_using(|| Ok(Some(stored_auth(Some("fixture-client")))))
                .signed_in
        );
        assert!(!restarted.configure_app(&db, None, None).unwrap().configured);
        assert!(
            !GithubAuthState::from_settings(&db)
                .unwrap()
                .snapshot()
                .configured
        );
        assert!(GithubAuthState::isolated()
            .configure_app(&db, Some("fixture-client".into()), None)
            .is_err());
    }

    #[test]
    fn app_change_cancels_pending_device_login_and_rejects_stale_refresh() {
        use super::*;
        let fixture = tempfile::tempdir().unwrap();
        let db = Database::new(fixture.path().join("cvec.db")).unwrap();
        let state = configured_state();
        let previous = stored_auth(Some("fixture-client"));
        state.load_saved_auth_using(|| Ok(Some(previous.clone())));
        state.pending.lock().unwrap().login = Some(PendingGithubLogin {
            login_id: "old-login".into(),
            device_code: "synthetic-device-code".into(),
            user_code: "synthetic-user-code".into(),
            verification_uri: "https://github.com/login/device".into(),
            expires_at: OffsetDateTime::now_utc() + Duration::minutes(1),
            interval_seconds: 5,
        });
        assert!(state
            .configure_app(&db, Some("unsafe id".into()), None)
            .is_err());
        assert!(state
            .configure_app(
                &db,
                Some("new-client".into()),
                Some("https://github.com.example/".into())
            )
            .is_err());
        assert!(db.get_setting(APP_SETTING).unwrap().is_none());
        assert!(state.snapshot().signed_in);
        let changed = state
            .configure_app(&db, Some("new-client".into()), None)
            .unwrap();
        assert_eq!(changed.config_revision, 1);
        assert!(!state
            .consume_pending_login("old-login", || panic!("old device login attempted storage"))
            .unwrap());
        assert_eq!(
            state
                .store_refreshed_auth(&previous, previous.clone(), |_| panic!(
                    "old app attempted storage"
                ))
                .unwrap(),
            None
        );
    }

    #[test]
    fn qa_keychain_service_requires_an_explicit_non_empty_override() {
        assert_eq!(resolve_keychain_service(None), KEYCHAIN_SERVICE);
        assert_eq!(resolve_keychain_service(Some("  ")), KEYCHAIN_SERVICE);
        assert_eq!(
            resolve_keychain_service(Some(" dev.cvec.ui-qa.github ")),
            "dev.cvec.ui-qa.github"
        );
    }
}
