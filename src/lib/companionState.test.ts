import { describe, expect, it, vi } from "vitest";

import type { CompanionClient } from "./companionApi";
import { CompanionStore, timelineBytes } from "./companionState";
import type {
  CompanionEventEnvelope,
  CompanionThread,
  CompanionThreadSnapshot,
  CompanionRuntimeStatus,
  ContextPreviewRequest,
  RepositoryChatContext,
  ProviderConnectionSnapshot,
  CompanionTimelineItem,
} from "./types";

it("reuses immutable timeline weights and measures a replacement item again", () => {
  const serializeCitations = vi.fn(() => []);
  const item: CompanionTimelineItem = { id: "weight-fixture", kind: "assistant_message", status: "streaming", text: "Evidence", activity: null, citations: Object.assign([], { toJSON: serializeCitations }), error: null, createdAt: "now" };
  const bytes = timelineBytes([item]);
  expect(timelineBytes([item])).toBe(bytes);
  expect(serializeCitations).toHaveBeenCalledTimes(1);
  expect(timelineBytes([{ ...item, text: `${item.text} updated` }])).toBeGreaterThan(bytes);
  expect(serializeCitations).toHaveBeenCalledTimes(2);
});

function codexConnection(): ProviderConnectionSnapshot {
  return {
    schemaVersion: 1,
    connection: { schemaVersion: 1, id: "codex_default", driver: "codex", displayName: "Codex", accentColor: null, enabled: true, builtIn: true, config: { kind: "codex", binaryPath: null, homePath: null, profile: null, launchArgs: null }, hasServerPassword: false, createdAt: "now", updatedAt: "now" },
    health: "ready",
    installedVersion: "fake",
    authKind: "chatgpt",
    accountLabel: "dev@example.test",
    capabilities: { createThread: true, resumeThread: true, interrupt: true, orderedStreaming: true, readOnlyPolicy: true, accountStatus: true, rateLimits: true, tokenUsage: true, reasoningEffort: true, attachments: true },
    upstreamProviders: [{ id: "openai", displayName: "OpenAI", connected: true, inferenceBoundary: "external" }],
    models: [{ connectionId: "codex_default", driver: "codex", upstreamProviderId: "openai", inferenceBoundary: "external", id: "fake-model", displayName: "Fake model", description: null, isDefault: true, hidden: false, reasoningEfforts: ["low"], inputModalities: ["text"], contextWindow: 8192 }],
    checkedAt: "now",
    message: null,
    error: null,
  };
}

function grokConnection(): ProviderConnectionSnapshot {
  const codex = codexConnection();
  return {
    ...codex,
    connection: {
      ...codex.connection,
      id: "grok_default",
      driver: "grok",
      displayName: "Grok",
      config: { kind: "grok", binaryPath: null, customModels: [] },
    },
    authKind: "api_key",
    accountLabel: null,
    upstreamProviders: [{ id: "xai", displayName: "xAI", connected: true, inferenceBoundary: "external" }],
    models: [{
      ...codex.models[0],
      connectionId: "grok_default",
      driver: "grok",
      upstreamProviderId: "xai",
      id: "grok-4.6",
      displayName: "Grok 4.6",
      reasoningEfforts: ["high"],
    }],
  };
}

function thread(repositoryId: number, id = repositoryId): CompanionThread {
  return {
    id,
    repositoryId,
    provider: "codex",
    providerThreadId: `provider-${id}`,
    title: `Thread ${id}`,
    status: "ready",
    createdAt: "2026-08-19T12:00:00Z",
    updatedAt: "2026-08-19T12:00:00Z",
    lastOpenedAt: "2026-08-19T12:00:00Z",
    archivedAt: null,
    contextVersion: 1,
    lastModelId: "fake-model",
    providerConnectionId: "codex_default",
  };
}

function context(repositoryId: number, scanId = 10): RepositoryChatContext {
  return {
    schemaVersion: 1,
    repository: { id: repositoryId, name: `repo-${repositoryId}`, sourceType: "local", sourceMode: "local_source", approvedRoot: null, branch: "main", revision: "abc" },
    scan: { id: scanId, status: "completed", scannedAt: "2026-08-19T12:00:00Z", revision: "abc", evidenceComplete: true },
    freshness: { state: "matched", sourceRevision: "abc", scanRevision: "abc", sourceDirty: false, message: "Aligned" },
    attachments: [],
    sources: [],
    omissions: [],
    untrustedEvidence: [],
    instructions: ["Read-only"],
    estimatedBytes: 512,
    bounded: true,
  };
}

function snapshot(value: CompanionThread): CompanionThreadSnapshot {
  return { thread: value, items: [], usage: null };
}

function fakeClient(overrides: Partial<CompanionClient> = {}): { client: CompanionClient; emit: (event: CompanionEventEnvelope) => void } {
  let listener: ((event: CompanionEventEnvelope) => void) | null = null;
  const client: CompanionClient = {
    readDraft: async () => null,
    writeDraft: async () => undefined,
    runtimeStatus: async () => ({ schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 }),
    startRuntime: async () => ({ schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 }),
    stopRuntime: async () => undefined,
    accountStatus: async () => ({ schemaVersion: 1, signedIn: true, authKind: "chatgpt", planType: "team", accountLabel: "dev@example.test", message: null }),
    loginChatgpt: async () => ({ kind: "chatgpt", loginId: "login", authUrl: "https://example.test", verificationUrl: null, userCode: null, status: "pending" }),
    loginApiKey: async () => ({ kind: "api_key", loginId: null, authUrl: null, verificationUrl: null, userCode: null, status: "completed" }),
    cancelLogin: async () => undefined,
    logout: async () => undefined,
    listModels: async () => [{ id: "fake-model", displayName: "Fake model", description: null, isDefault: true, hidden: false, reasoningEfforts: ["low"], inputModalities: ["text"], contextWindow: 8192 }],
    usage: async () => ({ thread: null, primary: null, secondary: null, lifetimeTokens: null, peakDailyTokens: null, supported: false, unavailableReason: "Not available", fetchedAt: "now" }),
    listConnections: async () => [codexConnection()],
    refreshConnection: async () => codexConnection(),
    connectionConsent: async () => ({ provider: "fingerprint", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false }),
    setConnectionConsent: async (_connectionId, _modelId, accepted) => ({ provider: "fingerprint", disclosureVersion: 1, acceptedAt: accepted ? "now" : null, revokedAt: accepted ? null : "now", valid: accepted }),
    conversationIndex: async () => ({ schemaVersion: 1, threads: [thread(1), thread(2)] }),
    cachedThread: async (repositoryId, threadId) => ({
      schemaVersion: 1,
      thread: thread(repositoryId, threadId),
      segments: [{ id: threadId, threadId, ordinal: 0, provider: "codex", connectionId: "codex_default", providerThreadId: `provider-${threadId}`, initialModelId: "fake-model", lastModelId: "fake-model", status: "ready", createdAt: "now", updatedAt: "now", endedAt: null }],
      items: [],
      providerHistoryAvailable: true,
    }),
    listThreads: async (repositoryId) => [thread(repositoryId)],
    createThread: async (repositoryId) => thread(repositoryId, repositoryId * 100),
    resumeThread: async (repositoryId, threadId) => snapshot(thread(repositoryId, threadId)),
    renameThread: async (repositoryId, threadId, title) => ({ ...thread(repositoryId, threadId), title }),
    setThreadArchived: async (repositoryId, threadId, archived) => ({ ...thread(repositoryId, threadId), archivedAt: archived ? "now" : null }),
    privacyConsent: async () => ({ provider: "openai_codex", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false }),
    setPrivacyConsent: async (accepted) => ({ provider: "openai_codex", disclosureVersion: 1, acceptedAt: accepted ? "now" : null, revokedAt: accepted ? null : "now", valid: accepted }),
    previewContext: async (request: ContextPreviewRequest) => context(request.repositoryId),
    searchMentions: async () => [],
    sourceDetail: async () => { throw new Error("not used"); },
    startTurn: async (request) => ({ turnId: "turn-1", providerThreadId: `provider-${request.threadId}`, acceptedClientTurnId: request.clientTurnId }),
    interruptTurn: async () => undefined,
    listen: async (next) => { listener = next; return () => { listener = null; }; },
    ...overrides,
  };
  return { client, emit: (event) => listener?.(event) };
}

function event(sequence: number, repositoryId: number, threadId: number, kind: CompanionEventEnvelope["kind"], payload: unknown, providerThreadId = `provider-${threadId}`): CompanionEventEnvelope {
  return { schemaVersion: 1, repositoryId, threadId, providerThreadId, connectionId: "codex_default", driver: "codex", modelId: "fake-model", turnId: "turn-1", sequence, kind, payload };
}

it("keeps usage provenance when selection changes and never guesses an unreported model", async () => {
  const fixture = fakeClient();
  const store = new CompanionStore(fixture.client);
  await store.connect(); await store.loadGlobal(); await store.openRepository(1);
  const usage = { inputTokens: 80, cachedInputTokens: 0, outputTokens: 20, reasoningTokens: null, totalTokens: 100, contextWindow: 8192, contextUsed: 100 };
  fixture.emit(event(1, 1, 1, "usage", usage));
  expect(store.getSnapshot().repositories[1].usageAttributionByThread?.[1]).toEqual({ connectionId: "codex_default", modelId: "fake-model" });
  fixture.emit({ ...event(2, 1, 1, "usage", { ...usage, contextUsed: 200 }), connectionId: "unrelated" });
  expect(store.getSnapshot().repositories[1].usageByThread[1]?.contextUsed).toBe(100);
  fixture.emit({ ...event(3, 1, 1, "usage", usage), modelId: null });
  expect(store.getSnapshot().repositories[1].usageAttributionByThread?.[1]).toEqual({ connectionId: "codex_default", modelId: null });
  store.disconnect();
});

it("restores context usage with the resumed thread's recorded model", async () => {
  const usage = { inputTokens: 10, cachedInputTokens: 0, outputTokens: 1, reasoningTokens: null, totalTokens: 11, contextWindow: 8192, contextUsed: 11 };
  const fixture = fakeClient({ resumeThread: async (repositoryId, threadId) => ({ ...snapshot({ ...thread(repositoryId, threadId), lastModelId: "historical-model" }), usage }) });
  const store = new CompanionStore(fixture.client);
  await store.connect(); await store.loadGlobal(); await store.openRepository(1);
  expect(store.getSnapshot().repositories[1].usageAttributionByThread?.[1]).toEqual({ connectionId: "codex_default", modelId: "historical-model" });
  expect(store.getSnapshot().repositories[1].usageByThread[1]).toEqual(usage);
  store.disconnect();
});

describe("CompanionStore", () => {
  it("keeps the displayed and persisted Analysis draft identical when an emoji crosses the input bound", async () => {
    let saved: string | null = null;
    const writeDraft = vi.fn(async (_repositoryId: number, _scope: string, text: string) => { saved = text; });
    const fixture = fakeClient({ readDraft: async () => saved, writeDraft });
    const store = new CompanionStore(fixture.client);
    await store.openRepository(1);
    store.setDraft(1, "A".repeat(65_535) + "👋");
    await store.drafts.retry(1, "analysis");
    const expected = "A".repeat(65_535);
    expect(store.getSnapshot().repositories[1].draft).toBe(expected);
    expect(writeDraft).toHaveBeenLastCalledWith(1, "analysis", expected);
    const reopened = new CompanionStore(fixture.client);
    await reopened.openRepository(1);
    expect(reopened.getSnapshot().repositories[1]).toMatchObject({ draft: expected, draftLoaded: true, draftStorageError: null });
  });

  it("restores unsent Analysis text in a new store without rewriting it", async () => {
    const writeDraft = vi.fn(async () => undefined);
    const fixture = fakeClient({ readDraft: async () => "  Saved Analysis draft\nGrüße 👋", writeDraft });
    const store = new CompanionStore(fixture.client);
    await store.openRepository(1);
    expect(store.getSnapshot().repositories[1]).toMatchObject({ draft: "  Saved Analysis draft\nGrüße 👋", draftLoaded: true, draftStorageError: null });
    expect(writeDraft).not.toHaveBeenCalled();
  });

  it("keeps history available after a draft read failure and restores the draft on retry", async () => {
    const readDraft = vi.fn().mockRejectedValueOnce(new Error("disk unavailable")).mockResolvedValue("Recover this draft");
    const fixture = fakeClient({ readDraft });
    const store = new CompanionStore(fixture.client);
    await store.openRepository(1);
    expect(store.getSnapshot().repositories[1]).toMatchObject({ draftLoaded: false, draftStorageError: "restore" });
    expect(store.getSnapshot().repositories[1].threads).not.toHaveLength(0);
    await store.retryDraftStorage(1);
    expect(store.getSnapshot().repositories[1]).toMatchObject({ draft: "Recover this draft", draftLoaded: true, draftStorageError: null });
  });
  it.each(["disabled", "unavailable", "setup_required"] as const)("finishes selecting a %s conversation when its local cache cannot be read", async (health) => {
    const connection = codexConnection();
    connection.health = health;
    connection.connection.enabled = health !== "disabled";
    const resumeThread = vi.fn(async () => snapshot(thread(1)));
    const fixture = fakeClient({
      listConnections: async () => [connection],
      cachedThread: async () => { throw new Error("Synthetic cache read failure"); },
      resumeThread,
    });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal(); await store.loadConversationIndex();
    await store.activateConversation(1, 1);
    expect(store.getSnapshot().repositories[1]).toMatchObject({
      activeThreadId: 1, loaded: true, loading: false,
      error: { code: "cached_history_failed", message: "Synthetic cache read failure" },
    });
    expect(resumeThread).not.toHaveBeenCalled();
  });

  it("does not publish a late cache failure into a different selected conversation", async () => {
    let reject!: (error: Error) => void;
    const fixture = fakeClient({
      conversationIndex: async () => ({ schemaVersion: 1, threads: [thread(1, 1), thread(1, 2)] }),
      cachedThread: async (repo, id) => id === 1 ? new Promise((_resolve, fail) => { reject = fail; }) : {
        schemaVersion: 1, thread: thread(repo, id), segments: [], items: [], providerHistoryAvailable: true,
      },
    });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal(); await store.loadConversationIndex();
    const older = store.activateConversation(1, 1);
    await store.activateConversation(1, 2);
    reject(new Error("Synthetic late cache failure"));
    await older;
    expect(store.getSnapshot().repositories[1]).toMatchObject({ activeThreadId: 2, loaded: true, error: null });
  });

  it("hydrates once before resume and reconciles the durable page without publishing the provider transport snapshot", async () => {
    const answer: CompanionTimelineItem = { id: "segment:1:answer", kind: "assistant_message", status: "completed", text: "Durable synthetic answer", activity: null, citations: [], error: null, createdAt: "now" };
    const page = { schemaVersion: 1, thread: thread(1), segments: [], items: [answer], providerHistoryAvailable: true, olderCursor: null };
    const cachedThread = vi.fn(async () => structuredClone(page));
    const fixture = fakeClient({ cachedThread, resumeThread: async () => ({ ...snapshot(thread(1)), items: [{ ...answer, text: "Intermediate provider transport shape" }] }) });
    const store = new CompanionStore(fixture.client);
    const observed: string[] = [];
    let original: CompanionTimelineItem | undefined;
    store.subscribe(() => {
      const item = store.getSnapshot().repositories[1]?.itemsByThread[1]?.[0];
      if (item) { original ??= item; observed.push(item.text); }
    });
    await store.loadGlobal(); await store.openRepository(1);
    expect(cachedThread).toHaveBeenCalledTimes(2);
    expect(observed).not.toContain("Intermediate provider transport shape");
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0]).toBe(original);
  });

  it("coalesces overlapping opens and resumes of the same conversation", async () => {
    const cachedThread = vi.fn(async () => ({ schemaVersion: 1, thread: thread(1), segments: [], items: [], providerHistoryAvailable: true, olderCursor: null }));
    const resumeThread = vi.fn(async () => snapshot(thread(1)));
    const fixture = fakeClient({ cachedThread, resumeThread });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal();
    await Promise.all([store.openRepository(1), store.openRepository(1)]);
    expect(cachedThread).toHaveBeenCalledTimes(2);
    expect(resumeThread).toHaveBeenCalledTimes(1);
  });

  it("keeps hydrated history visible when another open arrives during provider synchronization", async () => {
    let finish!: (value: CompanionThreadSnapshot) => void;
    const resumeThread = vi.fn(() => new Promise<CompanionThreadSnapshot>(resolve => { finish = resolve; }));
    const fixture = fakeClient({ resumeThread });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal();
    const first = store.openRepository(1);
    await vi.waitFor(() => expect(resumeThread).toHaveBeenCalledTimes(1));
    expect(store.getSnapshot().repositories[1].loaded).toBe(true);
    const seen: boolean[] = [];
    store.subscribe(() => seen.push(store.getSnapshot().repositories[1].loaded));
    const second = store.openRepository(1);
    await Promise.resolve(); await Promise.resolve();
    expect(store.getSnapshot().repositories[1].loaded).toBe(true);
    finish(snapshot(thread(1)));
    await Promise.all([first, second]);
    expect(seen).not.toContain(false);
    expect(resumeThread).toHaveBeenCalledTimes(1);
  });

  it("reconciles changed durable items without replacing unchanged neighbors", async () => {
    const item = (id: string, text: string): CompanionTimelineItem => ({ id, kind: "assistant_message", status: "completed", text, activity: null, citations: [], error: null, createdAt: "now" });
    const first = [item("a", "Unchanged evidence"), item("b", "Original evidence")];
    const next = [first[0], { ...first[1], text: "Updated evidence", citations: [{ sourceId: "scan:10", label: "Scan #10", kind: "scan" as const, stale: true }] }];
    const cachedThread = vi.fn().mockResolvedValueOnce({ schemaVersion: 1, thread: thread(1), segments: [], items: structuredClone(first), providerHistoryAvailable: true, olderCursor: null }).mockResolvedValue({ schemaVersion: 1, thread: thread(1), segments: [], items: structuredClone(next), providerHistoryAvailable: true, olderCursor: null });
    const fixture = fakeClient({ cachedThread });
    const store = new CompanionStore(fixture.client);
    let initial: CompanionTimelineItem[] | undefined;
    store.subscribe(() => { initial ??= store.getSnapshot().repositories[1]?.itemsByThread[1]; });
    await store.loadGlobal(); await store.openRepository(1);
    const result = store.getSnapshot().repositories[1].itemsByThread[1];
    expect(result[0]).toBe(initial![0]);
    expect(result[1]).not.toBe(initial![1]);
    expect(result[1]).toMatchObject({ text: "Updated evidence", citations: [{ stale: true }] });
  });

  it("falls back to provider history if the durable refresh fails after resume", async () => {
    const answer: CompanionTimelineItem = { id: "answer", kind: "assistant_message", status: "completed", text: "Old cache", activity: null, citations: [], error: null, createdAt: "now" };
    const cachedThread = vi.fn().mockResolvedValueOnce({ schemaVersion: 1, thread: thread(1), segments: [], items: [answer], providerHistoryAvailable: true, olderCursor: null }).mockRejectedValue(new Error("Cache refresh failed"));
    const fixture = fakeClient({ cachedThread, resumeThread: async () => ({ ...snapshot(thread(1)), items: [{ ...answer, text: "Recovered provider answer" }] }) });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal(); await store.openRepository(1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].text).toBe("Recovered provider answer");
    expect(store.getSnapshot().repositories[1].historyUnavailable).toBe(false);
  });

  it("uses retained history on return and preserves unchanged item and array references", async () => {
    const cachedThread = vi.fn(async (repo: number, id: number) => ({ schemaVersion: 1, thread: thread(repo, id), segments: [], items: [{ id: `answer-${id}`, kind: "assistant_message" as const, status: "completed" as const, text: `Synthetic answer ${id}`, activity: null, citations: [], error: null, createdAt: "now" }], providerHistoryAvailable: true, olderCursor: null }));
    const fixture = fakeClient({ cachedThread });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal();
    await store.openRepository(1);
    const original = store.getSnapshot().repositories[1].itemsByThread[1];
    const before = cachedThread.mock.calls.length;
    await store.activateConversation(1, 1);
    expect(cachedThread.mock.calls.length - before).toBe(1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toBe(original);
  });

  it("keeps warnings with repeated sequence numbers across host restarts", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    for(const epoch of ["first-process","second-process"]){
      fixture.emit({...event(1,1,1,"warning",{code:"action_denied",message:epoch,retryable:false}),cacheEpoch:epoch});
    }
    expect(store.getSnapshot().repositories[1].itemsByThread[1].filter((item)=>item.kind==="warning").map((item)=>item.error?.message)).toEqual(["first-process","second-process"]);
    store.disconnect();
  });

  it("keeps warnings non-fatal while a response streams and preserves real errors", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Hey");
    await store.send(1);
    const pending = store.getSnapshot().repositories[1].pending;
    expect(pending).not.toBeNull();
    fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: "Hey!" }));
    const warning = { code: "codex_warning", message: "Under-development features enabled: chronicle.", retryable: false };
    fixture.emit(event(2, 1, 1, "warning", warning));
    fixture.emit(event(3, 1, 1, "error", warning)); // Older host compatibility.
    fixture.emit(event(4, 1, 1, "warning", { code: "action_denied", message: "Read-only", retryable: false }));
    fixture.emit(event(5, 1, 1, "error", { code: "action_denied", message: "Legacy denial", retryable: false }));
    const repository = store.getSnapshot().repositories[1];
    expect(repository.error).toBeNull();
    expect(repository.pending).toEqual({ ...pending, phase: "responding" });
    expect(repository.itemsByThread[1].find((item) => item.id === "answer")?.status).toBe("streaming");
    expect(repository.itemsByThread[1].filter((item) => item.kind === "warning")).toHaveLength(4);
    fixture.emit(event(6, 1, 1, "turn_completed", { status: "completed" }));
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
    fixture.emit(event(7, 1, 1, "error", { code: "offline", message: "Connection ended", retryable: true }));
    expect(store.getSnapshot().repositories[1].error?.code).toBe("offline");
  });

  it("coalesces background initialization and publishes provider state atomically", async () => {
    let resolveConnections!: (connections: ProviderConnectionSnapshot[]) => void;
    const listConnections = vi.fn(() => new Promise<ProviderConnectionSnapshot[]>((resolve) => { resolveConnections = resolve; }));
    const fixture = fakeClient({ listConnections });
    const store = new CompanionStore(fixture.client);
    const observed = [store.getSnapshot()];
    store.subscribe(() => observed.push(store.getSnapshot()));

    const first = store.initialize();
    const second = store.initialize();

    await vi.waitFor(() => expect(listConnections).toHaveBeenCalledTimes(1));
    expect(store.getSnapshot()).toMatchObject({ initializationPhase: "checking", connections: [] });
    resolveConnections([codexConnection()]);
    await Promise.all([first, second]);

    expect(store.getSnapshot()).toMatchObject({
      initializationPhase: "ready",
      selectedConnectionId: "codex_default",
      selectedModel: "fake-model",
      account: { signedIn: true },
    });
    expect(observed.some((snapshot) => snapshot.initializationPhase === "ready" && (
      snapshot.connections.length === 0 || !snapshot.selectedModel || !snapshot.account
    ))).toBe(false);

    await store.openRepository(1);
    expect(store.getSnapshot().repositories[1].loaded).toBe(true);
    store.synchronizeConnections([codexConnection()]);
    expect(store.getSnapshot().repositories[1].loaded).toBe(false);
  });

  it("hydrates local history before a delayed provider discovery finishes", async () => {
    let finish!: (connections: ProviderConnectionSnapshot[]) => void;
    const fixture = fakeClient({ listConnections: () => new Promise(resolve => { finish = resolve; }) });
    const resume = vi.spyOn(fixture.client, "resumeThread");
    const store = new CompanionStore(fixture.client);
    const initialization = store.initialize();
    await vi.waitFor(() => expect(store.getSnapshot().conversationIndex).not.toBeNull());
    await store.openRepository(1);
    expect(store.getSnapshot().repositories[1]).toMatchObject({ loaded: true, cached: true });
    expect(store.getSnapshot().initializationPhase).toBe("checking");
    expect(resume).not.toHaveBeenCalled();
    finish([codexConnection()]);
    await initialization;
    await vi.waitFor(() => expect(resume).toHaveBeenCalledTimes(1));
    store.disconnect();
  });

  it("initializes an empty conversation's model after delayed provider discovery", async () => {
    let finish!: (connections: ProviderConnectionSnapshot[]) => void;
    const fixture = fakeClient({ listConnections: () => new Promise(resolve => { finish = resolve; }), conversationIndex: async () => ({ schemaVersion: 1, threads: [] }) });
    const store = new CompanionStore(fixture.client);
    const initialization = store.initialize();
    await vi.waitFor(() => expect(store.getSnapshot().conversationIndex).not.toBeNull());
    await store.openRepository(1);
    store.setDraft(1, "Keep this question");
    expect(store.getSnapshot().repositories[1].loaded).toBe(true);
    finish([codexConnection()]);
    await initialization;
    await vi.waitFor(() => expect(store.getSnapshot().repositories[1].selection.modelId).toBe("fake-model"));
    expect(store.getSnapshot().repositories[1].draft).toBe("Keep this question");
    store.disconnect();
  });

  it("reads runtime readiness after provider discovery has started Codex", async () => {
    let runtime: CompanionRuntimeStatus = { schemaVersion: 1, phase: "initializing", codexPath: "/fake/codex", codexVersion: "fake", message: "Starting", startedAt: null, activeTurns: 0 };
    const fixture = fakeClient({
      runtimeStatus: async () => runtime,
      listConnections: async () => {
        runtime = { ...runtime, phase: "ready", message: null, startedAt: "now" };
        fixture.emit({ ...event(1, 1, 1, "runtime_status", runtime), repositoryId: null, threadId: null, turnId: null });
        return [codexConnection()];
      },
    });
    const store = new CompanionStore(fixture.client);

    await store.initialize();
    await store.openRepository(1);
    store.newConversation(1);

    expect(store.getSnapshot().runtime?.phase).toBe("ready");
    expect(store.getSnapshot().account?.signedIn).toBe(true);
    expect(store.getSnapshot().repositories[1].activeThreadId).toBeNull();
    store.disconnect();
  });

  it("shares account, model and consent reads across repository entry and refreshes them for a changed connection", async () => {
    const fixture = fakeClient();
    const accountReads = vi.spyOn(fixture.client, "accountStatus");
    const modelReads = vi.spyOn(fixture.client, "listModels");
    const consentReads = vi.spyOn(fixture.client, "connectionConsent");
    const store = new CompanionStore(fixture.client);
    await store.initialize();
    await Promise.all([store.openRepository(1), store.openRepository(2)]);
    expect(accountReads).toHaveBeenCalledTimes(1);
    expect(modelReads).toHaveBeenCalledTimes(1);
    expect(consentReads).toHaveBeenCalledExactlyOnceWith("codex_default", "fake-model");

    const changed = codexConnection();
    changed.connection.updatedAt = "changed-connection";
    store.synchronizeConnections([changed]);
    expect(store.getSnapshot().repositories[1].consent).toBeNull();
    await store.selectRepositoryConnection(1, "codex_default", "fake-model");
    expect(accountReads).toHaveBeenCalledTimes(2);
    expect(modelReads).toHaveBeenCalledTimes(2);
    expect(consentReads).toHaveBeenCalledTimes(2);
    store.disconnect();
  });

  it("retains a newer runtime event received while provider state is loading", async () => {
    const runtime: CompanionRuntimeStatus = { schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 };
    const fixture = fakeClient({
      runtimeStatus: async () => runtime,
      connectionConsent: async () => {
        fixture.emit({ ...event(1, 1, 1, "runtime_status", { ...runtime, phase: "offline", message: "Process exited" }), repositoryId: null, threadId: null, turnId: null });
        return { provider: "fingerprint", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false };
      },
    });
    const store = new CompanionStore(fixture.client);

    await store.initialize();

    expect(store.getSnapshot().runtime).toMatchObject({ phase: "offline", message: "Process exited" });
    store.disconnect();
  });

  it("restores connection health and repository models after a successful runtime retry", async () => {
    const unavailable = { ...codexConnection(), health: "unavailable" as const, models: [] };
    const fixture = fakeClient({ listConnections: async () => [unavailable] });
    const store = new CompanionStore(fixture.client);
    await store.initialize();
    await store.openRepository(1);
    store.newConversation(1);
    store.setDraft(1, "Keep my retry draft");

    await store.startRuntime();

    expect(store.getSnapshot().connections[0].health).toBe("ready");
    expect(store.getSnapshot().repositories[1]).toMatchObject({
      draft: "Keep my retry draft",
      selection: { connectionId: "codex_default", modelId: "fake-model" },
      selectionModels: [{ id: "fake-model" }],
    });
    store.disconnect();
  });

  it("unblocks repository chat after completing Codex sign-in", async () => {
    let signedIn = false;
    const fixture = fakeClient({
      listConnections: async () => [{ ...codexConnection(), health: "setup_required", models: [] }],
      accountStatus: async () => ({ schemaVersion: 1, signedIn, authKind: signedIn ? "api_key" : "none", planType: null, accountLabel: null, message: null }),
      loginApiKey: async () => { signedIn = true; return { kind: "api_key", loginId: null, userCode: null, authUrl: null, verificationUrl: null, status: "completed" }; },
    });
    const store = new CompanionStore(fixture.client);
    await store.initialize();
    await store.openRepository(1);
    await store.loginWithApiKey("synthetic-test-key");
    expect(store.getSnapshot().connections[0].health).toBe("ready");
    expect(store.getSnapshot().repositories[1].selection.modelId).toBe("fake-model");
    store.disconnect();
  });

  it("does not overwrite a disconnection event with an older runtime start reply", async () => {
    const fixture = fakeClient({
      startRuntime: async () => {
        fixture.emit({ ...event(1, 1, 1, "runtime_status", { schemaVersion: 1, phase: "offline", codexPath: "/fake/codex", codexVersion: "fake", message: "Process exited", startedAt: "now", activeTurns: 0 }), repositoryId: null, threadId: null, turnId: null });
        return { schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 };
      },
    });
    const store = new CompanionStore(fixture.client);
    await store.initialize();

    await expect(store.startRuntime()).rejects.toThrow("Process exited");
    expect(store.getSnapshot().runtime?.phase).toBe("offline");
    store.disconnect();
  });

  it("shares one in-flight runtime retry across callers", async () => {
    let resolve!: (value: Awaited<ReturnType<CompanionClient["startRuntime"]>>) => void;
    const startRuntime = vi.fn(() => new Promise<Awaited<ReturnType<CompanionClient["startRuntime"]>>>((done) => { resolve = done; }));
    const fixture = fakeClient({ startRuntime });
    const store = new CompanionStore(fixture.client);
    await store.initialize();

    const first = store.startRuntime();
    const second = store.startRuntime();
    expect(startRuntime).toHaveBeenCalledTimes(1);
    resolve({ schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 });
    await Promise.all([first, second]);
    store.disconnect();
  });

  it("accepts a ready start reply after intermediate progress events", async () => {
    const fixture = fakeClient({ startRuntime: async () => {
      fixture.emit({ ...event(1, 1, 1, "runtime_status", { schemaVersion: 1, phase: "initializing", codexPath: "/fake/codex", codexVersion: "fake", message: "Starting", startedAt: null, activeTurns: 0 }), repositoryId: null, threadId: null, turnId: null });
      return { schemaVersion: 1, phase: "ready", codexPath: "/fake/codex", codexVersion: "fake", message: null, startedAt: "now", activeTurns: 0 };
    } });
    const store = new CompanionStore(fixture.client);
    await store.initialize();
    await store.startRuntime();
    expect(store.getSnapshot().runtime?.phase).toBe("ready");
    store.disconnect();
  });

  it("switches Grok and Codex as coherent provider-model snapshots", async () => {
    let resolveCodexModels!: (models: Awaited<ReturnType<CompanionClient["listModels"]>>) => void;
    const listModels = vi.fn()
      .mockResolvedValueOnce(codexConnection().models)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveCodexModels = resolve; }));
    const fixture = fakeClient({
      listConnections: async () => [codexConnection(), grokConnection()],
      refreshConnection: async (connectionId) => connectionId === "grok_default" ? grokConnection() : codexConnection(),
      listModels,
    });
    const store = new CompanionStore(fixture.client);
    const observed = [store.getSnapshot()];
    store.subscribe(() => observed.push(store.getSnapshot()));

    await store.loadGlobal();
    await store.selectConnection("grok_default", true);
    expect(store.getSnapshot()).toMatchObject({
      selectedConnectionId: "grok_default",
      selectedModel: "grok-4.6",
      selectedEffort: "high",
      account: null,
    });

    const switchToCodex = store.selectConnection("codex_default", true);
    await vi.waitFor(() => expect(listModels).toHaveBeenCalledTimes(2));
    expect(store.getSnapshot()).toMatchObject({
      selectedConnectionId: "grok_default",
      selectedModel: "grok-4.6",
    });

    resolveCodexModels(codexConnection().models);
    await switchToCodex;
    expect(store.getSnapshot()).toMatchObject({
      selectedConnectionId: "codex_default",
      selectedModel: "fake-model",
      selectedEffort: "low",
      account: { signedIn: true },
    });
    expect(store.getSnapshot().models.map((model) => model.id)).toEqual(["fake-model"]);
    expect(observed.some((state) => state.selectedConnectionId === "codex_default" && (
      state.selectedModel === "grok-4.6" || state.models.some((model) => model.id === "grok-4.6")
    ))).toBe(false);
  });

  it("rejects model and reasoning selections that do not belong to the selected inventory", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal();

    store.setModel("grok-4.6");
    expect(store.getSnapshot()).toMatchObject({ selectedModel: null, selectedEffort: null });
    store.setModel("fake-model");
    store.setEffort("high");
    expect(store.getSnapshot()).toMatchObject({ selectedModel: "fake-model", selectedEffort: null });
  });

  it("partitions events by repository and quarantines duplicates and forged routes", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.openRepository(1);
    await store.openRepository(2);

    fixture.emit(event(1, 1, 1, "item_delta", { itemId: "assistant", text: "repo one" }));
    fixture.emit(event(1, 2, 2, "item_delta", { itemId: "assistant", text: "duplicate" }));
    fixture.emit(event(2, 2, 2, "item_delta", { itemId: "assistant", text: "forged" }, "wrong-provider"));
    fixture.emit(event(3, 2, 2, "item_delta", { itemId: "assistant", text: "repo two" }));

    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].text).toBe("repo one");
    expect(store.getSnapshot().repositories[2].itemsByThread[2][0].text).toBe("repo two");
  });

  it("reconciles a provider snapshot before applying queued stream events", async () => {
    let resolveResume!: (value: CompanionThreadSnapshot) => void;
    const resume = new Promise<CompanionThreadSnapshot>((resolve) => { resolveResume = resolve; });
    const fixture = fakeClient({ resumeThread: async () => resume });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.createThread(1);
    const pendingResume = store.resumeThread(1, 100);
    fixture.emit(event(1, 1, 100, "item_delta", { itemId: "assistant", text: "after snapshot" }));
    expect(store.getSnapshot().repositories[1].itemsByThread[100]).toBeUndefined();
    resolveResume(snapshot(thread(1, 100)));
    await pendingResume;
    expect(store.getSnapshot().repositories[1].itemsByThread[100][0].text).toBe("after snapshot");
  });

  it("shows the sent message immediately while context preparation is unresolved", async () => {
    let resolvePreview!: (value: RepositoryChatContext) => void;
    const previewContext = vi.fn(() => new Promise<RepositoryChatContext>((resolve) => { resolvePreview = resolve; }));
    const startTurn = vi.fn(async (request: Parameters<CompanionClient["startTurn"]>[0]) => ({ turnId: "turn-1", providerThreadId: "provider-1", acceptedClientTurnId: request.clientTurnId }));
    const fixture = fakeClient({ previewContext, startTurn });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain the critical findings");
    store.setAttachments(1, [{ kind: "finding", id: "critical" }]);
    const sending = store.send(1);
    const queued = store.getSnapshot().repositories[1];
    expect(queued.draft).toBe("");
    expect(queued.pending?.prompt).toBe("Explain the critical findings");
    expect(store.drafts.getSnapshot(1, "analysis").text).toBe("Explain the critical findings");
    expect(queued.itemsByThread[1].at(-1)?.text).toBe("Explain the critical findings");
    expect(startTurn).not.toHaveBeenCalled();
    await store.send(1); // Enter again during preparation must not duplicate the send.
    expect(previewContext).toHaveBeenCalledTimes(1);
    expect(previewContext).toHaveBeenCalledWith({ repositoryId: 1, query: "Explain the critical findings", attachments: [{ kind: "finding", id: "critical" }] });
    store.setDraft(1, "Next question");
    store.setAttachments(1, []);
    resolvePreview(context(1));
    await sending;
    expect(startTurn).toHaveBeenCalledTimes(1);
    expect(startTurn.mock.calls[0][0].prompt).toBe("Explain the critical findings");
    expect(store.getSnapshot().repositories[1].draft).toBe("Next question");
  });

  it("exposes a new-chat message before thread creation completes", async () => {
    let resolveThread!: (value: CompanionThread) => void;
    const fixture = fakeClient({ listThreads: async () => [], createThread: () => new Promise((resolve) => { resolveThread = resolve; }) });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "New question");
    const sending = store.send(1);
    expect(store.getSnapshot().repositories[1].pending?.userItem.text).toBe("New question");
    expect(store.getSnapshot().repositories[1].pending?.threadId).toBeNull();
    expect(store.getSnapshot().repositories[1].draft).toBe("");
    resolveThread(thread(1, 100));
    await sending;
    const repository = store.getSnapshot().repositories[1];
    expect(repository.activeThreadId).toBe(100);
    expect(repository.pending?.threadId).toBe(100);
    expect(repository.itemsByThread[100].filter((item) => item.text === "New question")).toHaveLength(1);
  });

  it("submits the reviewed context unchanged and keeps the draft if it needs another review", async () => {
    const previewContext = vi.fn(async () => context(1));
    const startTurn = vi.fn(async () => {
      throw new Error("Repository context changed. Review the updated context before sending.");
    });
    const { client } = fakeClient({ previewContext, startTurn });
    const store = new CompanionStore(client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Keep this question");
    const reviewed = await store.previewSend(1);

    expect(await store.send(1, reviewed)).toBe("review-required");
    expect(previewContext).toHaveBeenCalledTimes(1);
    expect(startTurn).toHaveBeenCalledWith(expect.objectContaining({ context: reviewed.context, prompt: reviewed.draft }));
    const repository = store.getSnapshot().repositories[1];
    expect(repository.draft).toBe("Keep this question");
    expect(repository.pending).toBeNull();
    expect(Object.values(repository.itemsByThread).flat().some((item) => item.id.startsWith("local:"))).toBe(false);
    expect(await store.send(1)).toBe("review-required");
    expect(startTurn).toHaveBeenCalledTimes(1);
  });

  it.each(["context", "thread"])("restores the draft when %s preparation fails", async (stage) => {
    const fixture = fakeClient(stage === "context"
      ? { previewContext: async () => { throw new Error("Context unavailable"); } }
      : { listThreads: async () => [], createThread: async () => { throw new Error("Thread unavailable"); } });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Keep this question");
    await store.send(1);
    const repository = store.getSnapshot().repositories[1];
    expect(repository.draft).toBe("Keep this question");
    expect(repository.pending).toBeNull();
    expect(Object.values(repository.itemsByThread).flat().filter((item) => item.id.startsWith("local:"))).toHaveLength(0);
    expect(repository.error?.code).toBe("turn_start_failed");
  });

  it("shows first text immediately and coalesces a burst into one update within 50 ms", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    vi.useFakeTimers();
    try {
      const listener = vi.fn();
      store.subscribe(listener);
      const answer = () => store.getSnapshot().repositories[1].itemsByThread[1].find((item) => item.id === "answer");
      fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: "First" }));
      expect(answer()?.text).toBe("First");
      listener.mockClear();
      for (let sequence = 2; sequence <= 101; sequence++) fixture.emit(event(sequence, 1, 1, "item_delta", { itemId: "answer", text: "." }));
      expect(listener).not.toHaveBeenCalled();
      vi.advanceTimersByTime(49);
      expect(answer()?.text).toBe("First");
      vi.advanceTimersByTime(1);
      expect(answer()?.text).toBe(`First${".".repeat(100)}`);
      expect(listener).toHaveBeenCalledTimes(1);
    } finally { store.disconnect(); vi.useRealTimers(); }
  });

  it("displays paced durable batches without a second 50-ms wait", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    vi.useFakeTimers();
    try {
      const answer = () => store.getSnapshot().repositories[1].itemsByThread[1].find((item) => item.id === "answer");
      fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: "A" }));
      vi.advanceTimersByTime(51);
      fixture.emit(event(2, 1, 1, "item_delta", { itemId: "answer", text: "B" }));
      expect(answer()?.text).toBe("AB");
      // A later delta inside the same update frame waits only the remainder.
      vi.advanceTimersByTime(35);
      fixture.emit(event(3, 1, 1, "item_delta", { itemId: "answer", text: "C" }));
      expect(answer()?.text).toBe("AB");
      vi.advanceTimersByTime(14);
      expect(answer()?.text).toBe("AB");
      vi.advanceTimersByTime(1);
      expect(answer()?.text).toBe("ABC");
    } finally { store.disconnect(); vi.useRealTimers(); }
  });

  it("keeps a 1000-delta-per-second burst bounded with the exact text", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    vi.useFakeTimers();
    try {
      const listener = vi.fn(); store.subscribe(listener);
      let expected = "";
      for (let sequence = 1; sequence <= 1000; sequence++) {
        const text = `${sequence} `; expected += text;
        fixture.emit(event(sequence, 1, 1, "item_delta", { itemId: "answer", text }));
        vi.advanceTimersByTime(1);
      }
      expect(listener.mock.calls.length).toBeLessThanOrEqual(21);
      expect(store.getSnapshot().repositories[1].itemsByThread[1].find((item) => item.id === "answer")?.text).toBe(expected);
      expect(store.getPerformanceMetrics().deltaBufferRoutes).toBe(0);
      expect(store.getPerformanceMetrics().deltaCadenceRoutes).toBeLessThanOrEqual(32);
    } finally { store.disconnect(); vi.useRealTimers(); }
    expect(store.getPerformanceMetrics().deltaCadenceRoutes).toBe(0);
  });

  it.each(["completed", "interrupted", "error", "runtime", "disconnect"])("flushes trailing text before %s and never reopens it on the timer", async (ending) => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Question"); await store.send(1);
    vi.useFakeTimers();
    try {
      fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: "First" }));
      fixture.emit(event(2, 1, 1, "item_delta", { itemId: "answer", text: " last" }));
      if (ending === "disconnect") store.disconnect();
      else if (ending === "runtime") fixture.emit(event(3, 1, 1, "runtime_status", { schemaVersion: 1, phase: "interrupted", codexPath: "/fake/codex", codexVersion: "fake", message: "Process exited", startedAt: "now", activeTurns: 0 }));
      else if (ending === "error") fixture.emit(event(3, 1, 1, "error", { code: "provider_error", message: "Failed", retryable: true }));
      else fixture.emit(event(3, 1, 1, "turn_completed", { status: ending }));
      const finished = store.getSnapshot();
      const answer = finished.repositories[1].itemsByThread[1].find((item) => item.id === "answer");
      expect(answer?.text).toBe("First last");
      if (ending !== "disconnect") expect(answer?.status).toBe(ending === "runtime" ? "interrupted" : ending === "error" ? "failed" : ending);
      vi.advanceTimersByTime(100);
      expect(store.getSnapshot()).toBe(finished);
    } finally { store.disconnect(); vi.useRealTimers(); }
  });

  it("buffers interleaved routes independently and preserves message order", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1); await store.openRepository(2);
    vi.useFakeTimers();
    try {
      fixture.emit(event(1, 1, 1, "item_delta", { itemId: "first", text: "A" }));
      fixture.emit(event(2, 1, 1, "item_delta", { itemId: "first", text: "B" }));
      fixture.emit(event(3, 1, 1, "item_delta", { itemId: "second", text: "C" }));
      fixture.emit(event(4, 1, 1, "item_delta", { itemId: "second", text: "D" }));
      fixture.emit(event(5, 2, 2, "item_delta", { itemId: "first", text: "Other repo" }));
      expect(store.getSnapshot().repositories[1].itemsByThread[1].map((item)=>item.text)).toEqual(["A","C"]);
      vi.advanceTimersByTime(50);
      const repositories = store.getSnapshot().repositories;
      expect(repositories[1].itemsByThread[1].map((item) => item.text)).toEqual(["AB", "CD"]);
      expect(repositories[2].itemsByThread[2].map((item) => item.text)).toEqual(["Other repo"]);
    } finally { store.disconnect(); vi.useRealTimers(); }
  });

  it("tracks turn start, thinking, streamed text and gaps between multiple answer steps", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Explain findings");
    await store.send(1);
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("waiting");
    fixture.emit(event(1, 1, 1, "turn_started", {}));
    const thinking = { id: "reasoning-1", kind: "activity", status: "streaming", text: "", activity: { kind: "reasoning", label: "Thinking", detail: null, sourceIds: [] }, citations: [], error: null, createdAt: "now" };
    fixture.emit(event(2, 1, 1, "item_started", thinking));
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("thinking");
    fixture.emit(event(3, 1, 1, "item_completed", { ...thinking, status: "completed" }));
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("working");
    fixture.emit(event(4, 1, 1, "item_delta", { itemId: "step-1", text: "First step" }));
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("responding");
    fixture.emit(event(5, 1, 1, "item_completed", { ...thinking, id: "step-1", kind: "assistant_message", activity: null, text: "First step", status: "completed" }));
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("working");
    fixture.emit(event(6, 1, 1, "item_started", { ...thinking, id: "reasoning-2" }));
    expect(store.getSnapshot().repositories[1].pending?.phase).toBe("thinking");
    fixture.emit(event(7, 1, 1, "item_delta", { itemId: "final", text: "Final answer" }));
    fixture.emit(event(8, 1, 1, "turn_completed", { status: "completed" }));
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
    expect(store.getSnapshot().repositories[1].itemsByThread[1].some((item) => item.status === "streaming")).toBe(false);
  });

  it("enables stop on the start event before the start request or cache read returns", async () => {
    let resolveStart!: (value: Awaited<ReturnType<CompanionClient["startTurn"]>>) => void;
    const interruptTurn = vi.fn(async () => undefined);
    const fixture = fakeClient({ startTurn: () => new Promise((resolve) => { resolveStart = resolve; }), interruptTurn });
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Question");
    const sending = store.send(1);
    await vi.waitFor(() => expect(resolveStart).toBeDefined());
    fixture.emit(event(1, 1, 1, "turn_started", {}));
    expect(store.getSnapshot().repositories[1].pending?.turnId).toBe("turn-1");
    await store.stop(1);
    expect(interruptTurn).toHaveBeenCalledWith(1, 1, "turn-1");
    fixture.emit(event(2, 1, 1, "turn_completed", { status: "completed" }));
    resolveStart({ turnId: "turn-1", providerThreadId: "provider-1", acceptedClientTurnId: "client" });
    await sending;
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
  });

  it("keeps a current turn alive when another thread or an older turn completes", async () => {
    const fixture = fakeClient({ listThreads: async () => [thread(1), thread(1, 11)] });
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Current question"); await store.send(1);
    const pending = store.getSnapshot().repositories[1].pending;
    fixture.emit(event(1, 1, 11, "turn_completed", { status: "completed" }));
    fixture.emit({ ...event(2, 1, 1, "turn_completed", { status: "completed" }), turnId: "old-turn" });
    expect(store.getSnapshot().repositories[1].pending).toEqual(pending);
    fixture.emit(event(3, 1, 1, "turn_completed", { status: "failed", message: "Provider failed" }));
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
    expect(store.getSnapshot().repositories[1].error?.message).toBe("Provider failed");
  });

  it("ends the activity indicator and keeps the prompt recoverable if the runtime disconnects", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Keep my question"); await store.send(1);
    fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: "Partial" }));
    fixture.emit({ ...event(2, 1, 1, "runtime_status", { schemaVersion: 1, phase: "interrupted", codexPath: "/fake/codex", codexVersion: "fake", message: "Process exited", startedAt: "now", activeTurns: 0 }), repositoryId: null, threadId: null, turnId: null });
    const repository = store.getSnapshot().repositories[1];
    expect(repository.pending).toBeNull();
    expect(repository.draft).toBe("Keep my question");
    expect(repository.error?.code).toBe("runtime_interrupted");
    expect(repository.itemsByThread[1].find((item) => item.id === "answer")?.status).toBe("interrupted");
  });

  it("guards duplicate sends and permits the next message after an interrupted turn", async () => {
    const startTurn = vi.fn(async (request: Parameters<CompanionClient["startTurn"]>[0]) => ({ turnId: `turn-${startTurn.mock.calls.length}`, providerThreadId: `provider-${request.threadId}`, acceptedClientTurnId: request.clientTurnId }));
    const fixture = fakeClient({ startTurn });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain finding");
    await Promise.all([store.send(1), store.send(1)]);
    expect(startTurn).toHaveBeenCalledTimes(1);

    store.setDraft(1, "Continue with remediation");
    fixture.emit(event(1, 1, 1, "item_delta", { itemId: "assistant", text: "partial answer" }));
    fixture.emit(event(2, 1, 1, "turn_completed", { status: "interrupted", message: null }));
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
    expect(store.getSnapshot().repositories[1].itemsByThread[1].find((item) => item.id === "assistant")?.status).toBe("interrupted");
    expect(store.getSnapshot().repositories[1].draft).toBe("Continue with remediation");
    await store.send(1);
    expect(startTurn).toHaveBeenCalledTimes(2);
    expect(startTurn.mock.calls[1][0].prompt).toBe("Continue with remediation");
  });

  it("ends a failed turn after the provider replaces its draft session identity", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain finding");
    await store.send(1);
    const turnId = store.getSnapshot().repositories[1].pending!.turnId;
    fixture.emit({ ...event(1, 1, 1, "item_delta", { itemId: "assistant", text: "" }), turnId });
    const updated = { ...store.getSnapshot().repositories[1].threads.find((thread) => thread.id === 1)!, providerThreadId: "real-session" };
    fixture.emit({ ...event(2, 1, 1, "thread_updated", updated, "real-session"), turnId });
    fixture.emit({ ...event(3, 1, 1, "error", { code: "provider_error", message: "OpenCode's free tier can only be used from within OpenCode", retryable: true }, "real-session"), turnId });
    const state = store.getSnapshot().repositories[1];
    expect(state.pending).toBeNull();
    expect(state.error?.message).toContain("free tier");
    expect(state.itemsByThread[1].find((item) => item.id === "assistant")?.status).toBe("failed");
  });

  it("does not resurrect pending when a provider fails before startTurn resolves", async () => {
    let resolveStart!: (value: Awaited<ReturnType<CompanionClient["startTurn"]>>) => void;
    const fixture = fakeClient({ startTurn: () => new Promise((resolve) => { resolveStart = resolve; }) });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain finding");
    const sending = store.send(1);
    await vi.waitFor(() => expect(resolveStart).toBeDefined());
    fixture.emit(event(1, 1, 1, "error", { code: "provider_error", message: "Rejected", retryable: true }));
    resolveStart({ turnId: "turn-1", providerThreadId: "provider-1", acceptedClientTurnId: "client" });
    await sending;
    expect(store.getSnapshot().repositories[1].pending).toBeNull();
  });

  it("keeps an active turn running for warnings and late errors from an older turn", async () => {
    const fixture = fakeClient();
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain finding");
    await store.send(1);
    const pending = store.getSnapshot().repositories[1].pending;
    fixture.emit({ ...event(1, 1, 1, "error", { code: "codex_warning", message: "Warning", retryable: false }), turnId: pending!.turnId });
    expect(store.getSnapshot().repositories[1].pending).toEqual(pending);
    fixture.emit({ ...event(2, 1, 1, "error", { code: "provider_error", message: "Old failure", retryable: true }), turnId: "older-turn" });
    expect(store.getSnapshot().repositories[1].pending).toEqual(pending);
  });

  it("reconciles an echoed provider user item with the optimistic local message", async () => {
    const fixture = fakeClient({
      listModels: async () => [
        { id: "fake-model", displayName: "Fake model", description: null, isDefault: true, hidden: false, reasoningEfforts: ["low"], inputModalities: ["text"], contextWindow: 8192 },
        { id: "alternate-model", displayName: "Alternate model", description: null, isDefault: false, hidden: false, reasoningEfforts: ["medium"], inputModalities: ["text"], contextWindow: 16_384 },
      ],
    });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setRepositoryModel(1, "alternate-model");
    store.setDraft(1, "When was this repository last scanned?");
    await store.send(1);

    const providerUserItem = {
      id: "provider-user-1",
      kind: "user_message",
      status: "streaming",
      text: "When was this repository last scanned?",
      activity: null,
      citations: [],
      error: null,
      createdAt: "2026-08-20T09:38:35Z",
    };
    fixture.emit(event(1, 1, 1, "item_started", providerUserItem));
    fixture.emit(event(2, 1, 1, "activity", { kind: "context", label: "Prepared bounded repository context", detail: null, sourceIds: [] }));
    fixture.emit(event(3, 1, 1, "item_completed", { ...providerUserItem, status: "completed" }));

    const items = store.getSnapshot().repositories[1].itemsByThread[1];
    expect(items.filter((item) => item.kind === "user_message")).toEqual([
      expect.objectContaining({ id: "provider-user-1", text: "When was this repository last scanned?", modelId: "alternate-model", status: "completed" }),
    ]);
    expect(items.filter((item) => item.kind === "activity")).toHaveLength(0);
  });

  it("continues an existing connection-bound chat with a newly selected model", async () => {
    const startTurn = vi.fn(async (request: Parameters<CompanionClient["startTurn"]>[0]) => ({
      turnId: "turn-model-change",
      providerThreadId: `provider-${request.threadId}`,
      acceptedClientTurnId: request.clientTurnId,
    }));
    const fixture = fakeClient({
      listModels: async () => [
        { id: "fake-model", displayName: "Fake model", description: null, isDefault: true, hidden: false, reasoningEfforts: ["low"], inputModalities: ["text"], contextWindow: 8192 },
        { id: "alternate-model", displayName: "Alternate model", description: null, isDefault: false, hidden: false, reasoningEfforts: ["medium"], inputModalities: ["text"], contextWindow: 16_384 },
      ],
      startTurn,
    });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);

    store.setRepositoryModel(1, "alternate-model");
    store.setDraft(1, "Continue with another model");
    await store.send(1);

    expect(startTurn).toHaveBeenCalledWith(expect.objectContaining({
      connectionId: "codex_default",
      model: "alternate-model",
      threadId: 1,
    }));
    expect(store.getSnapshot().repositories[1].threads[0].lastModelId).toBe("alternate-model");
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].modelId).toBe("alternate-model");
  });

  it("preserves the draft when turn start fails and tracks consent and changed scan context", async () => {
    const startTurn = vi.fn(async () => { throw new Error("offline"); });
    const fixture = fakeClient({ startTurn });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Keep this draft");
    await store.preview(1);
    store.markScanContext(1, 11);
    expect(store.getSnapshot().repositories[1].contextChanged).toBe(true);
    await store.send(1);
    expect(store.getSnapshot().repositories[1].draft).toBe("Keep this draft");
    expect(store.getSnapshot().repositories[1].error?.code).toBe("turn_start_failed");
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toEqual([]);
    await store.send(1);
    expect(startTurn).toHaveBeenCalledTimes(2);
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toEqual([]);
    await store.setConsent(true);
    expect(store.getSnapshot().consent?.valid).toBe(true);
  });

  it("keeps local thread state when provider history is unavailable", async () => {
    const fixture = fakeClient({ resumeThread: async () => { throw new Error("provider history missing"); } });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    expect(store.getSnapshot().repositories[1].activeThreadId).toBe(1);
    expect(store.getSnapshot().repositories[1].historyUnavailable).toBe(true);
    expect(store.getSnapshot().repositories[1].error).toBeNull();
    expect(store.getSnapshot().repositories[1].cached).toBe(true);
  });

  it("turns a missing rollout into explicit new-chat recovery without duplicate messages", async () => {
    let missingOnce = true;
    const startTurn = vi.fn(async (request: Parameters<CompanionClient["startTurn"]>[0]) => {
      if (request.threadId === 1 && missingOnce) {
        missingOnce = false;
        throw new Error("no rollout found for thread id provider-1");
      }
      return { turnId: "turn-recovered", providerThreadId: `provider-${request.threadId}`, acceptedClientTurnId: request.clientTurnId };
    });
    const fixture = fakeClient({ startTurn });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.loadGlobal();
    await store.openRepository(1);
    store.setDraft(1, "Explain finding");
    await store.send(1);

    let repository = store.getSnapshot().repositories[1];
    expect(repository.historyUnavailable).toBe(true);
    expect(repository.draft).toBe("Explain finding");
    expect(repository.itemsByThread[1]).toEqual([]);
    await store.send(1);
    expect(startTurn).toHaveBeenCalledTimes(2);
    repository = store.getSnapshot().repositories[1];
    expect(repository.pending).not.toBeNull();
    expect(repository.itemsByThread[1].filter((item) => item.kind === "user_message")).toHaveLength(1);
  });

  it("logs out only through the explicit shared-account action and clears provider display state", async () => {
    const logout = vi.fn(async () => undefined);
    const fixture = fakeClient({
      logout,
      accountStatus: async () => ({ schemaVersion: 1, signedIn: false, authKind: "none", planType: null, accountLabel: null, message: null }),
    });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal();
    await store.logoutSharedCodexAccount();
    expect(logout).toHaveBeenCalledWith(true);
    expect(store.getSnapshot().account?.signedIn).toBe(false);
    expect(store.getSnapshot().models).toEqual([]);
    expect(store.getSnapshot().usage).toBeNull();
  });

  it("tracks browser/device login waiting, cancellation, expiry, and external account changes", async () => {
    const fixture = fakeClient({
      accountStatus: async () => ({ schemaVersion: 1, signedIn: false, authKind: "none", planType: null, accountLabel: null, message: null }),
    });
    const store = new CompanionStore(fixture.client);
    await store.connect();
    await store.beginChatgptLogin(true);
    expect(store.getSnapshot().login).toMatchObject({ phase: "waiting", loginId: "login", userCode: null });
    await store.cancelLogin();
    expect(store.getSnapshot().login.phase).toBe("cancelled");

    await store.beginChatgptLogin(false);
    fixture.emit({ schemaVersion: 1, repositoryId: null, threadId: null, providerThreadId: null, connectionId: "codex_default", driver: "codex", modelId: null, turnId: null, sequence: 1, kind: "account_status", payload: { loginId: "login", success: false, error: "Device code expired" } });
    expect(store.getSnapshot().login.phase).toBe("expired");

    fixture.emit({ schemaVersion: 1, repositoryId: null, threadId: null, providerThreadId: null, connectionId: "codex_default", driver: "codex", modelId: null, turnId: null, sequence: 2, kind: "account_status", payload: { authMode: null, planType: null } });
    expect(store.getSnapshot().login.phase).toBe("external_change");
  });
});

describe("conversation history reconciliation", () => {
  it("reconciles persisted provider echoes without collapsing repeated sends", async () => {
    const sent: string[] = [];
    const fixture = fakeClient({
      startTurn: async (request) => {
        sent.push(request.prompt);
        return { turnId: `turn-${sent.length}`, providerThreadId: `provider-${request.threadId}`, acceptedClientTurnId: request.clientTurnId };
      },
      cachedThread: async () => ({
        schemaVersion: 1, thread: thread(1), segments: [], providerHistoryAvailable: true,
        items: sent.map((text, index) => ({ id: `user-${index + 1}`, kind: "user_message", status: "completed", text, activity: null, citations: [], error: null, createdAt: "now" })),
      }),
    });
    const store = new CompanionStore(fixture.client);
    await store.connect(); await store.loadGlobal(); await store.openRepository(1);
    store.setDraft(1, "Any critical findings?"); await store.send(1);
    fixture.emit(event(1, 1, 1, "turn_completed", { status: "completed" }));
    store.setDraft(1, "Any critical findings?"); await store.send(1);
    const items = store.getSnapshot().repositories[1].itemsByThread[1];
    expect(items.filter(item => item.kind === "user_message").map(item => item.id)).toEqual(["user-1", "user-2"]);
  });

  it("keeps the selected provider when another conversation's cache finishes late", async () => {
    let finish!: (value: Awaited<ReturnType<CompanionClient["cachedThread"]>>) => void;
    const older = { ...thread(1, 1), provider: "grok", providerConnectionId: "grok_default", lastModelId: "grok-4.6" };
    const current = thread(1, 2);
    const resumeThread = vi.fn(async () => snapshot(current));
    const fixture = fakeClient({
      conversationIndex: async () => ({ schemaVersion: 1, threads: [older, current] }),
      cachedThread: async (_repositoryId, id) => id === 1 ? new Promise(resolve => { finish = resolve; }) : { schemaVersion: 1, thread: current, segments: [], items: [], providerHistoryAvailable: true },
      resumeThread,
    });
    const store = new CompanionStore(fixture.client);
    await store.loadGlobal(); await store.loadConversationIndex();
    const loadingOlder = store.activateConversation(1, 1);
    await store.activateConversation(1, 2);
    finish({ schemaVersion: 1, thread: older, segments: [], items: [], providerHistoryAvailable: false });
    await loadingOlder;
    expect(store.getSnapshot().repositories[1]).toMatchObject({ activeThreadId: 2, historyUnavailable: false, selection: { connectionId: "codex_default" } });
    expect(resumeThread).toHaveBeenCalledTimes(1);
  });

  it("does not reactivate a conversation whose local cache finishes late", async () => {
    let finish!: (value: Awaited<ReturnType<CompanionClient["cachedThread"]>>) => void;
    const fixture = fakeClient({ cachedThread: async () => new Promise(resolve => { finish = resolve; }) });
    const store = new CompanionStore(fixture.client);
    const resume = store.resumeThread(1, 1);
    store.newConversation(1);
    finish({ schemaVersion: 1, thread: thread(1), segments: [], items: [], providerHistoryAvailable: false });
    await resume;
    expect(store.getSnapshot().repositories[1]).toMatchObject({ activeThreadId: null, historyUnavailable: false, error: null });
  });

  it("preserves separate messages when the user repeats the same question", async () => {
    const user = (id: string) => ({ id, kind: "user_message" as const, status: "completed" as const, text: "Any critical findings?", activity: null, citations: [], error: null, createdAt: "now" });
    const fixture = fakeClient({ resumeThread: async () => ({ ...snapshot(thread(1)), items: [user("user-1"), user("user-2")] }) });
    const store = new CompanionStore(fixture.client);
    await store.resumeThread(1, 1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1].filter(item => item.kind === "user_message")).toHaveLength(2);
  });
  it("keeps the most recently selected conversation when an earlier resume finishes late", async () => {
    const resolvers = new Map<number, (value: CompanionThreadSnapshot) => void>();
    const fixture = fakeClient({ resumeThread: async (_repo, id) => new Promise(resolve => resolvers.set(id, resolve)) });
    const store = new CompanionStore(fixture.client);
    const a = store.resumeThread(1, 1);
    await vi.waitFor(() => expect(resolvers.has(1)).toBe(true));
    const b = store.resumeThread(1, 2);
    await vi.waitFor(() => expect(resolvers.has(2)).toBe(true));
    resolvers.get(2)!(snapshot(thread(1, 2)));
    await b;
    resolvers.get(1)!(snapshot(thread(1, 1)));
    await a;
    expect(store.getSnapshot().repositories[1].activeThreadId).toBe(2);
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toBeDefined();
  });
  it("retains cached citation metadata when the provider resume has no source catalog", async () => {
    const citation = { sourceId: "scan:10", label: "Scan #10", kind: "scan" as const, stale: false };
    const answer = { id: "answer", kind: "assistant_message" as const, status: "completed" as const, text: "See [source:scan:10]", activity: null, citations: [citation], error: null, createdAt: "now" };
    const fixture = fakeClient({
      cachedThread: async () => ({ schemaVersion: 1, thread: thread(1), segments: [], items: [answer], providerHistoryAvailable: true }),
      resumeThread: async () => ({ ...snapshot(thread(1)), items: [{ ...answer, citations: [{ ...citation, label: "scan:10", stale: true }] }] }),
    });
    const store = new CompanionStore(fixture.client);
    await store.resumeThread(1, 1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].citations[0]).toMatchObject({ label: "Scan #10", stale: false });
  });
  it("keeps explicit stale source metadata from a known catalog", async () => {
    const citation = { sourceId: "scan:10", label: "Scan #10", kind: "scan" as const, stale: false };
    const answer = { id: "answer", kind: "assistant_message" as const, status: "completed" as const, text: "See [source:scan:10]", activity: null, citations: [citation], error: null, createdAt: "now" };
    const fixture = fakeClient({
      cachedThread: async () => ({ schemaVersion: 1, thread: thread(1), segments: [], items: [answer], providerHistoryAvailable: true }),
      resumeThread: async () => ({ ...snapshot(thread(1)), items: [{ ...answer, citations: [{ ...citation, stale: true }] }] }),
    });
    const store = new CompanionStore(fixture.client);
    await store.resumeThread(1, 1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].citations[0].stale).toBe(true);
  });
});

it("does not append deltas already included in an atomic native history page",async()=>{
  const fixture=fakeClient({cachedThread:async(repo,id)=>({schemaVersion:1,thread:thread(repo,id),segments:[{id:1,threadId:id,ordinal:1,provider:"codex",connectionId:"codex_default",providerThreadId:`provider-${id}`,initialModelId:null,lastModelId:null,status:"ready",createdAt:"now",updatedAt:"now",endedAt:null}],items:[{id:"segment:1:answer",kind:"assistant_message",status:"completed",text:"abc",activity:null,citations:[],error:null,createdAt:"now"}],providerHistoryAvailable:true,olderCursor:null,providerOrdinals:{"segment:1:answer":1},eventWatermarks:{"1:codex_default:epoch":3},streamingItemIds:[]}),resumeThread:async()=>{throw new Error("offline");}});
  const store=new CompanionStore(fixture.client);await store.connect();await store.loadGlobal();await store.openRepository(1);
  vi.useFakeTimers();try{
    fixture.emit({...event(3,1,1,"item_delta",{itemId:"answer",text:"c"}),providerSegmentId:1,cacheEpoch:"epoch"});
    fixture.emit({...event(4,1,1,"item_delta",{itemId:"answer",text:"d"}),providerSegmentId:1,cacheEpoch:"epoch"});
    vi.advanceTimersByTime(50);
    expect(store.getSnapshot().repositories[1].itemsByThread[1].map((item)=>item.text)).toEqual(["abcd"]);
  }finally{store.disconnect();vi.useRealTimers();}
});

it("retains the live tail while browsing beyond the bounded history window",async()=>{
  const item=(index:number)=>({id:`segment:1:answer-${index}`,kind:"assistant_message" as const,status:"completed" as const,text:`message-${index}`,activity:null,citations:[],error:null,createdAt:"now"});
  const page=(start:number,end:number,older:string|null)=>({schemaVersion:1,thread:thread(1),segments:[{id:1,threadId:1,ordinal:1,provider:"codex",connectionId:"codex_default",providerThreadId:"provider-1",initialModelId:null,lastModelId:null,status:"ready" as const,createdAt:"now",updatedAt:"now",endedAt:null}],items:Array.from({length:end-start},(_,index)=>item(start+index)),providerHistoryAvailable:true,olderCursor:older,providerOrdinals:{},eventWatermarks:{}});
  let latest=page(400,600,"older-400");
  const fixture=fakeClient({cachedThread:async()=>latest,historyPage:async(_repo,_thread,cursor)=>cursor==="older-400"?page(200,400,"older-200"):page(0,200,null),resumeThread:async()=>({...snapshot(thread(1)),items:[item(599)]})});
  const store=new CompanionStore(fixture.client);await store.connect();await store.loadGlobal();await store.openRepository(1);
  vi.useFakeTimers();try{
    fixture.emit({...event(1,1,1,"item_delta",{itemId:"answer-599",text:"+live-a"}),providerSegmentId:1});vi.advanceTimersByTime(50);
    await store.loadEarlierHistory(1,1);await store.loadEarlierHistory(1,1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toHaveLength(400);
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].id).toBe("segment:1:answer-0");
    store.newConversation(1);
    await store.activateConversation(1,1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1][0].id).toBe("segment:1:answer-0");
    expect(store.getSnapshot().repositories[1].historyPages?.[1]?.before).toBe("older-200");
    fixture.emit({...event(2,1,1,"item_delta",{itemId:"answer-599",text:"+live-b"}),providerSegmentId:1});vi.advanceTimersByTime(50);
    latest={...latest,items:[...latest.items.slice(0,-1),{...item(599),text:"message-599+live-a+live-b"}]};
    await store.loadLatestHistory(1,1);
    expect(store.getSnapshot().repositories[1].itemsByThread[1].at(-1)?.text).toBe("message-599+live-a+live-b");
    expect(store.getSnapshot().repositories[1].itemsByThread[1]).toHaveLength(200);
  }finally{store.disconnect();vi.useRealTimers();}
});

it("bounds timeline and provider/page metadata after fifty conversation switches",async()=>{
  const conversations=Array.from({length:50},(_,index)=>thread(1,index+1));
  const fixture=fakeClient({
    listConnections:async()=>[{...codexConnection(),connection:{...codexConnection().connection,enabled:false}}],
    conversationIndex:async()=>({schemaVersion:1,threads:conversations}),
    listThreads:async()=>conversations,
    cachedThread:async(repositoryId,threadId)=>({
      schemaVersion:1,thread:thread(repositoryId,threadId),providerHistoryAvailable:true,olderCursor:"older-page",
      segments:Array.from({length:25},(_,index)=>({id:threadId*100+index,threadId,ordinal:index+1,provider:"codex",connectionId:"codex_default",providerThreadId:`provider-${threadId}`,initialModelId:null,lastModelId:null,status:"ready" as const,createdAt:"now",updatedAt:"now",endedAt:null})),
      items:Array.from({length:200},(_,index)=>({id:`segment:${threadId*100+index%25}:message-${index}`,kind:"assistant_message" as const,status:"completed" as const,text:`Historical message ${index+1800} of 2000`,activity:null,citations:[],error:null,createdAt:"now"})),
      providerOrdinals:Object.fromEntries(Array.from({length:200},(_,index)=>[`segment:${threadId*100+index%25}:message-${index}`,index%25+1])),eventWatermarks:{},streamingItemIds:[],
    }),
  });
  const store=new CompanionStore(fixture.client);
  await store.connect();await store.loadGlobal();await store.openRepository(1);
  for(const conversation of conversations)await store.activateConversation(1,conversation.id);
  const repository=store.getSnapshot().repositories[1];
  expect(store.getPerformanceMetrics().historyCacheEntries).toBeLessThanOrEqual(12);
  expect(store.getPerformanceMetrics().historyCacheBytes).toBeLessThanOrEqual(16*1024*1024);
  for(const cached of [repository.itemsByThread,repository.segmentsByThread,repository.historyPages,repository.historyProviderOrdinals,repository.usageByThread,repository.usageAttributionByThread])expect(Object.keys(cached??{}).length).toBeLessThanOrEqual(12);
  expect(repository.itemsByThread[50]).toHaveLength(200);
  expect(repository.segmentsByThread[50]).toHaveLength(25);
  await store.activateConversation(1,1);
  expect(store.getSnapshot().repositories[1].itemsByThread[1]).toHaveLength(200);
  store.disconnect();
});

it("releases settled history requests over repeated cache eviction and reload cycles",async()=>{
  const conversations=Array.from({length:40},(_,index)=>thread(1,index+1));
  const cached=(repo:number,id:number)=>({schemaVersion:1,thread:thread(repo,id),segments:[],items:[{id:`message-${id}`,kind:"assistant_message" as const,status:"completed" as const,text:`history-${id}`,activity:null,citations:[],error:null,createdAt:"now"}],providerHistoryAvailable:false,olderCursor:null,providerOrdinals:{},eventWatermarks:{},streamingItemIds:[]});
  const fixture=fakeClient({listConnections:async()=>[{...codexConnection(),connection:{...codexConnection().connection,enabled:false}}],conversationIndex:async()=>({schemaVersion:1,threads:conversations}),listThreads:async()=>conversations,cachedThread:async(repo,id)=>cached(repo,id)});
  const store=new CompanionStore(fixture.client);
  await store.connect();await store.loadGlobal();await store.openRepository(1);
  for(let cycle=0;cycle<8;cycle++){
    for(const conversation of conversations){
      await store.activateConversation(1,conversation.id);
      await store.loadLatestHistory(1,conversation.id);
      expect(store.getPerformanceMetrics().historyInFlightRequests).toBe(0);
    }
    expect(store.getPerformanceMetrics().historyCacheEntries).toBeLessThanOrEqual(12);
    expect(Object.keys(store.getSnapshot().repositories[1].itemsByThread)).toHaveLength(12);
  }
  expect(store.getSnapshot().repositories[1].itemsByThread[40][0].text).toBe("history-40");
  store.disconnect();
});

it("does not let an older history response win after newer requests settle",async()=>{
  const fixture=fakeClient();const store=new CompanionStore(fixture.client);
  await store.connect();await store.loadGlobal();await store.openRepository(1);
  type Cached=Awaited<ReturnType<CompanionClient["cachedThread"]>>;
  const pending:Array<(value:Cached)=>void>=[];
  fixture.client.cachedThread=vi.fn(()=>new Promise<Cached>(resolve=>pending.push(resolve)));
  const old=store.loadLatestHistory(1,1);const fresh=store.loadLatestHistory(1,1);
  const page=(text:string):Cached=>({schemaVersion:1,thread:thread(1,1),segments:[],items:[{id:"answer",kind:"assistant_message",status:"completed",text,activity:null,citations:[],error:null,createdAt:"now"}],providerHistoryAvailable:false,olderCursor:null});
  pending[1](page("newest"));await fresh;
  expect(store.getPerformanceMetrics().historyInFlightRequests).toBe(0);
  const latest=store.loadLatestHistory(1,1);
  pending[0](page("stale"));await old;
  expect(store.getPerformanceMetrics().historyInFlightRequests).toBe(1);
  expect(store.getSnapshot().repositories[1].itemsByThread[1][0].text).toBe("newest");
  pending[2](page("latest"));await latest;
  expect(store.getSnapshot().repositories[1].itemsByThread[1][0].text).toBe("latest");
  expect(store.getPerformanceMetrics().historyInFlightRequests).toBe(0);store.disconnect();
});


it("preserves the hidden live response when the runtime disconnects during history browsing", async () => {
  const item = (id: string, text: string) => ({ id, kind: "assistant_message" as const, status: "completed" as const, text, activity: null, citations: [], error: null, createdAt: "now" });
  const page = (items: ReturnType<typeof item>[], olderCursor: string | null) => ({ schemaVersion: 1, thread: thread(1), segments: [], items, providerHistoryAvailable: true, olderCursor, eventWatermarks: {}, providerOrdinals: {} });
  const fixture = fakeClient({
    cachedThread: async () => page([item("answer", "saved prefix")], "older"),
    historyPage: async () => page(Array.from({length: 400}, (_, index) => item(`old-${index}`, `historical message ${index}`)), null),
    resumeThread: async () => snapshot(thread(1)),
  });
  const store = new CompanionStore(fixture.client);
  await store.connect(); await store.loadGlobal(); await store.openRepository(1);
  store.setDraft(1, "Continue"); await store.send(1);
  fixture.emit(event(1, 1, 1, "item_delta", { itemId: "answer", text: " + live suffix" }));
  await store.loadEarlierHistory(1, 1);
  expect(store.getSnapshot().repositories[1].itemsByThread[1].some((entry) => entry.id === "answer")).toBe(false);
  fixture.emit(event(2, 1, 1, "runtime_status", { schemaVersion: 1, phase: "interrupted", codexPath: null, codexVersion: null, message: "Disconnected", startedAt: null, activeTurns: 0 }));
  expect(store.getSnapshot().repositories[1].pending).toBeNull();
  // The saved snapshot lags the last delivered live event. Returning to latest
  // must retain the local interrupted tail, rather than the visible old page.
  await store.loadLatestHistory(1, 1);
  const answer = store.getSnapshot().repositories[1].itemsByThread[1].find((entry) => entry.id === "answer");
  expect(answer?.text).toBe("saved prefix + live suffix");
  expect(answer?.status).toBe("interrupted");
  fixture.client.cachedThread = async () => page([item("answer", "saved prefix + live suffix + recovered")], null);
  await store.loadLatestHistory(1, 1);
  expect(store.getSnapshot().repositories[1].itemsByThread[1].find((entry) => entry.id === "answer")).toMatchObject({text: "saved prefix + live suffix + recovered", status: "completed"});
  store.disconnect();
});


it("retains archived metadata without automatically opening an archived conversation", async () => {
  const archived = { ...thread(1, 2), archivedAt: "now" };
  const connection = codexConnection();
  const fixture = fakeClient({ listConnections: async () => [{ ...connection, health: "disabled", connection: { ...connection.connection, enabled: false } }], conversationIndex: async () => ({ schemaVersion: 1, threads: [archived, thread(1, 1)] }) });
  const store = new CompanionStore(fixture.client);
  await store.loadGlobal(); await store.loadConversationIndex(); await store.openRepository(1);
  expect(store.getSnapshot().repositories[1].activeThreadId).toBe(1);
  await store.setThreadArchived(1, 1, true);
  expect(store.getSnapshot().conversationIndex?.threads).toHaveLength(2);
  expect(store.getSnapshot().repositories[1].activeThreadId).toBeNull();
  store.disconnect();
});

it("keeps a running response cancellable when a new conversation is opened", async () => {
  const interruptTurn = vi.fn(async () => undefined);
  const fixture = fakeClient({ interruptTurn });
  const store = new CompanionStore(fixture.client);
  await store.loadGlobal(); await store.openRepository(1);
  await store.setRepositoryConsent(1, true);
  store.setDraft(1, "Keep running"); await store.send(1);
  const pending = store.getSnapshot().repositories[1].pending;
  expect(pending?.turnId).toBe("turn-1");
  store.newConversation(1);
  expect(store.getSnapshot().repositories[1].pending).toEqual(pending);
  await store.stop(1);
  expect(interruptTurn).toHaveBeenCalledTimes(1);
  store.disconnect();
});

it("does not replace current context with a preview requested for removed attachments", async () => {
  let finish!: (value: RepositoryChatContext) => void;
  const fixture = fakeClient({ previewContext: () => new Promise(resolve => { finish = resolve; }) });
  const store = new CompanionStore(fixture.client);
  await store.loadGlobal(); await store.openRepository(1);
  store.setAttachments(1, [{ kind: "finding", id: "old" }]);
  const pending = store.preview(1);
  store.setAttachments(1, []);
  finish(context(1)); await pending;
  expect(store.getSnapshot().repositories[1].context).toBeNull();
  store.disconnect();
});
