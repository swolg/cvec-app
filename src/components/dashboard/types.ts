import type { AppIconGlyph } from "@/components/ui/app-icon";
import type { RiskTrendSource } from "@/components/charts/RiskTrendChart";
import type {
  AppSnapshot,
  ExternalIntelFeedItem,
  RiskTrendRange,
} from "@/lib/types";

export type DashboardWidgetKind =
  | "summary_metric"
  | "workspace_status"
  | "risk_trend"
  | "repository_table"
  | "risk_quadrant"
  | "external_risk_metrics"
  | "external_intel_timeline"
  | "ecosystem_severity_heatmap"
  | "cvss_epss_quadrant"
  | "kev_watch_table"
  | "top_cwe_weaknesses";

export type SummaryMetricKey =
  | "repos_watched"
  | "new_findings"
  | "open_critical"
  | "open_high";

export interface DashboardWidgetConfig {
  id: string;
  kind: DashboardWidgetKind;
  title?: string;
  metricKey?: SummaryMetricKey;
}

export interface DashboardWidgetDefinition {
  kind: DashboardWidgetKind;
  title: string;
  description: string;
  icon: AppIconGlyph;
  minW: number;
  minH: number;
  defaultW: number;
  defaultH: number;
  create: () => DashboardWidgetConfig;
}

export interface DashboardState {
  version: 1;
  widgets: DashboardWidgetConfig[];
  layouts: Record<string, DashboardLayoutItem[]>;
}

export interface DashboardLayoutItem {
  i: string;
  x: number;
  y: number;
  w: number;
  h: number;
  minW?: number;
  minH?: number;
}

export interface DashboardDataContext {
  snapshot: AppSnapshot;
  riskTrend: RiskTrendSource;
  riskTrendRange: RiskTrendRange;
  externalIntelItems: ExternalIntelFeedItem[];
  externalIntelTotal?: number;
  externalIntelLoading?: boolean;
  externalIntelFailed?: boolean;
  onRetryIntel?: () => void;
  isScanning: boolean;
  selectedRepoId: number | null;
  onAddGithubRepository: () => void;
  onChooseFolder: () => void;
  onRangeChange: (range: RiskTrendRange) => void;
  onScanAll: () => void;
  onOpenScannerSettings?: () => void;
  onSelectRepo: (repoId: number) => void;
  onOpenReview: (viewId: string) => void;
  onOpenFailedScans: () => void;
}
