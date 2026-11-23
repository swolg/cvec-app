import { GitBranch, GithubLogo } from "@phosphor-icons/react";

import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  formatRepoProviderLabel,
  formatLastScanStatus,
  formatRelativeTimestamp,
} from "@/lib/format";
import type { RepoSummary } from "@/lib/types";
import { cn } from "@/lib/utils";

interface RepositoryTableProps {
  className?: string;
  repos: RepoSummary[];
  scrollAreaClassName?: string;
  selectedRepoId?: number | null;
  showHeader?: boolean;
  onSelectRepo?: (repoId: number) => void;
}

export function RepositoryTable({
  className,
  repos,
  scrollAreaClassName,
  selectedRepoId = null,
  showHeader = true,
  onSelectRepo,
}: RepositoryTableProps) {
  if (repos.length === 0) {
    return (
      <Card className={cn("app-panel rounded-lg border shadow-none", className)}>
        {showHeader ? (
          <CardHeader className="pb-3">
            <CardTitle className="text-base font-semibold tracking-tight text-ink">
              Repositories
            </CardTitle>
            <CardDescription>
              No repositories found under the current root.
            </CardDescription>
          </CardHeader>
        ) : null}
        <CardContent
          className={`${showHeader ? "pt-0" : "pt-6"} text-xs text-muted-foreground`}
        >
          Try choosing a different folder or check whether your repos contain a
          `.git` directory.
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className={cn("app-panel rounded-lg border shadow-none", className)}>
      {showHeader ? (
        <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle className="text-base font-semibold tracking-tight text-ink">
              Repositories
            </CardTitle>
            <CardDescription className="mt-1">
              Active repositories sorted by current risk and recent scan changes.
            </CardDescription>
          </div>
          <div className="app-count-pill app-count-pill-warning rounded-full border px-2.5 py-0.5 text-xs font-medium">
            {repos.length}
          </div>
        </CardHeader>
      ) : null}

      <CardContent className={cn("min-h-0 flex-1", showHeader ? "pt-0" : "pt-4")}>
        <ScrollArea className={cn("max-h-[34rem] pr-1", scrollAreaClassName)}>
          <div className="divide-y [border-color:var(--hairline)]">
            <div className="hidden bg-control-surface px-4 py-2.5 text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle md:grid md:grid-cols-[minmax(0,1fr)_minmax(9rem,0.34fr)_minmax(8rem,0.25fr)_minmax(7rem,0.2fr)] md:gap-4">
              <span>Repository</span>
              <span>Open</span>
              <span>New</span>
              <span>Last scan</span>
            </div>
            {repos.map((repo) => {
              const providerLabel = formatRepoProviderLabel(repo.provider);
              const secondaryMeta = repo.remoteHost
                ? `${providerLabel} • ${repo.remoteHost}`
                : providerLabel;
              const isSelected = selectedRepoId === repo.id;
              const risk = repoRisk(repo);
              const openTotal = repo.openCritical + repo.openHigh + repo.openMedium;
              const openIssues = issueBreakdown([
                ["Critical", repo.openCritical],
                ["High", repo.openHigh],
                ["Medium", repo.openMedium],
              ]);
              const newTotal = repo.newCritical + repo.newHigh + repo.newMedium;
              const newIssues = issueBreakdown([
                ["Critical", repo.newCritical],
                ["High", repo.newHigh],
                ["Medium", repo.newMedium],
              ]);

              return (
                <Button
                  aria-label={`Open ${repo.name}`}
                  className={`group grid h-auto w-full justify-stretch gap-4 rounded-none border-0 px-4 py-4 text-left hover:bg-row-hover-surface md:grid-cols-[minmax(0,1fr)_minmax(9rem,0.34fr)_minmax(8rem,0.25fr)_minmax(7rem,0.2fr)] md:items-center ${
                    isSelected ? "bg-row-selected-surface" : ""
                  } ${onSelectRepo ? "cursor-pointer" : "cursor-default"}`}
                  key={repo.id}
                  onClick={() => onSelectRepo?.(repo.id)}
                  type="button"
                  variant="ghost"
                >
                  <div className="flex min-w-0 items-start gap-3">
                    <div className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-md border border-hairline bg-control-surface text-ink-subtle">
                      <AppIcon className="size-[18px]" icon={repo.provider === "github" ? GithubLogo : GitBranch} />
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-semibold text-ink">
                        {repo.name}
                      </p>
                      <p className="mt-1 truncate text-xs font-medium text-ink-subtle">
                        {secondaryMeta}
                      </p>
                      <p className="mt-1 truncate text-xs text-ink-subtle">
                        {repo.path}
                      </p>
                    </div>
                  </div>

                  <MetricBlock
                    badge={risk.label}
                    badgeVariant={risk.variant}
                    detail={openTotal > 0 ? openIssues : risk.secondary}
                    label="Open"
                    value={risk.primary}
                  />
                  <MetricBlock
                    detail={newTotal > 0 ? newIssues : "No new findings"}
                    label="New"
                    value={newTotal > 0 ? `+${newTotal}` : "0"}
                    valueClassName={newTotal > 0 ? newTextClass(repo) : "text-ink-subtle"}
                  />
                  <MetricBlock
                    detail={formatLastScanStatus(repo.lastScanStatus)}
                    label="Last scan"
                    value={formatRelativeTimestamp(repo.lastScannedAt)}
                  />
                </Button>
              );
            })}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

function MetricBlock({
  badge,
  badgeVariant,
  detail,
  label,
  value,
  valueClassName = "text-ink",
}: {
  badge?: string;
  badgeVariant?: "success" | "warning" | "destructive" | "outline" | "secondary";
  detail: string;
  label: string;
  value: string;
  valueClassName?: string;
}) {
  return (
    <div className="min-w-0 space-y-1">
      <p className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle md:sr-only">
        {label}
      </p>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <p className={`truncate text-base font-semibold ${valueClassName}`}>
          {value}
        </p>
        {badge ? (
          <Badge className="h-6 shrink-0 rounded-full px-2.5 text-xs" variant={badgeVariant}>
            {badge}
          </Badge>
        ) : null}
      </div>
      <p className="truncate text-xs text-ink-subtle">{detail}</p>
    </div>
  );
}

function repoRisk(repo: RepoSummary): {
  label: string;
  primary: string;
  secondary: string;
  variant: "success" | "warning" | "destructive" | "outline" | "secondary";
} {
  const openTotal = repo.openCritical + repo.openHigh + repo.openMedium;

  if (repo.status === "error") {
    return {
      label: "Scan error",
      primary: "Needs retry",
      secondary: "Last scan failed",
      variant: "destructive",
    };
  }
  if (repo.status === "scanning") {
    return {
      label: "Scanning",
      primary: "In progress",
      secondary: "Scan is running",
      variant: "secondary",
    };
  }
  if (repo.openCritical > 0) {
    return {
      label: "Critical",
      primary: `${openTotal} open`,
      secondary: "Critical issues present",
      variant: "destructive",
    };
  }
  if (repo.openHigh > 0) {
    return {
      label: "High",
      primary: `${openTotal} open`,
      secondary: "High issues present",
      variant: "warning",
    };
  }
  if (repo.openMedium > 0) {
    return {
      label: "Medium",
      primary: `${openTotal} open`,
      secondary: "Medium issues present",
      variant: "secondary",
    };
  }
  if (repo.status === "clean") {
    return {
      label: "Clean",
      primary: "0 open",
      secondary: "No open issues",
      variant: "success",
    };
  }
  return {
    label: "Not scanned",
    primary: "No baseline",
    secondary: "Run a scan",
    variant: "outline",
  };
}

function newTextClass(repo: RepoSummary): string {
  if (repo.newCritical > 0) {
    return "text-[color:var(--status-critical)]";
  }
  if (repo.newHigh > 0) {
    return "text-[color:var(--status-high)]";
  }
  return "text-ink";
}

function issueBreakdown(items: Array<[string, number]>): string {
  return items
    .filter(([, value]) => value > 0)
    .map(([label, value]) => `${value} ${label}`)
    .join(" • ");
}
