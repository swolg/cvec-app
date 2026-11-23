import type {
  DashboardLayoutItem,
  DashboardState,
  DashboardWidgetConfig,
} from "./types";
import {
  createLegacyDefaultDashboardState,
  getWidgetDefinition,
} from "./dashboardRegistry";

export const DASHBOARD_STORAGE_KEY = "cvec.dashboard.layout.v3";
export const DASHBOARD_ROLLBACK_KEY = "cvec.dashboard.layout.v2";
const LEGACY_STORAGE_KEY = "cvec.dashboard.layout.v1";
const BREAKPOINT_COLUMNS: Record<string, number> = { lg: 12, md: 10, sm: 6, xs: 4, xxs: 1 };
const KNOWN_WIDGET_KINDS = new Set([
  "summary_metric",
  "workspace_status",
  "risk_trend",
  "repository_table",
  "risk_quadrant",
  "external_risk_metrics",
  "external_intel_timeline",
  "ecosystem_severity_heatmap",
  "cvss_epss_quadrant",
  "kev_watch_table",
  "top_cwe_weaknesses",
]);

export function loadDashboardState(defaultState: DashboardState): DashboardState {
  if (typeof window === "undefined") return defaultState;

  try {
    const v3 = window.localStorage.getItem(DASHBOARD_STORAGE_KEY);
    if (v3) return parseStoredState(v3, defaultState) ?? defaultState;

    const v2 = window.localStorage.getItem(DASHBOARD_ROLLBACK_KEY);
    if (v2) {
      const parsed = parseStoredState(v2, defaultState);
      if (!parsed) return defaultState;

      const migrated = isLegacyDefaultDashboardState(v2) ? defaultState : parsed;
      saveDashboardState(migrated);
      return migrated;
    }

    const v1 = window.localStorage.getItem(LEGACY_STORAGE_KEY);
    const parsedV1 = v1 ? parseStoredState(v1, defaultState) : null;
    if (parsedV1) saveDashboardState(parsedV1);
    return parsedV1 ?? defaultState;
  } catch {
    return defaultState;
  }
}

export function saveDashboardState(state: DashboardState) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(DASHBOARD_STORAGE_KEY, JSON.stringify(state));
  } catch (error) {
    console.warn("Unable to save dashboard state.", error);
  }
}

function parseStoredState(rawValue: string, defaultState: DashboardState): DashboardState | null {
  const parsed = JSON.parse(rawValue) as Partial<DashboardState>;
  if (parsed.version !== 1) return null;

  const widgets = sanitizeWidgets(parsed.widgets);
  const layouts = sanitizeLayouts(parsed.layouts);
  if (!widgets?.length || Object.keys(layouts).length === 0) return null;

  const widgetsById = new Map(widgets.map((widget) => [widget.id, widget]));
  const filteredLayouts = Object.fromEntries(
    Object.entries({ ...defaultState.layouts, ...layouts }).map(([breakpoint, items]) => [
      breakpoint,
      items.flatMap((item) => {
        const widget = widgetsById.get(item.i);
        if (!widget) return [];
        const definition = getWidgetDefinition(widget);
        const columns = BREAKPOINT_COLUMNS[breakpoint] ?? BREAKPOINT_COLUMNS.lg;
        const minW = Math.min(definition.minW, columns);
        return [{
          ...item,
          x: Math.min(Math.max(0, item.x), Math.max(0, columns - minW)),
          y: Math.max(0, item.y),
          w: Math.min(columns, Math.max(item.w, minW)),
          h: Math.max(item.h, definition.minH),
          minW,
          minH: definition.minH,
        }];
      }),
    ]),
  );

  return { version: 1, widgets, layouts: filteredLayouts };
}

export function isLegacyDefaultDashboardState(rawValue: string): boolean {
  try {
    const candidate = JSON.parse(rawValue) as Partial<DashboardState>;
    const legacy = createLegacyDefaultDashboardState();
    if (candidate.version !== 1 || !Array.isArray(candidate.widgets) || !candidate.layouts) return false;
    return sameWidgets(candidate.widgets, legacy.widgets) && sameLayouts(candidate.layouts, legacy.layouts);
  } catch {
    return false;
  }
}

function sameWidgets(candidate: unknown[], legacy: DashboardWidgetConfig[]) {
  if (candidate.length !== legacy.length) return false;
  return legacy.every((widget, index) => {
    const value = candidate[index] as Partial<DashboardWidgetConfig> | undefined;
    return value?.id === widget.id && value.kind === widget.kind && value.title === widget.title && value.metricKey === widget.metricKey;
  });
}

function sameLayouts(candidate: unknown, legacy: DashboardState["layouts"]) {
  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) return false;
  const record = candidate as Record<string, unknown>;
  return Object.keys(legacy).every((breakpoint) => {
    const items = record[breakpoint];
    const expected = legacy[breakpoint];
    if (!Array.isArray(items) || items.length !== expected.length) return false;
    return expected.every((item, index) => {
      const value = items[index] as Partial<DashboardLayoutItem> | undefined;
      return value?.i === item.i && value.x === item.x && value.y === item.y && value.w === item.w && value.h === item.h;
    });
  });
}

function sanitizeWidgets(value: unknown): DashboardWidgetConfig[] | null {
  if (!Array.isArray(value)) return null;
  const seen = new Set<string>();
  return value.filter((item): item is DashboardWidgetConfig => {
    if (!isWidgetConfig(item) || seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  });
}

function sanitizeLayouts(value: unknown): Record<string, DashboardLayoutItem[]> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).flatMap(([breakpoint, items]) =>
    Array.isArray(items) ? [[breakpoint, items.filter(isLayoutItem)]] : [],
  ));
}

function isWidgetConfig(value: unknown): value is DashboardWidgetConfig {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.id === "string" && typeof candidate.kind === "string" && KNOWN_WIDGET_KINDS.has(candidate.kind);
}

function isLayoutItem(value: unknown): value is DashboardLayoutItem {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.i === "string" && Number.isFinite(candidate.x) && Number.isFinite(candidate.y) && Number.isFinite(candidate.w) && Number.isFinite(candidate.h);
}
