import { describe, expect, it } from "vitest";

import { emptyRepoOverview, READY_SNAPSHOT } from "@/test/fixtures";
import type { RepoScanRun } from "./types";
import { deriveRepositoryMonitoring, formatCoverageWarning, RepositoryOverviewCache } from "./repositoryWorkspace";

const completed: RepoScanRun = {
  id: 10, status: "completed", startedAt: "2026-08-15T08:00:00Z", finishedAt: "2026-08-15T08:01:00Z",
  errorMessage: null, newMedium: 0, newHigh: 0, newCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
  sourceRevision: "abc",
};

describe("repository workspace projections", () => {
  it("explains the optional NuGet lockfile and the local restore alternative for saved scans", () => {
    const prefix = "NuGet dependency coverage for src/Example.csproj requires ";
    const original = `${prefix}a resolved packages.lock.json`;
    const current = `${prefix}packages.lock.json or a current project.assets.json for this project directory.`;

    const github = formatCoverageWarning(current, "github");
    expect(github).toContain("Resolved NuGet versions are unavailable for src/Example.csproj.");
    expect(github).toContain("a local restore, or a build SBOM");
    expect(formatCoverageWarning(original, "github")).toBe(github);
    expect(formatCoverageWarning(current, "local")).toContain("Run dotnet restore in a local checkout");
    expect(formatCoverageWarning("Could not read package metadata.", "github")).toBe("Could not read package metadata.");
  });

  it("uses strict monitoring precedence and explains missing revision evidence", () => {
    const repo = { ...READY_SNAPSHOT.repos[0], lastScanStatus: "completed" as const, lastScannedAt: completed.finishedAt };
    expect(deriveRepositoryMonitoring({ ...repo, status: "scanning" }, { ...completed, status: "running" }, "off").state).toBe("scanning");
    expect(deriveRepositoryMonitoring({ ...repo, status: "error", lastScanStatus: "failed" }, { ...completed, status: "failed" }, "off").state).toBe("failed");
    expect(deriveRepositoryMonitoring({ ...repo, lastScannedAt: null, lastScanStatus: null }, null, "off").state).toBe("never_scanned");
    expect(deriveRepositoryMonitoring({ ...repo, sourceType: "github", lastRemoteCommitSha: "new" }, completed, "off").state).toBe("source_ahead");
    expect(deriveRepositoryMonitoring({ ...repo, sourceType: "github", lastRemoteCommitSha: "new", lastSuccessfulSourceRevision: "abc" }, null, "off").state).toBe("source_ahead");
    expect(deriveRepositoryMonitoring(repo, completed, "12h", Date.parse("2026-08-16T00:00:01Z")).state).toBe("stale");
    expect(deriveRepositoryMonitoring(
      { ...repo, status: "error", lastScanStatus: "failed" },
      { ...completed, id: 11, status: "failed", finishedAt: "2026-08-15T10:01:00Z" },
      "off",
      Date.parse("2026-08-15T11:00:00Z"),
      completed,
    )).toMatchObject({ state: "failed", lastSuccessfulScanAt: completed.finishedAt });
  });

  it("bounds and expires the session LRU", () => {
    const detail = emptyRepoOverview(READY_SNAPSHOT.repos[0]);
    const cache = new RepositoryOverviewCache(2, 10);
    cache.set(1, detail, 0); cache.set(2, detail, 0); cache.get(1, 1); cache.set(3, detail, 1);
    expect(cache.get(2, 1)).toBeNull();
    expect(cache.size).toBe(2);
    expect(cache.get(1, 11)).toBeNull();
  });
});
