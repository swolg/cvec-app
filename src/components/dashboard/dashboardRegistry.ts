import {
  Bell,
  ChartLine,
  Crosshair,
  Kanban,
  Pulse,
  ShieldWarning,
  WarningCircle,
} from "@phosphor-icons/react";

import type {
  DashboardLayoutItem,
  DashboardState,
  DashboardWidgetConfig,
  DashboardWidgetDefinition,
  DashboardWidgetKind,
  SummaryMetricKey,
} from "./types";

const BREAKPOINT_COLUMNS = {
  lg: 12,
  md: 10,
  sm: 6,
  xs: 4,
  xxs: 1,
} as const;

const SUMMARY_METRICS: Array<{
  metricKey: SummaryMetricKey;
  title: string;
  id: string;
}> = [
  { id: "metric-repos", metricKey: "repos_watched", title: "Repos watched" },
  { id: "metric-new", metricKey: "new_findings", title: "New findings" },
  { id: "metric-critical", metricKey: "open_critical", title: "Open critical" },
  { id: "metric-high", metricKey: "open_high", title: "Open high" },
];

export const DASHBOARD_WIDGETS: Record<DashboardWidgetKind, DashboardWidgetDefinition> = {
  summary_metric: {
    kind: "summary_metric",
    title: "Summary metric",
    description: "Single workspace KPI tile.",
    icon: Pulse,
    minW: 2,
    minH: 2,
    defaultW: 3,
    defaultH: 2,
    create: () => ({
      id: createWidgetId("metric"),
      kind: "summary_metric",
      metricKey: "repos_watched",
    }),
  },
  workspace_status: {
    kind: "workspace_status",
    title: "Workspace status",
    description: "Scanner health and issue split.",
    icon: ShieldWarning,
    minW: 4,
    minH: 4,
    defaultW: 5,
    defaultH: 4,
    create: () => ({ id: createWidgetId("status"), kind: "workspace_status" }),
  },
  risk_trend: {
    kind: "risk_trend",
    title: "Risk trend",
    description: "Open findings by severity over time.",
    icon: ChartLine,
    minW: 5,
    minH: 5,
    defaultW: 7,
    defaultH: 5,
    create: () => ({ id: createWidgetId("trend"), kind: "risk_trend" }),
  },
  repository_table: {
    kind: "repository_table",
    title: "Repository table",
    description: "Repository inventory sorted by attention.",
    icon: Kanban,
    minW: 6,
    minH: 5,
    defaultW: 12,
    defaultH: 7,
    create: () => ({ id: createWidgetId("repos"), kind: "repository_table" }),
  },
  risk_quadrant: {
    kind: "risk_quadrant",
    title: "Risk quadrant",
    description: "Prioritize severe, locally relevant advisories.",
    icon: Crosshair,
    minW: 5,
    minH: 5,
    defaultW: 6,
    defaultH: 5,
    create: () => ({ id: createWidgetId("quadrant"), kind: "risk_quadrant" }),
  },
  external_risk_metrics: {
    kind: "external_risk_metrics",
    title: "Risk radar metrics",
    description: "New advisories, KEV, ransomware, and local critical signal.",
    icon: Pulse,
    minW: 6,
    minH: 3,
    defaultW: 12,
    defaultH: 3,
    create: () => ({
      id: createWidgetId("radar-metrics"),
      kind: "external_risk_metrics",
    }),
  },
  external_intel_timeline: {
    kind: "external_intel_timeline",
    title: "Intel timeline",
    description: "New advisories and KEV additions over 30 days.",
    icon: ChartLine,
    minW: 5,
    minH: 5,
    defaultW: 6,
    defaultH: 5,
    create: () => ({
      id: createWidgetId("intel-timeline"),
      kind: "external_intel_timeline",
    }),
  },
  ecosystem_severity_heatmap: {
    kind: "ecosystem_severity_heatmap",
    title: "Ecosystem severity heatmap",
    description: "Severity concentration by package ecosystem.",
    icon: Kanban,
    minW: 5,
    minH: 5,
    defaultW: 6,
    defaultH: 5,
    create: () => ({
      id: createWidgetId("ecosystem-heatmap"),
      kind: "ecosystem_severity_heatmap",
    }),
  },
  cvss_epss_quadrant: {
    kind: "cvss_epss_quadrant",
    title: "CVSS x EPSS quadrant",
    description: "Exploit likelihood against impact, sized by local exposure.",
    icon: Crosshair,
    minW: 5,
    minH: 5,
    defaultW: 6,
    defaultH: 5,
    create: () => ({
      id: createWidgetId("cvss-epss"),
      kind: "cvss_epss_quadrant",
    }),
  },
  kev_watch_table: {
    kind: "kev_watch_table",
    title: "KEV watch",
    description: "CISA KEV due dates, ransomware flags, and affected products.",
    icon: ShieldWarning,
    minW: 6,
    minH: 5,
    defaultW: 7,
    defaultH: 5,
    create: () => ({ id: createWidgetId("kev-watch"), kind: "kev_watch_table" }),
  },
  top_cwe_weaknesses: {
    kind: "top_cwe_weaknesses",
    title: "Top CWE weaknesses",
    description: "Most common weakness classes in external intel.",
    icon: WarningCircle,
    minW: 4,
    minH: 5,
    defaultW: 5,
    defaultH: 5,
    create: () => ({
      id: createWidgetId("top-cwe"),
      kind: "top_cwe_weaknesses",
    }),
  },
};

export function createDefaultDashboardState(): DashboardState {
  const widgets: DashboardWidgetConfig[] = [
    ...SUMMARY_METRICS.map((metric) => ({
      id: metric.id,
      kind: "summary_metric" as const,
      title: metric.title,
      metricKey: metric.metricKey,
    })),
    { id: "workspace-status", kind: "workspace_status" },
    { id: "risk-trend", kind: "risk_trend" },
    { id: "repository-table", kind: "repository_table" },
  ];

  const lg: DashboardLayoutItem[] = [
    layout("metric-repos", 0, 0, 3, 2),
    layout("metric-new", 3, 0, 3, 2),
    layout("metric-critical", 6, 0, 3, 2),
    layout("metric-high", 9, 0, 3, 2),
    layout("workspace-status", 0, 2, 5, 4),
    layout("risk-trend", 5, 2, 7, 5),
    layout("repository-table", 0, 7, 12, 7),
  ];

  return focusedDashboardState(widgets, lg);
}

/** Retained only to identify an untouched v2 default during the v3 migration. */
export function createLegacyDefaultDashboardState(): DashboardState {
  const widgets: DashboardWidgetConfig[] = [
    ...SUMMARY_METRICS.map((metric) => ({
      id: metric.id,
      kind: "summary_metric" as const,
      title: metric.title,
      metricKey: metric.metricKey,
    })),
    { id: "workspace-status", kind: "workspace_status" },
    { id: "risk-radar-metrics", kind: "external_risk_metrics" },
    { id: "intel-timeline", kind: "external_intel_timeline" },
    { id: "ecosystem-heatmap", kind: "ecosystem_severity_heatmap" },
    { id: "cvss-epss-quadrant", kind: "cvss_epss_quadrant" },
    { id: "kev-watch", kind: "kev_watch_table" },
    { id: "top-cwe", kind: "top_cwe_weaknesses" },
    { id: "risk-trend", kind: "risk_trend" },
    { id: "repository-table", kind: "repository_table" },
  ];

  const lg: DashboardLayoutItem[] = [
    layout("metric-repos", 0, 0, 3, 2),
    layout("metric-new", 3, 0, 3, 2),
    layout("metric-critical", 6, 0, 3, 2),
    layout("metric-high", 9, 0, 3, 2),
    layout("risk-radar-metrics", 0, 2, 12, 3),
    layout("intel-timeline", 0, 5, 6, 5),
    layout("ecosystem-heatmap", 6, 5, 6, 5),
    layout("cvss-epss-quadrant", 0, 10, 6, 5),
    layout("kev-watch", 6, 10, 6, 5),
    layout("top-cwe", 0, 15, 5, 5),
    layout("workspace-status", 5, 15, 5, 4),
    layout("risk-trend", 0, 19, 7, 5),
    layout("repository-table", 7, 19, 5, 7),
  ];

  return dashboardState(widgets, lg);
}

export function createWidgetLayout(
  widget: DashboardWidgetConfig,
  y: number,
): DashboardLayoutItem {
  const definition = DASHBOARD_WIDGETS[widget.kind];
  return {
    ...layout(
    widget.id,
    0,
    y,
    definition.defaultW,
    definition.defaultH,
    ),
    minW: definition.minW,
    minH: definition.minH,
  };
}

export function getWidgetDefinition(widget: DashboardWidgetConfig) {
  return DASHBOARD_WIDGETS[widget.kind];
}

export function getWidgetTitle(widget: DashboardWidgetConfig): string {
  if (widget.kind === "summary_metric") {
    return SUMMARY_METRICS.find((metric) => metric.metricKey === (widget.metricKey ?? "repos_watched"))?.title ?? "Repos watched";
  }
  return widget.title ?? getWidgetDefinition(widget).title;
}

export function getSummaryMetricIcon(metricKey: SummaryMetricKey | undefined) {
  switch (metricKey) {
    case "new_findings":
      return Bell;
    case "open_critical":
      return WarningCircle;
    case "open_high":
      return ShieldWarning;
    case "repos_watched":
    default:
      return Kanban;
  }
}

function layout(
  i: string,
  x: number,
  y: number,
  w: number,
  h: number,
): DashboardLayoutItem {
  return {
    i,
    x,
    y,
    w,
    h,
    minW: 2,
    minH: 2,
  };
}

function compactForColumns(
  items: readonly DashboardLayoutItem[],
  columns: number,
): DashboardLayoutItem[] {
  let y = 0;

  return items.map((item) => {
    const w = Math.min(item.w, columns);
    const next = {
      ...item,
      x: 0,
      y,
      w,
    };
    y += item.h;
    return next;
  });
}

function dashboardState(
  widgets: DashboardWidgetConfig[],
  lg: DashboardLayoutItem[],
): DashboardState {
  return {
    version: 1,
    widgets,
    layouts: {
      lg,
      md: compactForColumns(lg, BREAKPOINT_COLUMNS.md),
      sm: compactForColumns(lg, BREAKPOINT_COLUMNS.sm),
      xs: compactForColumns(lg, BREAKPOINT_COLUMNS.xs),
      xxs: compactForColumns(lg, BREAKPOINT_COLUMNS.xxs),
    },
  };
}

function focusedDashboardState(
  widgets: DashboardWidgetConfig[],
  lg: DashboardLayoutItem[],
): DashboardState {
  return {
    version: 1,
    widgets,
    layouts: {
      lg,
      md: [
        layout("metric-repos", 0, 0, 5, 2),
        layout("metric-new", 5, 0, 5, 2),
        layout("metric-critical", 0, 2, 5, 2),
        layout("metric-high", 5, 2, 5, 2),
        layout("workspace-status", 0, 4, 4, 5),
        layout("risk-trend", 4, 4, 6, 5),
        layout("repository-table", 0, 9, 10, 7),
      ],
      sm: [
        layout("metric-repos", 0, 0, 3, 2),
        layout("metric-new", 3, 0, 3, 2),
        layout("metric-critical", 0, 2, 3, 2),
        layout("metric-high", 3, 2, 3, 2),
        layout("workspace-status", 0, 4, 6, 5),
        layout("risk-trend", 0, 9, 6, 5),
        layout("repository-table", 0, 14, 6, 7),
      ],
      xs: [
        layout("metric-repos", 0, 0, 2, 2),
        layout("metric-new", 2, 0, 2, 2),
        layout("metric-critical", 0, 2, 2, 2),
        layout("metric-high", 2, 2, 2, 2),
        layout("workspace-status", 0, 4, 4, 5),
        layout("risk-trend", 0, 9, 4, 5),
        layout("repository-table", 0, 14, 4, 7),
      ],
      xxs: compactForColumns(lg, BREAKPOINT_COLUMNS.xxs),
    },
  };
}

function createWidgetId(prefix: string): string {
  const suffix =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `${prefix}-${suffix}`;
}

export function widgetNeedsIntel(kind:DashboardWidgetKind):boolean {
  return ["risk_quadrant","external_risk_metrics","external_intel_timeline","ecosystem_severity_heatmap","cvss_epss_quadrant","kev_watch_table","top_cwe_weaknesses"].includes(kind);
}
