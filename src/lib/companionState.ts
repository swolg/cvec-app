import {invoke} from "@tauri-apps/api/core";
import { useCallback, useRef, useSyncExternalStore } from "react";

import { companionClient, type CompanionClient } from "./companionApi";
import { ChatDraftStore } from "./chatDrafts";
import { limitDraftText } from "./chatDraftText";
import { countPerformanceRender } from "./performanceCounters";
import {
  parseCompanionCachedThreadSnapshot,
  parseCompanionRuntimeStatus,
  parseCompanionThread,
  parseCompanionThreadSnapshot,
} from "./contracts";
import type {
  CompanionAccountStatus,
  CompanionActivity,
  CompanionCachedThreadSnapshot,
  CompanionComposerSelection,
  CompanionConversationIndex,
  CompanionErrorInfo,
  CompanionEventEnvelope,
  CompanionModel,
  CompanionLoginStart,
  CompanionRuntimeStatus,
  CompanionProviderSegment,
  CompanionThread,
  CompanionThreadSnapshot,
  CompanionTimelineItem,
  CompanionTokenUsage,
  CompanionUsageStatus,
  ContextAttachmentRequest,
  PrivacyConsentStatus,
  ProviderConnectionSnapshot,
  RepositoryChatContext,
} from "./types";

export interface PendingCompanionTurn {
  clientTurnId: string;
  threadId: number | null;
  userItem: CompanionTimelineItem;
  connectionId: string;
  phase: "preparing" | "starting" | "waiting" | "thinking" | "responding" | "working";
  startedAt: number;
  turnId: string | null;
  prompt: string;
}

export interface RepositoryCompanionState {
  repositoryId: number;
  loaded: boolean;
  threads: CompanionThread[];
  activeThreadId: number | null;
  itemsByThread: Record<number, CompanionTimelineItem[]>;
  usageByThread: Record<number, CompanionTokenUsage | null>;
  usageAttributionByThread?: Record<number, { connectionId: string; modelId: string | null }>;
  segmentsByThread: Record<number, CompanionProviderSegment[]>;
  selection: CompanionComposerSelection;
  selectionModels: CompanionModel[];
  selectionLoading: boolean;
  consent: PrivacyConsentStatus | null;
  draft: string;
  draftLoaded?: boolean;
  draftStorageError?: "restore" | "save" | null;
  attachments: ContextAttachmentRequest[];
  focusedFinding: ContextAttachmentRequest | null;
  context: RepositoryChatContext | null;
  contextChanged: boolean;
  pending: PendingCompanionTurn | null;
  loading: boolean;
  historyUnavailable: boolean;
  cached: boolean;
  historyPages?: Record<number,{before:string|null;older:string|null}>;
  historyProviderOrdinals?: Record<number,Record<string,number>>;
  error: CompanionErrorInfo | null;
}

export type CompanionSendPreview = Pick<RepositoryCompanionState,
  "draft" | "activeThreadId" | "selection" | "attachments" | "focusedFinding"
> & { context: RepositoryChatContext };

export interface CompanionStateSnapshot {
  connected: boolean;
  initializationPhase: "idle" | "checking" | "ready" | "failed";
  runtime: CompanionRuntimeStatus | null;
  account: CompanionAccountStatus | null;
  models: CompanionModel[];
  connections: ProviderConnectionSnapshot[];
  selectedConnectionId: string;
  selectedModel: string | null;
  selectedEffort: string | null;
  usage: CompanionUsageStatus | null;
  consent: PrivacyConsentStatus | null;
  login: CompanionLoginUiState;
  conversationIndex: CompanionConversationIndex | null;
  conversationIndexLoading: boolean;
  repositories: Record<number, RepositoryCompanionState>;
}

export interface CompanionLoginUiState {
  phase: "idle" | "starting" | "waiting" | "success" | "cancelled" | "expired" | "failed" | "external_change";
  loginId: string | null;
  userCode: string | null;
  message: string | null;
}

const EMPTY_SNAPSHOT: CompanionStateSnapshot = {
  connected: false,
  initializationPhase: "idle",
  runtime: null,
  account: null,
  models: [],
  connections: [],
  selectedConnectionId: "codex_default",
  selectedModel: null,
  selectedEffort: null,
  usage: null,
  consent: null,
  login: { phase: "idle", loginId: null, userCode: null, message: null },
  conversationIndex: null,
  conversationIndexLoading: false,
  repositories: {},
};

const CHAT_BLOCKING_CONNECTION_HEALTH = new Set(["setup_required", "unavailable", "incompatible", "disabled"]);
const CONTEXT_REVIEW_REQUIRED = "Repository context changed. Review the updated context before sending.";
const contextReviewError: CompanionErrorInfo = { code: "context_review_required", message: CONTEXT_REVIEW_REQUIRED, retryable: true };
const DELTA_FRAME_MS = 50;
const MAX_DELTA_ROUTES = 32;

function connectionReadyForChat(snapshot: ProviderConnectionSnapshot | null | undefined): boolean {
  return Boolean(snapshot?.connection.enabled && !CHAT_BLOCKING_CONNECTION_HEALTH.has(snapshot.health));
}

export class CompanionStore {
  private snapshot: CompanionStateSnapshot = EMPTY_SNAPSHOT;
  private readonly listeners = new Set<() => void>();
  private unlisten: (() => void) | null = null;
  private connecting: Promise<void> | null = null;
  private initializing: Promise<void> | null = null;
  private startingRuntime: Promise<void> | null = null;
  private codexIdentity: { key: string; result: Promise<[CompanionAccountStatus, CompanionModel[] | null]> } | null = null;
  private connectionConsents = new Map<string, Promise<PrivacyConsentStatus>>();
  private consentEpoch = 0;
  private readonly lastSequenceByConnection = new Map<string, number>();
  private readonly resuming = new Set<number>();
  private readonly resumeRequests = new Map<string, Promise<void>>();
  private readonly initialHistoryRequests = new Map<string, Promise<void>>();
  private readonly openingRepositories = new Map<number, Promise<void>>();
  private readonly sending = new Set<number>();
  private readonly queuedEvents = new Map<number, CompanionEventEnvelope[]>();
  private readonly deltaBuffers=new Map<string,{event:CompanionEventEnvelope;timer:ReturnType<typeof setTimeout>}>();
  private readonly deltaFlushTimes = new Map<string, number>();
  private connectionSelectionSequence = 0;
  private readonly contextPreviewSequence = new Map<number, number>();
  private readonly repositorySelectionSequence = new Map<number, number>();

  private readonly liveHistory=new Map<string,CompanionTimelineItem[]>();
  private readonly historyAccess=new Map<string,{repositoryId:number;threadId:number;bytes:number}>();
  private readonly historyMetadata = new Map<string, { fields: readonly unknown[]; bytes: number }>();
  private readonly historyWatermarks=new Map<string,number>();
  private readonly historyNamespaces=new Set<number>();
  private readonly historyRequests=new Map<number,number>();
  private historyRequestSequence = 0;
  readonly drafts: ChatDraftStore;
  constructor(private readonly client: CompanionClient = companionClient) {
    this.drafts = new ChatDraftStore(client);
  }

  getSnapshot = (): CompanionStateSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async connect(): Promise<void> {
    if (this.unlisten) return;
    if (this.connecting) return this.connecting;
    this.connecting = (async () => {
      const unlisten = await this.client.listen((event) => this.handleEvent(event));
      this.unlisten = unlisten;
      this.patch({ connected: true });
    })();
    try {
      await this.connecting;
    } finally {
      this.connecting = null;
    }
  }

  disconnect(): void {
    this.flushDelta();
    this.deltaFlushTimes.clear();
    this.historyRequests.clear();
    this.unlisten?.();
    this.unlisten = null;
    this.invalidateProviderReads();
    this.patch({ connected: false });
  }

  async initialize(force = false): Promise<void> {
    if (!force && this.snapshot.initializationPhase === "ready") return;
    if (this.initializing) return this.initializing;
    this.patch({ initializationPhase: "checking" });
    if (force) this.invalidateProviderReads();
    this.initializing = (async () => {
      // Local history must remain usable even when provider discovery or the
      // event connection is slow or unavailable.
      await Promise.all([this.loadConversationIndex(), this.connect().then(() => this.loadGlobal())]);
      this.patch({ initializationPhase: "ready" });
      for (const repository of Object.values(this.snapshot.repositories)) {
        if (repository.loaded && repository.activeThreadId) {
          void this.synchronizeConversation(repository.repositoryId, repository.activeThreadId)
            .catch(() => undefined);
        } else if (repository.loaded && !repository.selection.modelId) {
          const connection = this.snapshot.connections.find(connectionReadyForChat);
          if (connection) {
            void this.selectRepositoryConnection(repository.repositoryId, connection.connection.id, null, false)
              .catch(() => undefined);
          }
        }
      }
    })();
    try {
      await this.initializing;
    } catch (cause) {
      this.patch({ initializationPhase: "failed" });
      throw cause;
    } finally {
      this.initializing = null;
    }
  }

  async loadConversationIndex(): Promise<void> {
    this.patch({ conversationIndexLoading: true });
    try {
      this.patch({ conversationIndex: await this.client.conversationIndex() });
    } finally {
      this.patch({ conversationIndexLoading: false });
    }
  }

  private invalidateProviderReads(): void {
    this.codexIdentity = null;
    this.connectionConsents.clear();
    this.consentEpoch += 1;
    const repositories = Object.values(this.snapshot.repositories);
    if (this.snapshot.consent || repositories.some((repository) => repository.consent)) {
      this.patch({
        consent: null,
        repositories: Object.fromEntries(repositories.map((repository) => [
          repository.repositoryId, repository.consent ? { ...repository, consent: null } : repository,
        ])),
      });
    }
  }

  private readCodexIdentity(connection: ProviderConnectionSnapshot, runtime: CompanionRuntimeStatus) {
    const key = JSON.stringify([connection.connection.id, connection.connection.updatedAt, runtime.startedAt]);
    if (this.codexIdentity?.key === key) return this.codexIdentity.result;
    const result = Promise.all([
      this.client.accountStatus(),
      this.client.listModels().catch(() => null),
    ]);
    this.codexIdentity = { key, result };
    void result.then(([, models]) => {
      if (!models && this.codexIdentity?.result === result) this.codexIdentity = null;
    }, () => {
      if (this.codexIdentity?.result === result) this.codexIdentity = null;
    });
    return result;
  }

  private readConnectionConsent(connectionId: string, modelId: string, connection = this.snapshot.connections.find((item) => item.connection.id === connectionId)) {
    if (!connection) return this.client.connectionConsent(connectionId, modelId);
    const key = JSON.stringify([connectionId, connection.connection.driver, connection.connection.updatedAt, connection.checkedAt, modelId]);
    const retained = this.connectionConsents.get(key);
    if (retained) return retained;
    const read = this.client.connectionConsent(connectionId, modelId);
    if (this.connectionConsents.size >= 64) this.connectionConsents.delete(this.connectionConsents.keys().next().value!);
    this.connectionConsents.set(key, read);
    void read.catch(() => {
      if (this.connectionConsents.get(key) === read) this.connectionConsents.delete(key);
    });
    return read;
  }

  async loadGlobal(retryInvalidatedRead = true, observedRuntime?: CompanionRuntimeStatus): Promise<void> {
    // Provider discovery starts Codex. Read its status afterwards so a captured
    // startup phase cannot overwrite the ready event emitted by discovery.
    const knownConnections = this.snapshot.connections;
    const queriedConnections = await this.client.listConnections();
    const connections = knownConnections === this.snapshot.connections ? queriedConnections : this.snapshot.connections;
    const runtimeBeforeRefresh = this.snapshot.runtime;
    const queriedRuntime = await this.client.runtimeStatus();
    const runtime = this.snapshot.runtime !== runtimeBeforeRefresh
      ? this.snapshot.runtime ?? queriedRuntime
      : observedRuntime ?? queriedRuntime;
    const fallback = connections.find((item) => item.connection.enabled && item.health === "ready")
      ?? connections.find((item) => item.connection.enabled);
    const current = connections.find((item) => item.connection.id === this.snapshot.selectedConnectionId);
    const selectedConnectionId = current
      && !(runtime.phase === "unavailable" && current.connection.driver === "codex" && fallback?.health === "ready")
      ? current.connection.id
      : fallback?.connection.id ?? "codex_default";
    const selected = connections.find((item) => item.connection.id === selectedConnectionId);
    const epoch = this.consentEpoch;
    const selectionSequence = this.connectionSelectionSequence;
    let account: CompanionAccountStatus | null = null;
    let models: CompanionModel[] = selected?.models ?? [];
    if (selected?.connection.id === "codex_default" && ["ready", "active_turn"].includes(runtime.phase)) {
      const [availableAccount, availableModels] = await this.readCodexIdentity(selected, runtime);
      account = availableAccount;
      models = availableModels ?? selected.models;
    }
    const selectedModel = selectedConnectionId === this.snapshot.selectedConnectionId
      && models.some((model) => model.id === this.snapshot.selectedModel)
      ? this.snapshot.selectedModel
      : models.find((model) => model.isDefault)?.id ?? models[0]?.id ?? null;
    const selectedModelSnapshot = models.find((model) => model.id === selectedModel);
    const consent = selectedModel
      ? await this.readConnectionConsent(selectedConnectionId, selectedModel, selected)
      : null;
    if (epoch !== this.consentEpoch || selectionSequence !== this.connectionSelectionSequence) {
      if (retryInvalidatedRead) await this.loadGlobal(false, this.snapshot.runtime ?? undefined);
      return;
    }
    this.patch({
      // Account/model/consent reads can also overlap a newer runtime event.
      runtime: this.snapshot.runtime !== runtimeBeforeRefresh ? this.snapshot.runtime ?? runtime : runtime,
      connections,
      selectedConnectionId,
      account,
      models,
      selectedModel,
      selectedEffort: selectedModelSnapshot?.reasoningEfforts.includes(this.snapshot.selectedEffort ?? "")
        ? this.snapshot.selectedEffort
        : selectedModelSnapshot?.reasoningEfforts[0] ?? null,
      consent,
      usage: selectedConnectionId === this.snapshot.selectedConnectionId ? this.snapshot.usage : null,
      login: account?.signedIn
        ? { phase: "success", loginId: null, userCode: null, message: null }
        : this.snapshot.login,
    });
  }

  synchronizeConnections(connections: ProviderConnectionSnapshot[]): void {
    this.invalidateProviderReads();
    const repositories = Object.fromEntries(Object.entries(this.snapshot.repositories).map(([id, repository]) => [
      id,
      repository.activeThreadId ? { ...repository, loaded: false } : repository,
    ]));
    this.patch({ connections, repositories });
  }

  async startRuntime(): Promise<void> {
    if (this.startingRuntime) return this.startingRuntime;
    this.invalidateProviderReads();
    const task = (async () => {
      const connectionId = "codex_default";
      const connection = this.snapshot.connections.find((item) => item.connection.id === connectionId);
      if (connection && !connection.connection.enabled) throw new Error("Enable Codex in provider settings before retrying.");
      const beforeStart = this.snapshot.runtime;
      const reply = await this.client.startRuntime();
      const observed = this.snapshot.runtime;
      // Progress events precede the final reply; a later failure/stop must win.
      const runtime = observed && observed !== beforeStart
        && !["discovering", "starting", "initializing"].includes(observed.phase)
        ? observed : reply;
      this.patch({ runtime });
      if (!["ready", "active_turn"].includes(runtime.phase)) {
        throw new Error(runtime.message ?? "The local Codex runtime could not start.");
      }
      const refreshed = await this.client.refreshConnection(connectionId);
      const current = this.snapshot.connections.find((item) => item.connection.id === connectionId);
      // A settings change made during the probe owns the newer configuration.
      if (!current || current.connection !== connection?.connection) return;
      this.patch({ connections: this.snapshot.connections.map((item) => item.connection.id === connectionId ? refreshed : item) });
      if (["unavailable", "incompatible"].includes(refreshed.health)) {
        throw new Error(refreshed.error?.message ?? refreshed.message ?? "Codex is unavailable.");
      }
      await this.loadProviderState();
      await Promise.all(Object.values(this.snapshot.repositories)
        .filter((repository) => repository.selection.connectionId === connectionId)
        .map((repository) => this.selectRepositoryConnection(repository.repositoryId, connectionId, repository.selection.modelId)));
    })();
    this.startingRuntime = task;
    try {
      await task;
    } finally {
      if (this.startingRuntime === task) this.startingRuntime = null;
    }
  }

  private async loadProviderState(): Promise<void> {
    const connectionId = this.snapshot.selectedConnectionId;
    const connection = this.snapshot.connections.find((item) => item.connection.id === connectionId);
    const runtime = this.snapshot.runtime;
    if (connection?.connection.id !== "codex_default" || !runtime) return;
    const epoch = this.consentEpoch;
    const [account, availableModels] = await this.readCodexIdentity(connection, runtime);
    const models = availableModels ?? connection.models;
    if (this.snapshot.selectedConnectionId !== connectionId) return;
    const selectedModel = models.some((model) => model.id === this.snapshot.selectedModel)
      ? this.snapshot.selectedModel
      : models.find((model) => model.isDefault)?.id ?? models[0]?.id ?? null;
    const selected = models.find((model) => model.id === selectedModel);
    const selectionBeforeConsent = this.snapshot.selectedModel;
    const consent = selectedModel
      ? await this.readConnectionConsent(connectionId, selectedModel)
      : null;
    if (epoch !== this.consentEpoch || this.snapshot.selectedConnectionId !== connectionId
      || this.snapshot.selectedModel !== selectionBeforeConsent) return;
    this.patch({
      account,
      models,
      selectedModel,
      selectedEffort: selected?.reasoningEfforts.includes(this.snapshot.selectedEffort ?? "")
        ? this.snapshot.selectedEffort
        : selected?.reasoningEfforts[0] ?? null,
      consent,
      login: account.signedIn
        ? { phase: "success", loginId: null, userCode: null, message: null }
        : this.snapshot.login,
    });
  }

  async beginChatgptLogin(deviceCode: boolean): Promise<CompanionLoginStart> {
    this.patch({ login: { phase: "starting", loginId: null, userCode: null, message: null } });
    try {
      const result = await this.client.loginChatgpt(deviceCode);
      if (this.snapshot.login.phase === "starting") {
        this.patch({
          login: {
            phase: "waiting",
            loginId: result.loginId,
            userCode: result.userCode,
            message: deviceCode ? "Enter the device code to finish signing in." : "Finish signing in in your browser.",
          },
        });
      }
      return result;
    } catch (cause) {
      this.patch({ login: loginFailure(cause) });
      throw cause;
    }
  }

  async loginWithApiKey(apiKey: string): Promise<void> {
    this.patch({ login: { phase: "starting", loginId: null, userCode: null, message: null } });
    try {
      await this.client.loginApiKey(apiKey);
      this.patch({ login: { phase: "success", loginId: null, userCode: null, message: null } });
      await this.startRuntime();
    } catch (cause) {
      this.patch({ login: loginFailure(cause) });
      throw cause;
    }
  }

  async cancelLogin(): Promise<void> {
    const loginId = this.snapshot.login.loginId;
    if (!loginId) return;
    await this.client.cancelLogin(loginId);
    this.patch({ login: { phase: "cancelled", loginId: null, userCode: null, message: "Sign-in was cancelled." } });
  }

  async openRepository(repositoryId: number): Promise<void> {
    const existing = this.openingRepositories.get(repositoryId);
    if (existing) return existing;
    const task = this.loadRepository(repositoryId);
    this.openingRepositories.set(repositoryId, task);
    try { await task; }
    finally { if (this.openingRepositories.get(repositoryId) === task) this.openingRepositories.delete(repositoryId); }
  }

  private async loadRepository(repositoryId: number): Promise<void> {
    this.updateRepository(repositoryId, { loading: true, error: null });
    await this.restoreDraft(repositoryId);
    try {
      const threads = this.snapshot.conversationIndex
        ? this.snapshot.conversationIndex.threads.filter((thread) => thread.repositoryId === repositoryId)
        : await this.client.listThreads(repositoryId, false);
      const activeThreadId = this.repository(repositoryId).activeThreadId
        ?? threads.find((thread) => !thread.archivedAt)?.id
        ?? null;
      const retained = activeThreadId !== null && Object.hasOwn(this.repository(repositoryId).itemsByThread, activeThreadId);
      this.updateRepository(repositoryId, { threads, activeThreadId, loading: false, loaded: !activeThreadId || retained });
      if (activeThreadId) {
        const active = threads.find((thread) => thread.id === activeThreadId);
        try {
          await this.ensureCachedHistory(repositoryId, activeThreadId);
        } catch {
          // Legacy rows can still hydrate from the provider and will populate the cache afterwards.
        }
        if (this.repository(repositoryId).activeThreadId !== activeThreadId) return;
        if (["checking", "failed"].includes(this.snapshot.initializationPhase)) {
          this.updateRepository(repositoryId, { loading: false, loaded: true });
          return;
        }
        const connection = active
          ? this.snapshot.connections.find((item) => item.connection.id === active.providerConnectionId)
          : null;
        if (active) {
          await this.selectRepositoryConnection(repositoryId, active.providerConnectionId, active.lastModelId, false);
        }
        if (this.repository(repositoryId).activeThreadId !== activeThreadId) return;
        if (!connectionReadyForChat(connection)) {
          this.updateRepository(repositoryId, { loading: false, loaded: true, error: null });
          return;
        }
        await this.resumeThread(repositoryId, activeThreadId);
      } else {
        const connection = this.snapshot.connections.find(connectionReadyForChat);
        if (connection) await this.selectRepositoryConnection(repositoryId, connection.connection.id, null, false);
      }
    } catch (cause) {
      this.updateRepository(repositoryId, {
        loading: false,
        loaded: true,
        error: localError("thread_list_failed", cause, true),
      });
    }
  }

  async loadEarlierHistory(repositoryId:number,threadId:number):Promise<void>{
    const page=this.repository(repositoryId).historyPages?.[threadId];
    if(!page?.older || !this.client.historyPage)return;
    const request=++this.historyRequestSequence;this.historyRequests.set(threadId,request);
    try {
      const snapshot=await this.client.historyPage(repositoryId,threadId,page.older);
      if(this.historyRequests.get(threadId)!==request)return;
      this.applyCachedSnapshot(snapshot,"earlier",page.older);
    } finally {
      if(this.historyRequests.get(threadId)===request)this.historyRequests.delete(threadId);
    }
  }
  async loadLatestHistory(repositoryId:number,threadId:number):Promise<void>{
    const request=++this.historyRequestSequence;this.historyRequests.set(threadId,request);
    try {
      const snapshot=await this.readCachedHistory(repositoryId,threadId);
      if(this.historyRequests.get(threadId)!==request)return;
      this.applyCachedSnapshot(snapshot,"latest",null);
    } finally {
      if(this.historyRequests.get(threadId)===request)this.historyRequests.delete(threadId);
    }
  }
  private historyItemId(repositoryId:number,threadId:number,id:string,event?:CompanionEventEnvelope):string{
    if(!this.historyNamespaces.has(threadId)||id.startsWith("local:")||id.startsWith("segment:"))return id;
    const segments=this.repository(repositoryId).segmentsByThread[threadId]??[];
    const segment=event?.providerSegmentId ?? [...segments].reverse().find((segment)=>(!event?.connectionId||segment.connectionId===event.connectionId)&&(!event?.providerThreadId||segment.providerThreadId===event.providerThreadId))?.id;
    return segment?`segment:${segment}:${id}`:id;
  }

  async activateConversation(repositoryId: number, threadId: number): Promise<void> {
    const indexedThreads = this.snapshot.conversationIndex?.threads.filter((thread) => thread.repositoryId === repositoryId) ?? [];
    if (!this.snapshot.repositories[repositoryId]) {
      this.updateRepository(repositoryId, { threads: indexedThreads });
    }
    const thread = this.repository(repositoryId).threads.find((candidate) => candidate.id === threadId)
      ?? indexedThreads.find((candidate) => candidate.id === threadId);
    if (!thread) throw new Error("The selected conversation is no longer available");
    this.updateRepository(repositoryId, {
      threads: [thread, ...this.repository(repositoryId).threads.filter((candidate) => candidate.id !== thread.id)],
      activeThreadId: threadId,
      loaded: Object.hasOwn(this.repository(repositoryId).itemsByThread, threadId),
      error: null,
    });
    try {
      await this.ensureCachedHistory(repositoryId, threadId);
    } catch (cause) {
      if (this.repository(repositoryId).activeThreadId === threadId) {
        // Selection has finished even when no local page is available. A ready
        // provider can still recover it below; unavailable providers must not
        // leave the conversation behind an indefinite loading screen.
        this.updateRepository(repositoryId, {
          loaded: true, loading: false,
          error: localError("cached_history_failed", cause, true),
        });
      }
    }
    if (this.repository(repositoryId).activeThreadId !== threadId) return;
    if (["checking", "failed"].includes(this.snapshot.initializationPhase)) return;
    await this.synchronizeConversation(repositoryId, threadId);
  }

  private async synchronizeConversation(repositoryId: number, threadId: number): Promise<void> {
    const thread = this.repository(repositoryId).threads.find((candidate) => candidate.id === threadId);
    if (!thread || this.repository(repositoryId).activeThreadId !== threadId) return;
    await this.selectRepositoryConnection(repositoryId, thread.providerConnectionId, thread.lastModelId, false);
    if (this.repository(repositoryId).activeThreadId !== threadId) return;
    const connection = this.snapshot.connections.find((candidate) => candidate.connection.id === thread.providerConnectionId);
    if (connectionReadyForChat(connection)) await this.resumeThread(repositoryId, threadId);
  }

  newConversation(repositoryId: number): void {
    this.updateRepository(repositoryId, {
      activeThreadId: null,
      loaded: true,
      historyUnavailable: false,
      error: null,
    });
  }

  setDraft(repositoryId: number, draft: string): void {
    this.updateRepository(repositoryId, { draft: limitDraftText(draft) });
  }

  async restoreDraft(repositoryId: number): Promise<void> {
    try {
      const text = await this.drafts.load(repositoryId, "analysis");
      this.updateRepository(repositoryId, { draft: text ?? "", draftLoaded: true, draftStorageError: this.drafts.getSnapshot(repositoryId, "analysis").error }, false);
    } catch {
      this.updateRepository(repositoryId, { draftStorageError: "restore" });
    }
  }

  async retryDraftStorage(repositoryId: number): Promise<void> {
    if (this.repository(repositoryId).draftLoaded === false) return this.restoreDraft(repositoryId);
    await this.drafts.retry(repositoryId, "analysis");
    this.updateRepository(repositoryId, { draftStorageError: this.drafts.getSnapshot(repositoryId, "analysis").error });
  }

  setAttachments(repositoryId: number, attachments: ContextAttachmentRequest[]): void {
    const unique = attachments
      .filter((attachment, index, values) => values.findIndex((candidate) => candidate.kind === attachment.kind && candidate.id === attachment.id) === index)
      .slice(0, 32);
    this.updateRepository(repositoryId, { attachments: unique, context: null });
  }

  setFocusedFinding(repositoryId: number, id: string | null): void {
    const current = this.snapshot.repositories[repositoryId]?.focusedFinding?.id ?? null;
    if (current === id) return;
    this.updateRepository(repositoryId, { focusedFinding: id ? { kind: "finding", id } : null, context: null });
  }

  setModel(model: string | null): void {
    const selected = this.snapshot.models.find((candidate) => candidate.id === model);
    if ((selected?.id ?? null) === this.snapshot.selectedModel) return;
    this.connectionSelectionSequence += 1;
    this.patch({
      consent: null,
      selectedModel: selected?.id ?? null,
      selectedEffort: selected?.reasoningEfforts[0] ?? null,
    });
    void this.refreshConsent().catch(() => undefined);
  }

  async selectConnection(connectionId: string, forceRefresh = false): Promise<void> {
    const current = this.snapshot.connections.find((item) => item.connection.id === connectionId);
    if (!current || !current.connection.enabled) throw new Error("The selected AI connection is unavailable");
    const selectionSequence = ++this.connectionSelectionSequence;
    if (forceRefresh) this.invalidateProviderReads();
    const epoch = this.consentEpoch;
    if (current.connection.driver === "codex" && current.connection.id === "codex_default") {
      const runtime = this.snapshot.runtime;
      const [account, availableModels] = runtime && ["ready", "active_turn"].includes(runtime.phase)
        ? await this.readCodexIdentity(current, runtime)
        : [null, null] as const;
      const models = availableModels ?? current.models;
      const selectedModel = connectionId === this.snapshot.selectedConnectionId
        && models.some((model) => model.id === this.snapshot.selectedModel)
        ? this.snapshot.selectedModel
        : models.find((model) => model.isDefault)?.id ?? models[0]?.id ?? null;
      const selected = models.find((model) => model.id === selectedModel);
      const consent = selectedModel
        ? await this.readConnectionConsent(connectionId, selectedModel)
        : null;
      if (epoch !== this.consentEpoch || selectionSequence !== this.connectionSelectionSequence) return;
      this.patch({
        selectedConnectionId: connectionId,
        account,
        models,
        selectedModel,
        selectedEffort: connectionId === this.snapshot.selectedConnectionId
          && selected?.reasoningEfforts.includes(this.snapshot.selectedEffort ?? "")
          ? this.snapshot.selectedEffort
          : selected?.reasoningEfforts[0] ?? null,
        consent,
        usage: null,
        login: account?.signedIn
          ? { phase: "success", loginId: null, userCode: null, message: null }
          : this.snapshot.login,
      });
      return;
    }
    const snapshot = forceRefresh || current.health === "unchecked"
      ? await this.client.refreshConnection(connectionId)
      : current;
    const connections = this.snapshot.connections.map((item) => item.connection.id === connectionId ? snapshot : item);
    const selectedModel = snapshot.models.find((model) => model.isDefault)?.id ?? snapshot.models[0]?.id ?? null;
    const consent = selectedModel
      ? await this.readConnectionConsent(connectionId, selectedModel, snapshot)
      : null;
    if (epoch !== this.consentEpoch || selectionSequence !== this.connectionSelectionSequence) return;
    this.patch({
      connections,
      selectedConnectionId: connectionId,
      account: null,
      models: snapshot.models,
      selectedModel,
      selectedEffort: snapshot.models.find((model) => model.id === selectedModel)?.reasoningEfforts[0] ?? null,
      consent,
      usage: null,
    });
  }

  async selectRepositoryConnection(
    repositoryId: number,
    connectionId: string,
    preferredModel: string | null = null,
    forceRefresh = false,
  ): Promise<void> {
    const current = this.snapshot.connections.find((item) => item.connection.id === connectionId);
    if (!current) throw new Error("The selected AI connection is unavailable");
    if (forceRefresh) this.invalidateProviderReads();
    const epoch = this.consentEpoch;
    const repository = this.repository(repositoryId);
    const selectionSequence = (this.repositorySelectionSequence.get(repositoryId) ?? 0) + 1;
    this.repositorySelectionSequence.set(repositoryId, selectionSequence);
    this.updateRepository(repositoryId, { selectionLoading: true });
    try {
      let connection = current;
      let account = this.snapshot.account;
      let models: CompanionModel[] = current.models;
      if (current.connection.enabled && current.connection.driver === "codex" && current.connection.id === "codex_default") {
        const runtime = this.snapshot.runtime;
        if (runtime && ["ready", "active_turn"].includes(runtime.phase)) {
          const [availableAccount, availableModels] = await this.readCodexIdentity(current, runtime);
          account = availableAccount;
          models = availableModels ?? current.models;
        }
      } else if (current.connection.enabled && (forceRefresh || current.health === "unchecked")) {
        connection = await this.client.refreshConnection(connectionId);
        models = connection.models;
      }
      const previous = repository.selection.connectionId === connectionId
        ? repository.selection
        : null;
      const modelId = [preferredModel, previous?.modelId]
        .find((candidate) => candidate && models.some((model) => model.id === candidate))
        ?? models.find((model) => model.isDefault)?.id
        ?? models[0]?.id
        ?? null;
      const model = models.find((candidate) => candidate.id === modelId);
      const effort = model?.reasoningEfforts.includes(previous?.effort ?? "")
        ? previous?.effort ?? null
        : model?.reasoningEfforts[0] ?? null;
      const consent = modelId && connection.connection.enabled
        ? await this.readConnectionConsent(connectionId, modelId, connection)
        : null;
      if (selectionSequence !== this.repositorySelectionSequence.get(repositoryId)) return;
      if (epoch !== this.consentEpoch) {
        this.updateRepository(repositoryId, { selectionLoading: false });
        return;
      }
      const connections = connection === current ? this.snapshot.connections : this.snapshot.connections.map((candidate) => candidate.connection.id === connectionId ? connection : candidate);
      this.patch({ connections, account });
      this.updateRepository(repositoryId, {
        selection: { repositoryId, connectionId, modelId, effort },
        selectionModels: models,
        selectionLoading: false,
        consent,
      });
    } catch (cause) {
      if (selectionSequence === this.repositorySelectionSequence.get(repositoryId)) {
        this.updateRepository(repositoryId, { selectionLoading: false });
      }
      throw cause;
    }
  }

  setRepositoryModel(repositoryId: number, modelId: string | null): void {
    const repository = this.repository(repositoryId);
    const model = repository.selectionModels.find((candidate) => candidate.id === modelId);
    if ((model?.id ?? null) === repository.selection.modelId) return;
    this.repositorySelectionSequence.set(repositoryId, (this.repositorySelectionSequence.get(repositoryId) ?? 0) + 1);
    this.updateRepository(repositoryId, {
      consent: null,
      selectionLoading: false,
      selection: {
        ...repository.selection,
        modelId: model?.id ?? null,
        effort: model?.reasoningEfforts[0] ?? null,
      },
    });
    void this.refreshRepositoryConsent(repositoryId).catch(() => undefined);
  }

  setRepositoryEffort(repositoryId: number, effort: string | null): void {
    const repository = this.repository(repositoryId);
    const model = repository.selectionModels.find((candidate) => candidate.id === repository.selection.modelId);
    this.updateRepository(repositoryId, {
      selection: {
        ...repository.selection,
        effort: model?.reasoningEfforts.includes(effort ?? "") ? effort : null,
      },
    });
  }

  setEffort(effort: string | null): void {
    const selected = this.snapshot.models.find((model) => model.id === this.snapshot.selectedModel);
    this.patch({ selectedEffort: selected?.reasoningEfforts.includes(effort ?? "") ? effort : null });
  }

  async preview(repositoryId: number): Promise<RepositoryChatContext> {
    const repository = this.repository(repositoryId);
    const sequence = (this.contextPreviewSequence.get(repositoryId) ?? 0) + 1;
    this.contextPreviewSequence.set(repositoryId, sequence);
    const context = await this.client.previewContext({
      repositoryId,
      query: repository.draft,
      attachments: attachmentsForPrompt(repository),
    });
    const current = this.repository(repositoryId);
    if (sequence === this.contextPreviewSequence.get(repositoryId)
      && current.attachments === repository.attachments && current.focusedFinding === repository.focusedFinding
      && current.draft === repository.draft
      && current.activeThreadId === repository.activeThreadId) {
      this.updateRepository(repositoryId, { context, contextChanged: false });
    }
    return context;
  }

  async previewSend(repositoryId: number): Promise<CompanionSendPreview> {
    const { draft, activeThreadId, selection, attachments, focusedFinding } = this.repository(repositoryId);
    const context = await this.preview(repositoryId);
    return { draft, activeThreadId, selection, attachments, focusedFinding, context };
  }

  markScanContext(repositoryId: number, scanId: number | null): void {
    const context = this.repository(repositoryId).context;
    if (context && context.scan.id !== scanId) {
      this.updateRepository(repositoryId, {
        contextChanged: true,
        context: {
          ...context,
          freshness: {
            ...context.freshness,
            state: scanId === null ? "unknown" : "newer_scan",
            message: scanId === null
              ? "No completed scan is available. The next send will rebuild this context."
              : "A newer completed scan is available. The next send will rebuild this context.",
          },
        },
      });
    }
  }

  async createThread(repositoryId: number, title?: string): Promise<CompanionThread> {
    const selection = this.repository(repositoryId).selection;
    const thread = await this.client.createThread(
      repositoryId,
      selection.connectionId,
      selection.modelId,
      title?.trim() || null,
    );
    const repository = this.repository(repositoryId);
    this.updateRepository(repositoryId, {
      threads: [thread, ...repository.threads.filter((candidate) => candidate.id !== thread.id)],
      activeThreadId: thread.id,
      historyUnavailable: false,
      error: null,
    });
    this.patch({
      conversationIndex: {
        schemaVersion: 1,
        threads: [thread, ...(this.snapshot.conversationIndex?.threads ?? []).filter((candidate) => candidate.id !== thread.id)],
      },
    });
    return thread;
  }

  async resumeThread(repositoryId: number, threadId: number): Promise<void> {
    const key = `${repositoryId}:${threadId}`;
    const existing = this.resumeRequests.get(key);
    if (existing) return existing;
    const task = this.resumeConversation(repositoryId, threadId);
    this.resumeRequests.set(key, task);
    try { await task; }
    finally { if (this.resumeRequests.get(key) === task) this.resumeRequests.delete(key); }
  }

  private async resumeConversation(repositoryId: number, threadId: number): Promise<void> {
    this.updateRepository(repositoryId, { activeThreadId: threadId, error: null });
    this.resuming.add(threadId);
    try {
      await this.ensureCachedHistory(repositoryId, threadId);
    } catch {
      // A provider snapshot can still recover and populate a missing legacy cache.
    }
    if (this.repository(repositoryId).activeThreadId === threadId) this.updateRepository(repositoryId, { loading: true });
    try {
      const snapshot = await this.client.resumeThread(repositoryId, threadId);
      // The durable page owns presentation IDs, citations and ordering. Do not
      // publish the transport snapshot only to replace it with that page again.
      let reconciled = false;
      if (this.repository(repositoryId).activeThreadId === threadId && this.historyNamespaces.has(threadId) && !this.readingEarlierHistory(repositoryId, threadId)) {
        const request = ++this.historyRequestSequence;
        this.historyRequests.set(threadId, request);
        try {
          const cached = await this.readCachedHistory(repositoryId, threadId);
          if (this.historyRequests.get(threadId) !== request) reconciled = true;
          else if (!this.readingEarlierHistory(repositoryId, threadId)) {
            this.applyCachedSnapshot(cached);
            reconciled = true;
          }
        } catch { /* A missing cache can still recover from the provider snapshot. */ }
        finally { if (this.historyRequests.get(threadId) === request) this.historyRequests.delete(threadId); }
      }
      if (!reconciled) this.applySnapshot(snapshot);
      else {
        const current = this.repository(repositoryId);
        this.updateRepository(repositoryId, threadUsageUpdate(current, snapshot.thread, snapshot.usage));
      }
      for (const event of this.queuedEvents.get(threadId) ?? []) this.applyRoutedEvent(event);
      this.queuedEvents.delete(threadId);
      if (this.repository(repositoryId).activeThreadId === threadId) {
        this.updateRepository(repositoryId, { loading: false, loaded: true, historyUnavailable: false });
      }
    } catch {
      if (this.repository(repositoryId).activeThreadId === threadId) {
        this.updateRepository(repositoryId, {
          loading: false,
          loaded: true,
          historyUnavailable: true,
          error: null,
        });
      }
    } finally {
      this.resuming.delete(threadId);
    }
  }

  async renameThread(repositoryId: number, threadId: number, title: string): Promise<void> {
    const updated = await this.client.renameThread(repositoryId, threadId, title);
    this.replaceThread(repositoryId, updated);
  }

  async setThreadArchived(repositoryId: number, threadId: number, archived: boolean): Promise<void> {
    const updated = await this.client.setThreadArchived(repositoryId, threadId, archived);
    this.replaceThread(repositoryId, updated);
    if (archived && this.repository(repositoryId).activeThreadId === threadId) {
      const next = this.repository(repositoryId).threads.find((thread) => !thread.archivedAt && thread.id !== threadId);
      if (next) await this.activateConversation(repositoryId, next.id);
      else this.updateRepository(repositoryId, { activeThreadId: null });
    }
  }

  async send(repositoryId: number, reviewed?: CompanionSendPreview): Promise<"review-required" | void> {
    if (this.sending.has(repositoryId)) return;
    let repository = this.repository(repositoryId);
    const prompt = repository.draft.trim();
    if (!prompt || repository.pending || repository.selectionLoading) return;
    if (!reviewed && repository.error?.code === contextReviewError.code) return "review-required";
    if (reviewed && (reviewed.context.repository.id !== repositoryId
      || reviewed.draft.trim() !== prompt
      || reviewed.activeThreadId !== repository.activeThreadId
      || reviewed.selection !== repository.selection
      || reviewed.attachments !== repository.attachments
      || reviewed.focusedFinding !== repository.focusedFinding)) {
      this.updateRepository(repositoryId, { error: contextReviewError });
      return "review-required";
    }
    this.sending.add(repositoryId);
    try {
      let threadId = repository.activeThreadId;
      const selection = repository.selection;
      const selectedConnection = this.snapshot.connections.find((candidate) => candidate.connection.id === selection.connectionId);
      if (!connectionReadyForChat(selectedConnection)) {
        throw new Error(`The selected AI connection '${selectedConnection?.connection.displayName ?? selection.connectionId}' is unavailable.`);
      }
      const effectiveModel = selection.modelId
        && repository.selectionModels.some((model) => model.id === selection.modelId)
        ? selection.modelId
        : null;
      if (!effectiveModel) {
        throw new Error("Select an available model for this AI connection before sending.");
      }
      const attachments = attachmentsForPrompt(repository);
      const clientTurnId = createClientTurnId();
      const userItem: CompanionTimelineItem = {
        id: `local:${clientTurnId}`,
        kind: "user_message",
        status: "completed",
        text: prompt,
        activity: null,
        citations: [],
        error: null,
        modelId: effectiveModel,
        createdAt: new Date().toISOString(),
      };
      repository = this.repository(repositoryId);
      this.updateRepository(repositoryId, {
        draft: "",
        pending: { clientTurnId, threadId, userItem, connectionId: selection.connectionId, phase: "preparing", startedAt: Date.now(), turnId: null, prompt },
        itemsByThread: threadId ? {
          ...repository.itemsByThread,
          [threadId]: [...(repository.itemsByThread[threadId] ?? []), userItem],
        } : repository.itemsByThread,
        error: null,
      });
      try {
        if (!threadId) {
          threadId = (await this.createThread(repositoryId, prompt)).id;
          const created = this.repository(repositoryId);
          this.updateRepository(repositoryId, {
            pending: created.pending ? { ...created.pending, threadId } : null,
            itemsByThread: {
              ...created.itemsByThread,
              [threadId]: [...(created.itemsByThread[threadId] ?? []), userItem],
            },
          });
        }
        const context = reviewed?.context ?? await this.client.previewContext({ repositoryId, query: prompt, attachments });
        this.updateRepository(repositoryId, { context, contextChanged: false });
        const activeThread = this.repository(repositoryId).threads.find((thread) => thread.id === threadId);
        if (!activeThread) throw new Error("Active chat thread is unavailable");
        const prepared = this.repository(repositoryId).pending;
        if (prepared?.clientTurnId === clientTurnId) this.updateRepository(repositoryId, { pending: { ...prepared, phase: "starting" } });
        const started = await this.client.startTurn({
          repositoryId,
          threadId,
          connectionId: selection.connectionId,
          prompt,
          model: effectiveModel,
          effort: selection.effort,
          context,
          clientTurnId,
        });
        // Keep the prepared prompt recoverable until the provider accepts it.
        // A follow-up edit already belongs to the user and must be retained.
        if (!this.repository(repositoryId).draft) this.persistAnalysisDraft(repositoryId, "");
        const current = this.repository(repositoryId).pending;
        if (current?.clientTurnId === clientTurnId) {
          this.updateRepository(repositoryId, { pending: { ...current, turnId: started.turnId, phase: current.phase === "starting" ? "waiting" : current.phase } });
        }
        try {
          this.applyCachedSnapshot(await this.readCachedHistory(repositoryId, threadId));
        } catch {
          // The accepted turn is still authoritative when the local cache cannot be re-read.
        }
        const refreshedThread = this.repository(repositoryId).threads.find((thread) => thread.id === threadId) ?? activeThread;
        this.replaceThread(repositoryId, {
          ...refreshedThread,
          provider: selectedConnection?.connection.driver.replace("open_code", "opencode") ?? refreshedThread.provider,
          providerConnectionId: selection.connectionId,
          lastModelId: effectiveModel,
          updatedAt: new Date().toISOString(),
        });
      } catch (cause) {
        const failed = this.repository(repositoryId);
        const missingHistory = isMissingProviderHistory(cause);
        const reviewRequired = (cause instanceof Error ? cause.message : String(cause)) === CONTEXT_REVIEW_REQUIRED;
        this.updateRepository(repositoryId, {
          draft: failed.draft || prompt,
          pending: null,
          historyUnavailable: missingHistory || failed.historyUnavailable,
          itemsByThread: threadId ? {
            ...failed.itemsByThread,
            [threadId]: (failed.itemsByThread[threadId] ?? []).filter((item) => item.id !== userItem.id),
          } : failed.itemsByThread,
          error: reviewRequired ? contextReviewError : missingHistory
            ? { code: "history_unavailable", message: "Provider history is unavailable. Send again to continue this local conversation in a new provider segment.", retryable: true }
            : localError("turn_start_failed", cause, true),
        });
        if (reviewRequired) return "review-required";
      }
    } finally {
      this.sending.delete(repositoryId);
    }
  }

  async stop(repositoryId: number): Promise<void> {
    const repository = this.repository(repositoryId);
    const pending = repository.pending;
    if (!pending?.turnId || !pending.threadId) return;
    await this.client.interruptTurn(repositoryId, pending.threadId, pending.turnId);
  }

  async setRepositoryConsent(repositoryId: number, accepted: boolean): Promise<void> {
    const repository = this.repository(repositoryId);
    const model = repository.selection.modelId ?? "unknown/model";
    this.connectionConsents.clear();
    const epoch = ++this.consentEpoch;
    const consent = await this.client.setConnectionConsent(repository.selection.connectionId, model, accepted);
    this.connectionConsents.clear();
    const current = this.repository(repositoryId).selection;
    if (epoch !== this.consentEpoch || current.connectionId !== repository.selection.connectionId || current.modelId !== repository.selection.modelId) {
      throw new Error("The selected connection or model changed. Review the disclosure again.");
    }
    this.consentEpoch += 1;
    this.updateRepository(repositoryId, { consent });
  }

  async setConsent(accepted: boolean): Promise<void> {
    this.connectionConsents.clear();
    const epoch = ++this.consentEpoch;
    const connectionId = this.snapshot.selectedConnectionId;
    const model = this.snapshot.selectedModel;
    const consent = await this.client.setConnectionConsent(connectionId, model ?? "unknown/model", accepted);
    this.connectionConsents.clear();
    if (epoch !== this.consentEpoch || connectionId !== this.snapshot.selectedConnectionId || model !== this.snapshot.selectedModel) {
      throw new Error("The selected connection or model changed. Review the disclosure again.");
    }
    this.consentEpoch += 1;
    this.patch({ consent });
  }

  private async refreshConsent(): Promise<void> {
    const connectionId = this.snapshot.selectedConnectionId;
    const model = this.snapshot.selectedModel;
    if (!model) {
      this.patch({ consent: null });
      return;
    }
    const epoch = this.consentEpoch;
    const consent = await this.readConnectionConsent(connectionId, model);
    if (epoch !== this.consentEpoch || this.snapshot.selectedConnectionId !== connectionId || this.snapshot.selectedModel !== model) return;
    this.patch({ consent });
  }

  private async refreshRepositoryConsent(repositoryId: number): Promise<void> {
    const repository = this.repository(repositoryId);
    const { connectionId, modelId } = repository.selection;
    if (!modelId) {
      this.updateRepository(repositoryId, { consent: null });
      return;
    }
    const epoch = this.consentEpoch;
    const consent = await this.readConnectionConsent(connectionId, modelId);
    const current = this.repository(repositoryId).selection;
    if (epoch !== this.consentEpoch || current.connectionId !== connectionId || current.modelId !== modelId) return;
    this.updateRepository(repositoryId, { consent });
  }

  async logoutSharedCodexAccount(): Promise<void> {
    this.invalidateProviderReads();
    await this.client.logout(true);
    this.invalidateProviderReads();
    const account = await this.client.accountStatus();
    this.patch({
      account,
      models: [],
      selectedModel: null,
      selectedEffort: null,
      usage: null,
    });
  }

  async refreshUsage(forceRefresh = false): Promise<void> {
    this.patch({ usage: await this.client.usage(forceRefresh) });
  }

  private handleEvent(event: CompanionEventEnvelope): void {
    const route = JSON.stringify([event.connectionId ?? "codex_default",event.cacheEpoch ?? ""]);
    const lastSequence = this.lastSequenceByConnection.get(route) ?? 0;
    if (event.sequence <= lastSequence) return;
    this.lastSequenceByConnection.set(route, event.sequence);
    if (event.threadId && this.resuming.has(event.threadId)) {
      const queued = this.queuedEvents.get(event.threadId) ?? [];
      queued.push(event);
      this.queuedEvents.set(event.threadId, queued.slice(-1000));
      return;
    }
    this.applyRoutedEvent(event);
  }

  private applyRoutedEvent(event: CompanionEventEnvelope): void {
    const watermarkKey=`${event.threadId}:${event.providerSegmentId??0}:${event.connectionId??""}:${event.cacheEpoch??""}`;
    if(["item_started","item_delta","item_completed","activity"].includes(event.kind)&&event.sequence<=(this.historyWatermarks.get(watermarkKey)??-1))return;
    const delta=event.kind==="item_delta"?parseDelta(event.payload):null;
    if(delta && this.eventRouteIsCurrent(event)) {
      const route=JSON.stringify([event.repositoryId,event.threadId,event.providerSegmentId,event.connectionId,event.providerThreadId,event.turnId,event.cacheEpoch,delta.itemId]);
      const buffered=this.deltaBuffers.get(route);
      if(buffered){
        const text=parseDelta(buffered.event.payload)?.text??"";
        buffered.event={...event,payload:{...record(event.payload),itemId:delta.itemId,text:text+delta.text, ...(typeof record(buffered.event.payload).perfReceivedAt==="number"?{perfReceivedAt:record(buffered.event.payload).perfReceivedAt}:{})}};
        if(text.length+delta.text.length>=64_000)this.flushDeltaRoute(route);
        return;
      }
      const id=this.historyItemId(event.repositoryId!,event.threadId!,delta.itemId,event);
      const existing=this.repository(event.repositoryId!).itemsByThread[event.threadId!]?.find((item)=>item.id===id);
      // Bound bursts relative to the last update. Durable backend batches are
      // already paced at 50 ms: do not add another whole frame to each batch.
      const delay = this.deltaDelay(route);
      if(!existing?.text || delay === 0) {this.applyDelta(route,event);return;}
      if(this.deltaBuffers.size>=MAX_DELTA_ROUTES)this.flushDeltaRoute(this.deltaBuffers.keys().next().value!);
      this.deltaBuffers.set(route,{event,timer:setTimeout(()=>this.flushDeltaRoute(route),delay)});
      return;
    }
    this.flushDelta();
    this.applyEventNow(event);
  }
  private flushDeltaRoute(route:string):void {
    const buffer=this.deltaBuffers.get(route);
    if(!buffer)return;
    clearTimeout(buffer.timer);this.deltaBuffers.delete(route);this.applyDelta(route,buffer.event);
  }
  private flushDelta():void {
    const buffers=[...this.deltaBuffers.entries()].sort((left,right)=>left[1].event.sequence-right[1].event.sequence);
    this.deltaBuffers.clear();
    for(const [route,buffer] of buffers){clearTimeout(buffer.timer);this.applyDelta(route,buffer.event);}
  }
  private pruneDeltaCadence(now: number): void {
    for (const [route, at] of this.deltaFlushTimes) {
      if (now - at >= DELTA_FRAME_MS) this.deltaFlushTimes.delete(route);
    }
  }
  private deltaDelay(route: string): number {
    const now = performance.now();
    this.pruneDeltaCadence(now);
    const last = this.deltaFlushTimes.get(route);
    if (last !== undefined) return Math.max(0, DELTA_FRAME_MS - (now - last));
    // Under unusually many active routes retain the old bounded trailing
    // behavior instead of growing cadence metadata or flushing every delta.
    return this.deltaFlushTimes.size < MAX_DELTA_ROUTES ? 0 : DELTA_FRAME_MS;
  }
  private applyDelta(route: string, event: CompanionEventEnvelope): void {
    const now = performance.now();
    this.pruneDeltaCadence(now);
    if (this.deltaFlushTimes.has(route) || this.deltaFlushTimes.size < MAX_DELTA_ROUTES) {
      this.deltaFlushTimes.set(route, now);
    }
    this.applyEventNow(event);
  }

  private applyEventNow(event: CompanionEventEnvelope): void {
    if (event.kind === "runtime_status") {
      try {
        const runtime = parseCompanionRuntimeStatus(event.payload);
        const prior = this.snapshot.runtime;
        if (runtime.startedAt !== prior?.startedAt || (prior?.phase !== runtime.phase && !["ready", "active_turn"].includes(runtime.phase))) {
          this.invalidateProviderReads();
        }
        this.patch({ runtime });
        if (["interrupted", "offline", "unavailable"].includes(runtime.phase)) {
          for (const repository of Object.values(this.snapshot.repositories)) {
            const pending = repository.pending;
            if (!pending || pending.phase === "preparing" || pending.connectionId !== (event.connectionId ?? "codex_default")) continue;
            if (pending.threadId) this.setThreadItems(repository.repositoryId, pending.threadId,
              (this.liveHistory.get(`${repository.repositoryId}:${pending.threadId}`) ?? repository.itemsByThread[pending.threadId] ?? []).map((item) => item.status === "streaming" ? { ...item, status: "interrupted" } : item));
            this.updateRepository(repository.repositoryId, {
              pending: null,
              draft: repository.draft || pending.prompt,
              error: { code: "runtime_interrupted", message: runtime.message ?? "The provider disconnected before completing the response.", retryable: true },
            });
          }
        }
      } catch { /* ignored malformed normalized payload */ }
      return;
    }
    if (event.kind === "account_status") {
      const payload = record(event.payload);
      const success = typeof payload.success === "boolean" ? payload.success : null;
      const error = typeof payload.error === "string" ? payload.error : null;
      const authModeChanged = Object.prototype.hasOwnProperty.call(payload, "authMode");
      if (success === true) {
        this.patch({ login: { phase: "success", loginId: null, userCode: null, message: null } });
      } else if (success === false) {
        this.patch({ login: loginFailure(error ?? "Codex sign-in failed.") });
      } else if (authModeChanged) {
        const signingIn = ["starting", "waiting"].includes(this.snapshot.login.phase);
        this.patch({ login: {
          phase: signingIn && payload.authMode ? "success" : "external_change",
          loginId: null,
          userCode: null,
          message: signingIn && payload.authMode ? null : "The shared Codex account changed outside cvec.",
        } });
      }
      if ((event.connectionId ?? "codex_default") === "codex_default") {
        void this.startRuntime().catch(() => undefined);
      }
      return;
    }
    if (event.kind === "usage") {
      if (!event.repositoryId || !event.threadId) return;
      const usage = parseTokenUsage(event.payload);
      if (!usage || !this.eventRouteIsCurrent(event)) return;
      const repository = this.repository(event.repositoryId);
      const thread = repository.threads.find((item) => item.id === event.threadId)!;
      this.updateRepository(event.repositoryId, threadUsageUpdate(repository, {
        id: thread.id,
        providerConnectionId: event.connectionId ?? thread.providerConnectionId,
        lastModelId: event.modelId ?? null,
      }, usage));
      this.retainHistory(event.repositoryId,event.threadId,repository.itemsByThread[event.threadId]??[]);
      return;
    }
    if (event.kind === "thread_updated") {
      try {
        const updated = parseCompanionThread(event.payload);
        if (updated.repositoryId !== event.repositoryId
          || updated.id !== event.threadId
          || (event.connectionId && updated.providerConnectionId !== event.connectionId)) return;
        this.replaceThread(updated.repositoryId, updated);
      } catch { /* ignored malformed normalized payload */ }
      return;
    }
    if (!event.repositoryId || !event.threadId || !this.eventRouteIsCurrent(event)) return;
    const watermarkKey=`${event.threadId}:${event.providerSegmentId??0}:${event.connectionId??""}:${event.cacheEpoch??""}`;
    if(["item_started","item_delta","item_completed","activity"].includes(event.kind) && event.sequence<=(this.historyWatermarks.get(watermarkKey)??-1))return;
    const repository = this.repository(event.repositoryId);
    const receivedAt=record(event.payload).perfReceivedAt;
    const rawItemId=record(event.payload).itemId;
    if(event.kind==="item_delta" && repository.activeThreadId===event.threadId && !this.liveHistory.has(`${event.repositoryId}:${event.threadId}`) && typeof receivedAt==="number" && Number.isFinite(receivedAt) && typeof rawItemId==="string"){
      const itemId=this.historyItemId(event.repositoryId,event.threadId,rawItemId,event);
      requestAnimationFrame(()=>requestAnimationFrame(()=>{
        const article=[...document.querySelectorAll<HTMLElement>("[data-message-id]")].find((node)=>node.dataset.messageId===itemId);
        const bounds=article?.getBoundingClientRect();
        if(document.visibilityState==="visible" && bounds && bounds.bottom>0 && bounds.top<window.innerHeight){
          void invoke("record_ui_timings",{sample:{streamToPaintMs:Math.max(0,Date.now()-receivedAt)}}).catch(()=>undefined);
        }
      }));
    }
    const items = this.liveHistory.get(`${event.repositoryId}:${event.threadId}`) ?? repository.itemsByThread[event.threadId] ?? [];
    const pending = repository.pending;
    const matchesPending = Boolean(pending && pending.phase !== "preparing"
      && pending.threadId === event.threadId
      && (!event.connectionId || pending.connectionId === event.connectionId)
      && (!pending.turnId || !event.turnId || pending.turnId === event.turnId));
    const progress = (phase: PendingCompanionTurn["phase"]) => {
      if (matchesPending && pending && (pending.phase !== phase || (!pending.turnId && event.turnId))) this.updateRepository(event.repositoryId!, {
        pending: { ...pending, turnId: event.turnId ?? pending.turnId, phase },
      });
    };
    if (event.kind === "turn_started") {
      progress("waiting");
    } else if (event.kind === "item_started" || event.kind === "item_completed") {
      const parsedItem = parseTimelineItem(event.payload);
      if (!parsedItem) return;
      const item={...parsedItem,modelId:parsedItem.modelId??event.modelId,id:this.historyItemId(event.repositoryId,event.threadId,parsedItem.id,event)};
      if (item.activity?.kind === "reasoning") progress(event.kind === "item_started" ? "thinking" : "working");
      else if (item.kind === "assistant_message") progress(event.kind === "item_completed" ? "working" : item.text ? "responding" : "waiting");
      if (item.kind === "user_message" && repository.pending && item.text.trim() === repository.pending.prompt) {
        const optimisticId = `local:${repository.pending.clientTurnId}`;
        const optimisticIndex = items.findIndex((candidate) => candidate.id === optimisticId);
        if (optimisticIndex >= 0) {
          const reconciled = [...items];
          reconciled[optimisticIndex] = { ...item, modelId: reconciled[optimisticIndex].modelId ?? item.modelId, status: "completed" };
          this.setThreadItems(event.repositoryId, event.threadId, reconciled);
          return;
        }
      }
      this.setThreadItems(event.repositoryId, event.threadId, upsertItem(items, item));
    } else if (event.kind === "item_delta") {
      const parsedDelta = parseDelta(event.payload);
      if (!parsedDelta) return;
      const delta={...parsedDelta,itemId:this.historyItemId(event.repositoryId,event.threadId,parsedDelta.itemId,event)};
      const pendingChange=matchesPending && pending && (pending.phase!=="responding" || (!pending.turnId && event.turnId)) ? {pending:{...pending,phase:"responding" as const,turnId:event.turnId??pending.turnId}} : {};
      const existing = items.find((item) => item.id === delta.itemId);
      const next: CompanionTimelineItem = existing
        ? { ...existing, text: `${existing.text}${delta.text}`, status: "streaming" }
        : { id: delta.itemId, kind: "assistant_message", status: "streaming", text: delta.text, activity: null, citations: [], error: null, modelId: event.modelId, createdAt: new Date().toISOString() };
      this.setThreadItems(event.repositoryId, event.threadId, upsertItem(items, next), pendingChange);
    } else if (event.kind === "activity") {
      const activity = parseActivity(event.payload);
      if (!activity || activity.kind === "context") return;
      if (activity.kind === "reasoning") { progress("thinking"); return; }
      if (activity.kind === "runtime") { progress("working"); return; }
      const item: CompanionTimelineItem = { id: this.historyItemId(event.repositoryId,event.threadId,generatedEventItemId(event,"activity"),event), kind: "activity", status: "completed", text: "", activity, citations: [], error: null, createdAt: new Date().toISOString() };
      this.setThreadItems(event.repositoryId, event.threadId, upsertItem(items, item));
    } else if (event.kind === "turn_completed") {
      if (!matchesPending) return;
      const status = record(event.payload).status;
      const interrupted = status === "interrupted";
      const failed = status === "failed";
      this.setThreadItems(event.repositoryId, event.threadId, items.map((item) => item.status === "streaming"
        ? { ...item, status: interrupted ? "interrupted" : failed ? "failed" : "completed" } : item));
      this.updateRepository(event.repositoryId, {
        pending: null,
        ...(failed ? { error: { code: "turn_failed", message: typeof record(event.payload).message === "string" ? String(record(event.payload).message) : "The provider could not complete this response.", retryable: true } } : {}),
      });
      void this.refreshUsage(false).catch(() => undefined);
    } else if (event.kind === "error" || event.kind === "warning") {
      const error = parseError(event.payload);
      if (error) {
        const warning = event.kind === "warning" || error.code === "codex_warning" || error.code === "action_denied";
        const item: CompanionTimelineItem = {
          id: this.historyItemId(event.repositoryId,event.threadId,generatedEventItemId(event,warning?"warning":"error"),event),
          kind: warning ? "warning" : "error",
          status: warning ? "completed" : "failed",
          text: "",
          activity: null,
          citations: [],
          error,
          createdAt: new Date().toISOString(),
        };
        const terminal = !warning && matchesPending;
        this.setThreadItems(event.repositoryId, event.threadId, upsertItem(
          terminal ? items.map((entry) => entry.status === "streaming" ? { ...entry, status: "failed" as const } : entry) : items,
          item,
        ));
        if (!warning) this.updateRepository(event.repositoryId, { error, ...(terminal ? { pending: null } : {}) });
      }
    }
  }

  private eventRouteIsCurrent(event: CompanionEventEnvelope): boolean {
    if (!event.repositoryId || !event.threadId) return false;
    const thread = this.repository(event.repositoryId).threads.find((candidate) => candidate.id === event.threadId);
    return Boolean(thread
      && (!event.providerThreadId || thread.providerThreadId === event.providerThreadId)
      && (!event.connectionId || thread.providerConnectionId === event.connectionId)
      && (!event.driver || thread.provider === event.driver.replace("open_code", "opencode")));
  }

  private applySnapshot(snapshot: CompanionThreadSnapshot): void {
    this.flushDelta();
    const repository = this.repository(snapshot.thread.repositoryId);
    const existingItems = this.liveHistory.get(`${snapshot.thread.repositoryId}:${snapshot.thread.id}`) ?? repository.itemsByThread[snapshot.thread.id] ?? [];
    const incoming=snapshot.items.map((item)=>({...item,id:this.historyItemId(snapshot.thread.repositoryId,snapshot.thread.id,item.id)}));
    const items=boundedTimeline(mergeTimelineItems(existingItems,incoming));
    this.setThreadItems(snapshot.thread.repositoryId, snapshot.thread.id, items, {
      threads: [snapshot.thread, ...repository.threads.filter((thread) => thread.id !== snapshot.thread.id)],
      ...threadUsageUpdate(repository, snapshot.thread, snapshot.usage),
    });
    this.replaceIndexThread(snapshot.thread);
  }

  private readingEarlierHistory(repositoryId: number, threadId: number): boolean {
    return Boolean(this.repository(repositoryId).historyPages?.[threadId]?.before && this.liveHistory.has(`${repositoryId}:${threadId}`));
  }

  private async ensureCachedHistory(repositoryId: number, threadId: number): Promise<void> {
    if (Object.hasOwn(this.repository(repositoryId).itemsByThread, threadId)) return;
    const key = `${repositoryId}:${threadId}`;
    const existing = this.initialHistoryRequests.get(key);
    if (existing) return existing;
    const task = (async () => this.applyCachedSnapshot(await this.readCachedHistory(repositoryId, threadId)))();
    this.initialHistoryRequests.set(key, task);
    try { await task; }
    finally { if (this.initialHistoryRequests.get(key) === task) this.initialHistoryRequests.delete(key); }
  }

  private async readCachedHistory(repositoryId: number, threadId: number) {
    countPerformanceRender("cachedHistoryReads");
    return this.client.cachedThread(repositoryId, threadId);
  }

  private applyCachedSnapshot(snapshot:CompanionCachedThreadSnapshot,mode:"latest"|"earlier"="latest",before:string|null=null):void {
    countPerformanceRender("cachedHistoryHydrations");
    this.flushDelta();
    const parsed=parseCompanionCachedThreadSnapshot(snapshot);
    const repository=this.repository(parsed.thread.repositoryId);
    const paged=parsed.olderCursor!==undefined;
    if(paged)this.historyNamespaces.add(parsed.thread.id);
    for(const [route,sequence] of Object.entries(parsed.eventWatermarks??{}))this.historyWatermarks.set(`${parsed.thread.id}:${route}`,Math.max(sequence,this.historyWatermarks.get(`${parsed.thread.id}:${route}`)??-1));
    const live=repository.pending?.threadId===parsed.thread.id;
    const streaming=new Set(parsed.streamingItemIds??[]);
    const incoming=parsed.items.map((item)=>live&&streaming.has(item.id)?{...item,status:"streaming" as const}:item);
    const historyKey=`${parsed.thread.repositoryId}:${parsed.thread.id}`;
    const visibleExisting=repository.itemsByThread[parsed.thread.id]??[];
    const existing=mode==="latest"?this.liveHistory.get(historyKey)??visibleExisting:visibleExisting;
    if(mode==="earlier"&&!this.liveHistory.has(historyKey))this.liveHistory.set(historyKey,boundedTimeline(existing));
    const base=paged&&mode==="latest"?existing.filter((item)=>item.id.startsWith("local:")||(live&&item.status==="streaming")):existing;
    // An in-flight cache response can predate the last delivered delta and a
    // runtime interruption. Preserve that longer tail even after pending clears.
    const protectedIncoming=incoming.map((item)=>{const current=existing.find((entry)=>entry.id===item.id);return current&&((live&&current.status==="streaming")||current.status==="interrupted")&&current.text.startsWith(item.text)&&current.text.length>item.text.length?{...current,citations:item.citations.length?item.citations:current.citations}:item;});
    const mergedItems=mode==="earlier"?boundedTimeline(mergeTimelineItems(incoming,base),true):boundedTimeline(mergeTimelineItems(base,protectedIncoming,!paged));
    const items = shareTimelineItems(mergedItems, visibleExisting);
    if(mode==="latest")this.liveHistory.delete(historyKey);
    const labels={...(repository.historyProviderOrdinals?.[parsed.thread.id]??{}),...(parsed.providerOrdinals??{})};
    const retained=new Set(items.map((item)=>item.id));for(const key of Object.keys(labels))if(!retained.has(key))delete labels[key];
    const mergedSegments=[...new Map([...(repository.segmentsByThread[parsed.thread.id]??[]),...parsed.segments].map((segment)=>[segment.id,segment])).values()].sort((a,b)=>a.ordinal-b.ordinal);
    const retainedOrdinals=new Set(Object.values(labels));
    const latestOrdinal=mergedSegments.at(-1)?.ordinal;
    const segmentWindow=paged?mergedSegments.filter((segment)=>retainedOrdinals.has(segment.ordinal)||segment.ordinal===latestOrdinal):mergedSegments.slice(-256);
    const oldSegments = repository.segmentsByThread[parsed.thread.id] ?? [];
    const segments = shareArray(segmentWindow.map((segment) => {
      const prior = oldSegments.find((candidate) => candidate.id === segment.id);
      return prior && sameRecord(prior, segment) ? prior : segment;
    }), oldSegments);
    const ordinals = shareRecord(labels, repository.historyProviderOrdinals?.[parsed.thread.id]);
    const page = shareRecord({ before, older: parsed.olderCursor ?? null }, repository.historyPages?.[parsed.thread.id]);
    this.updateRepository(parsed.thread.repositoryId,{
      threads:[parsed.thread,...repository.threads.filter((thread)=>thread.id!==parsed.thread.id)],
      itemsByThread:{...repository.itemsByThread,[parsed.thread.id]:items},
      segmentsByThread:{...repository.segmentsByThread,[parsed.thread.id]:segments},
      historyProviderOrdinals:{...repository.historyProviderOrdinals,[parsed.thread.id]:ordinals},
      historyPages:{...repository.historyPages,[parsed.thread.id]:page},
      cached:true,
      ...(repository.activeThreadId===parsed.thread.id?{loaded:true,historyUnavailable:!parsed.providerHistoryAvailable}:{}),
    });
    this.retainHistory(parsed.thread.repositoryId,parsed.thread.id,items);
    this.replaceIndexThread(parsed.thread);
  }
  getPerformanceMetrics(){return {historyCacheEntries:this.historyAccess.size,historyCacheBytes:[...this.historyAccess.values()].reduce((sum,entry)=>sum+entry.bytes,0),deltaBufferRoutes:this.deltaBuffers.size,deltaCadenceRoutes:this.deltaFlushTimes.size,historyInFlightRequests:this.historyRequests.size};}

  private retainHistory(repositoryId:number,threadId:number,items:CompanionTimelineItem[]):void {
    const repository=this.repository(repositoryId);
    const key=`${repositoryId}:${threadId}`;
    const fields = [repository.segmentsByThread[threadId] ?? null, repository.historyPages?.[threadId] ?? null, repository.historyProviderOrdinals?.[threadId] ?? null, repository.usageByThread[threadId] ?? null, repository.usageAttributionByThread?.[threadId] ?? null];
    let metadata = this.historyMetadata.get(key);
    if (!metadata || fields.some((field, index) => field !== metadata!.fields[index])) {
      metadata = { fields, bytes: JSON.stringify(fields).length * 2 };
      this.historyMetadata.set(key, metadata);
    }
    this.historyAccess.delete(key);this.historyAccess.set(key,{repositoryId,threadId,bytes:timelineBytes(items)+timelineBytes(this.liveHistory.get(key)??[])+metadata.bytes});
    let total=[...this.historyAccess.values()].reduce((sum,entry)=>sum+entry.bytes,0);
    for(const [oldKey,entry] of this.historyAccess){
      if(total<=16*1024*1024 && this.historyAccess.size<=12)break;
      const repository=this.repository(entry.repositoryId);
      if(oldKey===key || repository.pending?.threadId===entry.threadId)continue;
      this.historyAccess.delete(oldKey);this.historyMetadata.delete(oldKey);this.liveHistory.delete(oldKey);total-=entry.bytes;
      const next={...repository.itemsByThread};delete next[entry.threadId];
      const ordinals={...repository.historyProviderOrdinals};delete ordinals[entry.threadId];
      const segments={...repository.segmentsByThread};delete segments[entry.threadId];
      const pages={...repository.historyPages};delete pages[entry.threadId];
      const usage={...repository.usageByThread};delete usage[entry.threadId];
      const usageAttribution={...repository.usageAttributionByThread};delete usageAttribution[entry.threadId];
      for(const route of this.historyWatermarks.keys())if(route.startsWith(`${entry.threadId}:`))this.historyWatermarks.delete(route);
      this.updateRepository(entry.repositoryId,{itemsByThread:next,historyProviderOrdinals:ordinals,segmentsByThread:segments,historyPages:pages,usageByThread:usage,usageAttributionByThread:usageAttribution});
    }
  }

  private replaceThread(repositoryId: number, updated: CompanionThread): void {
    const repository = this.repository(repositoryId);
    this.updateRepository(repositoryId, {
      threads: repository.threads.map((thread) => thread.id === updated.id ? updated : thread),
    });
    this.replaceIndexThread(updated);
  }

  private replaceIndexThread(updated: CompanionThread): void {
    if (!this.snapshot.conversationIndex) return;
    this.patch({
      conversationIndex: {
        ...this.snapshot.conversationIndex,
        threads: [updated, ...this.snapshot.conversationIndex.threads.filter((thread) => thread.id !== updated.id)],
      },
    });
  }

  private setThreadItems(repositoryId: number, threadId: number, items: CompanionTimelineItem[], change:Partial<RepositoryCompanionState>={}): void {
    const repository = this.repository(repositoryId);
    const key=`${repositoryId}:${threadId}`;
    if(this.liveHistory.has(key)){
      this.liveHistory.set(key,boundedTimeline(items));
      if(Object.keys(change).length)this.updateRepository(repositoryId,change);
      this.retainHistory(repositoryId,threadId,repository.itemsByThread[threadId]??[]);
      return;
    }
    const bounded=boundedTimeline(items);
    this.updateRepository(repositoryId, { ...change, itemsByThread: { ...repository.itemsByThread, [threadId]: bounded } });
    this.retainHistory(repositoryId,threadId,bounded);
  }

  private repository(repositoryId: number): RepositoryCompanionState {
    return this.snapshot.repositories[repositoryId] ?? emptyRepository(repositoryId);
  }

  private updateRepository(repositoryId: number, change: Partial<RepositoryCompanionState>, persistDraft = true): void {
    if (persistDraft && change.draft !== undefined) {
      change = { ...change, draftLoaded: true };
      const text = !change.draft && change.pending?.phase === "preparing" ? change.pending.prompt : change.draft!;
      this.persistAnalysisDraft(repositoryId, text);
    }
    this.patch({
      repositories: {
        ...this.snapshot.repositories,
        [repositoryId]: { ...this.repository(repositoryId), ...change },
      },
    });
  }

  private persistAnalysisDraft(repositoryId: number, text: string): void {
    void this.drafts.save(repositoryId, "analysis", text).then(() => {
      this.updateRepository(repositoryId, { draftStorageError: this.drafts.getSnapshot(repositoryId, "analysis").error });
    });
  }

  private patch(change: Partial<CompanionStateSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...change };
    this.listeners.forEach((listener) => listener());
  }
}

function threadUsageUpdate(repository: RepositoryCompanionState, thread: Pick<CompanionThread, "id" | "providerConnectionId" | "lastModelId">, usage: CompanionTokenUsage | null) {
  return {
    usageByThread: { ...repository.usageByThread, [thread.id]: usage },
    usageAttributionByThread: {
      ...repository.usageAttributionByThread,
      [thread.id]: { connectionId: thread.providerConnectionId, modelId: thread.lastModelId },
    },
  };
}

function emptyRepository(repositoryId: number): RepositoryCompanionState {
  return {
    repositoryId,
    loaded: false,
    threads: [],
    activeThreadId: null,
    itemsByThread: {},
    usageByThread: {},
    usageAttributionByThread: {},
    segmentsByThread: {},
    selection: {
      repositoryId,
      connectionId: "codex_default",
      modelId: null,
      effort: null,
    },
    selectionModels: [],
    selectionLoading: false,
    consent: null,
    draft: "",
    draftLoaded: false,
    draftStorageError: null,
    attachments: [],
    focusedFinding: null,
    context: null,
    contextChanged: false,
    pending: null,
    loading: false,
    historyUnavailable: false,
    cached: false,
    error: null,
  };
}

function attachmentsForPrompt(repository: RepositoryCompanionState): ContextAttachmentRequest[] {
  const focused = repository.focusedFinding;
  if (!focused) return repository.attachments;
  const attachments = [focused, ...repository.attachments.filter((attachment) => attachment.kind !== focused.kind || attachment.id !== focused.id)];
  if (attachments.length > 32) throw new Error("Remove an attachment to include the selected finding.");
  return attachments;
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function parseTimelineItem(value: unknown): CompanionTimelineItem | null {
  try {
    const snapshot = parseCompanionThreadSnapshot({
      thread: { id: 1, repositoryId: 1, provider: "codex", providerThreadId: "validation", providerConnectionId: "codex_default", title: "validation", status: "ready", createdAt: "now", updatedAt: "now", lastOpenedAt: "now", archivedAt: null, contextVersion: 1, lastModelId: null },
      items: [value],
      usage: null,
    });
    return snapshot.items[0] ?? null;
  } catch {
    return null;
  }
}

function parseDelta(value: unknown): { itemId: string; text: string } | null {
  const item = record(value);
  return typeof item.itemId === "string" && item.itemId.length <= 512 && typeof item.text === "string" && item.text.length <= 2_000_000
    ? { itemId: item.itemId, text: item.text }
    : null;
}

function parseTokenUsage(value: unknown): CompanionTokenUsage | null {
  const usage = record(value);
  const fields = ["inputTokens", "cachedInputTokens", "outputTokens", "reasoningTokens", "totalTokens", "contextWindow", "contextUsed"] as const;
  if (!fields.every((field) => usage[field] === null || (Number.isInteger(usage[field]) && Number(usage[field]) >= 0))) return null;
  return value as CompanionTokenUsage;
}

function parseActivity(value: unknown): CompanionActivity | null {
  const activity = record(value);
  if (!["search", "read", "context", "runtime", "reasoning", "provider_transition"].includes(String(activity.kind)) || typeof activity.label !== "string" || !Array.isArray(activity.sourceIds)) return null;
  return value as CompanionActivity;
}

function parseError(value: unknown): CompanionErrorInfo | null {
  const error = record(value);
  return typeof error.code === "string" && typeof error.message === "string" && typeof error.retryable === "boolean" ? value as CompanionErrorInfo : null;
}

function upsertItem(items: CompanionTimelineItem[], item: CompanionTimelineItem): CompanionTimelineItem[] {
  const index = items.findIndex((candidate) => candidate.id === item.id);
  if (index < 0) return [...items, item];
  const next = [...items];
  const existing = next[index];
  next[index] = item.kind === "user_message"
    && existing.kind === "user_message"
    && existing.modelId
    ? { ...item, modelId: existing.modelId }
    : item;
  return next;
}

const timelineWeights = new WeakMap<CompanionTimelineItem, number>();
function timelineItemBytes(item: CompanionTimelineItem): number {
  let weight = timelineWeights.get(item);
  if (weight === undefined) {
    weight = item.text.length * 2 + JSON.stringify(item.citations).length * 2 + 512;
    timelineWeights.set(item, weight);
  }
  return weight;
}
export function timelineBytes(items:CompanionTimelineItem[]):number {
  return items.reduce((sum,item)=>sum+timelineItemBytes(item),0);
}
function boundedTimeline(items:CompanionTimelineItem[],fromStart=false):CompanionTimelineItem[]{
  const result:CompanionTimelineItem[]=[];let bytes=0;
  for(let index=0;index<items.length;index++){
    const item = items[fromStart ? index : items.length - 1 - index];
    const size=timelineItemBytes(item);if(result.length>=400 || (result.length>0&&bytes+size>8*1024*1024))break;result.push(item);bytes+=size;
  }
  return fromStart?result:result.reverse();
}
function sameRecord(left: object | null | undefined, right: object | null | undefined): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  const a = left as Record<string, unknown>, b = right as Record<string, unknown>;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every((key) => Object.hasOwn(b, key) && a[key] === b[key]);
}
function shareRecord<T extends object>(next: T, previous: T | undefined): T {
  return previous && sameRecord(previous, next) ? previous : next;
}
function shareArray<T>(next: T[], previous: T[]): T[] {
  return next.length === previous.length && next.every((value, index) => value === previous[index]) ? previous : next;
}
function sameTimelineItem(a: CompanionTimelineItem, b: CompanionTimelineItem): boolean {
  return a === b || (a.id === b.id && a.kind === b.kind && a.status === b.status && a.text === b.text
    && a.createdAt === b.createdAt && (a.modelId ?? null) === (b.modelId ?? null)
    && sameRecord(a.error, b.error)
    && (a.activity === b.activity || Boolean(a.activity && b.activity && a.activity.kind === b.activity.kind
      && a.activity.label === b.activity.label && a.activity.detail === b.activity.detail
      && a.activity.sourceIds.length === b.activity.sourceIds.length
      && a.activity.sourceIds.every((id, index) => id === b.activity!.sourceIds[index])))
    && a.citations.length === b.citations.length && a.citations.every((citation, index) => sameRecord(citation, b.citations[index])));
}
function shareTimelineItems(next: CompanionTimelineItem[], previous: CompanionTimelineItem[]): CompanionTimelineItem[] {
  const old = new Map(previous.map((item) => [item.id, item]));
  return shareArray(next.map((item) => {
    const prior = old.get(item.id);
    return prior && sameTimelineItem(prior, item) ? prior : item;
  }), previous);
}
function mergeTimelineItems(base:CompanionTimelineItem[],incoming:CompanionTimelineItem[],preferExisting=false):CompanionTimelineItem[]{
  const merged=base.filter((item)=>item.kind!=="activity"||item.activity?.kind!=="context");
  const positions=new Map(merged.map((item,index)=>[item.id,index]));
  const echoes=new Map<string,number[]>();
  merged.forEach((item,index)=>{if(item.kind==="user_message"&&item.id.startsWith("local:")){const text=item.text.trim();const slots=echoes.get(text)??[];slots.push(index);echoes.set(text,slots);}});
  for(const item of incoming){
    if(item.kind==="activity"&&item.activity?.kind==="context")continue;
    const exact=positions.get(item.id);
    const optimistic=exact===undefined&&item.kind==="user_message"&&!item.id.startsWith("local:")?echoes.get(item.text.trim())?.shift():undefined;
    const index=exact??optimistic;
    if(index!==undefined){
      const existing=merged[index];const next=preferExisting&&exact!==undefined?existing:item;
      const citations=next.citations.map((citation)=>{const cached=existing.citations.find((entry)=>entry.sourceId===citation.sourceId);return cached&&citation.stale&&citation.label===Array.from(citation.sourceId).slice(0,256).join("")?cached:citation;});
      const modelId=item.kind==="user_message"&&existing.kind==="user_message"?existing.modelId??next.modelId:next.modelId;
      positions.delete(existing.id);positions.set(item.id,index);
      const candidate = { ...next, citations, modelId };
      merged[index] = sameTimelineItem(existing, candidate) ? existing : candidate;
    }else{positions.set(item.id,merged.length);merged.push(item);}
  }
  return merged;
}

function generatedEventItemId(event:CompanionEventEnvelope,kind:"warning"|"error"|"activity"):string{
  if(kind==="error"&&event.turnId)return `error:${event.turnId}`;
  return event.cacheEpoch?`${kind}:${event.cacheEpoch}:${event.sequence}`:`${kind}:${event.sequence}`;
}

function localError(code: string, cause: unknown, retryable: boolean): CompanionErrorInfo {
  return { code, message: cause instanceof Error ? cause.message : String(cause), retryable };
}

function isMissingProviderHistory(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /no rollout found for thread id/i.test(message);
}

function loginFailure(cause: unknown): CompanionLoginUiState {
  const message = cause instanceof Error ? cause.message : String(cause);
  const expired = /expired|timed?\s*out|timeout/i.test(message);
  return {
    phase: expired ? "expired" : "failed",
    loginId: null,
    userCode: null,
    message: message.slice(0, 1024),
  };
}

function createClientTurnId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `turn-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export const companionStore = new CompanionStore();

export function useCompanionSelector<T>(selector:(state:CompanionStateSnapshot)=>T, equals:(left:T,right:T)=>boolean=Object.is, store:CompanionStore=companionStore):T {
  const current=useRef<{selector:typeof selector;equals:typeof equals}>({selector,equals});
  current.current={selector,equals};
  const selected=useRef<{value:T}|null>(null);
  const get=useCallback(()=>{
    const value=current.current.selector(store.getSnapshot());
    if(!selected.current || !current.current.equals(selected.current.value,value)) selected.current={value};
    return selected.current.value;
  },[store]);
  return useSyncExternalStore(store.subscribe,get,get);
}
export function useCompanionRepositoryState(repositoryId:number, mode: "full" | "shell" = "full"):CompanionStateSnapshot {
  return useCompanionSelector((state)=>state,(left,right)=>{
    if ((left.conversationIndex === null) !== (right.conversationIndex === null)) return false;
    const a=left.repositories[repositoryId],b=right.repositories[repositoryId];
    if(a!==b){
      if(!a||!b||a.activeThreadId!==b.activeThreadId)return false;
      const maps=["itemsByThread","usageByThread","usageAttributionByThread","segmentsByThread","historyPages","historyProviderOrdinals"] as const;
      if(!(Object.keys(a) as (keyof RepositoryCompanionState)[]).every((key)=>maps.includes(key as typeof maps[number]) || (mode === "shell" && key === "draft") || a[key]===b[key]))return false;
      const id=a.activeThreadId;
      if (mode === "shell") {
        if (id && (a.itemsByThread[id]?.length ?? 0) !== (b.itemsByThread[id]?.length ?? 0)) return false;
      } else if(id&&maps.some((key)=>a[key]?.[id]!==b[key]?.[id]))return false;
    }
    return (Object.keys(left) as (keyof CompanionStateSnapshot)[]).every((key)=>key==="repositories"||key==="conversationIndex"||left[key]===right[key]);
  });
}

export function shallowCompanionSelection<T extends object>(left: T, right: T): boolean {
  return (Object.keys(left) as (keyof T)[]).every((key) => left[key] === right[key]);
}

export function useCompanionState(): CompanionStateSnapshot {
  return useSyncExternalStore(companionStore.subscribe, companionStore.getSnapshot, companionStore.getSnapshot);
}
