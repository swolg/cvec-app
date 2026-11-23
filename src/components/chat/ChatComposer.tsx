import { CaretDown, Check, Cpu, Gauge, PaperPlaneTilt, Plus, Stop } from "@phosphor-icons/react";
import { memo, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import openAiOnDark from "@/assets/provider-logos/openai-on-dark.svg?no-inline";
import openAiOnLight from "@/assets/provider-logos/openai-on-light.svg?no-inline";
import openCodeOnDark from "@/assets/provider-logos/opencode-on-dark.svg?no-inline";
import openCodeOnLight from "@/assets/provider-logos/opencode-on-light.svg?no-inline";
import { countPerformanceRender } from "@/lib/performanceCounters";
import { CHAT_DRAFT_MAX_LENGTH, limitDraftText } from "@/lib/chatDraftText";

import { AppIcon } from "@/components/ui/app-icon";
import { Button, IconButton } from "@/components/ui/button";
import { SearchField, Select } from "@/components/ui/form-controls";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "@/components/ui/overlays";
import type { CompanionComposerSelection, CompanionModel, CompanionTokenUsage, CompanionUsageStatus, ProviderConnectionSnapshot, ProviderDriverKind } from "@/lib/types";

const providerLogos: Record<Exclude<ProviderDriverKind, "grok">, { light: string; dark: string; sizeClass: string }> = {
  codex: { light: openAiOnLight, dark: openAiOnDark, sizeClass: "size-8" },
  open_code: { light: openCodeOnLight, dark: openCodeOnDark, sizeClass: "size-5" },
};

export interface ChatComposerProps {
  inputLabel?: string;
  sendLabel?: string;
  maxLength?: number;
  focusRequest?: string | number | null;
  draft: string;
  selection: CompanionComposerSelection;
  models: CompanionModel[];
  connections: ProviderConnectionSnapshot[];
  selectionLoading: boolean;
  threadUsage: CompanionTokenUsage | null;
  contextUsageCurrent?: boolean;
  contextUsageLabel?: string;
  providerUsage: CompanionUsageStatus | null;
  contextControls: ReactNode;
  contextChips: ReactNode;
  statusMessage: string;
  draftNotice?: ReactNode;
  placeholder: string;
  disabled: boolean;
  inputDisabled?: boolean;
  sending: boolean;
  preparing?: boolean;
  canSend: boolean;
  historyCachedOnly: boolean;
  retryableError: boolean;
  onDraftChange: (value: string) => void;
  onSelectModel: (connectionId: string, modelId: string) => void;
  onEffortChange: (effort: string) => void;
  onOpenSettings?: () => void;
  onSend: () => void;
  onRetry: () => void;
  onStop: () => void;
}

export const ChatComposer = memo(function ChatComposer({
  inputLabel = "Message the security companion",
  sendLabel = "Send message",
  maxLength = CHAT_DRAFT_MAX_LENGTH,
  focusRequest,
  draft,
  selection,
  models,
  connections,
  selectionLoading,
  threadUsage,
  contextUsageCurrent = true,
  contextUsageLabel,
  providerUsage,
  contextControls,
  contextChips,
  statusMessage,
  draftNotice,
  placeholder,
  disabled,
  inputDisabled = disabled,
  sending,
  preparing = false,
  canSend,
  historyCachedOnly,
  retryableError,
  onDraftChange,
  onSelectModel,
  onEffortChange,
  onOpenSettings,
  onSend,
  onRetry,
  onStop,
}: ChatComposerProps) {
  countPerformanceRender("composerRenders");
  const textarea = useRef<HTMLTextAreaElement>(null);
  const clippedPaste = useRef(false);
  const [shortenedDraft, setShortenedDraft] = useState<string | null>(null);
  const textShortened = shortenedDraft !== null && shortenedDraft === draft;
  const statusId = useId();
  const hintId = useId();
  const lengthNoticeId = useId();
  const selectedModel = models.find((model) => model.id === selection.modelId) ?? null;

  useLayoutEffect(() => {
    const node = textarea.current;
    if (!node) return;
    node.style.height = "0px";
    node.style.height = `${Math.min(node.scrollHeight, 192)}px`;
  }, [draft]);

  useLayoutEffect(() => {
    const node = textarea.current;
    if (focusRequest == null || !node || inputDisabled) return;
    node.focus();
    node.setSelectionRange(node.value.length, node.value.length);
  }, [focusRequest]);

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    clippedPaste.current = false;
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (canSend) onSend();
    }
  }

  return (
    <div className="chat-composer shrink-0 border-t border-hairline bg-surface-2 px-2 pb-2 pt-2 sm:px-3">
      <div className="mx-auto max-w-3xl">
        {draftNotice}
        {historyCachedOnly ? <p className="mb-1.5 text-xs text-ink-subtle">Local conversation restored. Your next message can continue in a fresh provider segment.</p> : null}
        {contextChips ? <div aria-label="Attached context" role="region" tabIndex={0} className="mb-1.5 flex max-h-[min(7rem,18vh)] flex-wrap items-center gap-1.5 overflow-y-auto rounded-md outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus [scrollbar-gutter:stable]">{contextChips}</div> : null}
        <div className="chat-composer-input rounded-xl border border-hairline-strong bg-surface-1 px-2.5 pb-2 pt-1.5 shadow-sm focus-within:border-primary-focus focus-within:ring-2 focus-within:ring-primary-focus/20">
          <textarea
            aria-label={inputLabel}
            aria-describedby={`${hintId}${statusMessage ? ` ${statusId}` : ""}${textShortened ? ` ${lengthNoticeId}` : ""}`}
            maxLength={maxLength}
            className="max-h-[min(12rem,20dvh)] min-h-11 w-full resize-none overflow-y-auto bg-transparent px-1 py-1.5 text-sm leading-6 text-ink outline-none placeholder:text-ink-subtle"
            disabled={inputDisabled}
            onChange={(event) => {
              const value = event.target.value;
              const next = limitDraftText(value, maxLength);
              setShortenedDraft(((clippedPaste.current && next.length >= maxLength - 1) || next.length < value.length) ? next : null);
              clippedPaste.current = false;
              onDraftChange(next);
            }}
            onKeyDown={onKeyDown}
            onPaste={(event) => {
              const node = event.currentTarget;
              const remaining = maxLength - node.value.length + node.selectionEnd - node.selectionStart;
              const pastedText = event.clipboardData.getData("text").replace(/\r\n?/g, "\n");
              clippedPaste.current = pastedText.length > remaining;
              setShortenedDraft(clippedPaste.current ? draft : null);
            }}
            placeholder={placeholder}
            ref={textarea}
            rows={1}
            value={draft}
          />
          <div className="chat-composer-toolbar flex min-w-0 flex-wrap items-center gap-1.5 pt-1">
            <ConnectionModelPicker
              connections={connections}
              loading={selectionLoading}
              selection={selection}
              onOpenSettings={onOpenSettings}
              onSelect={onSelectModel}
            />
            {selectedModel && selectedModel.reasoningEfforts.length > 1 ? (
              <Select aria-label="Reasoning effort" className="h-8 w-auto min-w-28 text-xs" value={selection.effort ?? selectedModel.reasoningEfforts[0]} onValueChange={onEffortChange}>
                {selectedModel.reasoningEfforts.map((effort) => (
                  <option key={effort} value={effort}>
                    <ReasoningLevelLabel effort={effort} efforts={selectedModel.reasoningEfforts} />
                  </option>
                ))}
              </Select>
            ) : null}
            {contextControls}
            <div className="ml-auto flex items-center gap-1.5">
              <ContextWindowMeter providerUsage={providerUsage} usage={threadUsage} current={contextUsageCurrent} attribution={contextUsageLabel} />
              {sending ? (
                <Button aria-label={preparing ? "Preparing message" : "Stop response"} disabled={preparing} size="sm" variant="secondary" onClick={onStop}>{preparing ? <span>Preparing…</span> : <><AppIcon className="size-4" icon={Stop} weight="fill" /><span className="hidden sm:inline">Stop</span></>}</Button>
              ) : (
                <IconButton aria-label={sendLabel} title={`${sendLabel} (Enter)`} disabled={!canSend} onClick={onSend}><AppIcon className="size-4" icon={PaperPlaneTilt} weight="fill" /></IconButton>
              )}
            </div>
          </div>
        </div>
        {textShortened ? <p id={lengthNoticeId} role="status" className="chat-composer-length-note mt-1 text-xs text-status-warning-foreground">Text was shortened to fit the input limit.</p> : null}
        <p id={hintId} className="chat-composer-hint mt-1.5 text-right text-xs text-ink-subtle">Enter to send · Shift+Enter for a new line</p>
        {statusMessage || retryableError ? <div className="chat-composer-status mt-1.5 flex min-h-5 flex-wrap items-center justify-center gap-2 text-xs text-ink-subtle" aria-live="polite">
          <span id={statusId} title={statusMessage}>{statusMessage}</span>
          {retryableError ? <Button className="h-6 px-2 text-xs" size="xs" variant="ghost" onClick={onRetry}>Retry</Button> : null}
        </div> : null}
      </div>
    </div>
  );
});

function ReasoningLevelLabel({ effort, efforts }: { effort: string; efforts: string[] }) {
  const optionIndex = Math.max(0, efforts.indexOf(effort));
  const segmentCount = Math.min(Math.max(efforts.length, 1), 6);
  const activeSegments = Math.max(1, Math.ceil(((optionIndex + 1) / efforts.length) * segmentCount));

  return (
    <span className="flex min-w-0 items-center gap-1.5" data-reasoning-level={effort}>
      <span aria-hidden="true" className="flex h-3 shrink-0 items-end gap-0.5" data-reasoning-indicator="bars">
        {Array.from({ length: segmentCount }, (_, index) => {
          const active = index < activeSegments;
          const height = segmentCount === 1 ? 8 : 4 + (index / (segmentCount - 1)) * 8;
          return <span className={`w-1 rounded-sm ${active ? "bg-primary" : "bg-hairline-strong"}`} data-active={active} key={index} style={{ height }} />;
        })}
      </span>
      <span className="truncate">{reasoningEffortLabel(effort)}</span>
    </span>
  );
}

function reasoningEffortLabel(effort: string): string {
  const knownLabels: Record<string, string> = {
    none: "Off",
    minimal: "Minimal",
    low: "Low",
    medium: "Medium",
    high: "High",
    xhigh: "Extra high",
    max: "Max",
    ultra: "Ultra",
  };
  const normalized = effort.trim().toLowerCase();
  return knownLabels[normalized]
    ?? normalized.replaceAll(/[-_]+/g, " ").replace(/\b\w/g, (character) => character.toUpperCase());
}

interface ConnectionModelPickerProps {
  selection: CompanionComposerSelection;
  connections: ProviderConnectionSnapshot[];
  loading: boolean;
  onSelect: (connectionId: string, modelId: string) => void;
  onOpenSettings?: () => void;
}

export function ConnectionModelPicker({ selection, connections, loading, onSelect, onOpenSettings }: ConnectionModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const connection = connections.find((candidate) => candidate.connection.id === selection.connectionId) ?? null;
  const model = connection?.models.find((candidate) => candidate.id === selection.modelId) ?? null;
  const ready = connections.filter((candidate) => candidate.connection.enabled && !["setup_required", "unavailable", "incompatible", "disabled"].includes(candidate.health));
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const selectedReadyConnectionId = ready.some((candidate) => candidate.connection.id === selection.connectionId)
    ? selection.connectionId
    : ready[0]?.connection.id ?? "";
  const [visibleConnectionId, setVisibleConnectionId] = useState(selectedReadyConnectionId);
  const visibleConnection = ready.find((candidate) => candidate.connection.id === visibleConnectionId)
    ?? ready.find((candidate) => candidate.connection.id === selectedReadyConnectionId)
    ?? ready[0]
    ?? null;
  const entries = useMemo(() => {
    if (!open) return [];
    countPerformanceRender("modelPickerProjections");
    return modelPickerEntries(connections.filter((candidate) => candidate.connection.enabled && !["setup_required", "unavailable", "incompatible", "disabled"].includes(candidate.health)), visibleConnection?.connection.id ?? null, normalizedQuery);
  }, [open, connections, visibleConnection?.connection.id, normalizedQuery]);

  function onOpenChange(nextOpen: boolean) {
    setOpen(nextOpen);
    setQuery("");
    if (nextOpen) setVisibleConnectionId(selectedReadyConnectionId);
  }

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger render={<Button aria-label="AI connection and model" className="chat-model-picker h-9 min-w-0 max-w-56 gap-1.5 px-2 text-xs" disabled={loading} variant="ghost" />}>
        {connection ? <ProviderConnectionIcon driver={connection.connection.driver} /> : null}
        <span className="truncate">{loading ? "Checking selection…" : model?.displayName ?? selection.modelId ?? "Choose model"}</span>
        {connection ? <span className="max-w-24 truncate text-ink-subtle" title={connection.connection.displayName}>{connection.connection.displayName}</span> : null}
        <AppIcon className="size-3" icon={CaretDown} />
      </PopoverTrigger>
      <PopoverContent align="start" className="flex max-h-[var(--available-height)] w-[min(22.5rem,calc(100vw-2rem))] flex-col overflow-hidden p-0">
        <PopoverTitle className="sr-only">Choose connection and model</PopoverTitle>
        <div className="shrink-0 border-b border-hairline p-2">
          <SearchField aria-label="Search AI models" autoFocus className="h-8 border-0 bg-transparent pl-8 shadow-none focus:ring-0" onChange={(event) => setQuery(event.target.value)} placeholder="Search models…" value={query} />
        </div>
        <div className={`flex min-h-0 ${ready.length ? "h-[19rem]" : "h-24"}`} data-model-viewport="true">
          {!normalizedQuery && ready.length > 1 ? (
            <div aria-label="AI connections" className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-hairline bg-surface-2 p-1" role="navigation">
              {ready.map((snapshot) => {
                const selected = snapshot.connection.id === visibleConnection?.connection.id;
                return (
                  <Button
                    aria-current={selected ? "true" : undefined}
                    aria-label={`${snapshot.connection.displayName} models`}
                    className="size-10 shrink-0 justify-center p-0"
                    key={snapshot.connection.id}
                    onClick={() => setVisibleConnectionId(snapshot.connection.id)}
                    title={snapshot.connection.displayName}
                    variant={selected ? "secondary" : "ghost"}
                  >
                    <ProviderConnectionIcon driver={snapshot.connection.driver} />
                  </Button>
                );
              })}
            </div>
          ) : null}
          <div className="h-full min-h-0 flex-1 overflow-y-auto p-1.5" data-model-list="true">
            {entries.map(({ snapshot, model: candidate, upstreamLabel }) => {
              const selected = snapshot.connection.id === selection.connectionId && candidate.id === selection.modelId;
              return (
                <Button
                  aria-pressed={selected}
                  className="h-auto min-h-12 w-full justify-start gap-2 px-2 py-2 text-left whitespace-normal"
                  key={`${snapshot.connection.id}:${candidate.id}`}
                  onClick={() => { onSelect(snapshot.connection.id, candidate.id); onOpenChange(false); }}
                  variant={selected ? "secondary" : "ghost"}
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium text-ink">{candidate.displayName}</span>
                    <span className="mt-0.5 block truncate text-xs font-normal text-ink-subtle">{snapshot.connection.displayName}{upstreamLabel ? ` · ${upstreamLabel}` : ""}</span>
                  </span>
                  {selected ? <AppIcon aria-hidden className="size-3.5 text-primary" icon={Check} weight="bold" /> : null}
                </Button>
              );
            })}
            {!ready.length ? <p className="px-2 py-6 text-center text-xs text-ink-subtle">No AI connection is ready.</p> : null}
            {ready.length && !entries.length ? <p className="px-2 py-6 text-center text-xs text-ink-subtle">No models found.</p> : null}
          </div>
        </div>
        {onOpenSettings ? <Button className="w-full shrink-0 justify-start rounded-none border-t border-hairline" size="sm" variant="ghost" onClick={() => { onOpenChange(false); onOpenSettings(); }}><AppIcon className="size-4" icon={Plus} />Manage connections</Button> : null}
      </PopoverContent>
    </Popover>
  );
}

function modelPickerEntries(connections: ProviderConnectionSnapshot[], visibleConnectionId: string | null, query: string) {
  const searchedConnections = query
    ? connections
    : connections.filter((snapshot) => snapshot.connection.id === visibleConnectionId);
  return searchedConnections.flatMap((snapshot) => snapshot.models.flatMap((model) => {
    if (model.hidden) return [];
    const upstream = snapshot.upstreamProviders.find((candidate) => candidate.id === model.upstreamProviderId);
    const upstreamLabel = upstream?.displayName ?? model.upstreamProviderId ?? null;
    const searchText = `${snapshot.connection.displayName} ${snapshot.connection.driver} ${upstreamLabel ?? ""} ${model.displayName} ${model.id} ${model.description ?? ""}`.toLocaleLowerCase();
    if (query && !searchText.includes(query)) return [];
    return [{ snapshot, model, upstreamLabel }];
  }));
}

function ProviderConnectionIcon({ driver }: { driver: ProviderDriverKind }) {
  const logo = driver === "grok" ? null : providerLogos[driver];
  return (
    <span aria-hidden="true" className="grid h-9 w-8 shrink-0 place-items-center" data-provider-icon={driver}>
      {logo ? (
        <>
          <img alt="" className={`${logo.sizeClass} object-contain dark:hidden`} src={logo.light} />
          <img alt="" className={`${logo.sizeClass} hidden object-contain dark:block`} src={logo.dark} />
        </>
      ) : <AppIcon icon={Cpu} className="size-5" />}
    </span>
  );
}

export function ContextWindowMeter({ usage, providerUsage, current = true, attribution }: { usage: CompanionTokenUsage | null; providerUsage: CompanionUsageStatus | null; current?: boolean; attribution?: string }) {
  const used = usage?.contextUsed ?? null;
  const maximum = usage?.contextWindow ?? null;
  if (used === null || maximum === null || maximum <= 0) {
    if (!providerUsage?.supported) return null;
    return <ProviderUsage usage={providerUsage} />;
  }
  const percent = Math.min(100, Math.max(0, (used / maximum) * 100));
  return (
    <Popover>
      <PopoverTrigger render={<Button aria-label={current ? `Context window ${percent.toFixed(0)} percent used` : "Previous context reading"} className="h-8 gap-1.5 px-2 text-xs" variant="ghost" />}>
        {current ? <><span aria-hidden="true" className="h-1.5 w-10 overflow-hidden rounded-full bg-surface-3"><span className="block h-full rounded-full bg-primary" style={{ width: `${percent}%` }} /></span><span>{percent.toFixed(0)}%</span></> : <span>Previous context</span>}
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72">
        <PopoverTitle className="text-sm font-semibold text-ink">{current ? "Current context estimate" : "Last recorded context"}</PopoverTitle>
        {attribution ? <p className="mt-1 break-words text-xs text-ink-subtle">{attribution}</p> : null}
        {!current ? <p className="mt-2 text-xs text-ink-subtle">This reading is from an earlier turn. No current context measurement is available for the selected model.</p> : null}
        <div aria-label={`${used.toLocaleString()} of ${maximum.toLocaleString()} estimated context tokens used`} aria-valuemax={maximum} aria-valuemin={0} aria-valuenow={Math.min(used, maximum)} className="mt-3 h-2 overflow-hidden rounded-full bg-surface-3" role="progressbar"><div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} /></div>
        <p className="mt-2 text-xs text-ink-subtle">{used.toLocaleString()} / {maximum.toLocaleString()} tokens · {percent.toFixed(1)}%</p>
        <p className="mt-1 text-xs text-ink-subtle">Estimated from the latest provider turn; lifetime thread usage is not treated as active context.</p>
        {providerUsage?.supported ? <div className="mt-3 border-t border-hairline pt-3"><ProviderUsageDetails usage={providerUsage} /></div> : null}
      </PopoverContent>
    </Popover>
  );
}

function ProviderUsage({ usage }: { usage: CompanionUsageStatus }) {
  return <Popover><PopoverTrigger render={<Button aria-label="Provider usage" className="h-8 px-2 text-xs" variant="ghost" />}><AppIcon className="size-3.5" icon={Gauge} />Provider usage</PopoverTrigger><PopoverContent align="end" className="w-72"><ProviderUsageDetails usage={usage} /></PopoverContent></Popover>;
}

function ProviderUsageDetails({ usage }: { usage: CompanionUsageStatus }) {
  return <div><p className="text-xs font-medium text-ink">Provider usage</p><dl className="mt-2 grid grid-cols-2 gap-2 text-xs"><dt className="text-ink-subtle">Primary limit</dt><dd className="text-right text-ink">{usage.primary ? `${usage.primary.usedPercent.toFixed(1)}% used` : "Not reported"}</dd><dt className="text-ink-subtle">Lifetime tokens</dt><dd className="text-right text-ink">{usage.lifetimeTokens?.toLocaleString() ?? "Not reported"}</dd></dl></div>;
}
