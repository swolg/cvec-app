import { severityLabel } from "./format";
import { SEVERITIES, type ReviewActionBasis, type ReviewFinding, type ReviewGroupCounts, type ReviewLaneCount, type ReviewState, type RepoFinding } from "./types";
import type { ReviewGrouping } from "./reviewViews";

export interface ReviewLane {
  id: string;
  label: string;
  mutable: boolean;
  reviewState?: ReviewState;
  items: ReviewFinding[];
  totalCount: number;
}
export interface ReviewScrollAnchor { key: string | null; offset: number }
export interface ReviewScrollAnchors { table: ReviewScrollAnchor; board: ReviewScrollAnchor }

export function reviewFindingKey(item: ReviewFinding): string { return `${item.repoId}:${item.finding.fingerprint}`; }

export function findingDisplayTitle(finding: RepoFinding): string | null {
  const firstLine = (finding.intel?.title || finding.title)?.split("\n", 1)[0]?.replace(/^#+\s*/, "").trim();
  if (!firstLine || firstLine.toUpperCase() === (finding.advisoryId + " affects " + finding.packageName).toUpperCase()) return null;
  const idLength = finding.advisoryId.length;
  if (firstLine.slice(0, idLength).toUpperCase() !== finding.advisoryId.toUpperCase()) return firstLine;
  const remainder = firstLine.slice(idLength);
  return !remainder || /^\s*[:·–—-]/.test(remainder) ? remainder.replace(/^\s*[:·–—-]\s*/, "").trim() || null : firstLine;
}

export function reviewActionBasis(item: ReviewFinding): ReviewActionBasis {
  return {
    scanRunId: item.scanRunId,
    riskBasis: {
      severity: item.advisorySeverity ?? item.finding.severity,
      fixVersion: item.finding.fixVersion,
      knownExploited: item.knownExploited,
      ransomwareKnown: item.ransomwareKnown,
      scannerState: item.finding.state,
    },
    reviewState: item.reviewState,
    reviewedAt: item.reviewedAt,
    snoozeUntil: item.snoozeUntil,
    reviewNote: item.reviewNote,
  };
}

export function isStaleReviewEvidence(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return message.startsWith("REVIEW_EVIDENCE_STALE:");
}

export function canonicalReviewFindings(items: ReviewFinding[]): ReviewFinding[] {
  const seen = new Set<string>();
  return items.filter((item) => { const key = reviewFindingKey(item); if (seen.has(key)) return false; seen.add(key); return true; });
}

export function transitionReviewFinding(item: ReviewFinding, state: ReviewState, details: { reviewedAt?: string | null; snoozeUntil?: string | null; reviewNote?: string | null } = {}): ReviewFinding {
  const reviewedAt = state === "reviewed" ? details.reviewedAt ?? item.reviewedAt : null;
  const snoozeUntil = state === "snoozed" ? details.snoozeUntil ?? item.snoozeUntil : null;
  const reviewNote = state === "snoozed" ? details.reviewNote ?? item.reviewNote : null;
  return {
    ...item,
    reviewState: state,
    reviewedAt,
    snoozeUntil,
    reviewNote,
    finding: { ...item.finding, reviewState: state, reviewedAt, snoozeUntil, reviewNote },
  };
}

export function transitionReviewGroupCounts(counts: ReviewGroupCounts | null, from: ReviewState, to: ReviewState): ReviewGroupCounts | null {
  if (!counts || from === to) return counts;
  return {
    ...counts,
    reviewState: counts.reviewState.map((count) => count.id === from ? { ...count, count: Math.max(0, count.count - 1) } : count.id === to ? { ...count, count: count.count + 1 } : count),
  };
}

export function projectReviewLanes(items: ReviewFinding[], grouping: Exclude<ReviewGrouping, "none">, groupCounts?: ReviewGroupCounts | null): ReviewLane[] {
  const canonical = canonicalReviewFindings(items);
  if (grouping === "reviewState") return fixedLanes(canonical, [
    ["unreviewed", "Needs review"], ["reviewed", "Reviewed"], ["snoozed", "Snoozed"],
  ], (item) => item.reviewState, true, groupCounts?.reviewState);
  if (grouping === "severity") {
    return fixedLanes(canonical, SEVERITIES.map((severity) => [severity, severityLabel(severity)] as const), (item) => item.finding.severity, false, groupCounts?.severity)
      .filter((lane, index) => index < 3 || lane.totalCount > 0);
  }
  if (grouping === "fixAvailability") return fixedLanes(canonical, [["available", "Fix available"], ["unavailable", "No fix listed"]], (item) => item.finding.fixVersion ? "available" : "unavailable", false, groupCounts?.fixAvailability);
  const repositoryBuckets = new Map<string, ReviewFinding[]>();
  const repositoryNames = new Map<string, string>();
  for (const item of canonical) {
    const id = `repository:${item.repoId}`;
    repositoryNames.set(id, item.repoName);
    const bucket = repositoryBuckets.get(id);
    if (bucket) bucket.push(item); else repositoryBuckets.set(id, [item]);
  }
  const repositories = groupCounts?.repository ?? [...repositoryNames.entries()].sort((a, b) => a[1].localeCompare(b[1]) || a[0].localeCompare(b[0])).map(([id, label]) => ({ id, label, count: repositoryBuckets.get(id)?.length ?? 0 }));
  return repositories.map(({ id, label, count }) => ({ id, label, mutable: false, items: repositoryBuckets.get(id) ?? [], totalCount: count }));
}

function fixedLanes<T extends string>(items: ReviewFinding[], definitions: readonly (readonly [T, string])[], value: (item: ReviewFinding) => T, mutable: boolean, counts?: ReviewLaneCount[]): ReviewLane[] {
  const totalById = new Map(counts?.map((count) => [count.id, count.count]));
  return definitions.map(([id, label]) => {
    const laneItems = items.filter((item) => value(item) === id);
    return { id: `${id}`, label, mutable, reviewState: mutable ? id as ReviewState : undefined, items: laneItems, totalCount: totalById.get(id) ?? laneItems.length };
  });
}

export function nearestEligibleKey(previous: ReviewFinding[], next: ReviewFinding[], key: string | null): string | null {
  if (!next.length) return null;
  if (key && next.some((item) => reviewFindingKey(item) === key)) return key;
  const previousIndex = key ? previous.findIndex((item) => reviewFindingKey(item) === key) : -1;
  return reviewFindingKey(next[Math.min(next.length - 1, Math.max(0, previousIndex))]);
}

export function reorderLanes(lanes: ReviewLane[], requestedOrder: string[]): ReviewLane[] {
  const order = new Map(requestedOrder.map((id, index) => [id, index]));
  return [...lanes].sort((a, b) => (order.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (order.get(b.id) ?? Number.MAX_SAFE_INTEGER));
}
