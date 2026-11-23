import { z } from "zod/mini";

import type { ReviewFilters, ReviewQuery, ReviewSort } from "./types";

export const REVIEW_VIEWS_STORAGE_KEY = "cvec.review.views.v2";
export const REVIEW_VIEWS_LEGACY_KEY = "cvec.review.views.v1";
export const REVIEW_VIEWS_ROLLBACK_KEY = "cvec.review.views.v2.rollback";
export type BuiltInReviewViewId = "needs-review" | "all-open" | "critical" | "fix-available" | "snoozed" | "recently-resolved";
export type ReviewFieldId = "severity" | "finding" | "repository" | "package" | "fix" | "reviewState" | "scannerState" | "scannedAt";
export type ReviewDensity = "compact" | "comfortable";
export type ReviewGrouping = "none" | "reviewState" | "severity" | "repository" | "fixAvailability";
export type ReviewPresentationMode = "table" | "board";

export const REVIEW_FIELD_REGISTRY: Readonly<Record<ReviewFieldId, { label: string; required?: boolean; min: number; max: number; defaultWidth: number }>> = {
  severity: { label: "Severity", min: 96, max: 180, defaultWidth: 112 },
  finding: { label: "Finding", required: true, min: 220, max: 680, defaultWidth: 360 },
  repository: { label: "Repository", min: 140, max: 420, defaultWidth: 220 },
  package: { label: "Package", min: 140, max: 420, defaultWidth: 200 },
  fix: { label: "Fix", min: 100, max: 220, defaultWidth: 130 },
  reviewState: { label: "Review state", min: 120, max: 220, defaultWidth: 150 },
  scannerState: { label: "Scanner state", min: 120, max: 220, defaultWidth: 150 },
  scannedAt: { label: "Scanned", min: 120, max: 240, defaultWidth: 160 },
};
export const REVIEW_GROUPING_REGISTRY: Readonly<Record<ReviewGrouping, { label: string; mutable: boolean }>> = {
  none: { label: "No grouping", mutable: false },
  reviewState: { label: "Review state", mutable: true },
  severity: { label: "Severity", mutable: false },
  repository: { label: "Repository", mutable: false },
  fixAvailability: { label: "Fix availability", mutable: false },
};

const reviewSortSchema = z.enum(["priority", "severity", "recent", "repository", "package"] satisfies ReviewSort[]);
const fieldSchema = z.enum(Object.keys(REVIEW_FIELD_REGISTRY) as [ReviewFieldId, ...ReviewFieldId[]]);
const densitySchema = z.enum(["compact", "comfortable"]);
const groupingSchema = z.enum(["none", "reviewState", "severity", "repository", "fixAvailability"]);
const widthsSchema = z._default(z.partialRecord(fieldSchema, z.int().check(z.minimum(80), z.maximum(720))), {});
const tablePresentationSchema = z.object({
  mode: z.literal("table"),
  fields: z.array(fieldSchema).check(z.minLength(1), z.maxLength(8), z.refine(unique, "duplicate fields"), z.refine((fields) => fields.includes("finding"), "finding field is required")),
  widths: widthsSchema,
  density: densitySchema,
  grouping: groupingSchema,
  sorts: z.array(z.object({ field: reviewSortSchema, direction: z.enum(["asc", "desc"]) })).check(z.maxLength(3), z.refine((sorts) => unique(sorts.map((sort) => sort.field)), "duplicate sorts")),
});
const boardPresentationSchema = z.object({
  mode: z.literal("board"),
  grouping: z.enum(["reviewState", "severity", "repository", "fixAvailability"]),
  cardFields: z.array(fieldSchema).check(z.minLength(1), z.maxLength(6), z.refine(unique, "duplicate card fields"), z.refine((fields) => fields.includes("finding"), "finding field is required")),
  density: densitySchema,
  laneOrder: z.array(z.string().check(z.minLength(1), z.maxLength(120))).check(z.maxLength(100), z.refine(unique, "duplicate lanes")),
  collapsedLaneIds: z.array(z.string().check(z.minLength(1), z.maxLength(120))).check(z.maxLength(100), z.refine(unique, "duplicate collapsed lanes")),
});
export const reviewPresentationSchema = z.discriminatedUnion("mode", [tablePresentationSchema, boardPresentationSchema]);
export type ReviewPresentation = z.infer<typeof reviewPresentationSchema>;

const filtersSchema = z.object({
  reviewStates: z.array(z.enum(["unreviewed", "reviewed", "snoozed"])).check(z.maxLength(3)),
  scannerStates: z.array(z.enum(["new", "open", "resolved"])).check(z.maxLength(3)),
  severities: z.array(z.enum(["MEDIUM", "HIGH", "CRITICAL"])).check(z.maxLength(3)),
  repoIds: z.array(z.int().check(z.positive())).check(z.maxLength(200)),
  sourceTypes: z.array(z.enum(["local", "github", "unknown"])).check(z.maxLength(3)),
  fixAvailable: z.nullable(z.boolean()), packages: z.array(z.string().check(z.trim(), z.minLength(1), z.maxLength(200))).check(z.maxLength(200)),
  ecosystems: z.array(z.string().check(z.trim(), z.minLength(1), z.maxLength(200))).check(z.maxLength(100)), knownExploited: z.nullable(z.boolean()),
  ransomwareKnown: z.nullable(z.boolean()), changedSince: z.nullable(z.iso.datetime()), scanRunIds: z.array(z.int().check(z.positive())).check(z.maxLength(200)),
  scanChangeRunIds: z.optional(z.array(z.int().check(z.positive())).check(z.maxLength(20_000))),
  scanResolvedRunIds: z.optional(z.array(z.int().check(z.positive())).check(z.maxLength(20_000))),
  resolvedInScanRunId: z.optional(z.int().check(z.positive())),
});
const querySchema = z.object({ sorts: z.optional(z.array(z.object({field:reviewSortSchema,direction:z.enum(["asc","desc"])})).check(z.maxLength(3))), filters: filtersSchema, search: z.nullable(z.string().check(z.maxLength(200))), sort: reviewSortSchema, cursor: z.null(), pageSize: z.nullable(z.int().check(z.minimum(1), z.maximum(200))) });
export const reviewViewDefinitionV2Schema = z.object({ id: z.string().check(z.regex(/^[a-z0-9][a-z0-9-]{2,79}$/i)), name: z.string().check(z.trim(), z.minLength(1), z.maxLength(80)), query: querySchema, presentation: reviewPresentationSchema, builtIn: z.optional(z.boolean()) });
const reviewViewsStateV2Schema = z.object({ version: z.literal(2), customViews: z.array(reviewViewDefinitionV2Schema).check(z.maxLength(100)), favoriteIds: z.array(z.string()).check(z.maxLength(100)), defaultViewId: z.string() });

export interface ReviewViewDefinition { id: string; name: string; query: ReviewQuery; presentation: ReviewPresentation; builtIn?: boolean }
export interface ReviewViewsState { version: 2; customViews: ReviewViewDefinition[]; favoriteIds: string[]; defaultViewId: string }

export const DEFAULT_TABLE_PRESENTATION: ReviewPresentation = { mode: "table", fields: ["severity", "finding", "repository", "reviewState", "scannerState"], widths: {}, density: "comfortable", grouping: "none", sorts: [{ field: "priority", direction: "desc" }] };
export const DEFAULT_BOARD_PRESENTATION: ReviewPresentation = { mode: "board", grouping: "reviewState", cardFields: ["finding", "severity", "repository", "package", "fix"], density: "comfortable", laneOrder: [], collapsedLaneIds: [] };

export function emptyReviewFilters(): ReviewFilters { return { reviewStates: [], scannerStates: [], severities: [], repoIds: [], sourceTypes: [], fixAvailable: null, packages: [], ecosystems: [], knownExploited: null, ransomwareKnown: null, changedSince: null, scanRunIds: [] }; }
export function reviewQuery(filters: Partial<ReviewFilters> = {}, sort: ReviewSort = "priority"): ReviewQuery { return { filters: { ...emptyReviewFilters(), ...filters }, search: null, sort, cursor: null, pageSize: 100 }; }
export function updateReviewSort(query: ReviewQuery, sorts: NonNullable<ReviewQuery["sorts"]>): ReviewQuery {
  if (!sorts.length) return query;
  const defaultSort = { field: query.sort, direction: query.sort === "repository" || query.sort === "package" ? "asc" : "desc" };
  const current = query.sorts?.length ? query.sorts : [defaultSort];
  return current.length === sorts.length && current.every((sort, index) =>
    sort.field === sorts[index].field && sort.direction === sorts[index].direction
  ) ? query : { ...query, sort: sorts[0].field, sorts, cursor: null };
}

const builtIn = (id: BuiltInReviewViewId, name: string, query: ReviewQuery, presentation: ReviewPresentation = DEFAULT_TABLE_PRESENTATION): ReviewViewDefinition => ({ id, name, query, presentation: structuredClone(presentation), builtIn: true });
export const CRITICAL_TO_REVIEW_VIEW = "attention:critical";
export const OTHER_TO_REVIEW_VIEW = "attention:other";
export const BUILT_IN_REVIEW_VIEWS: readonly ReviewViewDefinition[] = [
  builtIn("needs-review", "Needs review", reviewQuery({ scannerStates: ["new", "open"], reviewStates: ["unreviewed"] })),
  builtIn("all-open", "All open", reviewQuery({ scannerStates: ["new", "open"] }), DEFAULT_BOARD_PRESENTATION),
  builtIn("critical", "Critical", reviewQuery({ scannerStates: ["new", "open"], severities: ["CRITICAL"] })),
  builtIn("fix-available", "Fix available", reviewQuery({ scannerStates: ["new", "open"], fixAvailable: true })),
  builtIn("snoozed", "Snoozed", reviewQuery({ reviewStates: ["snoozed"], scannerStates: ["new", "open"] })),
  builtIn("recently-resolved", "Recently resolved", reviewQuery({ scannerStates: ["resolved"] }, "recent")),
] as const;

export const DASHBOARD_REVIEW_VIEWS: readonly ReviewViewDefinition[] = [
  { id: CRITICAL_TO_REVIEW_VIEW, name: "Critical to review", query: reviewQuery({ scannerStates: ["new", "open"], reviewStates: ["unreviewed"], severities: ["CRITICAL"] }), presentation: DEFAULT_TABLE_PRESENTATION, builtIn: true },
  { id: OTHER_TO_REVIEW_VIEW, name: "Other findings to review", query: reviewQuery({ scannerStates: ["new", "open"], reviewStates: ["unreviewed"], severities: ["HIGH", "MEDIUM"] }), presentation: DEFAULT_TABLE_PRESENTATION, builtIn: true },
];

export function defaultReviewViewsState(): ReviewViewsState { return { version: 2, customViews: [], favoriteIds: [], defaultViewId: "needs-review" }; }

export function loadReviewViews(): { state: ReviewViewsState; warning: string | null; migrated?: boolean } {
  const raw = window.localStorage.getItem(REVIEW_VIEWS_STORAGE_KEY);
  if (raw) return loadRaw(raw, REVIEW_VIEWS_STORAGE_KEY);
  const legacy = window.localStorage.getItem(REVIEW_VIEWS_LEGACY_KEY);
  if (legacy) {
    try { return { state: migrateLegacy(JSON.parse(legacy)), warning: "Saved views are ready to migrate and will be written only when you save.", migrated: true }; }
    catch { return { state: defaultReviewViewsState(), warning: "Legacy views could not be migrated. The stored value was kept for recovery." }; }
  }
  return { state: defaultReviewViewsState(), warning: null };
}

function loadRaw(raw: string, key: string): { state: ReviewViewsState; warning: string | null } {
  try { return { state: parseReviewViews(JSON.parse(raw)), warning: null }; }
  catch {
    const rollback = window.localStorage.getItem(REVIEW_VIEWS_ROLLBACK_KEY);
    if (rollback) try { return { state: parseReviewViews(JSON.parse(rollback)), warning: "The latest saved views were invalid. The previous local version was restored." }; } catch { /* preserve both */ }
    return { state: defaultReviewViewsState(), warning: `Custom views could not be loaded. The stored value at ${key} was kept for recovery.` };
  }
}

export function saveReviewViews(state: ReviewViewsState): void {
  const parsed = parseReviewViews(state);
  const previous = window.localStorage.getItem(REVIEW_VIEWS_STORAGE_KEY);
  if (previous) window.localStorage.setItem(REVIEW_VIEWS_ROLLBACK_KEY, previous);
  window.localStorage.setItem(REVIEW_VIEWS_STORAGE_KEY, JSON.stringify(parsed));
}

export function parseReviewViews(value: unknown): ReviewViewsState {
  if (isLegacy(value)) return migrateLegacy(value);
  const parsed = reviewViewsStateV2Schema.parse(value);
  const ids = new Set(BUILT_IN_REVIEW_VIEWS.map((view) => view.id));
  const names = new Set(BUILT_IN_REVIEW_VIEWS.map((view) => view.name.toLowerCase()));
  const customViews = parsed.customViews.map((view) => {
    if (view.builtIn || ids.has(view.id) || names.has(view.name.toLowerCase())) throw new Error("invalid or duplicate view");
    ids.add(view.id); names.add(view.name.toLowerCase());
    return { ...view, query: normalizeQuery(view.query), presentation: clampPresentation(view.presentation), builtIn: undefined };
  });
  const validIds = new Set([...ids]);
  return { version: 2, customViews, favoriteIds: [...new Set(parsed.favoriteIds.filter((id) => customViews.some((view) => view.id === id)))], defaultViewId: validIds.has(parsed.defaultViewId) ? parsed.defaultViewId : "needs-review" };
}

function migrateLegacy(value: Record<string, unknown>): ReviewViewsState {
  const rawViews = Array.isArray(value.customViews) ? value.customViews : [];
  return parseReviewViews({ version: 2, customViews: rawViews.map((entry) => ({ ...(entry as object), presentation: structuredClone(DEFAULT_TABLE_PRESENTATION), builtIn: undefined })), favoriteIds: Array.isArray(value.favoriteIds) ? value.favoriteIds : [], defaultViewId: typeof value.defaultViewId === "string" ? value.defaultViewId : "needs-review" });
}
function isLegacy(value: unknown): value is Record<string, unknown> { return Boolean(value && typeof value === "object" && ((value as { version?: unknown }).version === 0 || (value as { version?: unknown }).version === 1)); }
function normalizeQuery(query: ReviewQuery): ReviewQuery { return { ...query, filters: { ...emptyReviewFilters(), ...query.filters }, cursor: null, pageSize: Math.min(200, Math.max(1, query.pageSize ?? 100)) }; }
function clampPresentation(presentation: ReviewPresentation): ReviewPresentation {
  if (presentation.mode === "board") return presentation;
  return { ...presentation, widths: Object.fromEntries(Object.entries(presentation.widths).map(([field, width]) => { const limits = REVIEW_FIELD_REGISTRY[field as ReviewFieldId]; return [field, Math.max(limits.min, Math.min(limits.max, width))]; })) };
}
function unique<T>(values: T[]): boolean { return new Set(values).size === values.length; }

export function allReviewViews(state: ReviewViewsState): ReviewViewDefinition[] { return [...BUILT_IN_REVIEW_VIEWS, ...state.customViews]; }
export function createCustomView(state: ReviewViewsState, name: string, query: ReviewQuery, presentation: ReviewPresentation = DEFAULT_TABLE_PRESENTATION): ReviewViewsState {
  const normalized = name.trim();
  if (!normalized) throw new Error("View name is required");
  if (allReviewViews(state).some((view) => view.name.trim().toLowerCase() === normalized.toLowerCase())) throw new Error("A view with this name already exists");
  const id = `view-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  return parseReviewViews({ ...state, customViews: [...state.customViews, { id, name: normalized, query, presentation }] });
}
export function updateCustomView(state: ReviewViewsState, id: string, update: Partial<Pick<ReviewViewDefinition, "name" | "query" | "presentation">>): ReviewViewsState {
  if (!state.customViews.some((view) => view.id === id)) throw new Error("Custom view not found");
  return parseReviewViews({ ...state, customViews: state.customViews.map((view) => view.id === id ? { ...view, ...update, name: update.name?.trim() || view.name } : view) });
}
export function deleteCustomView(state: ReviewViewsState, id: string): ReviewViewsState { return { ...state, customViews: state.customViews.filter((view) => view.id !== id), favoriteIds: state.favoriteIds.filter((value) => value !== id), defaultViewId: state.defaultViewId === id ? "needs-review" : state.defaultViewId }; }
export function toggleFavoriteView(state: ReviewViewsState, id: string): ReviewViewsState { if (!state.customViews.some((view) => view.id === id)) return state; return { ...state, favoriteIds: state.favoriteIds.includes(id) ? state.favoriteIds.filter((value) => value !== id) : [...state.favoriteIds, id] }; }
export function setDefaultView(state: ReviewViewsState, id: string): ReviewViewsState { if (!allReviewViews(state).some((view) => view.id === id)) throw new Error("View not found"); return { ...state, defaultViewId: id }; }
export function reorderFavoriteView(state: ReviewViewsState, id: string, direction: -1 | 1): ReviewViewsState { const index = state.favoriteIds.indexOf(id); if (index < 0) return state; const target = Math.min(state.favoriteIds.length - 1, Math.max(0, index + direction)); if (target === index) return state; const favoriteIds = [...state.favoriteIds]; [favoriteIds[index], favoriteIds[target]] = [favoriteIds[target], favoriteIds[index]]; return { ...state, favoriteIds }; }
