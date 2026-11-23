import { useCallback } from "react";
import { companionStore, shallowCompanionSelection, useCompanionSelector } from "@/lib/companionState";
import { ChatComposer, type ChatComposerProps } from "./ChatComposer";
import { DraftStorageNotice } from "./DraftStorageNotice";

type Props = Pick<ChatComposerProps, "contextChips" | "contextControls" | "disabled" | "inputDisabled" | "placeholder" | "statusMessage" | "focusRequest" | "onOpenSettings" | "onSend" | "onRetry" | "onSelectModel"> & { repositoryId: number };

export function ConnectedChatComposer({ repositoryId, ...props }: Props) {
  const data = useCompanionSelector((state) => {
    const chat = state.repositories[repositoryId];
    return {
      draft: chat.draft, selection: chat.selection, models: chat.selectionModels,
      draftLoaded: chat.draftLoaded, draftStorageError: chat.draftStorageError,
      selectionLoading: chat.selectionLoading, pending: chat.pending,
      connections: state.connections, usage: chat.selection.connectionId === "codex_default" ? state.usage : null,
      threadUsage: chat.activeThreadId ? chat.usageByThread[chat.activeThreadId] ?? null : null,
      usageAttribution: chat.activeThreadId ? chat.usageAttributionByThread?.[chat.activeThreadId] : undefined,
      historyCachedOnly: chat.historyUnavailable, error: chat.error,
    };
  }, shallowCompanionSelection);
  const onDraftChange = useCallback((value: string) => companionStore.setDraft(repositoryId, value), [repositoryId]);
  const onEffortChange = useCallback((value: string) => companionStore.setRepositoryEffort(repositoryId, value), [repositoryId]);
  const onStop = useCallback(() => void companionStore.stop(repositoryId), [repositoryId]);
  const attribution = data.usageAttribution;
  const usageCurrent = Boolean(attribution?.modelId && attribution.connectionId === data.selection.connectionId && attribution.modelId === data.selection.modelId);
  const usageLabel = attribution ? `${data.connections.find((connection) => connection.connection.id === attribution.connectionId)?.connection.displayName ?? attribution.connectionId}${attribution.modelId ? ` · ${attribution.modelId}` : " · model not recorded"}` : "Provider/model not recorded";
  return <ChatComposer {...props} draft={data.draft} selection={data.selection} models={data.models}
    statusMessage={data.draftLoaded === false && !data.draftStorageError ? "Restoring local draft…" : props.statusMessage}
    inputDisabled={props.inputDisabled || data.draftLoaded === false}
    draftNotice={<DraftStorageNotice error={data.draftStorageError} onRetry={() => void companionStore.retryDraftStorage(repositoryId)} />}
    connections={data.connections} selectionLoading={data.selectionLoading}
    preparing={Boolean(data.pending && !data.pending.turnId)} sending={Boolean(data.pending)}
    canSend={Boolean(data.draftLoaded !== false && data.draft.trim() && !data.pending && data.selection.modelId && !props.disabled && !data.selectionLoading)}
    historyCachedOnly={data.historyCachedOnly} retryableError={Boolean(data.error?.retryable && data.draft.trim())}
    providerUsage={data.usage} threadUsage={data.threadUsage}
    contextUsageCurrent={usageCurrent} contextUsageLabel={usageLabel}
    onDraftChange={onDraftChange} onEffortChange={onEffortChange} onStop={onStop} />;
}
