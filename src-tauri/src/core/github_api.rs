use std::time::Duration;
use time::OffsetDateTime;

use reqwest::{header, Client, RequestBuilder, Response, StatusCode};
use serde::Deserialize;

use crate::core::timestamps;
use crate::Error;

#[derive(Debug, Clone)]
pub struct GithubApiClient {
    client: Client,
    token: Option<String>,
}

pub enum ConditionalJson<T> {
    Modified(T, Option<String>),
    NotModified,
    RateLimited(u64),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRateLimit {
    pub remaining: u32,
    pub limit: u32,
    pub reset_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRateLimit {
    resources: RawRateLimitResources,
}

#[derive(Debug, Deserialize)]
struct RawRateLimitResources {
    core: RawRateLimitCore,
}

#[derive(Debug, Deserialize)]
struct RawRateLimitCore {
    limit: u32,
    remaining: u32,
    reset: i64,
}

impl GithubApiClient {
    pub fn new(token: Option<String>) -> Result<Self, Error> {
        let client = Client::builder()
            .user_agent(format!("cvec/{}", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many GitHub redirects")
                } else if is_allowed_github_redirect(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("blocked redirect outside allowed GitHub HTTPS hosts")
                }
            }))
            .build()
            .map_err(|cause| Error::App(format!("failed to build GitHub client: {cause}")))?;

        Ok(Self { client, token })
    }

    pub async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, Error> {
        let (body, _) = self.get_json_page(url).await?;
        Ok(body)
    }

    /// Optional advisory metadata has a smaller deadline and body budget than
    /// repository operations, and never sleeps through a rate-limit window.
    pub async fn get_advisory_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<ConditionalJson<T>, Error> {
        validate_github_api_url(url)?;
        let mut request = self
            .github_request(self.client.get(url), "application/vnd.github+json")
            .timeout(Duration::from_secs(6));
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let mut response = request
            .send()
            .await
            .map_err(|cause| Error::App(format!("Advisory request failed: {cause}")))?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(ConditionalJson::NotModified);
        }
        if let Some(retry_at) = advisory_retry_at(response.status(), response.headers()) {
            return Ok(ConditionalJson::RateLimited(retry_at));
        }
        response = self.map_status(response).await?;
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        const MAX_BYTES: usize = 2 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(Error::App("Advisory response exceeded 2 MiB".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|cause| Error::App(format!("Advisory response read failed: {cause}")))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                return Err(Error::App("Advisory response exceeded 2 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice(&bytes)
            .map_err(|cause| Error::App(format!("Invalid advisory response: {cause}")))?;
        Ok(ConditionalJson::Modified(body, etag))
    }

    pub async fn get_json_page<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
    ) -> Result<(T, Option<String>), Error> {
        const MAX_JSON_PAGE_BYTES: usize = 25 * 1024 * 1024;
        validate_github_api_url(url)?;
        let response = self
            .github_request(self.client.get(url), "application/vnd.github+json")
            .send()
            .await
            .map_err(|cause| Error::App(format!("failed to reach GitHub: {cause}")))?;
        let mut response = self.map_status(response).await?;
        let next_url = next_link(response.headers())?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_PAGE_BYTES as u64)
        {
            return Err(Error::App("GitHub API response exceeded 25 MiB".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|cause| Error::App(format!("failed to read GitHub response: {cause}")))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_JSON_PAGE_BYTES {
                return Err(Error::App("GitHub API response exceeded 25 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice::<T>(&bytes)
            .map_err(|cause| Error::App(format!("failed to decode GitHub response: {cause}")))?;
        Ok((body, next_url))
    }

    pub async fn get_raw_bytes_checked<F>(
        &self,
        url: &str,
        path: &str,
        mut check_size: F,
    ) -> Result<Vec<u8>, Error>
    where
        F: FnMut(u64) -> Result<(), Error>,
    {
        validate_github_api_url(url)?;
        let response = self
            .github_request(self.client.get(url), "application/vnd.github.raw+json")
            .send()
            .await
            .map_err(|cause| {
                Error::App(format!(
                    "failed to fetch GitHub file {path}: {}",
                    cause.without_url()
                ))
            })?;
        let response = self.map_status(response).await?;

        if let Some(content_length) = response.content_length() {
            check_size(content_length)?;
        }

        let mut bytes =
            Vec::with_capacity(response.content_length().unwrap_or(0).min(1024 * 1024) as usize);
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(|cause| {
            Error::App(format!(
                "failed to read GitHub file {path}: {}",
                cause.without_url()
            ))
        })? {
            let next_size = (bytes.len() as u64).saturating_add(chunk.len() as u64);
            check_size(next_size)?;
            bytes.extend_from_slice(&chunk);
        }

        Ok(bytes)
    }

    pub async fn rate_limit(&self) -> Result<GithubRateLimit, Error> {
        let raw = self
            .get_json::<RawRateLimit>("https://api.github.com/rate_limit")
            .await?;
        let reset_at = OffsetDateTime::from_unix_timestamp(raw.resources.core.reset)
            .ok()
            .map(timestamps::format_seconds);
        Ok(GithubRateLimit {
            remaining: raw.resources.core.remaining,
            limit: raw.resources.core.limit,
            reset_at,
        })
    }

    fn github_request(&self, request: RequestBuilder, accept: &'static str) -> RequestBuilder {
        let request = request
            .header(header::ACCEPT, accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        match self.token.as_deref() {
            Some(token) if !token.trim().is_empty() => request.bearer_auth(token),
            _ => request,
        }
    }

    async fn map_status(&self, response: Response) -> Result<Response, Error> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        if status == StatusCode::NOT_FOUND {
            return Err(Error::App(if self.token.is_some() {
                "GitHub repository or file was not found. Check that your GitHub App is installed for this repository."
                    .into()
            } else {
                "GitHub repository or file was not found. Sign in with GitHub to scan private repositories."
                    .into()
            }));
        }
        if status == StatusCode::FORBIDDEN {
            let rate_limited = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                == Some("0");
            let sso_required = response
                .headers()
                .get("x-github-sso")
                .and_then(|value| value.to_str().ok())
                .is_some();
            return Err(Error::App(if rate_limited && self.token.is_some() {
                "GitHub API rate limit reached for this account. Wait until the limit resets."
                    .into()
            } else if rate_limited {
                "GitHub API rate limit reached. Sign in with GitHub or wait until the limit resets."
                    .into()
            } else if sso_required {
                "GitHub requires SSO authorization before cvec can read this repository.".into()
            } else if self.token.is_some() {
                "GitHub refused access. Confirm your GitHub App is installed on this repository and your account can read it."
                    .into()
            } else {
                "GitHub refused access. Sign in with GitHub to scan private repositories.".into()
            }));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(Error::App(
                "GitHub authentication expired. Sign in again to continue.".into(),
            ));
        }

        Err(Error::App(format!(
            "GitHub request failed with status {status}."
        )))
    }
}

fn advisory_retry_at(status: StatusCode, headers: &header::HeaderMap) -> Option<u64> {
    if status != StatusCode::TOO_MANY_REQUESTS
        && !(status == StatusCode::FORBIDDEN
            && headers
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                == Some("0"))
    {
        return None;
    }
    let now = OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
    let retry = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .parse::<u64>()
                .ok()
                .map(|seconds| now.saturating_add(seconds))
                .or_else(|| {
                    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822)
                        .ok()
                        .map(|at| at.unix_timestamp().max(0) as u64)
                })
        });
    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    Some(retry.or(reset).unwrap_or(now + 60).max(now + 1))
}

fn next_link(headers: &header::HeaderMap) -> Result<Option<String>, Error> {
    let Some(link_value) = headers.get(header::LINK) else {
        return Ok(None);
    };
    let Ok(link) = link_value.to_str() else {
        return Ok(None);
    };
    let Some(url) = link.split(',').find_map(|part| {
        let mut segments = part.trim().split(';');
        let url = segments.next()?;
        if !segments.any(|parameter| parameter.trim() == "rel=\"next\"") {
            return None;
        }
        Some(
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string(),
        )
    }) else {
        return Ok(None);
    };

    validate_github_api_url(&url)?;
    Ok(Some(url))
}

fn validate_github_api_url(url: &str) -> Result<(), Error> {
    if is_github_api_url(url) {
        Ok(())
    } else {
        Err(Error::App(
            "GitHub pagination returned an unexpected API URL.".into(),
        ))
    }
}

fn is_github_api_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed == "https://api.github.com" || trimmed.starts_with("https://api.github.com/")
}

fn is_allowed_github_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "codeload.github.com"
                    | "raw.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_rate_limits_honor_retry_after_and_reset_without_waiting() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::RETRY_AFTER, header::HeaderValue::from_static("120"));
        let now = OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
        assert!(advisory_retry_at(StatusCode::TOO_MANY_REQUESTS, &headers).unwrap() >= now + 120);
        assert_eq!(advisory_retry_at(StatusCode::OK, &headers), None);
        headers.remove(header::RETRY_AFTER);
        headers.insert(
            "x-ratelimit-remaining",
            header::HeaderValue::from_static("0"),
        );
        headers.insert(
            "x-ratelimit-reset",
            header::HeaderValue::from_str(&(now + 600).to_string()).unwrap(),
        );
        assert_eq!(
            advisory_retry_at(StatusCode::FORBIDDEN, &headers),
            Some(now + 600)
        );
    }

    #[test]
    fn parses_next_link_header() {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::LINK,
            header::HeaderValue::from_static(
                "<https://api.github.com/user/installations?page=2>; rel=\"next\", <https://api.github.com/user/installations?page=4>; rel=\"last\"",
            ),
        );

        assert_eq!(
            next_link(&headers).unwrap().as_deref(),
            Some("https://api.github.com/user/installations?page=2")
        );
    }

    #[test]
    fn ignores_link_header_without_next_relation() {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::LINK,
            header::HeaderValue::from_static(
                "<https://api.github.com/user/installations?page=1>; rel=\"prev\"",
            ),
        );

        assert_eq!(next_link(&headers).unwrap(), None);
    }

    #[test]
    fn rejects_next_link_outside_github_api() {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::LINK,
            header::HeaderValue::from_static(
                "<https://example.test/user/installations?page=2>; rel=\"next\"",
            ),
        );

        assert!(next_link(&headers).is_err());
    }

    #[test]
    fn allows_only_https_github_redirects() {
        assert!(is_allowed_github_redirect(
            &reqwest::Url::parse("https://api.github.com/repositories/123").unwrap()
        ));
        assert!(is_allowed_github_redirect(
            &reqwest::Url::parse("https://raw.githubusercontent.com/example/cvec/main/Cargo.toml")
                .unwrap()
        ));
        assert!(!is_allowed_github_redirect(
            &reqwest::Url::parse("http://api.github.com/repositories/123").unwrap()
        ));
        assert!(!is_allowed_github_redirect(
            &reqwest::Url::parse("https://example.test/evil").unwrap()
        ));
        assert!(!is_allowed_github_redirect(
            &reqwest::Url::parse("http://127.0.0.1:8080/internal").unwrap()
        ));
    }
}
