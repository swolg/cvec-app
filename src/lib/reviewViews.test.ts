import { beforeEach, describe, expect, it } from "vitest";
import { BUILT_IN_REVIEW_VIEWS, REVIEW_VIEWS_LEGACY_KEY, REVIEW_VIEWS_ROLLBACK_KEY, REVIEW_VIEWS_STORAGE_KEY, createCustomView, defaultReviewViewsState, deleteCustomView, loadReviewViews, parseReviewViews, reorderFavoriteView, saveReviewViews, setDefaultView, toggleFavoriteView, updateCustomView } from "./reviewViews";

describe("review views", () => {
  beforeEach(() => window.localStorage.clear());

  it("provides the required immutable built-in views", () => {
    expect(BUILT_IN_REVIEW_VIEWS.map((view) => view.id)).toEqual(["needs-review", "all-open", "critical", "fix-available", "snoozed", "recently-resolved"]);
    expect(BUILT_IN_REVIEW_VIEWS.every((view) => view.builtIn)).toBe(true);
    expect(BUILT_IN_REVIEW_VIEWS[0].presentation).toMatchObject({ mode: "table" });
    expect(BUILT_IN_REVIEW_VIEWS[0].query.filters).toMatchObject({ reviewStates: ["unreviewed"], scannerStates: ["new", "open"] });
    expect(BUILT_IN_REVIEW_VIEWS[1].query.filters).toMatchObject({ reviewStates: [], scannerStates: ["new", "open"] });
    expect(BUILT_IN_REVIEW_VIEWS[1].presentation).toMatchObject({ mode: "board", grouping: "reviewState" });
    expect(BUILT_IN_REVIEW_VIEWS.slice(2).every((view) => view.presentation.mode === "table")).toBe(true);
  });

  it("creates, persists, favorites, and deletes custom views without findings", () => {
    let state = createCustomView(defaultReviewViewsState(), "Critical fixes", BUILT_IN_REVIEW_VIEWS[3].query);
    const custom = state.customViews[0];
    state = toggleFavoriteView(state, custom.id);
    saveReviewViews(state);
    expect(loadReviewViews().state.favoriteIds).toEqual([custom.id]);
    state = deleteCustomView(state, custom.id);
    expect(state.customViews).toHaveLength(0);
    expect(state.favoriteIds).toHaveLength(0);
  });

  it("rejects duplicate case-insensitive names", () => {
    const state = createCustomView(defaultReviewViewsState(), "My View", BUILT_IN_REVIEW_VIEWS[0].query);
    expect(() => createCustomView(state, " my view ", BUILT_IN_REVIEW_VIEWS[1].query)).toThrow(/already exists/i);
  });

  it("keeps malformed state untouched and falls back safely", () => {
    window.localStorage.setItem(REVIEW_VIEWS_STORAGE_KEY, "not-json");
    const result = loadReviewViews();
    expect(result.state.defaultViewId).toBe("needs-review");
    expect(result.warning).toMatch(/could not be loaded/i);
    expect(window.localStorage.getItem(REVIEW_VIEWS_STORAGE_KEY)).toBe("not-json");
  });

  it("repairs a deleted default and filters invalid favorite ids", () => {
    const parsed = parseReviewViews({ ...defaultReviewViewsState(), defaultViewId: "missing", favoriteIds: ["missing"] });
    expect(parsed.defaultViewId).toBe("needs-review");
    expect(parsed.favoriteIds).toEqual([]);
  });

  it("updates custom views, orders favorites, and persists the default", () => {
    let state = createCustomView(defaultReviewViewsState(), "One", BUILT_IN_REVIEW_VIEWS[0].query);
    const first = state.customViews[0].id;
    state = updateCustomView(state, first, { name: "Primary" });
    state = createCustomView(state, "Second", BUILT_IN_REVIEW_VIEWS[1].query);
    const second = state.customViews[1].id;
    state = toggleFavoriteView(toggleFavoriteView(state, first), second);
    state = reorderFavoriteView(state, second, -1);
    state = setDefaultView(state, second);
    expect(state.favoriteIds).toEqual([second, first]);
    expect(state.defaultViewId).toBe(second);
    saveReviewViews(state);
    expect(loadReviewViews().state.defaultViewId).toBe(second);
  });

  it("restores the rollback copy without overwriting malformed data", () => {
    const valid = createCustomView(defaultReviewViewsState(), "Offline", BUILT_IN_REVIEW_VIEWS[1].query);
    window.localStorage.setItem(REVIEW_VIEWS_STORAGE_KEY, "broken");
    window.localStorage.setItem(REVIEW_VIEWS_ROLLBACK_KEY, JSON.stringify(valid));
    const result = loadReviewViews();
    expect(result.state.customViews[0].name).toBe("Offline");
    expect(result.warning).toMatch(/restored/i);
    expect(window.localStorage.getItem(REVIEW_VIEWS_STORAGE_KEY)).toBe("broken");
  });

  it("migrates version zero documents and rejects unsupported filter values", () => {
    const old = createCustomView(defaultReviewViewsState(), "Legacy", BUILT_IN_REVIEW_VIEWS[0].query);
    expect(parseReviewViews({ ...old, version: 0 }).version).toBe(2);
    const invalid = structuredClone(old);
    invalid.customViews[0].query.filters.severities = ["UNSUPPORTED" as never];
    expect(() => parseReviewViews(invalid)).toThrow(/severities|invalid option/i);
  });

  it("migrates v1 from its original key without writing until save", () => {
    const legacy = { version: 1, customViews: [{ id: "legacy-view", name: "Legacy", query: BUILT_IN_REVIEW_VIEWS[0].query }], favoriteIds: ["legacy-view"], defaultViewId: "legacy-view" };
    window.localStorage.setItem(REVIEW_VIEWS_LEGACY_KEY, JSON.stringify(legacy));
    const result = loadReviewViews();
    expect(result.state.customViews[0].presentation.mode).toBe("table");
    expect(result.migrated).toBe(true);
    expect(window.localStorage.getItem(REVIEW_VIEWS_STORAGE_KEY)).toBeNull();
  });

  it("rejects duplicate ids, excessive arrays, invalid enums, and missing primary content", () => {
    const state = createCustomView(defaultReviewViewsState(), "Valid view", BUILT_IN_REVIEW_VIEWS[0].query);
    expect(() => parseReviewViews({ ...state, customViews: [state.customViews[0], { ...state.customViews[0], name: "Other" }] })).toThrow(/duplicate/i);
    const excessive = structuredClone(state); excessive.customViews[0].presentation = { mode: "board", grouping: "severity", density: "compact", cardFields: Array(7).fill("finding"), laneOrder: [], collapsedLaneIds: [] } as never;
    expect(() => parseReviewViews(excessive)).toThrow();
    const invalid = structuredClone(state); invalid.customViews[0].presentation = { ...invalid.customViews[0].presentation, density: "tiny" } as never;
    expect(() => parseReviewViews(invalid)).toThrow();
    const missing = structuredClone(state); missing.customViews[0].presentation = { mode: "table", fields: ["severity"], widths: {}, density: "compact", grouping: "none", sorts: [] };
    expect(() => parseReviewViews(missing)).toThrow(/finding field/i);
  });

  it("clamps field-specific widths and safely strips forward fields", () => {
    const state = createCustomView(defaultReviewViewsState(), "Widths", BUILT_IN_REVIEW_VIEWS[0].query);
    const candidate = structuredClone(state) as typeof state & { future?: string };
    candidate.future = "ignored";
    const view = candidate.customViews[0];
    if (view.presentation.mode !== "table") throw new Error("expected table");
    view.presentation.widths.finding = 700;
    const parsed = parseReviewViews(candidate);
    const presentation = parsed.customViews[0].presentation;
    expect(presentation.mode === "table" && presentation.widths.finding).toBe(680);
    expect("future" in parsed).toBe(false);
  });

  it("keeps trimmed filters, defaults and ISO dates compatible with stored documents", () => {
    const state=createCustomView(defaultReviewViewsState(),"Stored document",BUILT_IN_REVIEW_VIEWS[1].query);
    const view=state.customViews[0];view.query.filters.packages=["  router  "];view.query.filters.ecosystems=[" npm "];view.query.filters.changedSince="2026-08-19T12:00:00Z";
    const raw=JSON.parse(JSON.stringify(state));delete raw.customViews[0].presentation.widths;
    const parsed=parseReviewViews(raw).customViews[0];
    expect(parsed.query.filters.packages).toEqual(["router"]);expect(parsed.query.filters.ecosystems).toEqual(["npm"]);
    expect(parsed.query.filters.changedSince).toBe("2026-08-19T12:00:00Z");
    expect(parsed.presentation.mode==="table"&&parsed.presentation.widths).toEqual({});
  });

  it.each(["blank-package","invalid-date","fractional-repo","infinite-width","unknown-width","duplicate-sort","board-without-grouping"])("rejects malformed stored %s data", (kind) => {
    const state=createCustomView(defaultReviewViewsState(),"Stored document",BUILT_IN_REVIEW_VIEWS[1].query);const view=state.customViews[0];
    if(kind==="blank-package")view.query.filters.packages=["  "];
    if(kind==="invalid-date")view.query.filters.changedSince="2026-99-40";
    if(kind==="fractional-repo")view.query.filters.repoIds=[1.5];
    if(view.presentation.mode!=="table")throw new Error("table fixture expected");
    if(kind==="infinite-width")view.presentation.widths.finding=Infinity;
    if(kind==="unknown-width")Object.assign(view.presentation.widths,{unknownField:100});
    if(kind==="duplicate-sort")view.presentation.sorts=[{field:"severity",direction:"asc"},{field:"severity",direction:"desc"}];
    if(kind==="board-without-grouping")view.presentation={mode:"board",grouping:"none" as never,cardFields:["finding"],density:"compact",laneOrder:[],collapsedLaneIds:[]};
    expect(()=>parseReviewViews(state)).toThrow();
  });
});
