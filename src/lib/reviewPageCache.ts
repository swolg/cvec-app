import type { ReviewPage, ReviewQuery } from "./types";

export const REVIEW_PAGE_CACHE_MAX_ENTRIES = 12;
export const REVIEW_PAGE_CACHE_TTL_MS = 5 * 60 * 1000;

type ReviewPageCacheEntry = {
  page: ReviewPage;
  bytes: number;
  lastAccessedAt: number;
};

export class ReviewPageCache {
  private readonly entries = new Map<string, ReviewPageCacheEntry>();

  constructor(
    private readonly maxEntries = REVIEW_PAGE_CACHE_MAX_ENTRIES,
    private readonly ttlMs = REVIEW_PAGE_CACHE_TTL_MS,
    private readonly now: () => number = Date.now,
    private readonly maxBytes = 8 * 1024 * 1024,
  ) {}

  get(query: ReviewQuery): ReviewPage | undefined {
    const now = this.now();
    this.pruneExpired(now);
    const key = reviewQueryCacheKey(query);
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    if(entry.page.validUntil&&Date.parse(entry.page.validUntil)<=now){this.entries.delete(key);return undefined;}
    if(entry.page.items.some((item)=>item.reviewState==="snoozed" && item.snoozeUntil!==null && Date.parse(item.snoozeUntil)<=now)){this.entries.delete(key);return undefined;}

    this.entries.delete(key);
    this.entries.set(key, { ...entry, lastAccessedAt: now });
    return entry.page;
  }

  set(query: ReviewQuery, page: ReviewPage): void {
    const now = this.now();
    this.pruneExpired(now);
    const key = reviewQueryCacheKey(query);
    this.entries.delete(key);
    const bytes=JSON.stringify(page).length*2;
    if(bytes>this.maxBytes)return;
    this.entries.set(key, { page, bytes, lastAccessedAt: now });

    while (this.entries.size > this.maxEntries || [...this.entries.values()].reduce((sum,entry)=>sum+entry.bytes,0)>this.maxBytes) {
      const oldestKey = this.entries.keys().next().value;
      if (oldestKey === undefined) break;
      this.entries.delete(oldestKey);
    }
  }

  delete(query: ReviewQuery): void {
    this.entries.delete(reviewQueryCacheKey(query));
  }

  clear(): void {
    this.entries.clear();
  }

  get size(): number {
    this.pruneExpired(this.now());
    return this.entries.size;
  }

  private pruneExpired(now: number): void {
    for (const [key, entry] of this.entries) {
      if (now - entry.lastAccessedAt >= this.ttlMs) this.entries.delete(key);
    }
  }
}

function reviewQueryCacheKey(query: ReviewQuery): string {
  return JSON.stringify({ ...query, cursor: null });
}

let sessionRevision: number | undefined;
const sessionCache = new ReviewPageCache();
export function reviewSessionCache(revision: number): ReviewPageCache {
  if (sessionRevision !== revision) { sessionCache.clear(); sessionRevision = revision; }
  return sessionCache;
}
