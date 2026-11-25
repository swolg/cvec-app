import { useMemo } from "react";

type RecordValue = Record<string, unknown>;
function record(value: unknown): RecordValue | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as RecordValue : null;
}
function text(value: unknown): string | null { return typeof value === "string" && value.trim() ? value : null; }
function count(value: unknown): string {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value.toLocaleString() : "Not reported";
}
function timestamp(value: unknown): string {
  const raw = text(value);
  if (!raw) return "Not reported";
  const parsed = new Date(raw);
  return Number.isNaN(parsed.getTime()) ? raw : parsed.toLocaleString();
}

function alignment(freshness: unknown): string {
  switch (freshness) {
    case "matched": return "Aligned";
    case "mismatched": return "Revision differs";
    case "dirty": return "Uncommitted changes";
    default: return "Not reported";
  }
}

export function FindingSourceContent({ content }: { content: string }) {
  const finding = useMemo(() => {
    try { return record(JSON.parse(content)); } catch { return null; }
  }, [content]);
  const fields = [
    ["Package", text(finding?.packageName)],
    ["Detected version", text(finding?.packageVersion)],
    ["Scanner state", text(finding?.state)],
    ["Severity", text(finding?.severity)],
    ["Scanner-reported target", text(finding?.fixVersion)],
    ["Scan completed", text(finding?.scanCompletedAt) ? timestamp(finding?.scanCompletedAt) : null],
  ] as const;
  return <div className="grid gap-3">
    {finding ? <dl className="grid gap-3 rounded-md border border-hairline bg-surface-1 p-3 sm:grid-cols-2" aria-label="Finding details">
      {fields.map(([label, value]) => <div className="min-w-0" key={label}><dt className="text-xs text-ink-subtle">{label}</dt><dd className="break-words text-sm text-ink">{value ?? "Not reported"}</dd></div>)}
    </dl> : <p className="text-sm text-ink-subtle">Structured finding details are unavailable.</p>}
    <details className="min-w-0 rounded-md border border-hairline bg-surface-1">
      <summary className="cursor-pointer p-3 text-sm font-medium text-ink">Technical evidence</summary>
      <pre tabIndex={0} className="max-h-[40vh] overflow-auto whitespace-pre p-3 pt-0 text-xs text-ink">{finding ? JSON.stringify(finding, null, 2) : content}</pre>
    </details>
  </div>;
}

export function ScanSourceContent({ content }: { content: string }) {
  const parsed = useMemo(() => {
    try { const value: unknown = JSON.parse(content); return { scan: record(value), raw: JSON.stringify(value, null, 2) }; }
    catch { return { scan: null, raw: content }; }
  }, [content]);
  const scan = parsed.scan;
  const summary = record(scan?.findingSummary);
  const severity = record(summary?.openBySeverity);
  const fields = scan ? [
    ["Status", text(scan.status) ?? "Not reported"],
    ["Completed", timestamp(scan.finishedAt ?? scan.scannedAt)],
    ["Revision", text(scan.revision) ?? "Not reported"],
    ["Packages", count(scan.packageCount)],
    ["Open findings", count(summary?.totalOpen)],
    ["New findings", count(scan.new)],
    ["Resolved since previous scan", count(summary?.resolvedSincePreviousScan ?? scan.resolved)],
    ["Revision alignment", alignment(scan.freshness)],
    ["Recorded evidence", scan.evidenceComplete === true ? "Complete" : scan.evidenceComplete === false ? "Incomplete" : "Not reported"],
  ] : [];

  return <div className="grid gap-4">
    {scan ? <section aria-label="Scan summary" className="rounded-md border border-hairline bg-surface-1 p-3">
      <h4 className="text-sm font-semibold text-ink">Scan summary</h4>
      <dl className="mt-3 grid gap-3 sm:grid-cols-2">{fields.map(([label, value]) => <div key={label} className="min-w-0"><dt className="text-xs text-ink-subtle">{label}</dt><dd className={`mt-1 break-words text-sm text-ink${label === "Revision" ? " break-all font-mono text-xs" : ""}`}>{value}</dd></div>)}</dl>
      {severity ? <dl aria-label="Open findings by severity" className="mt-4 flex flex-wrap gap-x-4 gap-y-2 border-t border-hairline pt-3">{["CRITICAL", "HIGH", "MEDIUM", "LOW", "NEGLIGIBLE", "UNKNOWN"].filter((key) => Object.hasOwn(severity, key)).map((key) => <div key={key}><dt className="text-xs text-ink-subtle">{key}</dt><dd className="text-sm font-medium text-ink">{count(severity[key])}</dd></div>)}</dl> : null}
      <p className="mt-3 text-xs leading-5 text-ink-subtle">Finding counts are not counts of unique CVEs. Complete recorded evidence does not mean complete vulnerability coverage.</p>
    </section> : <p role="status" className="text-sm text-ink-subtle">A structured scan summary is unavailable. The recorded source remains available below.</p>}
    <details open={!scan} className="min-w-0 rounded-md border border-hairline bg-surface-1">
      <summary className="cursor-pointer p-3 text-sm font-medium text-ink">Raw scan data</summary>
      <pre aria-label="Raw scan data" tabIndex={0} className="max-h-[50vh] overflow-auto whitespace-pre p-3 pt-0 text-xs leading-5 text-ink outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus">{parsed.raw}</pre>
    </details>
  </div>;
}
