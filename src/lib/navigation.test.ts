import { describe, expect, it } from "vitest";
import { WorkspaceHistory, normalizeWorkspace } from "./navigation";

describe("workspace navigation", () => {
  it("normalizes the legacy inbox destination", () => {
    expect(normalizeWorkspace("inbox")).toBe("review");
  });

  it("preserves independent workspace context in the session", () => {
    const history = new WorkspaceHistory();
    history.push({ workspace: "review", context: { viewId: "critical", selectedKey: "1:fp", scrollTop: 420 } });
    history.push({ workspace: "repositories", context: { repoId: 1 } });
    expect(history.back()?.workspace).toBe("review");
    expect(history.context("review")).toEqual({ viewId: "critical", selectedKey: "1:fp", scrollTop: 420 });
  });

  it("keeps filters and sort isolated and never persists stale selection across sessions", () => {
    const filters = { reviewStates: ["unreviewed" as const], scannerStates: [], severities: [], repoIds: [], sourceTypes: [], fixAvailable: null, packages: [], ecosystems: [], knownExploited: null, ransomwareKnown: null, changedSince: null, scanRunIds: [] };
    const first = new WorkspaceHistory();
    first.push({ workspace: "review", context: { viewId: "needs-review", selectedKey: "1:fp", scrollTop: 88, reviewQuery: { filters, search: "openssl", sort: "severity", cursor: null, pageSize: 50 } } });
    const stored = first.context("review");
    stored.reviewQuery!.search = "mutated";
    expect(first.context("review").reviewQuery?.search).toBe("openssl");
    expect(new WorkspaceHistory().context("review").selectedKey).toBeUndefined();
  });

  it("replaces repeated workspace entries and supports live context updates", () => {
    const history = new WorkspaceHistory();
    history.push({ workspace: "overview", context: {} });
    history.push({ workspace: "review", context: { viewId: "needs-review" } });
    history.push({ workspace: "review", context: { viewId: "critical" } });
    history.replaceContext("review", { viewId: "critical", scrollTop: 320 });
    expect(history.back()?.workspace).toBe("overview");
    expect(history.context("review")).toEqual({ viewId: "critical", scrollTop: 320 });
  });

  it("preserves the origin of an explicit link within the same workspace", () => {
    const history = new WorkspaceHistory();
    const original = { repoId: 7, repositorySection: "dependencies" as const, repositoryPackageQuery: "router", repositoryPackageLinkSequence: 2, scrollTop: 340 };
    history.push({ workspace: "repositories", context: original });
    history.push({ workspace: "repositories", context: { repoId: 7, repositorySection: "history", repositoryScanRunId: 42 } }, { preserveCurrent: true });
    expect(history.canGoBack).toBe(true);
    expect(history.back()).toEqual({ workspace: "repositories", context: original });
    expect(history.context("repositories")).toEqual(original);
    expect(history.canGoBack).toBe(false);
  });

  it("keeps remediation grouping ephemeral while restoring repository return context", () => {
    const history = new WorkspaceHistory();
    history.push({ workspace: "repositories", context: { repoId: 7, repositorySection: "overview", repositoryScrollAnchor: "remediation" } });
    history.push({ workspace: "review", context: {
      viewId: "all-open", selectedKey: "7:fp", ephemeralReviewContext: true,
      remediationGroup: { packageName: "router", packageUrl: "pkg:npm/router@1", packageType: "npm", installedVersion: "1", fixVersion: "2", preferredFingerprint: "fp" },
    } });
    expect(history.back()).toEqual({ workspace: "repositories", context: { repoId: 7, repositorySection: "overview", repositoryScrollAnchor: "remediation" } });
    expect(new WorkspaceHistory().context("review").remediationGroup).toBeUndefined();
  });
});
