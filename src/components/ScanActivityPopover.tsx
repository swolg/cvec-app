import { ArrowClockwise, CaretDown, CheckCircle, Clock, Info, WarningCircle } from "@phosphor-icons/react";
import { AppIcon } from "./ui/app-icon";
import { Button, IconButton } from "./ui/button";
import { Badge } from "./ui/badge";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "./ui/overlays";
import { incompleteInventoryReason } from "@/lib/format";
import type { ScanActivity } from "@/lib/types";

const stageLabels: Record<ScanActivity["stage"], string> = {
  queued: "Queued",
  preparing_source: "Preparing source",
  inventorying_packages: "Inventorying packages",
  matching_vulnerabilities: "Matching vulnerabilities",
  enriching_advisories: "Enriching advisories",
  saving_results: "Saving results",
  completed: "Completed",
  failed: "Failed",
};

const localResultsSuffix = "; local scanner results remain complete.";

function isAdvisoryNote(message: string) {
  return message.endsWith(localResultsSuffix)
    || /^\d+ additional optional advisory lookups were unavailable\.$/.test(message);
}

function findingChanges(findings: Pick<ScanActivity, "newFindings" | "resolvedFindings" | "worsenedFindings">) {
  return [
    { label: "new", count: findings.newFindings },
    { label: "resolved", count: findings.resolvedFindings },
    { label: "worsened", count: findings.worsenedFindings },
  ].filter(({ count }) => count > 0);
}

export function ScanActivityPopover({ activity, open, onOpenChange, onReview, onResolved, onHistory, onRetry, retryDisabledReason }: {
  activity: ScanActivity | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onReview: (scanRunIds: number[]) => void;
  onResolved: (scanRunIds: number[]) => void;
  onHistory: (repoId: number, scanRunId: number) => void;
  onRetry: (repoId: number) => void;
  retryDisabledReason?: string;
}) {
  if (!activity) return null;
  const running = activity.status === "running";
  const failed = activity.status === "failed" || activity.status === "partial_failure";
  const interrupted = activity.interrupted === true;
  const incomplete = activity.status === "failed" && activity.outcomes.length > 0
    && activity.outcomes.every((outcome) => incompleteInventoryReason(outcome.error));
  const statusLabel = interrupted ? "Interrupted" : incomplete ? "Incomplete" : activity.status === "partial_failure" ? "Completed with failures" : stageLabels[activity.stage];
  const repoName = activity.currentRepoName ?? (activity.scopeRepoId ? activity.outcomes[0]?.repoName : null);
  const processed = activity.completedRepositories + activity.failedRepositories;
  const hasProgress = running && activity.scopeRepoId === null && activity.totalRepositories > 0;
  const finished = !running && !interrupted && activity.finishedAt;
  const timestamp = finished || activity.startedAt;
  const time = new Date(timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const batchResult = activity.failedRepositories && !interrupted
    ? `${activity.completedRepositories} completed · ${activity.failedRepositories} failed`
    : `${activity.totalRepositories} ${activity.totalRepositories === 1 ? "repository" : "repositories"} ${interrupted ? "interrupted" : "scanned"}`;
  const scope = activity.scopeRepoId
    ? repoName ?? "Repository scan"
    : activity.totalRepositories ? batchResult : failed ? "All repositories" : "No repositories scanned";
  const scanRunIds = activity.outcomes.flatMap((outcome) =>
    outcome.succeeded && outcome.scanRunId && (outcome.newFindings || outcome.worsenedFindings) ? [outcome.scanRunId] : []);
  const resolvedRunIds = activity.outcomes.flatMap((outcome) =>
    outcome.succeeded && outcome.scanRunId && outcome.resolvedFindings ? [outcome.scanRunId] : []);
  const singleRun = activity.outcomes.length === 1 ? activity.outcomes[0] : null;
  const singleRunId = singleRun?.scanRunId;
  const changes = findingChanges(activity);
  const showChanges = changes.length > 0 || (!running && activity.completedRepositories > 0);
  const advisoryNotes = activity.warnings.filter(isAdvisoryNote);
  const scanWarnings = activity.warnings.filter((message) => !isAdvisoryNote(message));
  const terminalError = activity.terminalError
    && !activity.outcomes.some((outcome) => outcome.error === activity.terminalError)
    ? activity.terminalError : null;

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger render={<IconButton aria-label={`Scan activity: ${statusLabel}`} variant="ghost" />}>
        <AppIcon aria-hidden className={running ? "size-4 motion-safe:animate-spin" : "size-4"} icon={running ? ArrowClockwise : failed ? WarningCircle : CheckCircle} />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="flex max-h-[min(30rem,var(--available-height,100dvh))] w-[min(25rem,calc(100vw-2rem))] flex-col overflow-hidden p-0 shadow-lg"
      >
        <div className="shrink-0 px-4 py-3.5">
          <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
            <PopoverTitle className="text-sm font-semibold leading-5">Scan activity</PopoverTitle>
            {running ? (
              <time className="text-xs text-ink-subtle" dateTime={timestamp}>Started {time}</time>
            ) : (
              <Badge variant={incomplete ? "warning" : activity.status === "failed" ? "destructive" : failed ? "warning" : "success"}>{statusLabel}</Badge>
            )}
          </div>
          <div className="mt-2 flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-xs text-ink-muted" role="status">
            {running ? (
              <>
                <p className="min-w-0 [overflow-wrap:anywhere]">
                  {repoName ? <><span className="font-medium text-ink">{repoName}</span>{" · "}</> : null}
                  {activity.totalRepositories === 0 ? "Finding repositories" : statusLabel}
                </p>
                {hasProgress ? (
                  <span className="tabular-nums">
                    {processed} of {activity.totalRepositories} {activity.totalRepositories === 1 ? "repository" : "repositories"}
                    {activity.failedRepositories ? ` · ${activity.failedRepositories} failed` : ""}
                  </span>
                ) : null}
              </>
            ) : (
              <>
                <p className="min-w-0 [overflow-wrap:anywhere]">{scope}</p>
                <time className="text-ink-subtle" dateTime={timestamp}>{finished ? "Finished" : "Started"} {time}</time>
              </>
            )}
          </div>
          {hasProgress ? (
            <div
              aria-label="Repositories processed"
              aria-valuemax={activity.totalRepositories}
              aria-valuemin={0}
              aria-valuenow={Math.min(processed, activity.totalRepositories)}
              className="mt-3 h-1 overflow-hidden rounded-full bg-surface-4"
              role="progressbar"
            >
              <div className="h-full rounded-full bg-primary" style={{ width: `${Math.min(processed / activity.totalRepositories, 1) * 100}%` }} />
            </div>
          ) : null}
        </div>

        {showChanges || activity.outcomes.length > 0 || activity.warnings.length > 0 || terminalError ? (
          <div className="min-h-0 space-y-2.5 overflow-y-auto overscroll-contain border-t border-hairline px-4 py-3">
            {terminalError ? <ScanError title="Scan could not finish" detail={terminalError} /> : null}
            {scanWarnings.length ? <ScanNotices messages={scanWarnings} advisory={false} /> : null}
            {showChanges ? (
              <div aria-label="Finding changes" className="flex flex-wrap gap-x-4 gap-y-1 rounded-md bg-surface-2 px-3 py-2 text-xs text-ink-muted" role="group">
                {changes.length ? changes.map(({ label, count }) => (
                  <span key={label}><strong className="font-semibold tabular-nums text-ink">{count}</strong> {label}</span>
                )) : "No changes in findings"}
              </div>
            ) : null}
            {activity.outcomes.length ? (
              <ul aria-label="Repository results" className="divide-y divide-hairline">
                {activity.outcomes.map((outcome) => {
                  const runId = outcome.scanRunId;
                  const outcomeChanges = findingChanges(outcome);
                  return (
                    <li key={`${outcome.repoId}-${runId ?? "failed"}`} className="flex min-h-10 items-start gap-2 py-1">
                      <div className="flex min-w-0 flex-1 flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1.5">
                        <p className="min-w-0 text-sm font-medium leading-5 text-ink [overflow-wrap:anywhere]">{outcome.repoName}</p>
                        {outcome.succeeded ? (
                          <p className="text-xs text-ink-subtle">
                            {outcomeChanges.length
                              ? outcomeChanges.map(({ label, count }) => `${count} ${label}`).join(" · ")
                              : "No changes"}
                          </p>
                        ) : (
                          <div className="basis-full">
                            <ScanError
                              title={interrupted ? "CVEC closed before this scan finished" : "Scan failed"}
                              detail={interrupted ? null : outcome.error}
                            />
                          </div>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        {!outcome.succeeded ? (
                          <Button
                            aria-label={`Retry scan for ${outcome.repoName}`}
                            disabled={running || Boolean(retryDisabledReason)}
                            size="xs"
                            variant="secondary"
                            onClick={() => onRetry(outcome.repoId)}
                          >Retry</Button>
                        ) : null}
                        {(!singleRun || running) && runId ? (
                          <IconButton
                            aria-label={`Scan history for ${outcome.repoName}, scan #${runId}`}
                            title="Open scan history"
                            size="icon-xs"
                            variant="ghost"
                            onClick={() => onHistory(outcome.repoId, runId)}
                          ><AppIcon aria-hidden className="size-4" icon={Clock} /></IconButton>
                        ) : outcome.succeeded ? (
                          <span className="flex size-8 items-center justify-center">
                            <AppIcon aria-hidden className="size-4 text-status-success" icon={CheckCircle} />
                          </span>
                        ) : null}
                      </div>
                    </li>
                  );
                })}
              </ul>
            ) : null}
            {!running && retryDisabledReason && activity.outcomes.some((outcome) => !outcome.succeeded) ? (
              <p className="text-xs text-ink-muted">{retryDisabledReason}</p>
            ) : null}
            {advisoryNotes.length ? (
              <ScanNotices key={activity.id} messages={advisoryNotes} advisory />
            ) : null}
          </div>
        ) : null}

        {!running && (scanRunIds.length > 0 || resolvedRunIds.length > 0 || singleRunId) ? (
          <div className="flex shrink-0 flex-wrap gap-2 border-t border-hairline px-4 py-3">
            {scanRunIds.length ? <Button size="xs" onClick={() => onReview(scanRunIds)}>Review changes</Button> : null}
            {resolvedRunIds.length ? <Button size="xs" variant="secondary" onClick={() => onResolved(resolvedRunIds)}>View resolved findings</Button> : null}
            {singleRun && singleRunId ? <Button size="xs" variant="ghost" onClick={() => onHistory(singleRun.repoId, singleRunId)}>Scan history</Button> : null}
          </div>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

function ScanNotices({ messages, advisory }: { messages: string[]; advisory: boolean }) {
  return (
    <details className="group/notes rounded-md bg-surface-2 text-xs">
      <summary className={`flex cursor-pointer list-none items-center gap-2 rounded-md px-3 py-2 focus-visible:outline-2 focus-visible:outline-primary-focus [&::-webkit-details-marker]:hidden ${advisory ? "text-ink-muted" : "text-status-high"}`}>
        <AppIcon aria-hidden className="size-4" icon={advisory ? Info : WarningCircle} />
        <span className="flex-1">{advisory ? "Advisory lookup notes" : "Scan warnings"}</span>
        <AppIcon aria-hidden className="size-3.5 group-open/notes:rotate-180" icon={CaretDown} />
      </summary>
      <div className="space-y-2 px-3 pb-3 text-ink-muted [overflow-wrap:anywhere]">
        {advisory ? <p>Local scan results are available.</p> : null}
        <ul className="list-disc space-y-1 pl-4">
          {messages.map((message) => (
            <li key={message}>{advisory && message.endsWith(localResultsSuffix) ? message.slice(0, -localResultsSuffix.length) + "." : message}</li>
          ))}
        </ul>
      </div>
    </details>
  );
}

function ScanError({ title, detail }: { title: string; detail: string | null }) {
  const incomplete = incompleteInventoryReason(detail);
  if (incomplete) {
    return <div className="space-y-1 text-xs">
      <p className="font-medium text-status-warning-foreground">Incomplete package inventory</p>
      <p className="text-ink-muted [overflow-wrap:anywhere]">{incomplete}</p>
    </div>;
  }
  return (
    <div className="space-y-1 text-xs">
      <p className="font-medium text-severity-critical">{title}</p>
      {detail ? (
        <details className="text-ink-muted">
          <summary className="w-fit cursor-pointer rounded-sm underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-primary-focus">Technical details</summary>
          <p className="mt-1.5 max-h-28 overflow-y-auto whitespace-pre-wrap [overflow-wrap:anywhere]">{detail}</p>
        </details>
      ) : null}
    </div>
  );
}
