import { createContext } from "react";
import type { ExternalIntelDetail, RepoAsset, RepositoryChatScanStatus } from "./types";

class RevisionReadCache<T> {
  private readonly entries = new Map<string, Promise<T>>();

  constructor(private readonly limit: number) {}

  read(key: string, load: () => Promise<T>): Promise<T> {
    const retained = this.entries.get(key);
    if (retained) {
      this.entries.delete(key);
      this.entries.set(key, retained);
      return retained;
    }
    const request = Promise.resolve().then(load);
    this.entries.set(key, request);
    if (this.entries.size > this.limit) this.entries.delete(this.entries.keys().next().value!);

    const discard = () => {
      if (this.entries.get(key) === request) this.entries.delete(key);
    };
    void request.then((value) => { if (value == null) discard(); }, discard);
    return request;
  }
}

export function createEvidenceReads() {
  return {
    intel: new RevisionReadCache<ExternalIntelDetail | null>(16),
    products: new RevisionReadCache<RepoAsset[]>(12),
    chatScan: new RevisionReadCache<RepositoryChatScanStatus>(16),
  };
}

export const EvidenceReadsContext = createContext<ReturnType<typeof createEvidenceReads> | null>(null);
