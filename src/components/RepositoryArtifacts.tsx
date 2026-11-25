import { useEffect, useState } from "react";
import { Archive } from "@phosphor-icons/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState, Spinner } from "@/components/ui/feedback";
import { Select } from "@/components/ui/form-controls";
import { getArtifactFindings, listArtifactScans, openExternalUrl, scanArtifact } from "@/lib/api";
import { bundledToolLabel, FINDING_MATCH_LABEL, findingMatchNeedsReview } from "@/lib/findingMatch";
import { formatTimestamp } from "@/lib/format";
import type { ArtifactFindings, ArtifactScan, ArtifactScanKind } from "@/lib/types";

const KIND_LABELS: Record<ArtifactScanKind, string> = {
  build_file: "Build file or image archive",
  build_folder: "Build folder or OCI layout",
  sbom: "SBOM",
};
const PAGE_SIZE = 100;

export function RepositoryArtifacts({ repoId, canScan }: { repoId: number; canScan: boolean }) {
  const [scans, setScans] = useState<ArtifactScan[] | null>(null);
  const [scanId, setScanId] = useState<number>();
  const [kind, setKind] = useState<ArtifactScanKind>("build_file");
  const [scanning, setScanning] = useState(false);
  const [historyError, setHistoryError] = useState<string>();
  const [scanError, setScanError] = useState<string>();

  useEffect(() => {
    let disposed = false;
    listArtifactScans(repoId).then(
      (scans) => { if (!disposed) setScans(scans); },
      (cause: unknown) => { if (!disposed) setHistoryError(String(cause)); },
    );
    return () => { disposed = true; };
  }, [repoId]);

  async function runScan() {
    setScanning(true);
    setScanError(undefined);
    try {
      const scan = await scanArtifact(repoId, kind);
      if (scan) {
        setScans((previous) => [scan, ...(previous ?? [])].slice(0, 50));
        setScanId(scan.id);
      }
    } catch (cause) {
      setScanError(String(cause));
    } finally {
      setScanning(false);
    }
  }

  const current = scans?.find((scan) => scan.id === scanId) ?? scans?.[0];
  const historyLoading = scans === null && !historyError;

  return (
    <div className="grid gap-4">
      <section aria-label="Scan a build artifact" className="grid gap-3 rounded-lg border border-hairline bg-surface-1 px-4 py-3">
        <div className="flex flex-wrap items-end gap-2">
          <Select label="Scan input" value={kind} onValueChange={(value) => setKind(value as ArtifactScanKind)} disabled={scanning}>
            {Object.entries(KIND_LABELS).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
          </Select>
          <Button disabled={!canScan || scanning || historyLoading} onClick={() => void runScan()}>Choose and scan</Button>
        </div>
        <p className="text-xs text-ink-subtle">Choose the deployed build, an exported container image, or a Syft, CycloneDX or SPDX SBOM from the build.</p>
        {scanning ? <div role="status" className="flex items-center gap-2 text-sm text-ink-subtle"><Spinner label="Artifact scan in progress" />Scanning selected artifact</div> : null}
        {scanError ? <p role="alert" className="text-sm text-severity-critical [overflow-wrap:anywhere]">{scanError}</p> : null}
      </section>
      {historyLoading ? <Spinner label="Loading artifact scans" /> : null}
      {historyError ? <p role="alert" className="text-sm text-severity-critical">{historyError}</p> : null}
      {current ? (
        <>
          {scans && scans.length > 1 ? (
            <Select label="Recent artifact scans" value={String(current.id)} onValueChange={(value) => setScanId(Number(value))}>
              {scans.map(({ id, summary }) => <option key={id} value={id}>{summary.name} · {formatTimestamp(summary.scannedAt)}</option>)}
            </Select>
          ) : null}
          <ArtifactReport key={current.id} scan={current} />
        </>
      ) : scans ? <EmptyState icon={Archive} title="No artifact scans yet" description="Select a build or an SBOM to see its package findings here." /> : null}
    </div>
  );
}

function ArtifactReport({ scan: { id, repoId, summary: scan } }: { scan: ArtifactScan }) {
  return (
    <section className="min-w-0 rounded-lg border border-hairline bg-surface-1">
      <header className="grid gap-2 border-b border-hairline px-4 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-sm font-semibold text-ink [overflow-wrap:anywhere]">{scan.name}</h3>
          {scan.coverageWarnings.length ? <Badge variant="warning">Coverage incomplete</Badge> : null}
        </div>
        <p className="text-xs text-ink-subtle">{KIND_LABELS[scan.kind]} · {formatTimestamp(scan.scannedAt)}{scan.platform ? ` · ${scan.platform}` : ""}</p>
        <p className="text-sm text-ink">{scan.packageCount} {scan.packageCount === 1 ? "package" : "packages"} · {scan.findingCount} {scan.findingCount === 1 ? "finding" : "findings"}</p>
        <p className="text-xs text-ink-subtle [overflow-wrap:anywhere]"><span className="font-medium">{scan.kind === "sbom" ? "SBOM SHA-256" : scan.kind === "build_folder" ? "Snapshot SHA-256" : "File SHA-256"}</span> <code>{scan.digest}</code></p>
        {scan.kind === "sbom" ? <p className="text-xs text-ink-subtle">Coverage follows the contents of the supplied SBOM.</p> : null}
        {scan.coverageWarnings.length ? <ul className="list-disc space-y-1 pl-4 text-xs text-ink-subtle">{scan.coverageWarnings.map((warning) => <li key={warning}>{warning}</li>)}</ul> : null}
      </header>
      {scan.findingCount ? <ArtifactFindingList id={id} repoId={repoId} findingCount={scan.findingCount} /> : (
        <p className="px-4 py-3 text-sm text-ink-subtle">{scan.packageCount ? "Grype reported no findings for these packages." : "No package findings can be verified for this input."}</p>
      )}
    </section>
  );
}

function ArtifactFindingList({ id, repoId, findingCount }: { id: number; repoId: number; findingCount: number }) {
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<ArtifactFindings>();
  const [loadError, setLoadError] = useState<{ offset: number; message: string }>();
  const [linkError, setLinkError] = useState<string>();
  useEffect(() => {
    let disposed = false;
    getArtifactFindings(repoId, id, offset).then(
      (page) => { if (!disposed) setPage(page); },
      (cause: unknown) => { if (!disposed) setLoadError({ offset, message: String(cause) }); },
    );
    return () => { disposed = true; };
  }, [id, offset, repoId]);
  const error = loadError?.offset === offset ? loadError.message : undefined;
  const findings = page?.offset === offset ? page.findings : undefined;

  return (
    <>
      {error ? <p role="alert" className="px-4 py-3 text-sm text-severity-critical">{error}</p> : null}
      {linkError ? <p role="alert" className="px-4 py-3 text-sm text-severity-critical">{linkError}</p> : null}
      {!findings && !error ? <div className="px-4 py-3"><Spinner label="Loading artifact findings" /></div> : null}
      {findings ? (
        <ul aria-label="Artifact findings" className="divide-y divide-hairline">
          {findings.map((finding) => {
            const match = finding.matchEvidence;
            const caution = findingMatchNeedsReview(match);
            const variant = finding.severity === "CRITICAL" ? "destructive" : finding.severity === "HIGH" ? "warning" : "secondary";
            const url = finding.sourceUrl;
            return (
              <li key={finding.fingerprint} className="grid gap-1.5 px-4 py-3 text-sm">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant={variant}>{finding.severity}</Badge>
                  {url ? (
                    <Button type="button" variant="link" className="h-auto p-0 font-semibold" onClick={() => {
                      setLinkError(undefined);
                      void openExternalUrl(url).catch((cause: unknown) => setLinkError(String(cause)));
                    }}>{finding.advisoryId}</Button>
                  ) : <span className="font-semibold text-ink">{finding.advisoryId}</span>}
                  <span className="text-ink [overflow-wrap:anywhere]">{finding.packageName} {finding.packageVersion}</span>
                  {finding.fixVersion && !caution ? <span className="text-xs text-ink-subtle">Fix {finding.fixVersion}</span> : null}
                </div>
                {match?.bundledTool ? <p className="text-xs text-ink-subtle [overflow-wrap:anywhere]">{bundledToolLabel(match.bundledTool)}</p> : null}
                {match ? <p className="text-xs text-ink-subtle [overflow-wrap:anywhere]">{FINDING_MATCH_LABEL[match.kind]}{match.relativePath ? ` · ${match.relativePath}` : ""}</p> : null}
              </li>
            );
          })}
        </ul>
      ) : null}
      {findingCount > PAGE_SIZE ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-hairline px-4 py-3">
          <Button variant="secondary" size="sm" disabled={!offset} onClick={() => setOffset(offset - PAGE_SIZE)}>Previous</Button>
          <Button variant="secondary" size="sm" disabled={offset + PAGE_SIZE >= findingCount} onClick={() => setOffset(offset + PAGE_SIZE)}>Next</Button>
          <p className="text-xs text-ink-subtle">{offset + 1}–{Math.min(offset + PAGE_SIZE, findingCount)} of {findingCount}</p>
        </div>
      ) : null}
    </>
  );
}
