import type { FixSnapshot } from "@/lib/types";

export function fixSnapshot(): FixSnapshot {
  return {
    workspace: {
      schemaVersion: 1, revision: 0, id: "a".repeat(24), repositoryId: 1,
      sourcePath: "/tmp/source", worktreePath: "/tmp/workspaces/fix", branch: "fix/cvec-test", baseRevision: "b".repeat(40),
      baselineScanId: 10, baselineMatched: true, baselineFindings: [], findingFingerprint: null, providerThreadId: null, turnId: null,
      status: "ready", activity: "Workspace ready.", messages: [], checks: [], rescan: null, createdAt: "2026-11-03T00:00:00Z", updatedAt: "2026-11-03T00:00:00Z",
    },
    diff: { files: [], omittedFiles: 0, truncated: false, digest: "c".repeat(64) },
    sourceError: null, actions: ["npm:test:run", "npm:prepare"],
  };
}
