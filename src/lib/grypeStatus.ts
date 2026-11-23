import { formatRelativeTimestamp } from "./format";
import type { GrypeDbStatus, GrypeDbUpdateSummary, WorkspaceStatus } from "./types";

export function scannerIssue(status: WorkspaceStatus, database: GrypeDbStatus | null): string | null {
  if (!status.scannerReady) return status.scannerMessage ?? "Install Syft and Grype to scan repositories.";
  if (!database) return "Check the local vulnerability database before scanning.";
  if (!database.present) return "Download the vulnerability database before scanning.";
  if (!database.valid) return "Update the invalid vulnerability database before scanning.";
  return null;
}

export function describeGrypeDb(grypeDb: GrypeDbStatus | null) {
  if (!grypeDb) {
    return {
      label: "Unavailable",
      detail: "Grype is not available yet.",
      tone: "text-ink-subtle",
      badge: "Unavailable",
      badgeClass:
        "border border-[color:var(--hairline)] bg-[color:var(--surface-1)] text-ink-subtle",
    };
  }

  if (!grypeDb.present) {
    return {
      label: "Not downloaded",
      detail: "Download the first local database.",
      tone: "text-[color:var(--severity-high)]",
      badge: "Missing",
      badgeClass:
        "border border-[color:var(--alert-warning-border)] bg-[color:var(--alert-warning-bg)] text-[color:var(--alert-warning-fg)]",
    };
  }

  if (!grypeDb.valid) {
    return {
      label: "Needs attention",
      detail: "The local database is not valid.",
      tone: "text-[color:var(--severity-critical)]",
      badge: "Invalid",
      badgeClass:
        "border border-[color:var(--alert-danger-border)] bg-[color:var(--alert-danger-bg)] text-[color:var(--alert-danger-fg)]",
    };
  }

  if (grypeDb.updateAvailable) {
    return {
      label: "Update available",
      detail: grypeDb.latestBuiltAt
        ? `New build ${formatRelativeTimestamp(grypeDb.latestBuiltAt)}`
        : "A newer database is ready.",
      tone: "text-[color:var(--severity-high)]",
      badge: "Update available",
      badgeClass:
        "border border-[color:var(--alert-warning-border)] bg-[color:var(--alert-warning-bg)] text-[color:var(--alert-warning-fg)]",
    };
  }

  return {
    label: grypeDb.updateAvailable === false ? "Up to date" : "Installed",
    detail: grypeDb.builtAt
      ? `Current build ${formatRelativeTimestamp(grypeDb.builtAt)}`
      : "Database is ready.",
    tone: "text-[color:var(--status-success)]",
    badge: grypeDb.updateAvailable === false ? "Current" : "Installed",
    badgeClass:
      "border border-[color:color-mix(in_srgb,var(--status-success)_35%,var(--hairline))] bg-[color:var(--status-success-soft)] text-[color:var(--status-success)]",
  };
}

export function describeGrypeDbUpdateStatus(grypeDb: GrypeDbStatus | null) {
  if (!grypeDb) {
    return {
      title: "Not checked",
      description: "Grype is not available yet, so cvec cannot check the DB feed.",
      tone: "text-ink-subtle",
      badge: "Unknown",
      badgeClass:
        "border border-[color:var(--hairline)] bg-[color:var(--surface-1)] text-ink-subtle",
    };
  }

  if (grypeDb.updateCheckError) {
    return {
      title: "Check failed",
      description: grypeDb.updateCheckError,
      tone: "text-[color:var(--severity-high)]",
      badge: "Error",
      badgeClass:
        "border border-[color:var(--alert-warning-border)] bg-[color:var(--alert-warning-bg)] text-[color:var(--alert-warning-fg)]",
    };
  }

  if (grypeDb.updateAvailable === true) {
    return {
      title: "New DB build available",
      description: grypeDb.latestSchemaVersion
        ? `Update available for schema ${grypeDb.latestSchemaVersion}.`
        : "A newer Grype vulnerability DB build is available.",
      tone: "text-[color:var(--severity-high)]",
      badge: "Update available",
      badgeClass:
        "border border-[color:var(--alert-warning-border)] bg-[color:var(--alert-warning-bg)] text-[color:var(--alert-warning-fg)]",
    };
  }

  if (grypeDb.updateAvailable === false) {
    return {
      title: "No update available",
      description: "The local Grype vulnerability DB matches the latest checked feed.",
      tone: "text-[color:var(--status-success)]",
      badge: "Current",
      badgeClass:
        "border border-[color:color-mix(in_srgb,var(--status-success)_35%,var(--hairline))] bg-[color:var(--status-success-soft)] text-[color:var(--status-success)]",
    };
  }

  return {
    title: "Not checked",
    description: "Run a DB update check to see whether a newer build is available.",
    tone: "text-ink-subtle",
    badge: "Unknown",
    badgeClass:
      "border border-[color:var(--hairline)] bg-[color:var(--surface-1)] text-ink-subtle",
  };
}

export function buildDbUpdateFeedback(
  summary: GrypeDbUpdateSummary | null,
) {
  if (!summary) {
    return {
      title: "Database check finished",
      description: "cvec refreshed the local Grype database state.",
    };
  }

  if (summary.outcome === "already_current") {
    return {
      title: "Database already current",
      description:
        "No newer Grype DB build was downloaded. The local database is already current.",
    };
  }

  return {
    title: "Database updated",
    description:
      summary.outcome === "downloaded"
        ? "Downloaded the first local Grype DB build for this app-managed cache."
        : `Installed a newer Grype DB build. ${formatDbUpdateTransition(summary)}`,
  };
}

function formatDbUpdateTransition(summary: GrypeDbUpdateSummary): string {
  const before = formatDbBuildSummary(
    summary.previousSchemaVersion,
    summary.previousBuiltAt,
  );
  const after = formatDbBuildSummary(
    summary.currentSchemaVersion,
    summary.currentBuiltAt,
  );

  return `Changed from ${before} to ${after}.`;
}

export function formatDbBuildSummary(
  schemaVersion: string | null,
  builtAt: string | null,
): string {
  const versionLabel = schemaVersion ? `schema ${schemaVersion}` : "no local DB";
  if (!builtAt) {
    return versionLabel;
  }

  return `${versionLabel}, build ${formatRelativeTimestamp(builtAt)}`;
}
