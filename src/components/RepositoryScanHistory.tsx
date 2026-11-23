import { useEffect, useRef, useState } from "react";
import { Clock } from "@phosphor-icons/react";

import { getRepoScanComparison } from "@/lib/api";
import { formatLastScanStatus, formatTimestamp, incompleteInventoryReason } from "@/lib/format";
import type { RepoOverview, RepoScanComparison, RepoScanRun } from "@/lib/types";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { EmptyState } from "./ui/feedback";

interface RepositoryScanHistoryProps {
  detail: RepoOverview;
  selectedRunId?: number;
  onSelectRun: (id: number | undefined) => void;
  onOpenResolved: (runId: number) => void;
}

export function RepositoryScanHistory({ detail, selectedRunId, onSelectRun, onOpenResolved }: RepositoryScanHistoryProps) {
  const repoId = detail.summary.id;
  const selectedIndex = detail.scanHistory.findIndex((run) => run.id === selectedRunId);
  const recentRun = detail.scanHistory[selectedIndex];
  const latestCompleted = detail.overview.latestCompletedScan;
  const recentBaseline = recentRun?.status === "completed"
    ? (recentRun.id === latestCompleted?.id ? detail.previousScan : null)
      ?? detail.scanHistory.slice(selectedIndex + 1).find((run) => run.status === "completed")
    : undefined;
  const needsRead = selectedRunId !== undefined && (
    !recentRun || (recentRun.status === "completed" && !recentBaseline && recentRun.id !== latestCompleted?.id)
  );
  const [lookup, setLookup] = useState<{ repoId: number; runId: number; value: RepoScanComparison | null; failed?: boolean }>();
  const [retry, setRetry] = useState(0);
  const heading = useRef<HTMLHeadingElement>(null);
  const history = useRef<HTMLDivElement>(null);
  const lastFocusedRunId = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (selectedRunId === undefined || !needsRead) return;
    let active = true;
    setLookup(undefined);
    void getRepoScanComparison(repoId, selectedRunId)
      .then((value) => {
        if (active) setLookup({ repoId, runId: selectedRunId, value });
      })
      .catch(() => {
        if (active) setLookup({ repoId, runId: selectedRunId, value: null, failed: true });
      });
    return () => { active = false; };
  }, [repoId, selectedRunId, needsRead, retry]);

  const saved = lookup?.repoId === repoId && lookup.runId === selectedRunId ? lookup : undefined;
  const selectedRun = recentRun ?? saved?.value?.run;
  const baselineId = recentBaseline?.id ?? saved?.value?.previousRunId;
  const baselineCount = recentBaseline ? recentBaseline.packageCount : saved?.value?.previousPackageCount;
  const packageDifference = selectedRun?.status === "completed" && selectedRun.packageCount != null && baselineCount != null
    ? selectedRun.packageCount - baselineCount
    : null;
  const selectionReady = Boolean(selectedRun || saved);
  const compareMessage = needsRead && !saved ? "Loading the comparison scan…"
    : saved?.failed ? "The comparison scan could not be loaded."
    : baselineId == null && baselineCount == null ? "First comparable scan"
    : packageDifference === null ? "Package counts are missing for this comparison."
    : "Package inventory " + (packageDifference >= 0 ? "+" : "−") + Math.abs(packageDifference)
      + " compared with " + (baselineId == null ? "the previous completed scan" : "scan #" + baselineId) + ".";
  useEffect(() => {
    if (selectedRunId !== lastFocusedRunId.current && heading.current && selectionReady) {
      heading.current.focus();
      lastFocusedRunId.current = selectedRunId;
    }
    if (selectedRunId === undefined) lastFocusedRunId.current = undefined;
  }, [selectedRunId, selectionReady]);

  if (!detail.scanHistory.length && selectedRunId === undefined) {
    return <EmptyState icon={Clock} title="No scan attempts" description="Scan history will appear after the first repository scan." />;
  }

  function closeSelection() {
    const root = history.current;
    const row = root?.querySelector<HTMLButtonElement>(`[data-scan-history-run="${selectedRunId}"]`);
    (row ?? root)?.focus();
    onSelectRun(undefined);
  }

  return (
    <div ref={history} role="region" tabIndex={-1} aria-label="Repository scan attempts" className="grid gap-3">
      {selectedRunId !== undefined ? (
        <section className="rounded-lg border border-hairline bg-surface-2 px-4 py-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <h3 ref={heading} tabIndex={-1} className="text-sm font-semibold text-ink">Scan #{selectedRunId}</h3>
              {selectedRun ? (
                <>
                  <p className="mt-1 text-xs text-ink-subtle">
                    {formatLastScanStatus(selectedRun.status, selectedRun.errorMessage)} · {formatTimestamp(selectedRun.finishedAt ?? selectedRun.startedAt)}
                  </p>
                  {selectedRun.status === "completed" ? (
                    <>
                      <div className="mt-1 flex flex-wrap items-center gap-x-3 text-xs text-ink-subtle">
                        <span>{selectedRun.newCritical + selectedRun.newHigh + selectedRun.newMedium} new</span>
                        <ResolutionCount run={selectedRun} onOpenResolved={onOpenResolved} />
                      </div>
                      <p className="mt-2 text-xs text-ink-subtle">{compareMessage}</p>
                    </>
                  ) : null}
                  {selectedRun.sourceRevision ? <p className="mt-2 text-xs text-ink-subtle">Scanned revision {selectedRun.sourceRevision.slice(0, 8)}</p> : null}
                  {selectedRun.errorMessage ? <p className="mt-2 whitespace-pre-wrap text-xs text-severity-critical [overflow-wrap:anywhere]">{selectedRun.errorMessage}</p> : null}
                </>
              ) : (
                <p role="status" className="mt-1 text-xs text-ink-subtle">
                  {saved ? (saved.failed ? "This scan could not be loaded." : "This scan is no longer available for this repository.") : "Loading scan…"}
                </p>
              )}
              {saved?.failed ? <Button className="mt-2" size="sm" variant="secondary" onClick={() => setRetry((value) => value + 1)}>Retry loading scan</Button> : null}
            </div>
            <Button size="sm" variant="ghost" onClick={closeSelection}>Close</Button>
          </div>
        </section>
      ) : null}
      {detail.scanHistory.map((run, index) => {
        const completed = run.status === "completed";
        const duration = run.finishedAt ? formatDuration(Math.max(0, Date.parse(run.finishedAt) - Date.parse(run.startedAt))) : null;
        return (
          <article className={`rounded-lg border bg-surface-1 px-4 py-3 ${run.id === selectedRunId ? "border-hairline-strong bg-row-selected-surface" : "border-hairline"}`} key={run.id}>
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="flex items-center gap-2">
                <Badge variant={incompleteInventoryReason(run.errorMessage) ? "warning" : run.status === "failed" ? "destructive" : run.status === "running" ? "secondary" : "outline"}>{formatLastScanStatus(run.status, run.errorMessage)}</Badge>
                {index === 0 ? <span className="text-xs text-ink-subtle">Latest attempt</span> : null}
              </div>
              <div className="flex items-center gap-2">
                <time className="text-xs text-ink-subtle">{formatTimestamp(run.finishedAt ?? run.startedAt)}</time>
                <Button
                  aria-label={`${completed ? "Compare" : "Open"} scan #${run.id}`}
                  aria-pressed={run.id === selectedRunId}
                  data-scan-history-run={run.id}
                  size="sm"
                  variant="ghost"
                  onClick={() => onSelectRun(run.id)}
                >{completed ? "Compare" : "Details"}</Button>
              </div>
            </div>
            <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-ink-subtle sm:grid-cols-4">
              <span>Duration <strong className="font-medium text-ink">{duration ?? (run.status === "running" ? "In progress" : "Not recorded")}</strong></span>
              {completed ? (
                <>
                  <span>Packages <strong className="font-medium text-ink">{run.packageCount ?? "Not recorded"}</strong></span>
                  <span>New <strong className="font-medium text-ink">+{run.newCritical + run.newHigh + run.newMedium}</strong></span>
                  <ResolutionCount run={run} onOpenResolved={run.id === selectedRunId ? undefined : onOpenResolved} />
                </>
              ) : null}
            </div>
            {run.sourceRevision ? <p className="mt-2 text-xs text-ink-subtle">Source revision {run.sourceRevision.slice(0, 8)}</p> : null}
            {run.errorMessage ? (
              <details className="mt-2 rounded-md bg-[color:var(--alert-danger-bg)] px-3 py-2 text-xs [color:var(--alert-danger-fg)]">
                <summary className="cursor-pointer">Scan error details</summary>
                <p className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap [overflow-wrap:anywhere]">{run.errorMessage}</p>
              </details>
            ) : null}
            {run.warnings?.length ? <p className="mt-2 text-xs text-ink-subtle">{run.warnings.length} scanner warning{run.warnings.length === 1 ? "" : "s"}</p> : null}
            {completed && run.evidenceComplete === false ? <p className="mt-2 text-xs text-ink-subtle">This scan has incomplete historical evidence.</p> : null}
          </article>
        );
      })}
    </div>
  );
}

function ResolutionCount({ run, onOpenResolved }: { run: RepoScanRun; onOpenResolved?: (runId: number) => void }) {
  const count = run.resolvedCritical + run.resolvedHigh + run.resolvedMedium;
  if (run.evidenceComplete && count > 0 && onOpenResolved) {
    return (
      <Button
        aria-label={`View ${count} resolved ${count === 1 ? "finding" : "findings"} from scan #${run.id}`}
        className="-my-1 h-6 justify-self-start px-0"
        size="xs"
        variant="link"
        onClick={() => onOpenResolved(run.id)}
      >
        View {count} resolved
      </Button>
    );
  }
  return <span>Resolved <strong className="font-medium text-status-success">−{count}</strong></span>;
}

function formatDuration(milliseconds: number) {
  const seconds = Math.round(milliseconds / 1000);
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
