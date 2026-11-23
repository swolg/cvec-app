import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { ContextSource, RepositoryChatContext } from "@/lib/types";

export function ChatContextPreview({ context, onInspectSource }: { context: RepositoryChatContext | null; onInspectSource?: (source: ContextSource, trigger: HTMLButtonElement) => void }) {
  if (!context) return <p className="text-sm text-ink-subtle">Context is prepared locally when you inspect or send the next message.</p>;
  return <div className="grid gap-4 text-sm text-ink-subtle">
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2">
      <dt>Repository</dt><dd className="break-words text-ink">{context.repository.name}</dd>
      <dt>Revision</dt><dd className="break-all font-mono text-xs text-ink">{context.repository.revision ?? "Not available"}</dd>
      <dt>Scan</dt><dd className="text-ink">{context.scan.id ? `#${context.scan.id} · ${context.scan.status ?? "stored"}` : "No completed scan"}</dd>
      <dt>Source mode</dt><dd className="text-ink">{context.repository.sourceMode.replaceAll("_", " ")}</dd>
      <dt>Freshness</dt><dd className="break-words text-ink">{context.freshness.message}</dd>
    </dl>
    <section aria-label="Prepared context sources" className="min-w-0">
      <h3 className="font-medium text-ink">Prepared sources · {context.sources.length}</h3>
      <p className="mt-1 text-xs">Prepared locally for this Analysis message. Inspecting a source does not send it to a model.</p>
      {context.sources.length ? <ul className="mt-2 grid max-h-72 gap-2 overflow-auto">
        {context.sources.map((source) => <li key={source.id} className="min-w-0 rounded-md border border-hairline p-2">
          <div className="flex flex-wrap items-center gap-1.5"><Badge variant="outline">{source.kind}</Badge>{source.stale ? <Badge variant="warning">Stale or unavailable</Badge> : null}</div>
          <p className="mt-1 break-words text-ink">{source.label}</p>
          {source.repositoryRelativePath ? <p className="mt-1 break-all font-mono text-xs">{source.repositoryRelativePath}{source.lineStart !== null ? `:${source.lineStart}${source.lineEnd !== null && source.lineEnd !== source.lineStart ? `–${source.lineEnd}` : ""}` : ""}</p> : null}
          {onInspectSource ? <Button size="xs" variant="ghost" className="mt-1" aria-label={`Inspect source: ${source.label}`} onClick={(event) => onInspectSource(source, event.currentTarget)}>Inspect source</Button> : null}
        </li>)}
      </ul> : <p className="mt-2 text-xs">No repository sources are prepared.</p>}
    </section>
    {context.attachments.length ? <div className="flex flex-wrap gap-1.5">{context.attachments.map((attachment) => <Badge key={`${attachment.kind}:${attachment.id}`} variant={attachment.available ? "outline" : "warning"}>{attachment.label}</Badge>)}</div> : null}
    {context.omissions.length ? <section aria-label="Context omissions"><h3 className="font-medium text-ink">Not included</h3><ul className="mt-1 list-disc space-y-1 pl-4 text-xs">{context.omissions.map((omission) => <li key={`${omission.category}:${omission.reason}`}>{omission.count} {omission.category}: {omission.reason}</li>)}</ul></section> : null}
  </div>;
}
