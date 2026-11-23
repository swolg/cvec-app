import { describe, expect, it } from "vitest";

import type { ReviewPage, ReviewQuery } from "./types";
import { ReviewPageCache } from "./reviewPageCache";

const emptyPage: ReviewPage = {
  items: [],
  nextCursor: null,
  total: 0,
  revision: 0,
  groupCounts: { reviewState: [], severity: [], repository: [], fixAvailability: [] },
};

function query(search: string): ReviewQuery {
  return {
    filters: {
      reviewStates: [], scannerStates: [], severities: [], repoIds: [], sourceTypes: [],
      fixAvailable: null, packages: [], ecosystems: [], knownExploited: null,
      ransomwareKnown: null, changedSince: null, scanRunIds: [],
    },
    search,
    sort: "priority",
    cursor: null,
    pageSize: 100,
  };
}

describe("ReviewPageCache", () => {
  it("evicts the least recently used page when its entry limit is reached", () => {
    const cache = new ReviewPageCache(2, 60_000);
    cache.set(query("first"), emptyPage);
    cache.set(query("second"), emptyPage);
    expect(cache.get(query("first"))).toBe(emptyPage);

    cache.set(query("third"), emptyPage);

    expect(cache.get(query("first"))).toBe(emptyPage);
    expect(cache.get(query("second"))).toBeUndefined();
    expect(cache.get(query("third"))).toBe(emptyPage);
    expect(cache.size).toBe(2);
  });

  it("drops inactive entries after the TTL and can be cleared explicitly", () => {
    let now = 0;
    const cache = new ReviewPageCache(12, 1_000, () => now);
    cache.set(query("cached"), emptyPage);
    now = 999;
    expect(cache.get(query("cached"))).toBe(emptyPage);
    now = 1_999;
    expect(cache.get(query("cached"))).toBeUndefined();

    cache.set(query("again"), emptyPage);
    cache.clear();
    expect(cache.size).toBe(0);
  });

  it("normalizes cursors so pagination invalidates the first-page entry", () => {
    const cache = new ReviewPageCache();
    cache.set(query("paged"), emptyPage);

    cache.delete({ ...query("paged"), cursor: "next-page" });

    expect(cache.get(query("paged"))).toBeUndefined();
  });
});

it("expires every view at the next authoritative snooze deadline",()=>{
  let now=100;const cache=new ReviewPageCache(12,60_000,()=>now);
  cache.set(query("unreviewed"),{...emptyPage,validUntil:new Date(150).toISOString()});
  expect(cache.get(query("unreviewed"))).toBeDefined();now=150;expect(cache.get(query("unreviewed"))).toBeUndefined();
});
