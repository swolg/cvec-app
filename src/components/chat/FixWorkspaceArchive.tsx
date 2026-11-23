import { useEffect, useState } from "react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/overlays";
import { listArchivedFixWorkspaces } from "@/lib/fixWorkspaceApi";
import type { FixArchivePage } from "@/lib/types";

export function FixWorkspaceArchive({ repositoryId, findingFingerprint, open, onOpenChange, onSelect }: { repositoryId: number; findingFingerprint?: string; open: boolean; onOpenChange: (open: boolean) => void; onSelect: (id: string) => void }) {
  const [page, setPage] = useState<FixArchivePage | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [allFindings, setAllFindings] = useState(false);
  const scope = allFindings ? undefined : findingFingerprint;
  useEffect(() => { if (!open) { setPage(null); setCursor(null); setPrevious([]); setAllFindings(false); } }, [open, repositoryId]);
  useEffect(() => {
    if (!open) return;
    let disposed = false;
    setLoading(true); setError(null); setPage(null);
    listArchivedFixWorkspaces(repositoryId, cursor, scope).then((value) => {
      if (!disposed) setPage(value);
    }).catch((cause) => { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [repositoryId, open, cursor, retry, scope]);
  const entries = page?.entries ?? [];
  function toggleScope() {
    setPage(null);
    setCursor(null);
    setPrevious([]);
    setAllFindings((current) => !current);
  }
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent>
    <DialogHeader><DialogTitle data-slot="dialog-title">Archived fix workspaces</DialogTitle><DialogDescription data-slot="dialog-description">Saved locally with their branches, files, drafts and evidence. Open a workspace to inspect or restore it.</DialogDescription></DialogHeader>
    <DialogBody className="grid gap-3">
      {findingFingerprint ? <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-ink-subtle">{scope ? "Archived fixes for the selected finding" : "All archived fixes for this repository"}</p>
        <Button size="sm" variant="secondary" onClick={toggleScope}>{scope ? "Show all archived fixes" : "Show matching fixes"}</Button>
      </div> : null}
      {loading ? <p role="status" className="text-sm text-ink-subtle">Loading archived workspaces…</p> : null}
      {error ? <Alert variant="warning"><AlertTitle>Archive could not load</AlertTitle><AlertDescription>{error}</AlertDescription><Button className="mt-2" size="sm" onClick={() => setRetry((value) => value + 1)}>Retry archive</Button></Alert> : null}
      {page?.entries.length === 0 ? <p className="text-sm text-ink-subtle">{scope ? "No archived fixes for the selected finding." : "No archived workspaces on this page."}</p> : null}
      {entries.map((entry) => (
        <Button className="h-auto min-w-0 justify-start whitespace-normal py-3 text-left" variant="secondary" key={entry.id} onClick={() => { onSelect(entry.id); onOpenChange(false); }}>
          <span className="min-w-0 break-words">
            <span className="block">{entry.title}</span>
            <span className="mt-1 block text-xs text-ink-subtle">
              {entry.findingFingerprint && entry.findingFingerprint === findingFingerprint ? "Matches selected finding · " : ""}
              {entry.status ? entry.status.replaceAll("_", " ") + " · " : ""}
              {entry.id.slice(0, 8)} · Archived {new Date(entry.archivedAt).toLocaleString()}
            </span>
          </span>
        </Button>
      ))}
    </DialogBody>
    <DialogFooter><Button variant="ghost" onClick={() => onOpenChange(false)}>Close archive</Button><Button variant="secondary" disabled={loading || previous.length === 0} onClick={() => { setCursor(previous.at(-1) ?? null); setPrevious((values) => values.slice(0, -1)); }}>Previous page</Button><Button variant="secondary" disabled={loading || !page?.nextCursor} onClick={() => { setPrevious((values) => [...values, cursor]); setCursor(page!.nextCursor); }}>Next page</Button></DialogFooter>
  </DialogContent></Dialog>;
}
