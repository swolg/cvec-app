import { Archive, ArrowClockwise, CaretDown, DotsThree, LockSimple } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";

import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { Input } from "@/components/ui/form-controls";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "@/components/ui/overlays";
import type { CompanionThread, RepositoryChatScanStatus, RepoSummary, RepositoryChatContext } from "@/lib/types";

interface ConversationHeaderProps {
  repository: RepoSummary;
  scanStatus: RepositoryChatScanStatus | null;
  thread: CompanionThread | null;
  context: RepositoryChatContext | null;
  contextChanged: boolean;
  contextStatus?: "loading" | "ready" | "error";
  onInspectContext: (trigger: HTMLButtonElement) => void;
  onRename: (title: string) => void | Promise<void>;
  onArchive: (archived: boolean) => void | Promise<void>;
}

export function ConversationHeader({
  repository,
  scanStatus,
  thread,
  context,
  contextChanged,
  contextStatus = "ready",
  onInspectContext,
  onRename,
  onArchive,
}: ConversationHeaderProps) {
  const [title, setTitle] = useState(thread?.title ?? "");
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  useEffect(() => { setTitle(thread?.title ?? ""); }, [thread?.id, thread?.title]);
  useEffect(() => { setOpen(false); setPending(false); setError(null); return () => { request.current += 1; }; }, [thread?.id]);
  async function perform(action: () => void | Promise<void>) {
    if (pending) return;
    const sequence = ++request.current;
    setPending(true); setError(null);
    try { await action(); if (sequence === request.current) setOpen(false); }
    catch (cause) { if (sequence === request.current) setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { if (sequence === request.current) setPending(false); }
  }
  const completedId = scanStatus?.latestCompleted.id ?? null;
  const scanId = context?.scan.status === "completed" ? context.scan.id : completedId;
  const latestAttempt = scanStatus?.latestAttempt;
  const freshness = context?.freshness.state ?? "unknown";
  const archived = Boolean(thread?.archivedAt);
  const freshnessLabel = contextChanged && completedId && completedId !== scanId
    ? "Newer scan #" + completedId
    : freshness === "dirty" ? "Uncommitted changes"
    : freshness === "mismatched" ? "Revision differs"
    : freshness === "newer_scan" ? "Newer scan available"
    : null;

  return (
    <header className="chat-conversation-header shrink-0 border-b border-hairline bg-surface-2 px-3 py-2.5 sm:px-4">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="chat-conversation-breadcrumb flex min-w-0 items-center gap-1.5 text-xs text-ink-subtle">
            <span className="truncate">{repository.name}</span>
            <span aria-hidden="true">/</span>
            <span className="truncate text-ink">{thread?.title ?? "New conversation"}</span>
          </div>
          <div className="chat-conversation-meta mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5">
            <Badge variant="outline"><AppIcon className="size-3.5" icon={LockSimple} />Read-only</Badge>{archived ? <Badge variant="warning">Archived</Badge> : null}
            {contextStatus !== "ready" ? <Badge variant="outline">{contextStatus === "loading" ? "Loading scan context" : "Scan context unavailable"}</Badge> : scanId ? <Badge variant="secondary">Scan #{scanId}</Badge> : <Badge variant="outline">No completed scan</Badge>}
            {latestAttempt?.status === "failed" ? <Badge variant="warning" title={`Scan #${latestAttempt.id} failed`}>Latest scan failed</Badge> : null}
            {contextStatus === "ready" ? (
              <Button title={freshnessLabel ?? "Inspect context"} className="h-7 px-2 text-xs" variant="ghost" onClick={(event) => onInspectContext(event.currentTarget)}>Inspect context</Button>
            ) : null}
            {contextStatus === "ready" && freshnessLabel ? <Badge variant="warning">{freshnessLabel}</Badge> : null}
          </div>
        </div>
        <Popover open={open} onOpenChange={setOpen}>
          <PopoverTrigger render={<IconButton aria-label="Conversation actions" size="icon-sm" variant="ghost" />}>
            <AppIcon className="size-4" icon={thread ? DotsThree : CaretDown} weight="bold" />
          </PopoverTrigger>
          <PopoverContent align="end" className="w-72">
            <PopoverTitle className="text-sm font-semibold text-ink">Conversation</PopoverTitle>
            {thread ? (
              <>
                <Input containerClassName="mt-3" aria-label="Conversation title" maxLength={80} disabled={pending} value={title} onKeyDown={(event) => { if (event.key === "Enter" && title.trim() && title.trim() !== thread.title) { event.preventDefault(); void perform(() => onRename(title.trim())); } }} onChange={(event) => setTitle(event.target.value)} />
                <Button className="mt-2 w-full" disabled={pending || !title.trim() || title.trim() === thread.title} size="sm" variant="secondary" onClick={() => void perform(() => onRename(title.trim()))}>Rename</Button>
                <Button className="mt-1 w-full justify-start" size="sm" variant="ghost" disabled={pending} onClick={() => void perform(() => onArchive(!archived))}>
                  <AppIcon className="size-4" icon={archived ? ArrowClockwise : Archive} />{archived ? "Unarchive" : "Archive"}
                </Button>
                {error ? <p role="alert" className="mt-2 break-words text-xs text-status-warning-foreground">{error}</p> : null}
              </>
            ) : <p className="mt-2 text-xs leading-5 text-ink-subtle">The provider session starts only when you send the first message.</p>}
          </PopoverContent>
        </Popover>
      </div>
    </header>
  );
}
