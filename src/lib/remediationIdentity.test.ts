import { beforeEach, describe, expect, it, vi } from "vitest";
const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => native);
import { getFindingRemediation } from "./api";
import type { FindingRemediationProjection } from "./types";

const projection: FindingRemediationProjection = {
  schemaVersion: 1, repoId: 1, repoName: "fixture", fingerprint: "fingerprint", advisoryId: "CVE-TEST", packageName: "package",
  installedVersion: "1.0.0", packageType: "npm", packageUrl: null, scannerReportedTarget: null, evidenceScanRunId: 10,
  evidenceScannedAt: null, sourceSnapshot: { revision: null, dirty: null }, relativePath: null, locationKind: null,
  relationship: "unknown", parentPackage: null, evidenceQuality: "unavailable", evidenceSource: "none", locationCount: 0,
  verificationState: "unverifiable", verificationScanRunId: null, verificationScannedAt: null, canOpenManifest: false,
  priorityReasons: [], briefMarkdown: "A bounded local fixture",
};
beforeEach(() => native.invoke.mockReset());
describe("remediation request identity", () => {
  it("accepts evidence for exactly the requested repository, finding and scan", async () => {
    native.invoke.mockResolvedValue(projection);
    await expect(getFindingRemediation(1, "fingerprint", 10)).resolves.toEqual(projection);
  });
  it.each([{ repoId: 2 }, { fingerprint: "other" }, { evidenceScanRunId: 11 }])("rejects a structurally valid response for a different identity %j", async mismatch => {
    native.invoke.mockResolvedValue({ ...projection, ...mismatch });
    await expect(getFindingRemediation(1, "fingerprint", 10)).rejects.toThrow(/different finding or scan/);
  });
});
