import type { FixSnapshot, FixWorkspace } from "@/lib/types";

export function fixStatusLabel(status: FixWorkspace["status"]) {
  const labels: Record<string, string> = { ready: "Worktree ready", working: "Provider working", testing: "Running check", scanning: "Rescanning worktree", interrupted: "Stopped", failed: "Action failed" };
  return labels[status] ?? status;
}

export function fixWorkspaceLabel(workspace: FixWorkspace) {
  const finding = workspace.baselineFindings.find((item) => item.fingerprint === workspace.findingFingerprint);
  const task = workspace.messages.find((item) => item.role === "user")?.text.trim().split(/\r?\n/)[0];
  const title = finding?.advisoryId || task || workspace.findingFingerprint?.split("|")[0] || "New repository fix";
  const shortTitle = Array.from(title).slice(0, 72).join("");
  const date = new Date(workspace.createdAt);
  const created = Number.isNaN(date.getTime()) ? "Saved locally" : date.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  return `${shortTitle}${shortTitle !== title ? "…" : ""} · ${workspace.provider?.displayName ?? "No provider task yet"} · ${created} · ${workspace.id.slice(0, 8)}`;
}

export const isDependencyPreparation = (action: string) => /^(npm:(prepare|lockfile)|(cargo|tauri):(fetch|update))$/.test(action);

export function fixEvidenceFreshness(evidence: { digest: string; stale: boolean }, diff: FixSnapshot["diff"]) {
  if (!diff) return "freshness unavailable";
  return evidence.stale || evidence.digest !== diff.digest ? "stale" : null;
}

export function fixEvidenceSummary(snapshot: FixSnapshot) {
  const latestByAction = [...new Map(snapshot.workspace.checks.map((check) => [check.action, check])).values()];
  const checks = latestByAction.filter((check) => !isDependencyPreparation(check.action));
  const failed = checks.filter((check) => check.status === "failed").length;
  const incomplete = checks.some((check) => check.status !== "passed");
  const checkFreshness = checks.map((check) => fixEvidenceFreshness(check, snapshot.diff)).find(Boolean);
  const preparationFailed = latestByAction.some((check) => isDependencyPreparation(check.action) && check.status !== "passed");
  const checkLabel = `${!checks.length ? "Checks: not run" : `Checks: ${failed ? `${failed} failed` : incomplete ? "incomplete" : "passed"}${checkFreshness ? ` · ${checkFreshness}` : ""}`}${preparationFailed ? " · preparation failed" : ""}`;
  const scan = snapshot.workspace.rescan;
  const scanFreshness = scan ? fixEvidenceFreshness(scan, snapshot.diff) : null;
  return {
    checkLabel,
    checkWarning: failed > 0 || incomplete || Boolean(checkFreshness) || preparationFailed,
    checkPassed: checks.length > 0 && !incomplete && !checkFreshness && !preparationFailed,
    scanLabel: scan ? `Rescan: ${scan.findingCount} ${scan.findingCount === 1 ? "finding" : "findings"}${scanFreshness ? ` · ${scanFreshness}` : ""}` : "Rescan: not run",
    scanWarning: Boolean(scan && (scan.findingCount > 0 || scanFreshness)),
  };
}
