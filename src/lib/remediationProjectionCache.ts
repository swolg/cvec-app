import type { FindingRemediationProjection } from "./types";

export interface RemediationProjectionKey {
  repoId: number;
  fingerprint: string;
  evidenceScanRunId: number;
}

export class RemediationProjectionCache {
  readonly #entries = new Map<string, FindingRemediationProjection>();
  readonly #pending = new Map<string, Promise<FindingRemediationProjection>>();

  constructor(readonly limit = 16) {
    if (!Number.isInteger(limit) || limit < 1) throw new Error("Remediation cache limit must be a positive integer");
  }

  get(key: RemediationProjectionKey): FindingRemediationProjection | undefined {
    const normalized = cacheKey(key);
    const value = this.#entries.get(normalized);
    if (!value) return undefined;
    this.#entries.delete(normalized);
    this.#entries.set(normalized, value);
    return value;
  }

  set(key: RemediationProjectionKey, value: FindingRemediationProjection): void {
    const normalized = cacheKey(key);
    this.#entries.delete(normalized);
    this.#entries.set(normalized, value);
    while (this.#entries.size > this.limit) this.#entries.delete(this.#entries.keys().next().value!);
  }

  getOrLoad(key: RemediationProjectionKey, loader: () => Promise<FindingRemediationProjection>): Promise<FindingRemediationProjection> {
    const cached = this.get(key);
    if (cached) return Promise.resolve(cached);
    const normalized = cacheKey(key);
    const pending = this.#pending.get(normalized);
    if (pending) return pending;
    const request = Promise.resolve()
      .then(loader)
      .then((value) => {
        if (this.#pending.get(normalized) === request) this.set(key, value);
        return value;
      })
      .finally(() => {
        if (this.#pending.get(normalized) === request) this.#pending.delete(normalized);
      });
    this.#pending.set(normalized, request);
    return request;
  }

  invalidateRepository(repoId:number):void {
    const prefix=`${repoId}:`;
    for(const key of this.#entries.keys())if(key.startsWith(prefix))this.#entries.delete(key);
    for(const key of this.#pending.keys())if(key.startsWith(prefix))this.#pending.delete(key);
  }

  clear(): void {
    this.#entries.clear();
    this.#pending.clear();
  }

  get size(): number {
    return this.#entries.size;
  }
}

export const remediationProjectionCache = new RemediationProjectionCache();

function cacheKey({ repoId, fingerprint, evidenceScanRunId }: RemediationProjectionKey): string {
  return `${repoId}:${evidenceScanRunId}:${fingerprint}`;
}
