use super::{FixFinding, FixProvider, FixWorkspace};
use crate::core::provider_connections::ProviderConnectionId;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn short(value: &str) -> bool {
    value.len() <= 4096
}

fn optional_short(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(short)
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn finding(value: &FixFinding) -> bool {
    [
        &value.fingerprint,
        &value.advisory_id,
        &value.package_name,
        &value.severity,
    ]
    .into_iter()
    .all(|value| short(value))
        && [
            &value.version,
            &value.package_type,
            &value.package_url,
            &value.fix_version,
        ]
        .into_iter()
        .all(optional_short)
        && matches!(value.severity.as_str(), "MEDIUM" | "HIGH" | "CRITICAL")
}

impl FixWorkspace {
    pub(super) fn require_active(&self) -> Result<(), crate::Error> {
        if self.archived_at.is_some() {
            return Err(crate::Error::App(
                "Restore this archived workspace before running tasks, checks or rescans.".into(),
            ));
        }
        Ok(())
    }
    /// Keep corrupted or future records on disk without letting one invalid
    /// record invalidate the whole strictly validated frontend inventory.
    pub(crate) fn is_supported_record(&self) -> bool {
        matches!(self.schema_version, 1..=3)
            && self.archived_at.as_deref().is_none_or(|value| {
                self.schema_version == 3
                    && !value.is_empty()
                    && value.len() <= 64
                    && !super::is_busy(&self.status)
            })
            && self.provider.as_ref().is_none_or(provider)
            && self.revision <= MAX_SAFE_INTEGER
            && self.repository_id > 0
            && self.repository_id as u64 <= MAX_SAFE_INTEGER
            && hex(&self.id, 24)
            && hex(&self.base_revision, 40)
            && [
                &self.source_path,
                &self.worktree_path,
                &self.branch,
                &self.activity,
                &self.created_at,
                &self.updated_at,
            ]
            .into_iter()
            .all(|value| short(value))
            && [
                &self.finding_fingerprint,
                &self.provider_thread_id,
                &self.turn_id,
            ]
            .into_iter()
            .all(optional_short)
            && self
                .baseline_scan_id
                .is_none_or(|id| id > 0 && id as u64 <= MAX_SAFE_INTEGER)
            && matches!(
                self.status.as_str(),
                "ready" | "working" | "testing" | "scanning" | "interrupted" | "failed"
            )
            && self.baseline_findings.iter().all(finding)
            && self.messages.len() <= 100
            && self.messages.iter().all(|message| {
                message.provider.as_ref().is_none_or(provider)
                    && matches!(message.role.as_str(), "user" | "assistant")
                    && message.text.len() <= 32_000
            })
            && self.checks.len() <= 20
            && self.checks.iter().all(|check| {
                short(&check.action)
                    && short(&check.finished_at)
                    && matches!(
                        check.status.as_str(),
                        "passed" | "failed" | "unavailable" | "interrupted"
                    )
                    && check.output.len() <= 128 * 1024
                    && hex(&check.digest, 64)
            })
            && self.rescan.as_ref().is_none_or(|scan| {
                scan.findings.len() <= 500
                    && scan.findings.iter().all(finding)
                    && scan.finding_count >= scan.findings.len()
                    && scan.finding_count as u64 <= MAX_SAFE_INTEGER
                    && scan
                        .not_detected
                        .is_none_or(|count| count <= self.baseline_findings.len())
                    && hex(&scan.digest, 64)
                    && short(&scan.scanned_at)
                    && short(&scan.scanner_versions)
            })
    }
}

fn provider(value: &FixProvider) -> bool {
    ProviderConnectionId::parse(value.connection_id.clone()).is_ok()
        && short(&value.display_name)
        && value.model.as_ref().is_none_or(|model| model.len() <= 256)
}
