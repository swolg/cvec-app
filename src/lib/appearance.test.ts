import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  APPEARANCE_STORAGE_KEY,
  bootstrapAppearance,
  parseAppearance,
  resolveAppearance,
  setAppearance,
} from "./appearance";

describe("appearance", () => {
  beforeEach(() => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query.includes("dark"),
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }));
  });

  it("parses only supported preferences", () => {
    expect(parseAppearance("dark")).toBe("dark");
    expect(parseAppearance("light")).toBe("light");
    expect(parseAppearance("system")).toBe("system");
    expect(parseAppearance("sepia")).toBe("system");
    expect(parseAppearance(null)).toBe("system");
  });

  it("resolves system and explicit preferences", () => {
    expect(resolveAppearance("system", true)).toBe("dark");
    expect(resolveAppearance("system", false)).toBe("light");
    expect(resolveAppearance("light", true)).toBe("light");
  });

  it("applies and persists an explicit preference", () => {
    bootstrapAppearance();
    setAppearance("light");
    expect(window.localStorage.getItem(APPEARANCE_STORAGE_KEY)).toBe("light");
    expect(document.documentElement.dataset.appearance).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
