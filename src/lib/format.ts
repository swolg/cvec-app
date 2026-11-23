import type { ScanAllSummary } from "./types";

export function formatRelativeTimestamp(value: string | null): string {
  if (!value) {
    return "Never";
  }

  const timestamp = new Date(value).getTime();
  if (Number.isNaN(timestamp)) {
    return value;
  }

  const diffMs = Date.now() - timestamp;
  const diffMinutes = Math.max(0, Math.round(diffMs / 60000));

  if (diffMinutes < 1) {
    return "Just now";
  }

  if (diffMinutes < 60) {
    return `${diffMinutes}m ago`;
  }

  const diffHours = Math.round(diffMinutes / 60);
  if (diffHours < 24) {
    return `${diffHours}h ago`;
  }

  const diffDays = Math.round(diffHours / 24);
  return `${diffDays}d ago`;
}

export function formatTimestamp(value: string | null): string {
  if (!value) {
    return "Never";
  }

  const timestamp = new Date(value);
  if (Number.isNaN(timestamp.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(timestamp);
}

export function formatScheduleLabel(value: "off" | "12h"): string {
  return value === "12h" ? "Every 12h" : "Off";
}

export function formatNewFindingsLabel(
  newMedium: number,
  newHigh: number,
  newCritical: number,
): string {
  const total = newMedium + newHigh + newCritical;
  if (total === 0) {
    return "No new findings";
  }

  return `${total} new finding${total === 1 ? "" : "s"}`;
}

export function incompleteInventoryReason(message?: string | null): string | undefined {
  const prefix = ["Incomplete npm inventory: ", "Incomplete scan evidence: "]
    .find((prefix) => message?.startsWith(prefix));
  return prefix ? message?.slice(prefix.length) : undefined;
}

export function formatLastScanStatus(status: "running" | "completed" | "failed" | "unknown" | null, errorMessage?: string | null): string {
  switch (status) {
    case "running":
      return "Scanning";
    case "failed":
      return incompleteInventoryReason(errorMessage) ? "Incomplete" : "Failed";
    case "completed":
      return "Scanned";
    default:
      return "Not scanned";
  }
}

export function formatStatusLabel(status: string): string {
  switch (status) {
    case "clean":
      return "Clean";
    case "warning":
      return "Open issues";
    case "critical":
      return "New critical";
    case "error":
      return "Scan error";
    case "scanning":
      return "Scanning";
    default:
      return "Not scanned";
  }
}

export function formatRepoProviderLabel(provider: string | null): string {
  switch (provider) {
    case "github":
      return "GitHub";
    case "gitlab":
      return "GitLab";
    case "bitbucket":
      return "Bitbucket";
    case "azure_devops":
      return "Azure DevOps";
    case "sourcehut":
      return "SourceHut";
    case "custom":
      return "Custom Git";
    default:
      return "Local Git";
  }
}

export function statusBadgeVariant(status: string): "success" | "warning" | "destructive" | "outline" | "secondary" {
  switch (status) {
    case "clean":
      return "success";
    case "warning":
      return "warning";
    case "critical":
    case "error":
      return "destructive";
    case "scanning":
      return "secondary";
    default:
      return "outline";
  }
}

export function formatExternalSourceLabel(source: "osv" | "github" | "nvd" | "cisa_kev" | "vendor"): string {
  switch (source) {
    case "osv":
      return "OSV";
    case "github":
      return "GitHub";
    case "nvd":
      return "NVD";
    case "cisa_kev":
      return "CISA KEV";
    case "vendor":
      return "Vendor";
  }
}

export function formatExternalSeverityLabel(
  severity: "malware" | "critical" | "high" | "medium" | "low" | "unknown",
): string {
  switch (severity) {
    case "malware":
      return "Malicious package advisory";
    case "critical":
      return "Critical";
    case "high":
      return "High";
    case "medium":
      return "Medium";
    case "low":
      return "Low";
    default:
      return "Unknown";
  }
}

export function externalSeverityBadgeVariant(
  severity: "malware" | "critical" | "high" | "medium" | "low" | "unknown",
): "success" | "warning" | "destructive" | "outline" | "secondary" {
  switch (severity) {
    case "malware":
    case "critical":
      return "destructive";
    case "high":
      return "warning";
    case "medium":
      return "secondary";
    case "low":
      return "outline";
    default:
      return "outline";
  }
}

export function describeScanResult(summary: ScanAllSummary) {
  const { reposAttempted, reposScanned, reposFailed } = summary;
  const counts = `Scanned ${reposScanned} of ${reposAttempted} repositories.`;

  if (reposAttempted === 0) {
    return {
      kind: "info" as const,
      title: "No repositories to scan",
      description: "Choose a folder with Git repositories and try again.",
    };
  }
  if (reposFailed > 0) {
    return {
      kind: reposScanned === 0 ? "error" as const : "warning" as const,
      title: reposScanned === 0 ? "Scan failed" : "Scan finished with failures",
      description: `${counts} ${reposFailed} failed. Open Scan activity for details.`,
    };
  }
  const alerts = summary.alertsCreated;
  return { kind: "success" as const, title: "Scan complete", description: alerts ? `${counts} Created ${alerts} alert${alerts === 1 ? "" : "s"}.` : counts };
}
