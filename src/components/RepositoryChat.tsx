import { ArrowClockwise, ChatCircleDots, FileText, Plus, SignIn, Warning, X } from "@phosphor-icons/react";
import { lazy, Suspense, useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { Icon } from "@phosphor-icons/react";

import { ChatProfiler } from "@/components/chat/ChatProfiler";
import { ChatContextPreview } from "@/components/chat/ChatContextPreview";
import { ConnectedChatComposer } from "@/components/chat/ConnectedChatComposer";
import { ConnectedChatTimeline } from "@/components/chat/ConnectedChatTimeline";
import { ConversationHeader } from "@/components/chat/ConversationHeader";
import { AppIcon } from "@/components/ui/app-icon";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button, IconButton } from "@/components/ui/button";
import { EmptyState, Spinner } from "@/components/ui/feedback";
import { Input, Tabs } from "@/components/ui/form-controls";
import { FindingSourceContent, ScanSourceContent } from "@/components/chat/ScanSourceContent";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "@/components/ui/overlays";
import { openExternalUrl } from "@/lib/api";
import { companionSearchMentions, companionSourceDetail } from "@/lib/companionApi";
import { companionStore, useCompanionRepositoryState, type CompanionSendPreview } from "@/lib/companionState";
import type { CompanionCitation, CompanionMention, CompanionSourceDetail, ContextAttachmentKind, FindingChatRequest, ProviderConnectionSnapshot, RepositoryChatScanStatus, RepoSummary } from "@/lib/types";

export { visibleUserPrompt } from "@/components/chat/ChatTimeline";

interface RepositoryChatProps {
  scanStatus: RepositoryChatScanStatus | null;
  contextStatus?: "loading" | "ready" | "error";
  analysisRequestKey?: number;
  focusComposerRequest?: number | null;
  repository: RepoSummary;
  findingRequest?: FindingChatRequest;
  onClearFinding?: () => void;
  onOpenFinding?: (fingerprint: string) => void;
  onOpenRepository?: () => void;
  onChooseLocalCheckout?: () => void;
  onOpenScannerSettings?: () => void;
  scannerUnavailable?: string | null;
  onOpenAiConnections?: () => void;
  onError: (message: string) => void;
}

const STARTERS = [
  "Explain highest-risk findings.",
  "Trace vulnerable packages.",
  "Plan remediation and verification.",
] as const;

const MENTION_KINDS: Array<{ kind: ContextAttachmentKind; label: string; description: string }> = [
  { kind: "finding", label: "Finding", description: "A current local CVE finding" },
  { kind: "package", label: "Package", description: "An exact scanned package coordinate" },
  { kind: "scan", label: "Scan", description: "A completed local scan" },
  { kind: "file", label: "File", description: "A safe repository-relative text file" },
  { kind: "changes", label: "Changes", description: "A bounded uncommitted diff" },
];

const BLOCKING_HEALTH = new Set(["setup_required", "unavailable", "incompatible", "disabled"]);

function connectionReady(snapshot: ProviderConnectionSnapshot | null): boolean {
  return Boolean(snapshot?.connection.enabled && !BLOCKING_HEALTH.has(snapshot.health));
}

const FixWorkspace = lazy(() => import("@/components/chat/FixWorkspacePanel").then((module) => ({ default: module.FixWorkspacePanel })));

export function RepositoryChat(props: RepositoryChatProps) {
  const requestedMode = props.findingRequest?.mode ?? "analysis";
  const [mode, setMode] = useState<"analysis" | "fix">(requestedMode);
  const modeId = useId();
  const state = useCompanionRepositoryState(props.repository.id, "shell");
  const chat = state.repositories[props.repository.id];
  const selection = chat?.selection;
  const useRepositorySelection = Boolean(chat?.loaded || selection?.modelId);
  const connectionId = useRepositorySelection ? selection?.connectionId : state.selectedConnectionId;
  const model = useRepositorySelection
    ? chat?.selectionModels.find((item) => item.id === selection?.modelId)
    : state.models.find((item) => item.id === state.selectedModel);
  const connection = state.connections.find((item) => item.connection.id === connectionId) ?? null;

  useEffect(() => {
    void companionStore.initialize().catch(() => undefined);
  }, []);

  useEffect(() => { setMode(requestedMode); }, [props.analysisRequestKey, props.findingRequest, requestedMode]);
  const focus = props.findingRequest;
  const focusId = focus && (focus.scanRunId ? ["scan", focus.scanRunId, focus.fingerprint].join(":") : focus.fingerprint);
  useEffect(() => {
    if (mode !== "analysis" || !focusId) return;
    const repositoryId = props.repository.id;
    companionStore.setFocusedFinding(repositoryId, focusId);
    return () => companionStore.setFocusedFinding(repositoryId, null);
  }, [props.repository.id, mode, focusId]);

  useEffect(() => {
    if (mode === "analysis" && focusId && chat?.loaded && chat.threads.some((thread) => thread.id === chat.activeThreadId && thread.archivedAt)) {
      companionStore.newConversation(props.repository.id);
    }
  }, [props.repository.id, mode, focusId, chat?.loaded, chat?.activeThreadId, chat?.threads]);
  return <div className="chat-mode-shell flex min-h-0 min-w-0 flex-1 flex-col">
    <div className="shrink-0 px-3 pt-1.5"><Tabs idPrefix={modeId} label="Repository chat mode" value={mode} onValueChange={setMode} options={[{ value: "analysis", label: "Analysis" }, { value: "fix", label: "Fix" }]} /></div>
    <div role="tabpanel" id={`${modeId}-panel-analysis`} aria-labelledby={`${modeId}-tab-analysis`} hidden={mode !== "analysis"} className={mode === "analysis" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
      {mode === "analysis" ? <RepositoryAnalysisChat {...props} /> : null}
    </div>
    <div role="tabpanel" id={`${modeId}-panel-fix`} aria-labelledby={`${modeId}-tab-fix`} hidden={mode !== "fix"} className={mode === "fix" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
      {mode === "fix" ? <Suspense fallback={<Spinner label="Loading fix workspace" />}><FixWorkspace key={props.repository.id} repository={props.repository} findingRequest={props.findingRequest} onOpenFinding={props.onOpenFinding} model={model?.id ?? null} modelLabel={model?.displayName ?? "Default model"} effort={(useRepositorySelection ? selection?.effort : state.selectedEffort) ?? null} connectionId={connectionId ?? ""}
        providerReady={Boolean(connectionReady(connection) && (connection?.connection.id !== "codex_default" || state.account?.signedIn !== false))} connections={state.connections} selectionLoading={chat?.selectionLoading ?? false}
        onSelectModel={(connectionId, modelId) => { void companionStore.selectRepositoryConnection(props.repository.id, connectionId, modelId, false).catch((cause) => props.onError(errorMessage(cause))); }}
        onEffortChange={(effort) => companionStore.setRepositoryEffort(props.repository.id, effort)} onOpenSettings={props.onOpenAiConnections}
        onOpenRepository={props.onOpenRepository} onChooseLocalCheckout={props.onChooseLocalCheckout}
        onOpenScannerSettings={props.onOpenScannerSettings} scannerUnavailable={props.scannerUnavailable} /></Suspense> : null}
    </div>
  </div>;
}

function RepositoryAnalysisChat({ scanStatus, contextStatus = "ready", repository, focusComposerRequest, findingRequest, onClearFinding, onOpenFinding, onOpenAiConnections, onError }: RepositoryChatProps) {
  const state = useCompanionRepositoryState(repository.id, "shell");
  const chat = state.repositories[repository.id];
  const [attachmentLabels, setAttachmentLabels] = useState<Record<string, string>>({});
  const activeThread = chat?.threads.find((thread) => thread.id === chat.activeThreadId) ?? null;
  const selection = chat?.selection ?? { repositoryId: repository.id, connectionId: "codex_default", modelId: null, effort: null };
  const selectedConnection = state.connections.find((item) => item.connection.id === selection.connectionId) ?? null;
  const selectedModel = chat?.selectionModels.find((model) => model.id === selection.modelId) ?? null;
  const selectedIsCodex = selectedConnection?.connection.driver === "codex";
  const runtime = state.runtime;
  const runtimeUnavailable = Boolean(selectedIsCodex && runtime && ["unavailable", "incompatible", "offline", "interrupted"].includes(runtime.phase));
  const items = activeThread ? chat?.itemsByThread[activeThread.id] ?? [] : chat?.pending?.threadId === null ? [chat.pending.userItem] : [];
  const archived = Boolean(activeThread?.archivedAt);
  const unavailable = Boolean(selectedConnection && !connectionReady(selectedConnection));
  const initializedRepository = useRef<number | null>(null);
  const runtimeStartRequested = useRef(false);
  const runtimeStartInFlight = useRef(false);
  const [runtimeStartError, setRuntimeStartError] = useState<string | null>(null);
  const mentionRequest = useRef(0);
  const sourceRequest = useRef(0);
  const contextTrigger = useRef<HTMLButtonElement>(null);
  const contextReturnFocus = useRef<HTMLButtonElement | null>(null);
  const citationReturnFocus = useRef<HTMLElement | null>(null);
  const mentionInput = useRef<HTMLInputElement>(null);
  const [contextMenuOpen, setContextMenuOpen] = useState(false);
  const [contextLoading, setContextLoading] = useState(false);
  const [contextError, setContextError] = useState<string | null>(null);
  const [sendReview, setSendReview] = useState<{ preview: CompanionSendPreview; revised: boolean } | null>(null);
  const [loginBusy, setLoginBusy] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [sourceFromContext, setSourceFromContext] = useState(false);
  const [source, setSource] = useState<CompanionSourceDetail | null>(null);
  const [sourceLoading, setSourceLoading] = useState(false);
  const [mentionKind, setMentionKind] = useState<ContextAttachmentKind | null>(null);
  const [mentionQuery, setMentionQuery] = useState("");
  const [mentionResults, setMentionResults] = useState<CompanionMention[]>([]);
  const [mentionLoading, setMentionLoading] = useState(false);
  const [starterFocusRequest, setStarterFocusRequest] = useState(0);
  const applyStarter = useCallback((value: string) => {
    const current = companionStore.getSnapshot().repositories[repository.id];
    if (current?.draftLoaded === false) return;
    const draft = current?.draft ?? "";
    companionStore.setDraft(repository.id, draft.trim() ? `${draft}\n\n${value}` : value);
    setStarterFocusRequest((current) => current + 1);
  }, [repository.id]);
  const composerFocusRequest = focusComposerRequest != null || starterFocusRequest > 0 ? `${focusComposerRequest ?? 0}:${starterFocusRequest}` : null;

  const startCodexRuntime = useCallback(async () => {
    if (runtimeStartInFlight.current) return;
    runtimeStartRequested.current = true;
    runtimeStartInFlight.current = true;
    setRuntimeStartError(null);
    try {
      await companionStore.startRuntime();
    } catch (cause) {
      const message = errorMessage(cause);
      setRuntimeStartError(message);
      onError(message);
    } finally {
      runtimeStartInFlight.current = false;
    }
  }, [onError]);

  useEffect(() => {
    if (initializedRepository.current === repository.id || !state.conversationIndex || chat?.loaded) return;
    initializedRepository.current = repository.id;
    void companionStore.openRepository(repository.id).finally(() => {
      if (initializedRepository.current === repository.id) initializedRepository.current = null;
    });
  }, [chat?.loaded, repository.id, state.conversationIndex]);

  useEffect(() => {
    if (!chat?.loaded || !selectedIsCodex || !selectedConnection?.connection.enabled || !runtime || runtime.phase !== "disabled" || runtimeStartRequested.current) return;
    void startCodexRuntime();
  }, [chat?.loaded, runtime, selectedIsCodex, selectedConnection?.connection.enabled, startCodexRuntime]);

  useEffect(() => {
    if (!selectedIsCodex || runtime?.phase === "ready" || runtime?.phase === "active_turn") {
      setRuntimeStartError(null);
    }
  }, [runtime?.phase, selectedIsCodex]);

  const completedScanId = scanStatus?.latestCompleted.id ?? null;
  useEffect(() => {
    if (contextStatus === "ready") companionStore.markScanContext(repository.id, completedScanId);
  }, [contextStatus, completedScanId, repository.id]);

  useEffect(() => {
    mentionRequest.current += 1;
    setMentionKind(null);
    setMentionQuery("");
    setMentionResults([]);
  }, [repository.id]);

  useEffect(() => {
    sourceRequest.current += 1;
    setSource(null); setSourceLoading(false);
    return () => { sourceRequest.current += 1; mentionRequest.current += 1; };
  }, [repository.id, chat?.activeThreadId]);

  async function reviewSend(revised: boolean) {
    setSendReview({ preview: await companionStore.previewSend(repository.id), revised });
  }

  async function prepareSend() {
    const current = companionStore.getSnapshot().repositories[repository.id];
    if (!current?.draft.trim() || current.pending || archived || unavailable || !selectedModel || chat.selectionLoading || contextStatus !== "ready" || state.initializationPhase !== "ready") return;
    try {
      if (!current.consent?.valid) {
        await reviewSend(false);
        return;
      }
      if (await companionStore.send(repository.id) === "review-required") await reviewSend(true);
    } catch (cause) {
      onError(errorMessage(cause));
    }
  }

  async function inspectContext(trigger: HTMLButtonElement | null) {
    contextReturnFocus.current = trigger;
    setContextOpen(true);
    setContextLoading(true);
    setContextError(null);
    try {
      await companionStore.preview(repository.id);
    } catch (cause) {
      setContextError(errorMessage(cause));
    } finally { setContextLoading(false); }
  }

  async function acceptAndSend() {
    if (!sendReview) return;
    const { preview } = sendReview;
    try {
      const current = companionStore.getSnapshot().repositories[repository.id];
      if (current?.selection !== preview.selection) {
        await reviewSend(true);
        return;
      }
      if (!current.consent?.valid) await companionStore.setRepositoryConsent(repository.id, true);
      setSendReview(null);
      if (await companionStore.send(repository.id, preview) === "review-required") await reviewSend(true);
    } catch (cause) {
      onError(errorMessage(cause));
    }
  }

  async function selectModel(connectionId: string, modelId: string) {
    try {
      await companionStore.selectRepositoryConnection(repository.id, connectionId, modelId, false);
    } catch (cause) {
      onError(errorMessage(cause));
    }
  }

  async function beginLogin(deviceCode: boolean) {
    setLoginBusy(true);
    try {
      const login = await companionStore.beginChatgptLogin(deviceCode);
      const url = login.authUrl ?? login.verificationUrl;
      if (url) await openExternalUrl(url);
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setLoginBusy(false);
    }
  }

  async function loadMentions(kind: ContextAttachmentKind, query = "") {
    const sequence = ++mentionRequest.current;
    setContextMenuOpen(false);
    setMentionKind(kind);
    setMentionQuery(query);
    setMentionLoading(true);
    try {
      const results = await companionSearchMentions(repository.id, kind, query);
      if (sequence === mentionRequest.current) setMentionResults(results);
    } catch (cause) {
      if (sequence === mentionRequest.current) { setMentionResults([]); onError(errorMessage(cause)); }
    } finally {
      if (sequence === mentionRequest.current) setMentionLoading(false);
    }
  }

  function attachMention(mention: CompanionMention) {
    if (!chat || !mentionKind || !mention.available) return;
    setAttachmentLabels((current) => {
      const labels = Object.fromEntries(chat.attachments.flatMap(({ kind, id }) => {
        const key = `${kind}:${id}`;
        return current[key] ? [[key, current[key]]] : [];
      }));
      labels[`${mentionKind}:${mention.id}`] = mention.label;
      return labels;
    });
    companionStore.setAttachments(repository.id, [...chat.attachments, { kind: mentionKind, id: mention.id }]);
    mentionRequest.current += 1;
    setMentionKind(null);
    setMentionQuery("");
    setMentionResults([]);
  }

  const inspectCitation = useCallback(async (citation: CompanionCitation, contextSourceTrigger?: HTMLButtonElement) => {
    const request = ++sourceRequest.current;
    const focused = document.activeElement;
    citationReturnFocus.current = contextSourceTrigger ?? (focused instanceof HTMLElement && focused !== document.body ? focused : null);
    setSourceFromContext(Boolean(contextSourceTrigger));
    setSourceLoading(true);
    setSource(null);
    try {
      const result = await companionSourceDetail(repository.id, citation.sourceId);
      if (request === sourceRequest.current) setSource(result);
    } catch (cause) {
      if (request !== sourceRequest.current) return;
      setSource({
        source: { id: citation.sourceId, kind: citation.kind, label: citation.label, repositoryRelativePath: null, lineStart: null, lineEnd: null, scanId: null, findingFingerprint: null, packageIdentity: null, revision: null, stale: true },
        content: null,
        unavailableReason: errorMessage(cause),
      });
    } finally {
      if (request === sourceRequest.current) setSourceLoading(false);
    }
  }, [repository.id]);

  if (state.initializationPhase === "failed" && !chat?.loaded) {
    return <ContainedState state="error" icon={Warning} title="AI connections could not be checked" description="Your local conversations are unchanged. Retry provider discovery when ready." action={<Button onClick={() => void companionStore.initialize(true)}><AppIcon className="size-4" icon={ArrowClockwise} />Retry</Button>} />;
  }
  if (!chat?.loaded) {
    return <div data-chat-state="loading" className="flex h-full min-h-80 items-center justify-center"><Spinner label="Loading local conversation" /></div>;
  }
  const connectionDisabled = selectedConnection?.connection.enabled === false;
  const runtimeFailed = Boolean(selectedIsCodex && !connectionDisabled && (runtimeStartError || runtimeUnavailable));
  const codexStarting = Boolean(!runtimeFailed && !connectionDisabled && selectedIsCodex && runtime && ["discovering", "starting", "initializing"].includes(runtime.phase));
  const codexSignedOut = Boolean(!runtimeFailed && selectedIsCodex && !unavailable && state.account?.signedIn === false);

  const providerReady = state.initializationPhase === "ready";
  const showConnectionSetup = Boolean(providerReady && selectedConnection && unavailable && !runtimeFailed && !codexStarting);
  const connectionStatus = exactUnavailableMessage(selectedConnection, runtime?.message);
  const composerDisabled = !providerReady || contextStatus !== "ready" || archived || unavailable || runtimeFailed || codexStarting || (selectedIsCodex && state.account?.signedIn === false);
  const showsProviderPanel = !items.length && !archived && (showConnectionSetup || runtimeFailed || codexStarting || codexSignedOut);
  const statusMessage = chat.pending
    ? chat.pending.threadId === chat.activeThreadId ? "" : "A response is running in another conversation."
    : !providerReady ? state.initializationPhase === "failed" ? "AI connections could not be checked. Open provider settings to retry." : "Checking AI connections. Your saved conversation is available."
    : contextStatus !== "ready" ? contextStatus === "loading" ? "Loading scan context. Your saved conversation is available." : "Scan context is unavailable. Your saved conversation is available."
    : archived ? "This conversation is archived. Unarchive it from Conversation actions to continue."
    : showsProviderPanel ? ""
    : codexStarting ? "Starting Codex. You can choose another AI connection while it starts."
    : codexSignedOut ? "Codex is signed out. Sign in or choose another AI connection."
    : composerDisabled ? connectionStatus
    : chat.error?.message ?? "";

  const contextControls = <Popover open={contextMenuOpen} onOpenChange={setContextMenuOpen}><PopoverTrigger render={<Button ref={contextTrigger} aria-label="Add context" className="h-8 gap-1 px-2 text-xs" variant="ghost" />}><AppIcon className="size-3.5" icon={Plus} /><span className="hidden sm:inline">Context</span></PopoverTrigger><PopoverContent align="start" finalFocus={mentionKind || contextOpen ? false : contextTrigger} className="max-h-[var(--available-height)] w-72 overflow-auto p-1.5"><PopoverTitle className="sr-only">Repository context</PopoverTitle><Button className="w-full justify-start" size="sm" variant="ghost" onClick={() => { setContextMenuOpen(false); void inspectContext(contextTrigger.current); }}>Inspect context</Button>{MENTION_KINDS.map((entry) => <Button className="h-auto w-full justify-start whitespace-normal p-2 text-left" key={entry.kind} variant="ghost" onClick={() => void loadMentions(entry.kind)}><span><span className="block text-sm font-medium text-ink">@{entry.label.toLowerCase()}</span><span className="block text-xs text-ink-subtle">{entry.description}</span></span></Button>)}</PopoverContent></Popover>;

  const contextChips = findingRequest || chat.attachments.length ? <>
    {findingRequest ? <ContextChip wrap label={[
      findingRequest.advisoryId,
      [findingRequest.packageName, findingRequest.installedVersion].filter(Boolean).join(" "),
      findingRequest.scanRunId ? "Scan #" + findingRequest.scanRunId : "Scan unavailable",
      findingRequest.scanRunId && completedScanId && findingRequest.scanRunId !== completedScanId ? "Historical evidence" : null,
    ].filter(Boolean).join(" · ")} onRemove={() => {
      companionStore.setFocusedFinding(repository.id, null);
      onClearFinding?.();
    }} /> : null}
    {chat.attachments.map((attachment) => {
    const key = `${attachment.kind}:${attachment.id}`;
    const label = attachmentLabels[key]
      ?? chat.context?.attachments.find((item) => item.kind === attachment.kind && item.id === attachment.id && item.available)?.label
      ?? (attachment.kind === "scan" ? `Scan #${attachment.id}` : attachment.kind === "finding" ? attachment.id.split("|", 1)[0] : attachment.id);
    return <ContextChip key={key} label={label} onRemove={() => {
      setAttachmentLabels((current) => {
        const next = { ...current };
        delete next[key];
        return next;
      });
      companionStore.setAttachments(repository.id, chat.attachments.filter((candidate) => candidate.kind !== attachment.kind || candidate.id !== attachment.id));
    }} />;
  })}</> : null;

  const sourceDialog = (
    <Dialog open={sourceLoading || Boolean(source)} onOpenChange={(open) => {
      if (!open) {
        sourceRequest.current += 1;
        setSource(null);
        setSourceLoading(false);
      }
    }}>
      <DialogContent className="max-w-xl" positioned finalFocus={() =>
        citationReturnFocus.current?.isConnected ? citationReturnFocus.current : contextReturnFocus.current ?? contextTrigger.current}>
        <DialogHeader>
          <DialogTitle data-slot="dialog-title">Evidence source</DialogTitle>
          <DialogDescription data-slot="dialog-description">Resolved locally from this repository's bounded evidence.</DialogDescription>
        </DialogHeader>
        <DialogBody>
          {sourceLoading ? <Spinner label="Resolving local evidence source" /> : source ? <SourceInspector source={source} onOpenFinding={onOpenFinding} /> : null}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );

  return (
    <div data-chat-state={chat.loading || chat.selectionLoading ? "loading" : composerDisabled ? "read-only" : !items.length ? "empty" : "ready"} className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface-1">
      <ConversationHeader contextStatus={contextStatus} context={chat.context} contextChanged={chat.contextChanged} scanStatus={scanStatus} repository={repository} thread={activeThread} onArchive={(value) => activeThread ? companionStore.setThreadArchived(repository.id, activeThread.id, value) : undefined} onInspectContext={(trigger) => void inspectContext(trigger)} onRename={(title) => activeThread ? companionStore.renameThread(repository.id, activeThread.id, title) : undefined} />
      <Dialog open={Boolean(mentionKind)} onOpenChange={(open) => { if (!open) { mentionRequest.current += 1; setMentionKind(null); } }}>
        <DialogContent initialFocus={mentionInput} finalFocus={contextTrigger}>
          <DialogHeader><DialogTitle data-slot="dialog-title">Add {mentionKind ?? "repository"} context</DialogTitle><DialogDescription data-slot="dialog-description">Choose local evidence to include with your next message.</DialogDescription></DialogHeader>
          <DialogBody className="grid gap-3">
            <Input ref={mentionInput} aria-label={`Search ${mentionKind}`} placeholder={`Search ${mentionKind}`} value={mentionQuery} onChange={(event) => { if (mentionKind) void loadMentions(mentionKind, event.target.value); }} />
            <div aria-label={`${mentionKind} context results`} className="max-h-64 overflow-y-auto" role="region" aria-busy={mentionLoading}>
              {mentionLoading ? <Spinner label="Searching repository context" /> : mentionResults.length ? mentionResults.map((mention) => <Button className="h-auto w-full justify-start whitespace-normal px-2 py-2 text-left" disabled={!mention.available} key={`${mention.kind}:${mention.id}`} variant="ghost" onClick={() => attachMention(mention)}><span><span className="block text-sm font-medium text-ink">{mention.label}</span><span className="block text-xs text-ink-subtle">{mention.available ? mention.detail : mention.unavailableReason}</span></span></Button>) : <p role="status" className="px-2 py-3 text-sm text-ink-subtle">No matching local context.</p>}
            </div>
          </DialogBody>
        </DialogContent>
      </Dialog>
      {showsProviderPanel ? <div className="min-h-0 flex-1 overflow-y-auto">
        {showConnectionSetup ? (
          <ContainedState
            icon={Warning}
            title={`${selectedConnection?.connection.displayName} is ${connectionDisabled ? "disabled" : "unavailable"}`}
            description={connectionDisabled ? "Enable this connection in provider settings or choose another connection below." : connectionStatus}
            action={onOpenAiConnections ? <Button onClick={onOpenAiConnections}>Provider settings</Button> : null}
          />
        )
        : runtimeFailed ? <ContainedState state="error" icon={Warning} title="Codex could not start" description={runtimeStartError ?? runtime?.message ?? "The local Codex runtime is unavailable."} action={<div className="flex flex-wrap justify-center gap-2"><Button onClick={() => void startCodexRuntime()}><AppIcon className="size-4" icon={ArrowClockwise} />Retry</Button>{onOpenAiConnections ? <Button variant="ghost" onClick={onOpenAiConnections}>Provider settings</Button> : null}</div>} />
        : codexStarting ? <ContainedState busy icon={ChatCircleDots} title="Starting Codex" description="Choose another connection below, or wait for Codex to become ready." action={onOpenAiConnections ? <Button variant="ghost" onClick={onOpenAiConnections}>Provider settings</Button> : null} />
        : <ContainedState icon={SignIn} title="Connect Codex" description="Sign in to Codex, or choose another AI connection below." action={<div className="flex flex-wrap justify-center gap-2"><Button disabled={loginBusy} onClick={() => void beginLogin(false)}>Continue with ChatGPT</Button><Button disabled={loginBusy} variant="secondary" onClick={() => void beginLogin(true)}>Use device code</Button>{onOpenAiConnections ? <Button variant="ghost" onClick={onOpenAiConnections}>Provider settings</Button> : null}</div>} />}
      </div> : <ChatProfiler id="chat-timeline"><ConnectedChatTimeline repositoryId={repository.id} repositoryName={repository.name} starters={STARTERS} onStarter={applyStarter} onCitation={inspectCitation} onError={onError} /></ChatProfiler> }
      <ChatProfiler id="chat-composer"><ConnectedChatComposer repositoryId={repository.id} focusRequest={composerFocusRequest} contextChips={contextChips} contextControls={contextControls} inputDisabled={archived} disabled={composerDisabled} placeholder={archived ? "Unarchive this conversation to continue" : unavailable ? "Choose a ready AI connection" : repository.sourceType === "local" ? "Ask about findings, packages, or remediation…" : "Ask about stored scan evidence…"} statusMessage={statusMessage} onOpenSettings={onOpenAiConnections} onSelectModel={(connectionId, modelId) => void selectModel(connectionId, modelId)} onRetry={() => void prepareSend()} onSend={() => void prepareSend()} /></ChatProfiler>

      <Dialog open={sendReview !== null} onOpenChange={(open) => { if (!open) setSendReview(null); }}>
        <DialogContent showClose={false}>
          <DialogHeader>
            <DialogTitle data-slot="dialog-title">Send bounded repository context through {selectedConnection?.connection.displayName ?? "this provider"}?</DialogTitle>
            <DialogDescription data-slot="dialog-description">{sendReview?.revised
              ? "Repository context changed. Review this preview before sending."
              : "Consent is stored for this exact connection and inference boundary."}</DialogDescription>
          </DialogHeader>
          <DialogBody className="grid gap-3 text-sm text-ink-subtle">
            <p>cvec sends your prompt plus the bounded repository and scan evidence shown here. Repository content is treated as untrusted evidence, never as instructions.</p>
            <ChatContextPreview context={sendReview?.preview.context ?? null} />
            <p>cvec stores the normalized visible conversation locally so it remains readable after restart. Provider authentication fields and prepared context envelopes are not stored as chat messages. Prompt text is kept as entered, so do not paste API keys or other secrets into Chat.</p>
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setSendReview(null)}>Cancel and keep draft</Button>
            <Button onClick={() => void acceptAndSend()}>Accept and send</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={contextOpen} onOpenChange={setContextOpen}>
        <DialogContent className="max-w-xl" positioned finalFocus={contextReturnFocus}>
          <DialogHeader>
            <DialogTitle data-slot="dialog-title">Active repository context</DialogTitle>
            <DialogDescription data-slot="dialog-description">Prepared locally from the selected repository and its latest completed scan, when available.</DialogDescription>
          </DialogHeader>
          <DialogBody>
            {contextLoading ? <Spinner label="Preparing current context" /> : contextError ? <Alert variant="warning"><AlertTitle>Context unavailable</AlertTitle><AlertDescription>{contextError}</AlertDescription></Alert> : <ChatContextPreview context={chat.context} onInspectSource={(item, trigger) =>
              void inspectCitation({ sourceId: item.id, kind: item.kind, label: item.label, stale: item.stale }, trigger)} />}
          </DialogBody>
          {sourceFromContext ? sourceDialog : null}
        </DialogContent>
      </Dialog>
      {sourceFromContext ? null : sourceDialog}
    </div>
  );
}

function ContainedState({ icon, title, description, action, busy = false, state = "read-only" }: { icon: Icon; title: string; description: string; action?: ReactNode; busy?: boolean; state?: "read-only" | "error" }) {
  return <div data-chat-contained-state data-chat-state={busy ? "loading" : state} className="grid min-h-full content-center p-5"><EmptyState icon={icon} title={title} description={description} action={action} />{busy ? <div className="mx-auto mt-3"><Spinner label={title} /></div> : null}</div>;
}

function ContextChip({ label, onRemove, wrap = false }: { label: string; onRemove?: () => void; wrap?: boolean }) {
  return (
    <span className={"inline-flex min-h-7 min-w-0 items-center gap-1 rounded-full border border-hairline bg-surface-3 px-2 text-xs text-ink-subtle " + (wrap ? "max-w-full" : "max-w-60")}>
      <AppIcon className="size-3 shrink-0" icon={FileText} />
      <span className={wrap ? "py-1 [overflow-wrap:anywhere]" : "truncate"} title={label}>{label}</span>
      {onRemove ? <IconButton aria-label={"Remove " + label} className="size-5 shrink-0 rounded-full" size="icon-xs" variant="ghost" onClick={onRemove}><AppIcon className="size-3" icon={X} /></IconButton> : null}
    </span>
  );
}

function SourceInspector({ source, onOpenFinding }: { source: CompanionSourceDetail; onOpenFinding?: (fingerprint: string) => void }) {
  const { source: evidence, content, unavailableReason } = source;
  const scannerEvidence = evidence.kind === "scan" || evidence.kind === "finding" || evidence.kind === "advisory";
  const location = [evidence.repositoryRelativePath, evidence.scanId ? "Scan #" + evidence.scanId : null,
    evidence.revision ? "Revision " + evidence.revision.slice(0, 12) : null].filter(Boolean).join(" · ");
  let contents: ReactNode = null;
  if (content && evidence.kind === "scan") contents = <ScanSourceContent content={content} />;
  else if (content && evidence.kind === "finding") contents = <FindingSourceContent content={content} />;
  else if (content) contents = <pre tabIndex={0} className="max-h-[60vh] overflow-auto whitespace-pre-wrap break-words rounded-md border border-hairline bg-surface-1 p-3 text-xs leading-5 text-ink">{content}</pre>;
  return <div className="grid gap-3">
    <div>
      <h3 className="break-words text-sm font-semibold text-ink">{evidence.label}</h3>
      {location ? <p className="mt-1 break-words text-xs text-ink-subtle">{location}</p> : null}
    </div>
    {evidence.stale && !unavailableReason ? <p className="text-xs text-status-warning-foreground">{scannerEvidence ? "Historical or unaligned scan evidence" : "Source may have changed since the message"}</p> : null}
    {unavailableReason ? <Alert variant="warning"><AlertTitle>Source unavailable</AlertTitle><AlertDescription>{unavailableReason}</AlertDescription></Alert> : null}
    {contents}
    {evidence.findingFingerprint && onOpenFinding ? <Button variant="secondary" onClick={() => onOpenFinding(evidence.findingFingerprint!)}>{evidence.stale ? "Open current finding in Review" : "Open finding in Review"}</Button> : null}
  </div>;
}

function exactUnavailableMessage(connection: ProviderConnectionSnapshot | null, runtimeMessage?: string | null): string {
  if (!connection) return "Choose a ready AI connection in the composer.";
  if (!connection.connection.enabled) return `${connection.connection.displayName} is disabled. Enable it in provider settings or choose another connection.`;
  if (connection.connection.driver === "codex" && runtimeMessage) return runtimeMessage;
  return `${connection.connection.displayName} is not ready. Choose another connection or open provider settings.`;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
