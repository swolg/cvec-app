import { lazy, Suspense, useEffect, useId, useRef, useState, type ReactNode } from "react";

import type { FindingRemediationProjection, RepoFinding, ReviewState } from "@/lib/types";
import { Button } from "@/components/ui/button";

const ReactMarkdown = lazy(() => import("react-markdown"));

interface ChatAction {
  label: string;
  onOpen: () => void;
  loading?: boolean;
  analysisReason?: string;
}

export function FindingInspectorBody({ finding, scanResolution, priorityReasons = [], reviewState, snoozeUntil, onOpenUrl, remediation, remediationLoading = false, remediationError, onRetryRemediation, onOpenManifest, chatAction, fixPlanEnabled = true }: { finding: RepoFinding; scanResolution?: { runId: number; previousRunId: number | null }; priorityReasons?: string[]; reviewState?: ReviewState; snoozeUntil?: string | null; onOpenUrl?: (url: string) => void; remediation?: FindingRemediationProjection | null; remediationLoading?: boolean; remediationError?: string | null; onRetryRemediation?: () => void; onOpenManifest?: () => Promise<void>; chatAction?: ChatAction; fixPlanEnabled?: boolean }) {
  const summary = finding.intel?.summary?.trim();
  const title = finding.title?.trim();
  const description = summary && summary !== (finding.intel?.title || title)?.trim()
    ? summary
    : !summary && title && /\n|\[[^\]]+\]\([^)]+\)/.test(title) ? title : null;
  const reasons = priorityReasons.filter((reason) => reason !== "Fix available" && !/^(critical|high|medium) severity$/i.test(reason));
  const reasonId = useId();
  const handoff = chatAction && <Button aria-describedby={chatAction.analysisReason ? reasonId : undefined} size="sm" disabled={chatAction.loading} onClick={chatAction.onOpen}>{chatAction.label}</Button>;
  const analysisReason = chatAction?.analysisReason && <p id={reasonId} className="text-sm text-ink-subtle">{chatAction.analysisReason}</p>;
  const heading = finding.state === "resolved" ? "Historical evidence" : analysisReason ? "Remediation evidence" : "Fix plan";
  return (
    <div className="grid max-w-[72ch] gap-5">
      {scanResolution ? (
        <section className="rounded-lg border border-hairline bg-surface-2 p-3.5">
          <h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">Scan evidence</h3>
          <dl className="mt-2 grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-3 gap-y-2 text-sm">
            <Detail label="Resolved in" value={`Scan #${scanResolution.runId}`} />
            {scanResolution.previousRunId !== null ? <Detail label="Detected in" value={`Scan #${scanResolution.previousRunId}`} /> : null}
            <Detail label="Detected package" value={finding.packageName + " " + (finding.packageVersion ?? "· version unknown")} />
          </dl>
          {handoff ? <div className="mt-3 space-y-2">{analysisReason}{handoff}</div> : null}
        </section>
      ) : fixPlanEnabled ? <FixPlan heading={heading} finding={finding} remediation={remediation} loading={remediationLoading} loadError={remediationError} onRetry={onRetryRemediation} onOpenManifest={onOpenManifest} handoff={handoff} analysisReason={analysisReason} /> : <section className="rounded-lg border border-hairline bg-surface-2 p-3.5"><h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">{analysisReason ? heading : "Recommended fix"}</h3><p className="mt-1 text-sm font-semibold text-ink">{analysisReason ? finding.packageName : finding.fixVersion ? `Update ${finding.packageName} to ${finding.fixVersion}` : finding.packageName + " · No fixed version is listed."}</p>{handoff ? <div className="mt-3 space-y-2">{analysisReason}{handoff}</div> : null}</section>}
      {reasons.length ? <section><h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">Why it needs attention</h3><ul className="mt-2 grid gap-1.5">{reasons.map((reason) => <li key={reason} className="flex items-center gap-2 text-sm text-ink-muted"><span aria-hidden className="size-1.5 rounded-full bg-primary" />{reason}</li>)}</ul></section> : null}
      <section><h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">Finding state</h3><dl className="mt-2 grid grid-cols-[110px_minmax(0,1fr)] gap-x-3 gap-y-2 text-sm"><dt className="text-ink-subtle">Scanner state</dt><dd className="capitalize text-ink">{finding.state}</dd>{reviewState ? <><dt className="text-ink-subtle">Review state</dt><dd className="capitalize text-ink">{reviewState}</dd></> : null}{snoozeUntil ? <><dt className="text-ink-subtle">Returns</dt><dd className="text-ink">{new Date(snoozeUntil).toLocaleString()}</dd></> : null}</dl></section>
      {finding.intel ? <section><h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">Advisory source</h3><p className="mt-2 text-sm text-ink-muted">{finding.intel.sources.join(", ")}{finding.intel.severity.toUpperCase() !== finding.severity ? " · Source severity " + finding.intel.severity.toUpperCase() : ""}</p>{finding.intel.fetchedAt ? <p className="mt-1 text-xs text-ink-subtle">Retrieved {new Date(finding.intel.fetchedAt).toLocaleString()}</p> : null}{finding.intel.updatedAt ? <p className="mt-1 text-xs text-ink-subtle">Source updated {new Date(finding.intel.updatedAt).toLocaleString()}</p> : null}</section> : null}
      {description ? <section><h3 className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">Description</h3><div className="mt-2 text-ink-muted"><Suspense fallback={<p className="whitespace-pre-wrap text-sm leading-6">{stripMarkdown(description)}</p>}><ReactMarkdown components={{ h1: ({ children }) => <h4 className="mb-2 mt-5 text-base font-semibold text-ink first:mt-0">{children}</h4>, h2: ({ children }) => <h4 className="mb-2 mt-5 text-base font-semibold text-ink first:mt-0">{children}</h4>, h3: ({ children }) => <h4 className="mb-2 mt-4 text-sm font-semibold text-ink first:mt-0">{children}</h4>, p: ({ children }) => <p className="mb-3 text-sm leading-6 last:mb-0">{children}</p>, ul: ({ children }) => <ul className="mb-3 list-disc space-y-1 pl-5 text-sm leading-6">{children}</ul>, ol: ({ children }) => <ol className="mb-3 list-decimal space-y-1 pl-5 text-sm leading-6">{children}</ol>, li: ({ children }) => <li>{children}</li>, strong: ({ children }) => <strong className="font-semibold text-ink">{children}</strong>, a: ({ children, href }) => href ? <a href={href} className="text-primary underline-offset-4 hover:underline" onClick={onOpenUrl ? (event) => { event.preventDefault(); onOpenUrl(href); } : undefined}>{children}</a> : <span className="text-primary">{children}</span>, code: ({ children, className }) => className ? <code className="block overflow-x-auto whitespace-pre rounded-md border border-hairline bg-surface-3 p-3 text-xs leading-5 text-ink">{children}</code> : <code className="rounded border border-hairline bg-surface-3 px-1 py-0.5 text-xs text-ink">{children}</code>, pre: ({ children }) => <pre className="mb-3 overflow-x-auto">{children}</pre>, blockquote: ({ children }) => <blockquote className="mb-3 border-l-2 border-hairline-strong pl-3 text-sm leading-6 text-ink-subtle">{children}</blockquote> }}>{description}</ReactMarkdown></Suspense></div></section> : null}
    </div>
  );
}

function FixPlan({ heading, finding, remediation, loading, loadError, onRetry, onOpenManifest, handoff, analysisReason }: { heading: string; finding: RepoFinding; remediation?: FindingRemediationProjection | null; loading: boolean; loadError?: string | null; onRetry?: () => void; onOpenManifest?: () => Promise<void>; handoff: ReactNode; analysisReason: ReactNode }) {
  const headingId = useId();
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">("idle");
  const [actionError, setActionError] = useState<string | null>(null);
  const resetTimer = useRef<number | null>(null);
  useEffect(() => {
    setCopyState("idle");
    setActionError(null);
    if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    return () => { if (resetTimer.current !== null) window.clearTimeout(resetTimer.current); };
  }, [finding.fingerprint]);

  async function copyBrief() {
    if (!remediation) return;
    try {
      await navigator.clipboard.writeText(remediation.briefMarkdown);
      setActionError(null);
      setCopyState("copied");
      resetTimer.current = window.setTimeout(() => setCopyState("idle"), 1800);
    } catch {
      setCopyState("error");
      setActionError("Could not copy the fix brief. Try again.");
    }
  }

  async function openManifest() {
    if (!onOpenManifest) return;
    try {
      setActionError(null);
      await onOpenManifest();
    } catch {
      setActionError("The manifest is no longer available. Open the repository instead.");
    }
  }

  const installed = remediation?.installedVersion ?? finding.packageVersion ?? "version not listed";
  const target = remediation?.scannerReportedTarget ?? finding.fixVersion;
  return (
    <section aria-labelledby={headingId} className="rounded-lg border border-hairline bg-surface-2 p-3.5">
      <h3 id={headingId} className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">{heading}</h3>
      <p className="mt-2 break-words text-sm font-semibold text-ink">{finding.packageName} <span className="font-normal text-ink-muted">{installed}</span>{target ? <> <span aria-hidden>→</span> {target}</> : null}</p>
      {analysisReason ? <div className="mt-1">{analysisReason}</div> : <p className="mt-1 text-sm leading-5 text-ink-muted">{target ? "Check the scanner-reported target for compatibility with this repository." : "No fixed version is listed. Review upstream guidance before changing the dependency."}</p>}
      {loading ? <p role="status" className="mt-3 text-sm text-ink-subtle">Loading local fix evidence…</p> : null}
      {loadError ? <div className="mt-3 flex flex-wrap items-center gap-2" role="alert"><p className="text-sm text-severity-critical">{loadError}</p>{onRetry ? <Button size="sm" variant="secondary" onClick={onRetry}>Retry</Button> : null}</div> : null}
      {handoff || (remediation?.canOpenManifest && onOpenManifest) ? (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {handoff}
          {remediation?.canOpenManifest && onOpenManifest ? <Button size="sm" variant="ghost" onClick={() => void openManifest()}>Open manifest</Button> : null}
        </div>
      ) : null}
      {remediation ? <>
        <p className="sr-only" aria-live="polite">{copyState === "copied" ? "Fix brief copied." : copyState === "error" ? "Fix brief could not be copied." : ""}</p>
        {actionError ? <p role="alert" className="mt-2 text-sm text-severity-critical">{actionError}</p> : null}
        <p className="mt-3 text-sm text-ink-muted">{verificationCopy(remediation)}</p>
        <details key={finding.fingerprint} className="mt-3 border-t border-hairline pt-3">
          <summary className="min-h-8 cursor-pointer text-sm font-medium text-ink focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary">{analysisReason ? "Technical details" : "Fix plan details"}</summary>
          <dl className="mt-3 grid grid-cols-1 gap-2 text-sm sm:grid-cols-[130px_minmax(0,1fr)] sm:gap-x-3">
            {remediation.relativePath ? <Detail label="Location" value={remediation.relativePath} /> : <Detail label="Location" value="Not available from this scan" />}
            <Detail label="Relationship" value={remediation.relationship === "unknown" ? "Not proven" : remediation.relationship === "direct" ? "Direct" : "Transitive"} />
            <Detail label="Evidence" value={evidenceCopy(remediation)} />
            <Detail label="Evidence scan" value={`#${remediation.evidenceScanRunId}${remediation.evidenceScannedAt ? ` · ${new Date(remediation.evidenceScannedAt).toLocaleString()}` : ""}`} />
            {remediation.sourceSnapshot.revision ? <Detail label="Revision" value={`${remediation.sourceSnapshot.revision.slice(0, 12)}${remediation.sourceSnapshot.dirty ? " + local changes" : ""}`} /> : null}
            {remediation.verificationScanRunId && remediation.verificationScanRunId !== remediation.evidenceScanRunId ? <Detail label="Verification scan" value={`#${remediation.verificationScanRunId}${remediation.verificationScannedAt ? ` · ${new Date(remediation.verificationScannedAt).toLocaleString()}` : ""}`} /> : null}
          </dl>
          <Button className="mt-2" size="sm" variant="ghost" onClick={() => void copyBrief()}>{copyState === "copied" ? "Copied" : "Copy fix brief"}</Button>
        </details>
      </> : !loading && !loadError ? <p className="mt-3 text-sm text-ink-subtle">A compatible scan is needed to copy a fix brief.</p> : null}
    </section>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return <><dt className="font-medium text-ink-subtle">{label}</dt><dd className="break-words text-ink">{value}</dd></>;
}

function evidenceCopy(remediation: FindingRemediationProjection): string {
  if (remediation.evidenceQuality === "stale") return "Stale — run a new scan before relying on this evidence";
  if (remediation.evidenceQuality === "unavailable") return "Location and relationship evidence are unavailable from this scan";
  if (remediation.evidenceQuality === "partial") return "Partial local scan evidence";
  return "Current local scan evidence";
}

function verificationCopy(remediation: FindingRemediationProjection): string {
  if (remediation.verificationState === "still_detected") return "The latest complete scan still detected this advisory for the package.";
  if (remediation.verificationState === "not_detected") return "The latest complete scan did not detect this advisory for the package.";
  if (remediation.verificationState === "reappeared") return "The advisory was detected again after a complete scan without it.";
  return "The latest result could not verify whether this finding is still detected.";
}

function stripMarkdown(value: string): string {
  return value.replace(/^#{1,6}\s+/gm, "").replace(/\*\*([^*]+)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1").replace(/^[-*]\s+/gm, "• ");
}
