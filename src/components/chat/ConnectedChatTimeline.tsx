import { useCallback, useMemo, useState } from "react";
import { companionStore, shallowCompanionSelection, useCompanionSelector } from "@/lib/companionState";
import { ChatTimeline } from "./ChatTimeline";
import type { CompanionCitation, CompanionTimelineItem } from "@/lib/types";

const EMPTY_ITEMS: CompanionTimelineItem[] = [];
const EMPTY_SEGMENTS: NonNullable<ReturnType<typeof companionStore.getSnapshot>["repositories"][number]>["segmentsByThread"][number] = [];
const EMPTY_ORDINALS: Record<string, number> = {};
const PHASES = { preparing: "Preparing context…", starting: "Connecting…", waiting: "Waiting for response…", thinking: "Thinking…", responding: "Responding…", working: "Working…" };

export function ConnectedChatTimeline({ repositoryId, repositoryName, starters, onStarter, onCitation, onError }: {
  repositoryId: number; repositoryName: string; starters: readonly string[];
  onStarter: (value: string) => void;
  onCitation: (citation: CompanionCitation) => void; onError: (message: string) => void;
}) {
  const data = useCompanionSelector((state) => {
    const chat = state.repositories[repositoryId];
    const id = chat.activeThreadId;
    return {
      id, items: id ? chat.itemsByThread[id] ?? EMPTY_ITEMS : chat.pending?.threadId === null ? chat.pending.userItem : EMPTY_ITEMS,
      segments: id ? chat.segmentsByThread[id] ?? EMPTY_SEGMENTS : EMPTY_SEGMENTS,
      ordinals: id ? chat.historyProviderOrdinals?.[id] ?? EMPTY_ORDINALS : EMPTY_ORDINALS,
      page: id ? chat.historyPages?.[id] : undefined,
      connections: state.connections, connectionId: chat.selection.connectionId,
      startersDisabled: starters.length > 0 && chat.draftLoaded === false,
      pending: chat.pending?.threadId === id ? chat.pending : null,
    };
  }, shallowCompanionSelection);
  const items = useMemo(() => (Array.isArray(data.items) ? data.items : [data.items]).filter((item) => item.kind !== "activity" || item.activity?.kind !== "context"), [data.items]);
  const providerLabel = data.connections.find((entry) => entry.connection.id === data.connectionId)?.connection.displayName ?? "AI";
  const segmentLabels = useMemo(() => data.segments.map((segment) => data.connections.find((entry) => entry.connection.id === segment.connectionId)?.connection.displayName ?? segment.provider), [data.segments, data.connections]);
  const labels = useMemo(() => Object.fromEntries(Object.entries(data.ordinals).map(([id, ordinal]) => [id, segmentLabels[data.segments.findIndex((segment) => segment.ordinal === ordinal)] ?? providerLabel])), [data.ordinals, data.segments, segmentLabels, providerLabel]);
  const [loadingKey, setLoadingKey] = useState<string | null>(null);
  const key = `${repositoryId}:${data.id ?? "draft"}`;
  const load = useCallback(async (latest: boolean) => {
    if (!data.id) return;
    setLoadingKey(key);
    try {
      if (latest) await companionStore.loadLatestHistory(repositoryId, data.id);
      else await companionStore.loadEarlierHistory(repositoryId, data.id);
    } catch (cause) { onError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setLoadingKey((current) => current === key ? null : current); }
  }, [data.id, key, repositoryId, onError]);
  const loadEarlier = useCallback(() => load(false), [load]);
  const loadLatest = useCallback(() => load(true), [load]);
  const progress = useMemo(() => data.pending ? {
    label: PHASES[data.pending.phase], startedAt: data.pending.startedAt,
    providerLabel: data.connections.find((entry) => entry.connection.id === data.pending?.connectionId)?.connection.displayName ?? providerLabel,
    responding: data.pending.phase === "responding",
  } : null, [data.pending, data.connections, providerLabel]);
  return <ChatTimeline conversationKey={key} repositoryName={repositoryName} providerLabel={providerLabel}
    segmentProviderLabels={segmentLabels} providerLabelsByItem={labels} items={items}
    progress={progress} starters={starters} startersDisabled={data.startersDisabled} onStarter={onStarter} onCitation={onCitation}
    hasEarlier={Boolean(data.page?.older)} historyLoading={loadingKey === key}
    onLoadEarlier={data.page ? loadEarlier : undefined} onLoadLatest={data.page?.before ? loadLatest : undefined} />;
}
