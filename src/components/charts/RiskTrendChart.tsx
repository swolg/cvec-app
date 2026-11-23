import { useEffect, useState } from "react";

import type { RiskTrendPoint, RiskTrendRange } from "@/lib/types";
import {
  useWidgetContentMode,
  type WidgetContentMode,
} from "@/components/dashboard/widgetContentMode";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";

export interface RiskTrendSource {
  inventoryRevision: number;
  day: string;
  hasCompletedScan: boolean;
  read: () => Promise<RiskTrendPoint[]>;
}

interface RiskTrendChartProps {
  source: RiskTrendSource;
  range: RiskTrendRange;
  onRangeChange: (range: RiskTrendRange) => void;
}

type TrendResult = { revision: number; day: string; data: RiskTrendPoint[] | null };
const EMPTY_TREND: RiskTrendPoint[] = [];
type SeriesKey = "critical" | "high" | "medium";

interface SeriesConfig {
  key: SeriesKey;
  label: string;
  color: string;
  softColor: string;
}

const SERIES: readonly SeriesConfig[] = [
  {
    key: "critical",
    label: "Critical",
    color: "var(--status-critical)",
    softColor: "var(--status-critical-soft)",
  },
  {
    key: "high",
    label: "High",
    color: "var(--status-high)",
    softColor: "var(--status-high-soft)",
  },
  {
    key: "medium",
    label: "Medium",
    color: "var(--status-medium)",
    softColor: "var(--status-medium-soft)",
  },
];

const RANGES: readonly RiskTrendRange[] = [7, 14, 30];
const CHART_WIDTH = 720;
const CHART_HEIGHT = 260;
const PLOT = {
  top: 22,
  right: 18,
  bottom: 42,
  left: 52,
};

export function RiskTrendChart({
  source: { inventoryRevision, day, hasCompletedScan, read },
  range,
  onRangeChange,
}: RiskTrendChartProps) {
  const [result, setResult] = useState<TrendResult | null>(null);
  const [attempt, setAttempt] = useState(0);
  const current = hasCompletedScan && result?.revision === inventoryRevision && result.day === day ? result : null;
  const loading = hasCompletedScan && current === null;
  const failed = current?.data === null;
  const data = current?.data?.slice(-range) ?? EMPTY_TREND;

  useEffect(() => {
    if (!hasCompletedScan) return;
    let active = true;
    void read().then(
      (data) => {
        if (active) setResult({ revision: inventoryRevision, day, data });
      },
      () => {
        if (active) setResult({ revision: inventoryRevision, day, data: null });
      },
    );
    return () => { active = false; };
  }, [hasCompletedScan, inventoryRevision, day, attempt, read]);

  const maxValue = Math.max(
    1,
    ...data.flatMap((point) => [point.critical, point.high, point.medium]),
  );
  const roundedMax = Math.max(1, Math.ceil(maxValue / 5) * 5);
  const hasData = data.some(
    (point) => point.critical > 0 || point.high > 0 || point.medium > 0,
  );
  const latest = data[data.length - 1] ?? null;
  const [contentRef, mode] = useWidgetContentMode<HTMLDivElement>();
  const isCompact = mode === "compact";

  return (
    <Card className="app-panel flex h-full min-h-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <CardHeader className="shrink-0 flex flex-col gap-3 pb-3 md:flex-row md:items-start md:justify-between">
        <div className="min-w-0">
          <CardTitle className="text-base font-semibold text-ink">
            Risk trend
          </CardTitle>
          <CardDescription className="mt-1">
            Open local findings by severity over time.
          </CardDescription>
        </div>

        <div className="flex items-center gap-2">
          <div aria-label="Trend range" className="app-surface flex rounded-md border p-1" role="group">
            {RANGES.map((candidate) => (
              <Button
                aria-pressed={candidate === range}
                className={`h-7 rounded-sm px-3 ${
                  candidate === range
                    ? "border border-navigation-active-border bg-navigation-active-surface text-ink"
                    : "text-ink-subtle hover:bg-[color:var(--surface-3)] hover:text-[color:var(--ink)]"
                }`}
                key={candidate}
                onClick={() => onRangeChange(candidate)}
                type="button"
                size="xs"
                variant="ghost"
              >
                {candidate}d
              </Button>
            ))}
          </div>
        </div>
      </CardHeader>

      <CardContent
        className="flex min-h-0 flex-1 flex-col overflow-hidden pt-0"
        ref={contentRef}
      >
        <div
          className={`shrink-0 flex flex-wrap items-center ${
            isCompact ? "mb-2 gap-x-3 gap-y-1.5" : "mb-3 gap-x-4 gap-y-2"
          }`}
        >
          {SERIES.map((series) => (
            <div
              className={`flex min-w-0 items-center gap-2 ${isCompact ? "text-xs" : "text-xs"}`}
              key={series.key}
            >
              <span
                className={`${isCompact ? "size-2" : "size-2.5"} shrink-0 rounded-full`}
                style={{ backgroundColor: series.color }}
              />
              <span className="text-ink-subtle">{series.label}</span>
              <span className="font-semibold text-ink">
                {latest ? latest[series.key] : "—"}
              </span>
            </div>
          ))}
        </div>

        <div className={riskTrendChartFrameClass(mode)}>
          <svg
            aria-label={`Open findings over the last ${range} days`}
            className={riskTrendSvgClass(mode)}
            role="img"
            viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
          >
            <text
              fill="var(--ink-subtle)"
              fontFamily="var(--font-sans)"
              fontSize="11"
              transform={`translate(16 ${CHART_HEIGHT / 2}) rotate(-90)`}
              textAnchor="middle"
            >
              Open findings
            </text>

            {buildTicks(roundedMax).map((tick) => {
              const y = scaleY(tick, roundedMax);

              return (
                <g key={tick}>
                  <line
                    stroke="var(--hairline)"
                    strokeDasharray={tick === 0 ? undefined : "4 8"}
                    strokeOpacity={tick === 0 ? 0.9 : 0.7}
                    x1={PLOT.left}
                    x2={CHART_WIDTH - PLOT.right}
                    y1={y}
                    y2={y}
                  />
                  <text
                    fill="var(--ink-subtle)"
                    fontFamily="var(--font-sans)"
                    fontSize="11"
                    textAnchor="end"
                    x={PLOT.left - 10}
                    y={y + 4}
                  >
                    {tick}
                  </text>
                </g>
              );
            })}

            {SERIES.slice()
              .reverse()
              .map((series) => (
                <path
                  d={buildAreaPath(data, series.key, roundedMax)}
                  fill={series.softColor}
                  key={`${series.key}-area`}
                  opacity="0.5"
                />
              ))}

            {SERIES.map((series) => (
              <path
                d={buildLinePath(data, series.key, roundedMax)}
                fill="none"
                key={series.key}
                stroke={series.color}
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="3"
              />
            ))}

            {xLabels(data).map(({ index, label }) => (
              <text
                fill="var(--ink-subtle)"
                fontFamily="var(--font-sans)"
                fontSize="11"
                key={`${index}-${label}`}
                textAnchor={index === 0 ? "start" : index === data.length - 1 ? "end" : "middle"}
                x={scaleX(index, data.length)}
                y={CHART_HEIGHT - 16}
              >
                {label}
              </text>
            ))}
          </svg>

          {!hasData ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center px-4">
              <div className="rounded-md border bg-[color:var(--surface-1)] px-4 py-2 text-xs font-medium text-ink-subtle">
                <p role="status">
                  {loading ? "Loading risk trend…" : failed ? "Could not load the risk trend." : hasCompletedScan ? "No open findings in this range" : "No completed scans in this range"}
                </p>
                {failed ? (
                  <Button className="pointer-events-auto mt-2" size="sm" variant="secondary" onClick={() => {
                    setResult(null);
                    setAttempt((current) => current + 1);
                  }}>Retry</Button>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>
      </CardContent>
    </Card>
  );
}

function riskTrendChartFrameClass(mode: WidgetContentMode): string {
  const minHeight = mode === "compact" ? "min-h-[8.5rem]" : "min-h-[11rem]";
  const maxHeight = mode === "expanded" ? "max-h-[20rem]" : "";
  return `relative ${minHeight} ${maxHeight} flex-1 overflow-hidden rounded-md border bg-chart-surface [border-color:var(--hairline)]`;
}

function riskTrendSvgClass(mode: WidgetContentMode): string {
  const minHeight = mode === "compact" ? "min-h-[8.5rem]" : "min-h-[11rem]";
  return `block h-full ${minHeight} w-full`;
}

function scaleX(index: number, total: number): number {
  const width = CHART_WIDTH - PLOT.left - PLOT.right;

  if (total <= 1) {
    return PLOT.left + width / 2;
  }

  return PLOT.left + (index / (total - 1)) * width;
}

function scaleY(value: number, maxValue: number): number {
  const height = CHART_HEIGHT - PLOT.top - PLOT.bottom;
  return PLOT.top + (1 - value / maxValue) * height;
}

function buildLinePath(
  data: readonly RiskTrendPoint[],
  key: SeriesKey,
  maxValue: number,
): string {
  if (data.length === 0) {
    return "";
  }

  return data
    .map((point, index) => {
      const command = index === 0 ? "M" : "L";
      return `${command} ${scaleX(index, data.length).toFixed(2)} ${scaleY(point[key], maxValue).toFixed(2)}`;
    })
    .join(" ");
}

function buildAreaPath(
  data: readonly RiskTrendPoint[],
  key: SeriesKey,
  maxValue: number,
): string {
  const linePath = buildLinePath(data, key, maxValue);

  if (!linePath) {
    return "";
  }

  const baseline = scaleY(0, maxValue);
  const lastX = scaleX(data.length - 1, data.length);
  const firstX = scaleX(0, data.length);

  return `${linePath} L ${lastX.toFixed(2)} ${baseline.toFixed(2)} L ${firstX.toFixed(2)} ${baseline.toFixed(2)} Z`;
}

function buildTicks(maxValue: number): number[] {
  const midpoint = Math.round(maxValue / 2);
  return [0, midpoint, maxValue];
}

function xLabels(data: readonly RiskTrendPoint[]) {
  if (data.length === 0) {
    return [];
  }

  const indexes = Array.from(
    new Set([0, Math.floor((data.length - 1) / 2), data.length - 1]),
  );

  return indexes.map((index) => ({
    index,
    label: formatDateLabel(data[index].date),
  }));
}

function formatDateLabel(value: string): string {
  const date = new Date(`${value}T00:00:00`);

  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}
