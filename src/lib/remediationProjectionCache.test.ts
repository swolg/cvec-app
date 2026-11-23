import { describe, expect, it, vi } from "vitest";
import { RemediationProjectionCache } from "./remediationProjectionCache";
import type { FindingRemediationProjection } from "./types";

function projection(evidenceScanRunId: number): FindingRemediationProjection {
  return {
    schemaVersion: 1, repoId: 1, repoName: "cvec", fingerprint: `fp-${evidenceScanRunId}`, advisoryId: "CVE-1",
    packageName: "router", installedVersion: "1.0.0", packageType: "npm", packageUrl: null,
    scannerReportedTarget: "1.0.1", evidenceScanRunId, evidenceScannedAt: "2026-08-17T12:00:00Z",
    sourceSnapshot: { revision: null, dirty: null }, relativePath: null, locationKind: null, relationship: "unknown", parentPackage: null,
    evidenceQuality: "unavailable", evidenceSource: "none", locationCount: 0, verificationState: "unverifiable",
    verificationScanRunId: null, verificationScannedAt: null, canOpenManifest: false,
    priorityReasons: [], briefMarkdown: "## CVEC fix brief",
  };
}

describe("RemediationProjectionCache", () => {
  it("is bounded and evicts the least recently used immutable projection", () => {
    const cache = new RemediationProjectionCache(2);
    const first = projection(1);
    const second = projection(2);
    const third = projection(3);
    cache.set({ repoId: 1, fingerprint: first.fingerprint, evidenceScanRunId: 1 }, first);
    cache.set({ repoId: 1, fingerprint: second.fingerprint, evidenceScanRunId: 2 }, second);
    expect(cache.get({ repoId: 1, fingerprint: first.fingerprint, evidenceScanRunId: 1 })).toBe(first);
    cache.set({ repoId: 1, fingerprint: third.fingerprint, evidenceScanRunId: 3 }, third);
    expect(cache.size).toBe(2);
    expect(cache.get({ repoId: 1, fingerprint: second.fingerprint, evidenceScanRunId: 2 })).toBeUndefined();
    expect(cache.get({ repoId: 1, fingerprint: first.fingerprint, evidenceScanRunId: 1 })).toBe(first);
  });

  it("treats a completed scan as a new immutable key", () => {
    const cache = new RemediationProjectionCache();
    const oldScan = projection(10);
    cache.set({ repoId: 1, fingerprint: "same-finding", evidenceScanRunId: 10 }, oldScan);
    expect(cache.get({ repoId: 1, fingerprint: "same-finding", evidenceScanRunId: 11 })).toBeUndefined();
    cache.clear();
    expect(cache.size).toBe(0);
  });

  it("deduplicates in-flight projections and releases failures for retry", async () => {
    const cache = new RemediationProjectionCache();
    const key = { repoId: 1, fingerprint: "same-finding", evidenceScanRunId: 10 };
    const value = projection(10);
    let resolve!: (projection: FindingRemediationProjection) => void;
    const loader = vi.fn(() => new Promise<FindingRemediationProjection>((complete) => { resolve = complete; }));
    const first = cache.getOrLoad(key, loader);
    const second = cache.getOrLoad(key, loader);
    await Promise.resolve();
    expect(loader).toHaveBeenCalledTimes(1);
    resolve(value);
    await expect(Promise.all([first, second])).resolves.toEqual([value, value]);
    expect(cache.get(key)).toBe(value);

    const failedKey = { ...key, evidenceScanRunId: 11 };
    const failure = vi.fn().mockRejectedValueOnce(new Error("unavailable")).mockResolvedValueOnce(projection(11));
    await expect(cache.getOrLoad(failedKey, failure)).rejects.toThrow("unavailable");
    await expect(cache.getOrLoad(failedKey, failure)).resolves.toMatchObject({ evidenceScanRunId: 11 });
    expect(failure).toHaveBeenCalledTimes(2);
  });
});

describe("remediation cache invalidation during requests", () => {
  it("does not cache or remove a newer request when an invalidated request finishes", async () => {
    const cache = new RemediationProjectionCache();
    const key = { repoId: 1, fingerprint: "same-finding", evidenceScanRunId: 10 };
    let finishOld!: (value: FindingRemediationProjection) => void;
    let finishNew!: (value: FindingRemediationProjection) => void;
    const old = cache.getOrLoad(key, () => new Promise((resolve) => { finishOld = resolve; }));
    await Promise.resolve();
    cache.clear();
    const fresh = cache.getOrLoad(key, () => new Promise((resolve) => { finishNew = resolve; }));
    await Promise.resolve();
    finishOld(projection(10));
    await old;
    expect(cache.get(key)).toBeUndefined();
    expect(cache.getOrLoad(key, vi.fn())).toBe(fresh);
    const updated = { ...projection(10), verificationState: "not_detected" as const, verificationScanRunId: 11 };
    finishNew(updated);
    await fresh;
    expect(cache.get(key)).toBe(updated);
  });
});

it("invalidates only the repository whose evidence changed",()=>{
 const cache=new RemediationProjectionCache();const one={repoId:1,fingerprint:"one",evidenceScanRunId:1};const two={repoId:2,fingerprint:"two",evidenceScanRunId:1};const value=projection(1);
 cache.set(one,value);cache.set(two,{...value,repoId:2});cache.invalidateRepository(1);expect(cache.get(one)).toBeUndefined();expect(cache.get(two)?.repoId).toBe(2);
});
