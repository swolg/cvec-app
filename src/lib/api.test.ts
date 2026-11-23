import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { getFindingRemediation, openFindingManifest } from "./api";

const projection = {
  schemaVersion: 1, repoId: 1, repoName: "cvec", fingerprint: "fp", advisoryId: "CVE-1", packageName: "router",
  installedVersion: "1.0.0", packageType: "npm", packageUrl: "pkg:npm/router@1.0.0", scannerReportedTarget: "1.0.1",
  evidenceScanRunId: 10, evidenceScannedAt: "2026-08-17T12:00:00Z", sourceSnapshot: { revision: null, dirty: null }, relativePath: null,
  locationKind: null, relationship: "unknown", parentPackage: null, evidenceQuality: "unavailable", evidenceSource: "none",
  locationCount: 0, verificationState: "unverifiable", verificationScanRunId: null, verificationScannedAt: null,
  canOpenManifest: false, priorityReasons: [], briefMarkdown: "## CVEC fix brief",
};

describe("remediation API", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("loads and validates through the local Tauri command with immutable identity", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(projection);
    await expect(getFindingRemediation(1, "fp", 10)).resolves.toEqual(projection);
    expect(invoke).toHaveBeenCalledWith("get_finding_remediation", { repoId: 1, fingerprint: "fp", evidenceScanRunId: 10 });
  });

  it("rejects malformed local payloads and performs no network fallback", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    vi.mocked(invoke).mockResolvedValueOnce({ ...projection, relationship: "guessed" });
    await expect(getFindingRemediation(1, "fp", 10)).rejects.toThrow(/relationship/i);
    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });

  it("opens a manifest without accepting or forwarding a frontend path", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    await openFindingManifest(1, "fp", 10);
    expect(invoke).toHaveBeenCalledWith("open_finding_manifest", { repoId: 1, fingerprint: "fp", evidenceScanRunId: 10 });
  });
});
