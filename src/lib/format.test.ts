import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { formatRelativeTimestamp } from "./format";

describe("timestamp formatting", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-11T10:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("formats past relative timestamps and missing values", () => {
    expect(formatRelativeTimestamp("2026-08-11T09:30:00Z")).toBe("30m ago");
    expect(formatRelativeTimestamp(null)).toBe("Never");
  });
});
