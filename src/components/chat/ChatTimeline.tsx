import {countPerformanceRender,recordPerformanceCommit} from "@/lib/performanceCounters";
import { ArrowDown, ChatCircleDots, FileText, LinkSimple } from "@phosphor-icons/react";
import { lazy, Suspense, memo, Profiler, useEffect, useMemo, useLayoutEffect, useRef, useState, type ReactNode } from "react";
const ChatMarkdown=lazy(()=>import("./ChatMarkdown"));

import { AppIcon } from "@/components/ui/app-icon";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import type { CompanionCitation, CompanionTimelineItem } from "@/lib/types";
import { readTimelinePosition, restoreTimelinePosition, saveTimelinePosition } from "./timelinePosition";
import { CopyButton } from "./CopyButton";

interface ChatTimelineProps {
  ariaLabel?: string;
  emptyState?: ReactNode;
  conversationKey?: string;
  repositoryName: string;
  providerLabel: string;
  segmentProviderLabels: readonly string[];
  items: CompanionTimelineItem[];
  hasEarlier?: boolean;
  historyLoading?: boolean;
  providerLabelsByItem?: Readonly<Record<string,string>>;
  onLoadEarlier?:()=>Promise<void>;
  onLoadLatest?:()=>Promise<void>;
  progress?: { label: string; startedAt: number; providerLabel?: string; responding?: boolean } | null;
  starters: readonly string[];
  startersDisabled?: boolean;
  onStarter: (value: string) => void;
  onCitation: (citation: CompanionCitation) => void;
}

export const ChatTimeline = memo(function ChatTimeline(props: ChatTimelineProps) {
  return <ChatTimelineView key={props.conversationKey ?? props.repositoryName} {...props} />;
});

function ChatTimelineView({ ariaLabel = "Repository chat messages", emptyState, conversationKey, repositoryName, providerLabel, segmentProviderLabels, items, progress, starters, startersDisabled = false, onStarter, onCitation,hasEarlier=false,historyLoading=false,providerLabelsByItem,onLoadEarlier,onLoadLatest }: ChatTimelineProps) {
  const identity = conversationKey ?? repositoryName;
  const remembered = readTimelinePosition(identity);
  const timeline = useRef<HTMLDivElement>(null);
  const following = useRef(remembered?.following ?? true);
  const restored = useRef(false);
  const previousTail = useRef<string | undefined>(undefined);
  const [visibleLimit, setVisibleLimit] = useState(remembered?.visibleLimit ?? 200);
  const limit = useRef(visibleLimit);
  limit.current = visibleLimit;
  const [newContent, setNewContent] = useState(false);
  countPerformanceRender("timelineRenders");
  const labeledItems = useMemo(() => {
    countPerformanceRender("timelineProjections");
    return labelTimelineItems(items, segmentProviderLabels, providerLabel, providerLabelsByItem);
  }, [items, segmentProviderLabels, providerLabel, providerLabelsByItem]);
  const visibleItems = labeledItems.slice(-visibleLimit);
  const tail = items.at(-1);
  const hasContent = items.length > 0 || Boolean(progress);

  useLayoutEffect(() => {
    const node = timeline.current;
    if (!node || restored.current || !items.length) return;
    restoreTimelinePosition(identity, node);
    previousTail.current = `${tail?.id}:${tail?.text.length}:${tail?.status}:${progress?.label ?? ""}`;
    restored.current = true;
  }, [identity, items.length, tail?.id, tail?.text.length, tail?.status, progress?.label]);

  useLayoutEffect(() => {
    const node = timeline.current;
    return () => {
      if (node && restored.current) saveTimelinePosition(identity, node, following.current, limit.current);
    };
  }, [identity]);

  useLayoutEffect(() => {
    const node = timeline.current;
    if (!hasContent) { if (node) node.scrollTop = 0; return; }
    if (!node || typeof ResizeObserver === "undefined") return;
    let frame = 0;
    const observer = new ResizeObserver(() => {
      if (!following.current) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        node.scrollTop = node.scrollHeight;
      });
    });
    observer.observe(node.firstElementChild ?? node);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [hasContent]);

  useLayoutEffect(() => {
    const key = `${tail?.id}:${tail?.text.length}:${tail?.status}:${progress?.label ?? ""}`;
    if (key === previousTail.current) return;
    previousTail.current = key;
    const node = timeline.current;
    if (!node) return;
    if (!hasContent) { node.scrollTop = 0; return; }
    if (tail?.kind === "user_message" && tail.id.startsWith("local:")) {
      following.current = true;
      setNewContent(false);
    }
    if (following.current) {
      let settledFrame = 0;
      const frame = requestAnimationFrame(() => {
        settledFrame = requestAnimationFrame(() => {
          node.scrollTop = node.scrollHeight;
        });
      });
      return () => {
        cancelAnimationFrame(frame);
        cancelAnimationFrame(settledFrame);
      };
    }
    setNewContent(true);
  }, [hasContent, tail?.id, tail?.status, tail?.text.length, progress?.label]);

  async function loadEarlier() {
    const node = timeline.current;
    const top=node?.getBoundingClientRect().top??0;
    const anchor=[...(node?.querySelectorAll<HTMLElement>("[data-message-id]")??[])].find((element)=>element.getBoundingClientRect().bottom>=top);
    const key=anchor?.dataset.messageId;const offset=anchor?.getBoundingClientRect().top??top;
    following.current=false;
    setVisibleLimit((value)=>Math.min(400,value+200));
    if(onLoadEarlier)await onLoadEarlier();
    requestAnimationFrame(()=>requestAnimationFrame(()=>{
      const restored=[...(node?.querySelectorAll<HTMLElement>("[data-message-id]")??[])].find((element)=>element.dataset.messageId===key);
      if(node&&restored)node.scrollTop+=restored.getBoundingClientRect().top-offset;
    }));
  }

  async function jumpToLatest() {
    const node = timeline.current;
    if (!node) return;
    following.current = true;
    await onLoadLatest?.();
    if (typeof node.scrollTo === "function") node.scrollTo({ top: node.scrollHeight, behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    else node.scrollTop = node.scrollHeight;
    setNewContent(false);
  }

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      {progress ? <div className="mx-auto w-full max-w-3xl shrink-0 px-4 pb-2 pt-3 sm:px-6"><ChatTurnProgress {...progress} /></div> : null}
      <div
        aria-label={ariaLabel}
        className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus [scrollbar-gutter:stable]"
        tabIndex={0}
        onScroll={(event) => {
          const node = event.currentTarget;
          following.current = node.scrollHeight - node.scrollTop - node.clientHeight < 96;
          if (following.current) setNewContent(false);
        }}
        ref={timeline}
        role="log"
      >
        {!items.length && !progress ? emptyState ?? (
          <div className="chat-empty-state mx-auto grid min-h-full w-full max-w-3xl content-center gap-5 px-4 py-8">
            <div>
              <div className="flex size-9 items-center justify-center rounded-md bg-surface-3 text-ink-subtle"><AppIcon className="size-5" icon={ChatCircleDots} /></div>
              <h2 className="mt-3 text-base font-semibold text-ink">Ask about {repositoryName}</h2>
              <p className="mt-1 max-w-xl text-sm leading-6 text-ink-subtle">Use the current scan evidence to understand risk, trace vulnerable packages, or plan a fix. The companion stays read-only.</p>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              {starters.map((starter) => <Button disabled={startersDisabled} className="h-auto justify-start whitespace-normal p-3 text-left leading-5" key={starter} variant="secondary" onClick={() => onStarter(starter)}>{starter}</Button>)}
            </div>
          </div>
        ) : (
          <div className="chat-timeline-content mx-auto flex min-h-full min-w-0 w-full max-w-3xl flex-col justify-end px-4 pb-8 pt-5 sm:px-6">
            {hasEarlier || items.length > visibleLimit ? <Button className="mx-auto mb-4 flex" disabled={historyLoading} size="sm" variant="ghost" onClick={()=>void loadEarlier()}>{historyLoading?"Loading earlier messages…":"Load earlier messages"}</Button> : null}
            {onLoadLatest?<Button className="mx-auto mb-4 flex" size="sm" variant="secondary" disabled={historyLoading} onClick={jumpToLatest}>Latest messages</Button>:null}
            <div className="grid gap-4">{visibleItems.map(({ item, providerLabel: itemProviderLabel }) => <TimelineRenderer showStreaming={item.status === "streaming" && !progress} item={item} key={item.id} providerLabel={itemProviderLabel} onCitation={onCitation} />)}</div>
          </div>
        )}
      </div>
      {newContent ? (
        <Button className="absolute bottom-3 left-1/2 -translate-x-1/2 shadow-sm" size="sm" variant="secondary" onClick={jumpToLatest}>
          <AppIcon className="size-4" icon={ArrowDown} />Latest response
        </Button>
      ) : null}
    </div>
  );
}

function ChatTurnProgress({ label, startedAt, providerLabel, responding }: NonNullable<ChatTimelineProps["progress"]>) {
  return <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-ink-subtle">
    {providerLabel ? <span className="font-medium uppercase tracking-wide">{providerLabel}</span> : null}
    <span className="inline-flex items-center gap-2" role="status" aria-live="polite" aria-atomic="true">
      <span className="inline-flex gap-1" aria-hidden="true"><i className="size-1 animate-pulse rounded-full bg-current motion-reduce:animate-none" /><i className="size-1 animate-pulse rounded-full bg-current [animation-delay:150ms] motion-reduce:animate-none" /><i className="size-1 animate-pulse rounded-full bg-current [animation-delay:300ms] motion-reduce:animate-none" /></span>
      <span className={responding ? "sr-only" : undefined}>{label}</span>
    </span>
    <TurnElapsed startedAt={startedAt} />
  </div>;
}

function TurnElapsed({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [startedAt]);
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
  const elapsed = seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return <span aria-live="off" className="ml-auto tabular-nums text-ink-subtle" title="Time since your message was sent">Elapsed {elapsed}</span>;
}

export function labelTimelineItems(items: CompanionTimelineItem[], segmentProviderLabels: readonly string[], fallbackProviderLabel: string,providerLabelsByItem?:Readonly<Record<string,string>>) {
  let segmentIndex = 0;
  const warnings = new Set<string>();
  return items.flatMap((item) => {
    if (item.kind === "assistant_message" && item.status === "streaming" && !item.text) return [];
    // The shared CLI's development-feature notice is process configuration,
    // not conversation content. Also hide copies saved by older versions.
    if ((item.kind === "warning" || item.kind === "error")
      && item.error?.code === "codex_warning"
      && item.error.message.startsWith("Under-development features enabled:")) return [];
    // Older caches stored Codex configuration warnings as failed error items.
    if (item.kind === "error" && (item.error?.code === "codex_warning" || item.error?.code === "action_denied")) {
      item = { ...item, kind: "warning", status: "completed" };
    }
    if (item.kind === "warning") {
      const key = JSON.stringify([providerLabelsByItem?.[item.id]??segmentIndex, item.error?.code, item.error?.message ?? item.text]);
      if (warnings.has(key)) return [];
      warnings.add(key);
    }
    const labeled = {
      item,
      providerLabel: providerLabelsByItem?.[item.id] ?? segmentProviderLabels[segmentIndex] ?? fallbackProviderLabel,
    };
    if (item.kind === "activity" && item.activity?.kind === "provider_transition") segmentIndex += 1;
    return [labeled];
  });
}

const TimelineItem = memo(function TimelineItem({ item, providerLabel, onCitation, showStreaming }: { showStreaming: boolean; item: CompanionTimelineItem; providerLabel: string; onCitation: (citation: CompanionCitation) => void }) {
  countPerformanceRender(item.status==="streaming"?"streamingItemRenders":"completedItemRenders");
  if (item.kind === "warning") {
    return <Alert variant="warning"><AlertTitle>{item.error?.code === "action_denied" ? "Read-only action denied" : item.error?.code === "codex_warning" ? `${providerLabel} configuration warning` : `${providerLabel} warning`}</AlertTitle><AlertDescription>{item.error?.message ?? item.text}</AlertDescription></Alert>;
  }
  if (item.kind === "activity" && item.activity?.kind === "provider_transition") {
    return <div className="flex items-center gap-2 py-1 text-xs text-ink-subtle before:h-px before:flex-1 before:bg-hairline after:h-px after:flex-1 after:bg-hairline"><span>{item.activity.label}{item.activity.detail ? ` · ${item.activity.detail}` : ""}</span></div>;
  }
  if (item.kind === "activity") return null;
  if (item.kind === "error") {
    return <Alert variant="warning"><AlertTitle>{item.error?.code === "action_denied" ? "Read-only action denied" : "Response interrupted"}</AlertTitle><AlertDescription>{item.error?.message ?? item.text}</AlertDescription></Alert>;
  }
  const assistant = item.kind === "assistant_message";
  const visibleText = assistant ? stripCitationMarkup(item.text) : visibleUserPrompt(item.text);
  return (
    <article data-message-id={item.id} className={assistant ? "min-w-0 max-w-full pr-2 [overflow-wrap:anywhere]" : "ml-auto min-w-0 max-w-[88%] [overflow-wrap:anywhere] rounded-xl bg-message-user-surface px-3.5 py-2.5"}>
      <div className="mb-1 flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-ink-subtle">
        <span>{assistant ? providerLabel : "You"}</span>
        {showStreaming && item.status === "streaming" ? <span aria-label="Streaming response" className="inline-flex gap-0.5"><i className="size-1 animate-pulse rounded-full bg-current motion-reduce:animate-none" /><i className="size-1 animate-pulse rounded-full bg-current [animation-delay:120ms] motion-reduce:animate-none" /><i className="size-1 animate-pulse rounded-full bg-current [animation-delay:240ms] motion-reduce:animate-none" /></span> : null}
      </div>
      {assistant ? (
        <Suspense fallback={<StreamingText text={visibleText} />}><ChatMarkdown text={visibleText} streaming={item.status === "streaming"} /></Suspense>
      ) : <p className="whitespace-pre-wrap break-words text-sm leading-6 text-ink">{visibleText}</p>}
      {item.citations.length ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {item.citations.map((citation) => (
            <Button className="h-7 max-w-full gap-1 px-2 text-xs" key={citation.sourceId} size="xs" variant="secondary" onClick={() => onCitation(citation)}>
              <AppIcon className="size-3" icon={citation.kind === "file" || citation.kind === "changes" ? FileText : LinkSimple} />
              <span className="truncate">{citation.label}</span>{citation.stale ? <span className="text-status-warning-foreground">stale</span> : null}
            </Button>
          ))}
        </div>
      ) : null}
      {assistant && item.status !== "streaming" && visibleText ? <CopyButton className="mt-2" label="Copy answer as Markdown" text={visibleText} /> : null}
    </article>
  );
});

const ProfiledTimelineItem=memo(function ProfiledTimelineItem(props:React.ComponentProps<typeof TimelineItem>){
  return <Profiler id={props.item.status==="streaming"?"streaming-item":"completed-item"} onRender={recordPerformanceCommit}><TimelineItem {...props}/></Profiler>;
});
const TimelineRenderer=import.meta.env.VITE_CVEC_PERF_PROFILER==="on"?ProfiledTimelineItem:TimelineItem;

const TextChunk=memo(function TextChunk({text}:{text:string}){return <p className="whitespace-pre-wrap break-words text-sm leading-6 text-ink" style={{contentVisibility:"auto",containIntrinsicSize:"auto 180px"}}>{text}</p>;});
function StreamingText({text}:{text:string}) {
  const chunks=[];for(let offset=0;offset<text.length;offset+=4000)chunks.push(text.slice(offset,offset+4000));
  return <div>{chunks.map((chunk,index)=><TextChunk key={index} text={chunk} />)}</div>;
}


function stripCitationMarkup(text: string): string {
  return text.replace(/\s*\[source:[^\]\r\n]{1,2048}\]/g, "");
}

export function visibleUserPrompt(text: string): string {
  const markerIndex = text.indexOf("<repository_chat_context_v1>");
  return markerIndex < 0 ? text : text.slice(0, markerIndex).trimEnd();
}
