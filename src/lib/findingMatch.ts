import type { BundledToolEvidence, FindingMatchEvidence, FindingMatchKind } from "./types";

export const FINDING_MATCH_LABEL: Record<FindingMatchKind, string> = {
  package_version: "Package and version",
  go_symbol: "Go function symbols",
  upstream: "Package source or upstream dependency",
  go_module: "Go module version only",
  nested_workflow: "Workflow nested inside the scanned tree",
  local_installation: "Local installation only",
  cpe: "CPE product match",
  unverified: "Match details unavailable",
};

export function findingMatchNeedsReview(evidence: FindingMatchEvidence | undefined): boolean {
  const kind = evidence?.kind;
  return Boolean(evidence?.bundledTool) || kind === "cpe" || kind === "go_module" || kind === "nested_workflow" || kind === "local_installation" || kind === "unverified";
}

export function findingMatchReviewLabel(evidence: FindingMatchEvidence | undefined): string | null {
  if (evidence?.kind === "go_module") return "Go version match";
  if (evidence?.bundledTool) return "Bundled tool";
  return findingMatchNeedsReview(evidence) ? "Check match" : null;
}

export function bundledToolLabel(tool: BundledToolEvidence): string {
  return tool.packageName ? `Bundled in ${tool.packageName}` : "Bundled in an installed npm tool";
}
