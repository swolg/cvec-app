import { Bell, Kanban, ShieldWarning, WarningCircle } from "@phosphor-icons/react";

import { RepositoryTable } from "@/components/RepositoryTable";
import { RiskTrendChart } from "@/components/charts/RiskTrendChart";
import { WorkspaceIssuesDonutChart } from "@/components/charts/WorkspaceIssuesDonutChart";
import { AppIcon, type AppIconGlyph } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/feedback";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  formatExternalSourceLabel,
  formatRelativeTimestamp,
  formatScheduleLabel,
} from "@/lib/format";
import { scannerIssue } from "@/lib/grypeStatus";
import type {
  AppSnapshot,
  ExternalIntelFeedItem,
  ExternalIntelSeverity,
} from "@/lib/types";

import { getSummaryMetricIcon, widgetNeedsIntel } from "./dashboardRegistry";
import type {
  DashboardDataContext,
  DashboardWidgetConfig,
  SummaryMetricKey,
} from "./types";
import {
  useWidgetContentMode,
  type WidgetContentMode,
} from "./widgetContentMode";

interface DashboardWidgetRendererProps {
  context: DashboardDataContext;
  widget: DashboardWidgetConfig;
}

interface RiskPoint {
  id: number;
  label: string;
  risk: number;
  exposure: number;
  severity: ExternalIntelSeverity;
  repoCount: number;
}

type WorkspaceStatusIssue = {
  label: string;
  value: number;
  color: string;
};
type WorkspaceStatusRowData = {
  label: string;
  value: string;
  tone?: "success" | "warning";
};

const WORKSPACE_STATUS_VISUALS: Record<
  Exclude<WidgetContentMode, "compact">,
  { donutSize: number; donutThickness: number }
> = {
  regular: {
    donutSize: 132,
    donutThickness: 16,
  },
  expanded: {
    donutSize: 148,
    donutThickness: 17,
  },
};

export function DashboardWidgetRenderer({
  context,
  widget,
}: DashboardWidgetRendererProps) {
  if (widgetNeedsIntel(widget.kind)) {
    if (context.externalIntelLoading) {
      return <div role="status" className="grid h-full place-content-center p-4 text-sm text-ink-subtle">Loading local intelligence…</div>;
    }
    if (context.externalIntelFailed && context.externalIntelItems.length === 0) {
      return (
        <div className="grid h-full justify-items-center place-content-center gap-2 p-4 text-center text-sm text-ink-subtle">
          <p role="status">Could not load local intelligence.</p>
          <Button size="sm" variant="secondary" onClick={context.onRetryIntel}>Retry</Button>
        </div>
      );
    }
  }
  switch (widget.kind) {
    case "summary_metric":
      return (
        <SummaryMetricWidget
          metricKey={widget.metricKey ?? "repos_watched"}
          snapshot={context.snapshot}
        />
      );
    case "workspace_status":
      return (
        <WorkspaceStatusWidget
          isScanning={context.isScanning}
          overview={context.snapshot.overviewSnapshot}
          status={context.snapshot.workspaceStatus}
          scanIssue={scannerIssue(context.snapshot.workspaceStatus, context.snapshot.versionInfo.grypeDb)}
        />
      );
    case "risk_trend":
      return (
        <RiskTrendChart
          source={context.riskTrend}
          range={context.riskTrendRange}
          onRangeChange={context.onRangeChange}
        />
      );
    case "repository_table":
      return (
        <RepositoryTable
          className="h-full min-h-0"
          onSelectRepo={context.onSelectRepo}
          repos={context.snapshot.repos}
          scrollAreaClassName="h-full max-h-none"
          selectedRepoId={context.selectedRepoId}
          showHeader
        />
      );
    case "risk_quadrant":
      return <RiskQuadrantWidget items={context.externalIntelItems} total={context.externalIntelTotal ?? context.externalIntelItems.length} />;
    case "external_risk_metrics":
      return (
        <ExternalRiskMetricsWidget
          items={context.externalIntelItems}
          lastFetchedAt={context.snapshot.externalIntelSummary.lastFetchedAt}
        />
      );
    case "external_intel_timeline":
      return <ExternalIntelTimelineWidget items={context.externalIntelItems} />;
    case "ecosystem_severity_heatmap":
      return <EcosystemSeverityHeatmapWidget items={context.externalIntelItems} />;
    case "cvss_epss_quadrant":
      return <CvssEpssQuadrantWidget items={context.externalIntelItems} />;
    case "kev_watch_table":
      return <KevWatchTableWidget items={context.externalIntelItems} />;
    case "top_cwe_weaknesses":
      return <TopCweWeaknessesWidget items={context.externalIntelItems} />;
  }
}

function SummaryMetricWidget({
  metricKey,
  snapshot,
}: {
  metricKey: SummaryMetricKey;
  snapshot: AppSnapshot;
}) {
  const overview = snapshot.overviewSnapshot;
  const data = getMetricData(metricKey, overview);
  const icon = getSummaryMetricIcon(metricKey);

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <div className="flex min-w-0 items-center gap-2">
          <div
            className={`flex size-8 shrink-0 items-center justify-center rounded-md border ${data.iconSurfaceClass}`}
          >
            <AppIcon className={`size-4 ${data.iconClass}`} icon={icon} />
          </div>
          <CardTitle className="min-w-0 truncate text-sm text-ink">
            {data.title}
          </CardTitle>
        </div>
      </CardHeader>
      <CardContent className="min-h-0 pt-0">
        <p className="truncate text-2xl font-semibold tracking-tight text-ink">
          {data.value}
        </p>
        <p className="mt-1 truncate text-xs text-ink-subtle">
          {data.description}
        </p>
      </CardContent>
    </Card>
  );
}

function WorkspaceStatusWidget({
  status,
  scanIssue,
  overview,
  isScanning,
}: {
  status: AppSnapshot["workspaceStatus"];
  scanIssue: string | null;
  overview: AppSnapshot["overviewSnapshot"];
  isScanning: boolean;
}) {
  const issueBreakdown = [
    {
      label: "Critical",
      value: overview.openCritical,
      color: "var(--status-critical)",
    },
    {
      label: "High",
      value: overview.openHigh,
      color: "var(--status-high)",
    },
    {
      label: "Medium",
      value: overview.openMedium,
      color: "var(--status-medium)",
    },
  ] as const;
  const totalIssues = issueBreakdown.reduce((sum, item) => sum + item.value, 0);
  const scheduleLabel =
    status.schedule === "off" ? "Off" : formatScheduleLabel(status.schedule);
  const lastScanLabel = formatRelativeTimestamp(status.lastSuccessfulScanAt);
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const statusRows = [
    {
      label: "Scanner",
      value: !status.scannerReady ? "Tools needed" : scanIssue ? "Database needed" : "Ready to scan",
      tone: scanIssue ? "warning" : "success",
    },
    { label: "Schedule", value: scheduleLabel },
    { label: "Last scan", value: isScanning ? "Scanning now" : lastScanLabel },
  ] satisfies WorkspaceStatusRowData[];

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2">
        <CardTitle className="text-base font-semibold text-ink">Workspace status</CardTitle>
      </CardHeader>

      <CardContent
        className="min-h-0 flex-1 overflow-hidden pt-0"
        ref={contentRef}
      >
        <div className={workspaceStatusShellClass(mode)}>
          {mode === "compact" ? (
            <WorkspaceStatusCompact
              issueBreakdown={issueBreakdown}
              rows={statusRows}
              totalIssues={totalIssues}
            />
          ) : (
            <WorkspaceStatusStandard
              issueBreakdown={issueBreakdown}
              rows={statusRows}
              totalIssues={totalIssues}
              visualConfig={WORKSPACE_STATUS_VISUALS[mode]}
            />
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function WorkspaceStatusStandard({
  issueBreakdown,
  rows,
  totalIssues,
  visualConfig,
}: {
  issueBreakdown: readonly WorkspaceStatusIssue[];
  rows: readonly WorkspaceStatusRowData[];
  totalIssues: number;
  visualConfig: { donutSize: number; donutThickness: number };
}) {
  return (
    <>
      <div className="flex min-h-0 flex-col items-center gap-2 overflow-hidden">
        <WorkspaceIssuesDonutChart
          data={issueBreakdown}
          size={visualConfig.donutSize}
          thickness={visualConfig.donutThickness}
          total={totalIssues}
          totalLabel="Total issues"
        />
        <div className="grid w-full gap-1.5 overflow-auto">
          {issueBreakdown.map((item) => (
            <IssueLegendItem
              color={item.color}
              key={item.label}
              label={item.label}
              total={totalIssues}
              value={item.value}
            />
          ))}
        </div>
      </div>

      <div className="grid min-h-0 content-start gap-2 overflow-auto pr-1">
        {rows.map((row) => (
          <StatusRow
            key={row.label}
            label={row.label}
            tone={row.tone}
            value={row.value}
          />
        ))}
      </div>
    </>
  );
}

function workspaceStatusShellClass(mode: WidgetContentMode): string {
  switch (mode) {
    case "compact":
      return "grid h-full min-h-0 content-start overflow-hidden";
    case "expanded":
      return "mx-auto grid max-h-[18rem] min-h-0 w-full max-w-[54rem] grid-cols-[minmax(10rem,15rem)_minmax(18rem,1fr)] content-start gap-5 overflow-hidden";
    case "regular":
    default:
      return "grid h-full min-h-0 grid-cols-[minmax(9rem,13rem)_minmax(14rem,1fr)] content-center gap-4 overflow-hidden";
  }
}

function RiskQuadrantWidget({ items, total }: { items: ExternalIntelFeedItem[]; total: number }) {
  const points = buildRiskPoints(items);
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-base font-semibold text-ink">
              Risk quadrant
            </CardTitle>
            <CardDescription className="mt-1">
              Severity against local exposure.
            </CardDescription>
          </div>
          <Badge className="shrink-0" variant="outline">
            {points.length || total} signals
          </Badge>
        </div>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-hidden pt-0"
        ref={contentRef}
      >
        {points.length > 0 ? (
          <div className={quadrantWidgetShellClass(mode)}>
            <svg
              aria-label="Risk quadrant"
              className={quadrantWidgetSvgClass(mode)}
              role="img"
              viewBox="0 0 520 320"
            >
              <rect
                fill="var(--surface-2)"
                height="320"
                rx="16"
                width="520"
              />
              <line
                stroke="var(--hairline)"
                strokeDasharray="5 7"
                x1="260"
                x2="260"
                y1="32"
                y2="276"
              />
              <line
                stroke="var(--hairline)"
                strokeDasharray="5 7"
                x1="56"
                x2="488"
                y1="154"
                y2="154"
              />
              <text className="dashboard-chart-label" x="58" y="300">
                Lower exposure
              </text>
              <text className="dashboard-chart-label" textAnchor="end" x="486" y="300">
                Higher exposure
              </text>
              <text
                className="dashboard-chart-label"
                textAnchor="middle"
                transform="translate(20 155) rotate(-90)"
              >
                Higher severity
              </text>
              <text className="dashboard-quadrant-kicker" x="302" y="54">
                Act first
              </text>
              {points.map((point) => {
                const x = 56 + point.exposure * 432;
                const y = 276 - point.risk * 244;
                const radius = Math.max(7, Math.min(17, 7 + point.repoCount * 3));

                return (
                  <g key={point.id}>
                    <circle
                      cx={x}
                      cy={y}
                      fill={severityColor(point.severity)}
                      opacity="0.9"
                      r={radius}
                    />
                    <title>{`${point.label}: ${point.severity}, ${point.repoCount} repo matches`}</title>
                  </g>
                );
              })}
            </svg>
            {isCompact ? null : (
              <div className="min-h-0 overflow-auto pr-1">
                {points.slice(0, 5).map((point) => (
                  <div
                    className="border-b py-2 last:border-b-0 [border-color:var(--hairline)]"
                    key={`risk-list-${point.id}`}
                  >
                    <p className="truncate text-xs font-semibold text-ink">
                      {point.label}
                    </p>
                    <p className="mt-1 text-xs text-ink-subtle">
                      {point.severity} · {point.repoCount} repo match
                      {point.repoCount === 1 ? "" : "es"}
                    </p>
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : (
          <EmptyWidget icon={WarningCircle} title="No advisory points yet">
            Refresh external intel to plot severity and local exposure.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function quadrantWidgetShellClass(mode: WidgetContentMode): string {
  if (mode === "compact") {
    return "grid h-full min-h-0 overflow-hidden";
  }

  const maxHeight = mode === "expanded" ? "max-h-[20rem]" : "";
  return `grid h-full min-h-0 ${maxHeight} content-start gap-3 overflow-hidden grid-cols-[minmax(0,1fr)_minmax(8.5rem,10rem)]`;
}

function quadrantWidgetSvgClass(mode: WidgetContentMode): string {
  const minHeight = mode === "compact" ? "min-h-[10rem]" : "min-h-[12rem]";
  return `h-full ${minHeight} w-full`;
}

function ExternalRiskMetricsWidget({
  items,
  lastFetchedAt,
}: {
  items: ExternalIntelFeedItem[];
  lastFetchedAt: string | null;
}) {
  const activeItems = items.filter(isActiveIntelItem);
  const metrics = buildExternalRiskMetrics(activeItems);
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 flex flex-row items-start justify-between gap-3 pb-3">
        <div className="min-w-0">
          <CardTitle className="text-base font-semibold text-ink">
            CVE risk radar
          </CardTitle>
          <CardDescription className="mt-1">
            External intel signals, weighted toward items that can matter locally.
          </CardDescription>
        </div>
        <Badge className="shrink-0" variant="outline">
          Updated {formatRelativeTimestamp(lastFetchedAt)}
        </Badge>
      </CardHeader>
      <CardContent
        className={`grid min-h-0 flex-1 overflow-auto pt-0 ${
          isCompact
            ? "grid-cols-2 gap-2 xl:grid-cols-4"
            : "gap-3 sm:grid-cols-2 xl:grid-cols-4"
        }`}
        ref={contentRef}
      >
        {metrics.map((metric) => (
          <div
            className={`app-surface flex min-h-0 flex-col justify-between overflow-hidden rounded-md border ${
              isCompact ? "px-3 py-2" : "px-4 py-3"
            }`}
            key={metric.label}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-xs font-medium text-ink-subtle">
                  {metric.label}
                </p>
                <p
                  className={`${isCompact ? "mt-1 text-2xl" : "mt-2 text-3xl"} truncate font-semibold tracking-tight ${metric.valueClass}`}
                >
                  {metric.value}
                </p>
              </div>
              <div
                className={`flex shrink-0 items-center justify-center rounded-md bg-[color:var(--surface-3)] ${
                  isCompact ? "size-8" : "size-9"
                }`}
              >
                <AppIcon
                  className={`${isCompact ? "size-3.5" : "size-4"} ${metric.valueClass}`}
                  icon={metric.icon}
                />
              </div>
            </div>
            <p
              className={`${isCompact ? "mt-2 truncate" : "mt-3"} text-xs text-ink-subtle`}
            >
              {metric.detail}
            </p>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

function ExternalIntelTimelineWidget({ items }: { items: ExternalIntelFeedItem[] }) {
  const rows = buildTimelineRows(items.filter(isActiveIntelItem), 30);
  const maxValue = Math.max(1, ...rows.map((row) => row.newCount + row.kevCount));
  const hasData = rows.some((row) => row.newCount > 0 || row.kevCount > 0);
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const chartWidth = 720;
  const chartHeight = 260;
  const plot = { top: 20, right: 18, bottom: 38, left: 44 };
  const barGap = 4;
  const barWidth = Math.max(
    4,
    (chartWidth - plot.left - plot.right - barGap * (rows.length - 1)) / rows.length,
  );

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-base font-semibold text-ink">
              Intel timeline
            </CardTitle>
            <CardDescription className="mt-1">
              New advisories and KEV entries over the last 30 days.
            </CardDescription>
          </div>
          <div className="flex items-center gap-3 text-xs text-ink-subtle">
            <LegendDot color="var(--primary)" label="Advisories" />
            <LegendDot color="var(--status-critical)" label="KEV" />
          </div>
        </div>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-hidden pt-0"
        ref={contentRef}
      >
        {hasData ? (
          <div className={singleChartFrameClass(mode)}>
            <svg
              aria-label="External intel timeline"
              className={singleChartSvgClass(mode)}
              role="img"
              viewBox={`0 0 ${chartWidth} ${chartHeight}`}
            >
              {buildChartTicks(maxValue).map((tick) => {
                const y = scaleLinear(tick, 0, maxValue, chartHeight - plot.bottom, plot.top);
                return (
                  <g key={tick}>
                    <line
                      stroke="var(--hairline)"
                      strokeDasharray={tick === 0 ? undefined : "4 8"}
                      x1={plot.left}
                      x2={chartWidth - plot.right}
                      y1={y}
                      y2={y}
                    />
                    <text
                      fill="var(--ink-subtle)"
                      fontFamily="var(--font-sans)"
                      fontSize="11"
                      textAnchor="end"
                      x={plot.left - 8}
                      y={y + 4}
                    >
                      {tick}
                    </text>
                  </g>
                );
              })}
              {rows.map((row, index) => {
                const x = plot.left + index * (barWidth + barGap);
                const baseline = chartHeight - plot.bottom;
                const kevHeight = scaleBar(row.kevCount, maxValue, chartHeight, plot);
                const advisoryHeight = scaleBar(row.newCount, maxValue, chartHeight, plot);
                return (
                  <g key={row.date}>
                    <rect
                      fill="var(--primary)"
                      height={advisoryHeight}
                      opacity="0.88"
                      rx="3"
                      width={barWidth}
                      x={x}
                      y={baseline - advisoryHeight}
                    />
                    <rect
                      fill="var(--status-critical)"
                      height={kevHeight}
                      rx="3"
                      width={barWidth}
                      x={x}
                      y={baseline - advisoryHeight - kevHeight}
                    />
                    <title>{`${formatDateKey(row.date)}: ${row.newCount} advisories, ${row.kevCount} KEV`}</title>
                  </g>
                );
              })}
              {timelineLabelIndexes(rows.length).map((index) => (
                <text
                  fill="var(--ink-subtle)"
                  fontFamily="var(--font-sans)"
                  fontSize="11"
                  key={index}
                  textAnchor={index === 0 ? "start" : index === rows.length - 1 ? "end" : "middle"}
                  x={plot.left + index * (barWidth + barGap) + barWidth / 2}
                  y={chartHeight - 14}
                >
                  {formatDateKey(rows[index].date)}
                </text>
              ))}
            </svg>
          </div>
        ) : (
          <EmptyWidget icon={Bell} title="No recent external intel">
            Refresh external sources or widen watched topics to populate the 30-day timeline.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function singleChartFrameClass(mode: WidgetContentMode): string {
  const minHeight = mode === "compact" ? "min-h-[10rem]" : "min-h-[13rem]";
  const maxHeight = mode === "expanded" ? "max-h-[20rem]" : "";
  return `h-full ${minHeight} ${maxHeight} overflow-hidden rounded-md border bg-[color:var(--surface-2)] [border-color:var(--hairline)]`;
}

function singleChartSvgClass(mode: WidgetContentMode): string {
  const minHeight = mode === "compact" ? "min-h-[10rem]" : "min-h-[13rem]";
  return `h-full ${minHeight} w-full`;
}

function EcosystemSeverityHeatmapWidget({ items }: { items: ExternalIntelFeedItem[] }) {
  const rows = buildEcosystemSeverityRows(items.filter(isActiveIntelItem));
  const maxValue = Math.max(1, ...rows.flatMap((row) => SEVERITY_COLUMNS.map((column) => row.counts[column.key])));
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <CardTitle className="text-base font-semibold text-ink">
          Ecosystem severity heatmap
        </CardTitle>
        <CardDescription className="mt-1">
          Severity concentration by package ecosystem.
        </CardDescription>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-hidden pt-0"
        ref={contentRef}
      >
        {rows.length > 0 ? (
          <div
            className={`grid h-full min-h-0 grid-cols-[minmax(6rem,1fr)_repeat(4,minmax(3.5rem,0.7fr))] content-start overflow-auto rounded-md border bg-[color:var(--surface-2)] ${
              isCompact ? "text-xs" : "text-xs"
            } [border-color:var(--hairline)]`}
          >
            <div
              className={`sticky top-0 z-10 border-b bg-[color:var(--surface-2)] font-medium text-ink-subtle [border-color:var(--hairline)] ${
                isCompact ? "px-2 py-1.5" : "px-3 py-2"
              }`}
            >
              Ecosystem
            </div>
            {SEVERITY_COLUMNS.map((column) => (
              <div
                className={`sticky top-0 z-10 border-b bg-[color:var(--surface-2)] text-right font-medium text-ink-subtle [border-color:var(--hairline)] ${
                  isCompact ? "px-2 py-1.5" : "px-3 py-2"
                }`}
                key={column.key}
              >
                {column.label}
              </div>
            ))}
            {rows.map((row) => (
              <div className="contents" key={row.ecosystem}>
                <div
                  className={`border-b font-medium text-ink [border-color:var(--hairline)] ${
                    isCompact ? "px-2 py-1.5" : "px-3 py-2"
                  }`}
                >
                  {row.ecosystem}
                </div>
                {SEVERITY_COLUMNS.map((column) => {
                  const value = row.counts[column.key];
                  return (
                    <div
                      className={`border-b text-right font-semibold text-ink [border-color:var(--hairline)] ${
                        isCompact ? "px-2 py-1.5" : "px-3 py-2"
                      }`}
                      key={`${row.ecosystem}-${column.key}`}
                      style={{
                        backgroundColor: heatmapColor(column.key, value, maxValue),
                      }}
                      title={`${row.ecosystem}: ${value} ${column.label.toLowerCase()} advisories`}
                    >
                      {value}
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        ) : (
          <EmptyWidget icon={Kanban} title="No ecosystem data">
            Advisories with package ecosystem metadata will appear in this heatmap.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function CvssEpssQuadrantWidget({ items }: { items: ExternalIntelFeedItem[] }) {
  const points = buildCvssEpssPoints(items.filter(isActiveIntelItem));
  const skippedCount = items.filter(isActiveIntelItem).length - points.length;
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";
  const chartWidth = 560;
  const chartHeight = 340;
  const plot = { top: 34, right: 28, bottom: 48, left: 58 };

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-base font-semibold text-ink">
              CVSS x EPSS quadrant
            </CardTitle>
            <CardDescription className="mt-1">
              Impact against exploit probability, sized by local exposure.
            </CardDescription>
          </div>
          <Badge className="shrink-0" variant="outline">
            {points.length} scored
          </Badge>
        </div>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-hidden pt-0"
        ref={contentRef}
      >
        {points.length > 0 ? (
          <div className={quadrantWidgetShellClass(mode)}>
            <svg
              aria-label="CVSS and EPSS risk quadrant"
              className={quadrantWidgetSvgClass(mode)}
              role="img"
              viewBox={`0 0 ${chartWidth} ${chartHeight}`}
            >
              <rect fill="var(--surface-2)" height={chartHeight} rx="16" width={chartWidth} />
              <line
                stroke="var(--hairline)"
                strokeDasharray="5 7"
                x1={scaleLinear(7, 0, 10, plot.left, chartWidth - plot.right)}
                x2={scaleLinear(7, 0, 10, plot.left, chartWidth - plot.right)}
                y1={plot.top}
                y2={chartHeight - plot.bottom}
              />
              <line
                stroke="var(--hairline)"
                strokeDasharray="5 7"
                x1={plot.left}
                x2={chartWidth - plot.right}
                y1={scaleLinear(0.5, 0, 1, chartHeight - plot.bottom, plot.top)}
                y2={scaleLinear(0.5, 0, 1, chartHeight - plot.bottom, plot.top)}
              />
              <text className="dashboard-chart-label" x={plot.left} y={chartHeight - 14}>
                CVSS 0
              </text>
              <text className="dashboard-chart-label" textAnchor="end" x={chartWidth - plot.right} y={chartHeight - 14}>
                CVSS 10
              </text>
              <text
                className="dashboard-chart-label"
                textAnchor="middle"
                transform={`translate(18 ${chartHeight / 2}) rotate(-90)`}
              >
                EPSS probability
              </text>
              <text className="dashboard-quadrant-kicker" x={chartWidth - 190} y={56}>
                High exploit / high impact
              </text>
              <text className="dashboard-chart-label" x={plot.left + 8} y={56}>
                High exploit / lower impact
              </text>
              <text className="dashboard-chart-label" x={plot.left + 8} y={chartHeight - 72}>
                Lower exploit
              </text>
              {points.map((point) => {
                const x = scaleLinear(point.cvss, 0, 10, plot.left, chartWidth - plot.right);
                const y = scaleLinear(point.epss, 0, 1, chartHeight - plot.bottom, plot.top);
                return (
                  <g key={point.id}>
                    <circle
                      cx={x}
                      cy={y}
                      fill={pointColor(point)}
                      opacity="0.9"
                      r={Math.min(16, 5 + point.repoCount * 2)}
                    />
                    <title>{`${point.label}: CVSS ${point.cvss}, EPSS ${formatProbability(String(point.epss))}, ${point.repoCount} repo matches`}</title>
                  </g>
                );
              })}
            </svg>
            {isCompact ? null : (
              <div className="min-h-0 overflow-auto pr-1">
                {points
                  .slice()
                  .sort((left, right) => right.cvss * right.epss - left.cvss * left.epss)
                  .slice(0, 5)
                  .map((point) => (
                    <div
                      className="border-b py-2 last:border-b-0 [border-color:var(--hairline)]"
                      key={`scored-${point.id}`}
                    >
                      <p className="truncate text-xs font-semibold text-ink">
                        {point.label}
                      </p>
                      <p className="mt-1 text-xs text-ink-subtle">
                        CVSS {point.cvss} · EPSS {formatProbability(String(point.epss))}
                      </p>
                    </div>
                  ))}
                {skippedCount > 0 ? (
                  <p className="mt-2 text-xs text-ink-subtle">
                    {skippedCount} active items missing CVSS or EPSS.
                  </p>
                ) : null}
              </div>
            )}
          </div>
        ) : (
          <EmptyWidget icon={WarningCircle} title="No scored advisories">
            CVSS and EPSS values are both required for this quadrant.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function KevWatchTableWidget({ items }: { items: ExternalIntelFeedItem[] }) {
  const allRows = items
    .filter(isActiveIntelItem)
    .filter((item) => item.isKev)
    .slice()
    .sort(compareKevItems);
  const rows = allRows.slice(0, 8);
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-base font-semibold text-ink">
              KEV watch
            </CardTitle>
            <CardDescription className="mt-1">
              CISA KEV due dates, ransomware flags, and affected products.
            </CardDescription>
          </div>
          <Badge className="shrink-0" variant="destructive">
            {allRows.length} active
          </Badge>
        </div>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-auto pt-0"
        ref={contentRef}
      >
        {rows.length > 0 ? (
          <div
            className={`min-w-[36rem] rounded-md border bg-[color:var(--surface-2)] ${
              isCompact ? "text-xs" : "text-xs"
            } [border-color:var(--hairline)]`}
          >
            <div
              className={`grid grid-cols-[1.1fr_0.85fr_0.85fr_0.75fr_1.4fr] border-b font-medium text-ink-subtle [border-color:var(--hairline)] ${
                isCompact ? "px-2.5 py-1.5" : "px-3 py-2"
              }`}
            >
              <span>CVE</span>
              <span>Date added</span>
              <span>Due date</span>
              <span>Ransomware</span>
              <span>Product</span>
            </div>
            {rows.map((item) => (
              <div
                className={`grid grid-cols-[1.1fr_0.85fr_0.85fr_0.75fr_1.4fr] items-center gap-2 border-b last:border-b-0 [border-color:var(--hairline)] ${
                  isCompact ? "px-2.5 py-1.5" : "px-3 py-2"
                }`}
                key={item.id}
              >
                <span className="truncate font-semibold text-[color:var(--primary)]">
                  {formatAdvisoryId(item)}
                </span>
                <span className="text-ink-subtle">{formatCompactDate(item.kevDates?.dateAdded)}</span>
                <span className="font-medium text-ink">{formatCompactDate(item.kevDates?.dueDate)}</span>
                <span className={item.isRansomwareKnown ? "text-severity-critical" : "text-status-success"}>
                  {item.isRansomwareKnown ? "Yes" : "No"}
                </span>
                <span className="truncate text-ink-subtle">
                  {formatKevProduct(item)}
                </span>
              </div>
            ))}
          </div>
        ) : (
          <EmptyWidget icon={ShieldWarning} title="No active KEV items">
            CISA KEV advisories will appear here after external intel refresh.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function TopCweWeaknessesWidget({ items }: { items: ExternalIntelFeedItem[] }) {
  const rows = buildTopCweRows(items.filter(isActiveIntelItem)).slice(0, 9);
  const maxValue = Math.max(1, ...rows.map((row) => row.count));
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 pb-2.5">
        <CardTitle className="text-base font-semibold text-ink">
          Top CWE weaknesses
        </CardTitle>
        <CardDescription className="mt-1">
          Common weakness classes across active external intel.
        </CardDescription>
      </CardHeader>
      <CardContent
        className="min-h-0 flex-1 overflow-auto pt-0"
        ref={contentRef}
      >
        {rows.length > 0 ? (
          <div className={isCompact ? "space-y-1.5" : "space-y-2"}>
            {rows.map((row) => (
              <div
                className={`grid grid-cols-[5rem_minmax(0,1fr)_2.5rem] items-center gap-3 ${
                  isCompact ? "text-xs" : "text-xs"
                }`}
                key={row.id}
              >
                <span className="font-semibold text-ink">{row.id}</span>
                <div className="min-w-0">
                  <p className="truncate text-ink-subtle">{row.name || "Unknown weakness"}</p>
                  <div
                    className={`${isCompact ? "mt-0.5 h-1.5" : "mt-1 h-2"} overflow-hidden rounded-full bg-[color:var(--surface-3)]`}
                  >
                    <div
                      className="h-full rounded-full bg-[color:var(--status-high)]"
                      style={{ width: `${Math.max(5, (row.count / maxValue) * 100)}%` }}
                    />
                  </div>
                </div>
                <span className="text-right font-semibold text-ink">{row.count}</span>
              </div>
            ))}
          </div>
        ) : (
          <EmptyWidget icon={WarningCircle} title="No CWE data">
            Advisories with CWE metadata will appear in this ranked list.
          </EmptyWidget>
        )}
      </CardContent>
    </Card>
  );
}

function WorkspaceStatusCompact({
  issueBreakdown,
  rows,
  totalIssues,
}: {
  issueBreakdown: readonly WorkspaceStatusIssue[];
  rows: readonly WorkspaceStatusRowData[];
  totalIssues: number;
}) {
  return (
    <div className="grid h-full min-h-0 content-start gap-3 overflow-hidden">
      <div className="grid gap-2">
        <div className="flex items-end justify-between gap-3">
          <div className="min-w-0">
            <p className="text-2xl font-semibold leading-none text-ink">
              {totalIssues}
            </p>
            <p className="mt-1 text-xs text-ink-subtle">Total issues</p>
          </div>
          <div className="grid min-w-[9rem] gap-1.5">
            {issueBreakdown.map((item) => (
              <IssueLegendItem
                color={item.color}
                key={item.label}
                label={item.label}
                total={totalIssues}
                value={item.value}
              />
            ))}
          </div>
        </div>
        <WorkspaceSeverityBar data={issueBreakdown} total={totalIssues} />
      </div>

      <div className="grid min-h-0 grid-cols-2 gap-2 overflow-auto pr-1">
        {rows.map((row) => (
          <StatusRow
            compact
            key={row.label}
            label={row.label}
            tone={row.tone}
            value={row.value}
          />
        ))}
      </div>
    </div>
  );
}

function WorkspaceSeverityBar({
  data,
  total,
}: {
  data: readonly WorkspaceStatusIssue[];
  total: number;
}) {
  return (
    <div className="flex h-2.5 overflow-hidden rounded-full bg-[color:var(--surface-3)]">
      {data.map((item) => {
        const width = total > 0 ? (item.value / total) * 100 : 0;
        return (
          <span
            aria-label={`${item.label}: ${item.value}`}
            className="h-full"
            key={item.label}
            style={{
              backgroundColor: item.color,
              width: `${width}%`,
            }}
          />
        );
      })}
    </div>
  );
}

function IssueLegendItem({
  color,
  label,
  total,
  value,
}: {
  color: string;
  label: string;
  total: number;
  value: number;
}) {
  const percentage = total > 0 ? Math.round((value / total) * 100) : 0;

  return (
    <div className="flex min-w-0 items-center justify-between gap-3 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <span
          className="size-3 shrink-0 rounded-xs"
          style={{ backgroundColor: color }}
        />
        <span className="truncate text-ink-subtle">{label}</span>
      </div>
      <span className="shrink-0 font-medium text-ink">
        {value} ({percentage}%)
      </span>
    </div>
  );
}

function StatusRow({
  compact = false,
  label,
  value,
  tone,
}: {
  compact?: boolean;
  label: string;
  value: string;
  tone?: "success" | "warning";
}) {
  const toneClass =
    tone === "success"
      ? "text-status-success"
      : tone === "warning"
        ? "text-severity-high"
        : "text-ink";

  return (
    <div
      className={`app-surface flex min-w-0 items-center justify-between gap-3 rounded-md border px-3 text-xs ${
        compact ? "py-2" : "py-2.5"
      }`}
    >
      <span className="min-w-0 truncate text-ink-subtle">{label}</span>
      <span className={`min-w-0 truncate text-right font-medium ${toneClass}`}>
        {value}
      </span>
    </div>
  );
}

function EmptyWidget({
  children,
  icon,
  title,
}: {
  children: string;
  icon: AppIconGlyph;
  title: string;
}) {
  return (
    <EmptyState className="h-full min-h-[12rem] border-0 bg-transparent" icon={icon} title={title} description={children} />
  );
}

function getMetricData(
  metricKey: SummaryMetricKey,
  overview: AppSnapshot["overviewSnapshot"],
) {
  switch (metricKey) {
    case "new_findings":
      return {
        title: "New findings",
        value: String(
          overview.newMedium + overview.newHigh + overview.newCritical,
        ),
        description: "Across the latest repository scans",
        iconClass: "text-ink-subtle",
        iconSurfaceClass: "border-hairline bg-control-surface",
      };
    case "open_critical":
      return {
        title: "Open critical",
        value: String(overview.openCritical),
        description: "Needs immediate attention",
        iconClass: "text-severity-critical",
        iconSurfaceClass:
          "border-severity-critical/20 bg-[color:var(--severity-critical-soft)]",
      };
    case "open_high":
      return {
        title: "Open high",
        value: String(overview.openHigh),
        description: "Active scanner findings",
        iconClass: "text-severity-high",
        iconSurfaceClass:
          "border-severity-high/20 bg-[color:var(--severity-high-soft)]",
      };
    case "repos_watched":
    default:
      return {
        title: "Repos watched",
        value: String(overview.reposWatched),
        description:
          overview.reposWithNewFindings > 0
            ? `${overview.reposWithNewFindings} ${overview.reposWithNewFindings === 1 ? "repository" : "repositories"} with new findings`
            : "No repositories with new findings",
        iconClass: "text-ink-subtle",
        iconSurfaceClass: "border-hairline bg-control-surface",
      };
  }
}

function buildRiskPoints(items: ExternalIntelFeedItem[]): RiskPoint[] {
  return items
    .filter((item) => item.withdrawnAt === null)
    .filter((item) =>
      ["critical", "high", "malware"].includes(item.severity) || item.repoCount > 0,
    )
    .slice()
    .sort((left, right) => {
      const rightScore = severityScore(right.severity) + right.repoCount * 0.18;
      const leftScore = severityScore(left.severity) + left.repoCount * 0.18;
      return rightScore - leftScore;
    })
    .slice(0, 18)
    .map((item) => ({
      id: item.id,
      label: item.packageName ?? item.productKeys[0] ?? item.canonicalKey,
      risk: Math.min(1, severityScore(item.severity) / 5),
      exposure: Math.min(1, (item.repoCount + item.sources.length * 0.5) / 6),
      severity: item.severity,
      repoCount: item.repoCount,
    }));
}

function severityScore(severity: ExternalIntelSeverity): number {
  switch (severity) {
    case "malware":
    case "critical":
      return 5;
    case "high":
      return 4;
    case "medium":
      return 3;
    case "low":
      return 2;
    case "unknown":
    default:
      return 1;
  }
}

function severityColor(severity: ExternalIntelSeverity): string {
  switch (severity) {
    case "malware":
    case "critical":
      return "var(--status-critical)";
    case "high":
      return "var(--status-high)";
    case "medium":
      return "var(--status-medium)";
    case "low":
      return "var(--status-low)";
    case "unknown":
    default:
      return "var(--ink-subtle)";
  }
}

type SeverityColumnKey = "critical" | "high" | "medium" | "low";

interface TimelineRow {
  date: string;
  newCount: number;
  kevCount: number;
}

interface EcosystemSeverityRow {
  ecosystem: string;
  counts: Record<SeverityColumnKey, number>;
}

interface CvssEpssPoint {
  id: number;
  label: string;
  cvss: number;
  epss: number;
  repoCount: number;
}

interface CweRow {
  id: string;
  name: string;
  count: number;
}

const SEVERITY_COLUMNS: ReadonlyArray<{
  key: SeverityColumnKey;
  label: string;
}> = [
  { key: "critical", label: "Critical" },
  { key: "high", label: "High" },
  { key: "medium", label: "Medium" },
  { key: "low", label: "Low" },
];

function isActiveIntelItem(item: ExternalIntelFeedItem): boolean {
  return !item.withdrawnAt;
}

function buildExternalRiskMetrics(items: ExternalIntelFeedItem[]) {
  const newAdvisories = countPublishedInWindow(items, 30, 0);
  const previousAdvisories = countPublishedInWindow(items, 60, 30);
  const newKev = countPublishedInWindow(
    items.filter((item) => item.isKev),
    30,
    0,
  );
  const previousKev = countPublishedInWindow(
    items.filter((item) => item.isKev),
    60,
    30,
  );
  const ransomwareKnown = items.filter((item) => item.isRansomwareKnown).length;
  const localCritical = items.filter(
    (item) =>
      item.repoCount > 0 &&
      (item.severity === "critical" || item.severity === "malware"),
  ).length;

  return [
    {
      label: "New advisories",
      value: newAdvisories,
      detail: `${formatDelta(newAdvisories, previousAdvisories)} vs prior 30 days`,
      icon: Bell,
      valueClass: "text-[color:var(--primary)]",
    },
    {
      label: "New KEV",
      value: newKev,
      detail: `${formatDelta(newKev, previousKev)} vs prior 30 days`,
      icon: ShieldWarning,
      valueClass: "text-severity-critical",
    },
    {
      label: "Ransomware-known",
      value: ransomwareKnown,
      detail: "CISA or source data marks campaign use",
      icon: WarningCircle,
      valueClass: "text-severity-high",
    },
    {
      label: "Critical local signal",
      value: localCritical,
      detail: "Critical or malware advisories with repo matches",
      icon: ShieldWarning,
      valueClass: "text-purple-500",
    },
  ];
}

function countPublishedInWindow(
  items: ExternalIntelFeedItem[],
  maxDaysAgo: number,
  minDaysAgo: number,
): number {
  const now = Date.now();
  const maxAge = maxDaysAgo * 24 * 60 * 60 * 1000;
  const minAge = minDaysAgo * 24 * 60 * 60 * 1000;

  return items.filter((item) => {
    const publishedAt = parseDateMs(item.publishedAt);
    if (publishedAt === null) {
      return false;
    }

    const age = now - publishedAt;
    return age >= minAge && age < maxAge;
  }).length;
}

function formatDelta(current: number, previous: number): string {
  if (previous === 0) {
    return current > 0 ? `+${current}` : "0";
  }

  const delta = Math.round(((current - previous) / previous) * 100);
  return `${delta >= 0 ? "+" : ""}${delta}%`;
}

function buildTimelineRows(
  items: ExternalIntelFeedItem[],
  days: number,
): TimelineRow[] {
  const rows = lastDateKeys(days).map((date) => ({
    date,
    newCount: 0,
    kevCount: 0,
  }));
  const rowByDate = new Map(rows.map((row) => [row.date, row]));

  for (const item of items) {
    const date = parseDate(item.publishedAt);
    if (!date) {
      continue;
    }

    const row = rowByDate.get(formatDateForKey(date));
    if (!row) {
      continue;
    }

    if (item.isKev) {
      row.kevCount += 1;
    } else {
      row.newCount += 1;
    }
  }

  return rows;
}

function lastDateKeys(days: number): string[] {
  const today = new Date();
  today.setUTCHours(0, 0, 0, 0);

  return Array.from({ length: days }, (_, index) => {
    const date = new Date(today);
    date.setUTCDate(today.getUTCDate() - (days - 1 - index));
    return formatDateForKey(date);
  });
}

function timelineLabelIndexes(total: number): number[] {
  if (total <= 0) {
    return [];
  }

  return Array.from(new Set([0, Math.floor((total - 1) / 2), total - 1]));
}

function buildEcosystemSeverityRows(
  items: ExternalIntelFeedItem[],
): EcosystemSeverityRow[] {
  const rows = new Map<string, Record<SeverityColumnKey, number>>();

  for (const item of items) {
    if (!item.ecosystem) {
      continue;
    }

    const counts = rows.get(item.ecosystem) ?? {
      critical: 0,
      high: 0,
      medium: 0,
      low: 0,
    };
    counts[severityColumnForItem(item)] += 1;
    rows.set(item.ecosystem, counts);
  }

  return Array.from(rows.entries())
    .map(([ecosystem, counts]) => ({ ecosystem, counts }))
    .sort((left, right) => {
      const rightTotal = severityRowTotal(right);
      const leftTotal = severityRowTotal(left);
      if (rightTotal !== leftTotal) {
        return rightTotal - leftTotal;
      }

      return left.ecosystem.localeCompare(right.ecosystem);
    })
    .slice(0, 8);
}

function severityRowTotal(row: EcosystemSeverityRow): number {
  return SEVERITY_COLUMNS.reduce((sum, column) => sum + row.counts[column.key], 0);
}

function severityColumnForItem(item: ExternalIntelFeedItem): SeverityColumnKey {
  switch (item.severity) {
    case "malware":
    case "critical":
      return "critical";
    case "high":
      return "high";
    case "medium":
      return "medium";
    case "low":
    case "unknown":
    default:
      return "low";
  }
}

function heatmapColor(
  severity: SeverityColumnKey,
  value: number,
  maxValue: number,
): string {
  if (value === 0) {
    return "transparent";
  }

  const opacity = Math.max(0.16, Math.min(0.74, value / maxValue));
  const color = {
    critical: "255, 77, 79",
    high: "255, 138, 31",
    medium: "250, 204, 21",
    low: "56, 189, 248",
  }[severity];

  return `rgba(${color}, ${opacity})`;
}

function buildCvssEpssPoints(items: ExternalIntelFeedItem[]): CvssEpssPoint[] {
  return items
    .map((item) => {
      const cvss = parseScore(item.cvssScore);
      const epss = parseScore(item.epssPercentage);
      if (cvss === null || epss === null) {
        return null;
      }

      return {
        id: item.id,
        label: item.packageName ?? item.productKeys[0] ?? formatAdvisoryId(item),
        cvss: Math.max(0, Math.min(10, cvss)),
        epss: Math.max(0, Math.min(1, epss)),
        repoCount: item.repoCount,
      } satisfies CvssEpssPoint;
    })
    .filter((point): point is CvssEpssPoint => point !== null)
    .sort((left, right) => {
      const rightScore = right.cvss * right.epss + right.repoCount * 0.35;
      const leftScore = left.cvss * left.epss + left.repoCount * 0.35;
      return rightScore - leftScore;
    })
    .slice(0, 80);
}

function pointColor(point: CvssEpssPoint): string {
  if (point.cvss >= 7 && point.epss >= 0.5) {
    return "var(--status-critical)";
  }

  if (point.cvss >= 7) {
    return "var(--status-high)";
  }

  if (point.epss >= 0.5) {
    return "var(--status-success)";
  }

  return "var(--primary)";
}

function compareKevItems(left: ExternalIntelFeedItem, right: ExternalIntelFeedItem): number {
  const leftDue = parseDateMs(left.kevDates?.dueDate ?? null);
  const rightDue = parseDateMs(right.kevDates?.dueDate ?? null);
  if (leftDue !== null && rightDue !== null && leftDue !== rightDue) {
    return leftDue - rightDue;
  }

  if (leftDue !== null) {
    return -1;
  }

  if (rightDue !== null) {
    return 1;
  }

  const leftAdded = parseDateMs(left.kevDates?.dateAdded ?? left.publishedAt);
  const rightAdded = parseDateMs(right.kevDates?.dateAdded ?? right.publishedAt);
  return (rightAdded ?? 0) - (leftAdded ?? 0);
}

function formatKevProduct(item: ExternalIntelFeedItem): string {
  const product = item.productKeys[0] ?? item.packageName ?? item.ecosystem;
  return product ?? formatExternalSourceLabel(item.sources[0] ?? "vendor");
}

function buildTopCweRows(items: ExternalIntelFeedItem[]): CweRow[] {
  const rows = new Map<string, CweRow>();

  for (const item of items) {
    for (const cwe of item.cwes) {
      const existing = rows.get(cwe.cweId) ?? {
        id: cwe.cweId,
        name: cwe.name,
        count: 0,
      };
      existing.count += 1;
      if (!existing.name && cwe.name) {
        existing.name = cwe.name;
      }
      rows.set(cwe.cweId, existing);
    }
  }

  return Array.from(rows.values()).sort((left, right) => {
    if (right.count !== left.count) {
      return right.count - left.count;
    }

    return left.id.localeCompare(right.id);
  });
}

function scaleLinear(
  value: number,
  domainMin: number,
  domainMax: number,
  rangeMin: number,
  rangeMax: number,
): number {
  if (domainMax === domainMin) {
    return rangeMin;
  }

  return rangeMin + ((value - domainMin) / (domainMax - domainMin)) * (rangeMax - rangeMin);
}

function scaleBar(
  value: number,
  maxValue: number,
  chartHeight: number,
  plot: { top: number; bottom: number },
): number {
  const plotHeight = chartHeight - plot.top - plot.bottom;
  return (value / maxValue) * plotHeight;
}

function buildChartTicks(maxValue: number): number[] {
  const midpoint = Math.round(maxValue / 2);
  return [0, midpoint, maxValue];
}

function parseScore(value: string | null): number | null {
  if (!value) {
    return null;
  }

  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function parseDate(value: string | null): Date | null {
  if (!value) {
    return null;
  }

  const dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (dateOnly) {
    const [, year, month, day] = dateOnly;
    return new Date(
      Date.UTC(
        Number.parseInt(year, 10),
        Number.parseInt(month, 10) - 1,
        Number.parseInt(day, 10),
      ),
    );
  }

  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

function parseDateMs(value: string | null): number | null {
  return parseDate(value)?.getTime() ?? null;
}

function formatDateForKey(value: Date): string {
  const year = value.getUTCFullYear();
  const month = String(value.getUTCMonth() + 1).padStart(2, "0");
  const day = String(value.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatDateKey(value: string): string {
  const date = parseDate(value);
  if (!date) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}

function formatCompactDate(value: string | null | undefined): string {
  if (!value) {
    return "n/a";
  }

  const date = parseDate(value);
  if (!date) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}

function formatAdvisoryId(item: ExternalIntelFeedItem): string {
  return (
    item.aliases.find((alias) => alias.toUpperCase().startsWith("CVE-")) ??
    item.canonicalKey
  );
}

function formatProbability(value: string): string {
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed)) {
    return value;
  }

  return `${(parsed * 100).toLocaleString(undefined, {
    maximumFractionDigits: 3,
    minimumFractionDigits: parsed > 0 && parsed < 0.01 ? 3 : 0,
  })}%`;
}

function LegendDot({ color, label }: { color: string; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="size-2 rounded-full" style={{ backgroundColor: color }} />
      <span>{label}</span>
    </span>
  );
}
