import { beforeEach, describe, expect, it } from "vitest";

import {
  createDefaultDashboardState,
  createLegacyDefaultDashboardState,
} from "./dashboardRegistry";
import { adjustLayoutItem } from "./DashboardOverview";
import {
  DASHBOARD_ROLLBACK_KEY,
  DASHBOARD_STORAGE_KEY,
  isLegacyDefaultDashboardState,
  loadDashboardState,
  saveDashboardState,
} from "./dashboardStorage";

describe("dashboard v3 local migration", () => {
  beforeEach(() => window.localStorage.clear());

  it("uses the focused seven-widget default", () => {
    const state = createDefaultDashboardState();
    expect(state.widgets).toHaveLength(7);
    expect(state.widgets.map((widget) => widget.kind)).toEqual([
      "summary_metric", "summary_metric", "summary_metric", "summary_metric",
      "workspace_status", "risk_trend", "repository_table",
    ]);
    expect(state.layouts.md.slice(0, 4).map(({ x, y, w }) => ({ x, y, w }))).toEqual([
      { x: 0, y: 0, w: 5 },
      { x: 5, y: 0, w: 5 },
      { x: 0, y: 2, w: 5 },
      { x: 5, y: 2, w: 5 },
    ]);
    expect(state.layouts.xs.every((item) => item.x + item.w <= 4)).toBe(true);
    expect(state.layouts.xxs.every((item) => item.w <= 1)).toBe(true);
  });

  it("recognizes and migrates only the untouched thirteen-widget v2 default", () => {
    const legacy = createLegacyDefaultDashboardState();
    const raw = JSON.stringify(legacy);
    expect(isLegacyDefaultDashboardState(raw)).toBe(true);
    window.localStorage.setItem(DASHBOARD_ROLLBACK_KEY, raw);

    const migrated = loadDashboardState(createDefaultDashboardState());
    expect(migrated.widgets).toHaveLength(7);
    expect(window.localStorage.getItem(DASHBOARD_ROLLBACK_KEY)).toBe(raw);
    expect(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)).not.toBeNull();
  });

  it("preserves a customized v2 layout and its rollback value", () => {
    const custom = createLegacyDefaultDashboardState();
    custom.layouts.lg[0] = { ...custom.layouts.lg[0], x: 2, y: 4 };
    const raw = JSON.stringify(custom);
    window.localStorage.setItem(DASHBOARD_ROLLBACK_KEY, raw);

    const migrated = loadDashboardState(createDefaultDashboardState());
    expect(migrated.widgets).toHaveLength(13);
    expect(migrated.layouts.lg[0]).toMatchObject({ x: 2, y: 4 });
    expect(window.localStorage.getItem(DASHBOARD_ROLLBACK_KEY)).toBe(raw);
  });

  it("sanitizes invalid positions, duplicate widgets, and minimum sizes", () => {
    const state = createDefaultDashboardState();
    const widget = state.widgets[4];
    window.localStorage.setItem(DASHBOARD_STORAGE_KEY, JSON.stringify({
      version: 1,
      widgets: [widget, widget],
      layouts: { lg: [{ i: widget.id, x: -5, y: -4, w: 1, h: 1 }] },
    }));
    const loaded = loadDashboardState(state);
    expect(loaded.widgets).toHaveLength(1);
    expect(loaded.layouts.lg[0]).toMatchObject({ x: 0, y: 0, w: 4, h: 4, minW: 4, minH: 4 });
  });

  it("falls back on invalid v3 and saves only layout storage", () => {
    const fallback = createDefaultDashboardState();
    window.localStorage.setItem(DASHBOARD_STORAGE_KEY, "not-json");
    window.localStorage.setItem("security-data", "untouched");
    expect(loadDashboardState(fallback)).toEqual(fallback);
    saveDashboardState(fallback);
    expect(window.localStorage.getItem("security-data")).toBe("untouched");
    expect(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)).toBe(JSON.stringify(fallback));
  });

  it("supports bounded keyboard movement and resize alternatives", () => {
    const item = { i: "metric-repos", x: 0, y: 0, w: 5, h: 2, minW: 2, minH: 2 };

    expect(adjustLayoutItem(item, "ArrowLeft", false, 10)).toMatchObject({ x: 0, y: 0 });
    expect(adjustLayoutItem(item, "ArrowUp", false, 10)).toMatchObject({ x: 0, y: 0 });
    expect(adjustLayoutItem(item, "ArrowRight", false, 10)).toMatchObject({ x: 1, y: 0 });
    expect(adjustLayoutItem(item, "ArrowDown", false, 10)).toMatchObject({ x: 0, y: 1 });
    expect(adjustLayoutItem(item, "ArrowLeft", true, 10)).toMatchObject({ w: 4, h: 2 });
    expect(adjustLayoutItem({ ...item, w: 2 }, "ArrowLeft", true, 10)).toMatchObject({ w: 2 });
    expect(adjustLayoutItem(item, "ArrowUp", true, 10)).toMatchObject({ w: 5, h: 2 });
    expect(adjustLayoutItem(item, "ArrowDown", true, 10)).toMatchObject({ w: 5, h: 3 });
  });
});
