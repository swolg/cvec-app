import axe from "axe-core";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CompanionSendPreview, CompanionStateSnapshot } from "@/lib/companionState";
import type { CompanionThread, FindingChatRequest, ProviderConnectionSnapshot, RepositoryChatContext, RepositoryChatScanStatus, RepoSummary } from "@/lib/types";

const mocks = vi.hoisted(() => ({
  state: null as CompanionStateSnapshot | null,
  initialize: vi.fn(async () => undefined),
  startRuntime: vi.fn(async () => undefined),
  openRepository: vi.fn(async () => undefined),
  markScanContext: vi.fn(),
  setDraft: vi.fn(),
  retryDraftStorage: vi.fn(async () => undefined),
  newConversation: vi.fn(),
  setAttachments: vi.fn(),
  setFocusedFinding: vi.fn(),
  preview: vi.fn(async () => undefined),
  previewSend: vi.fn(async (): Promise<CompanionSendPreview> => { throw new Error("No prepared context"); }),
  send: vi.fn(async (): Promise<"review-required" | void> => undefined),
  stop: vi.fn(async () => undefined),
  setRepositoryConsent: vi.fn(async () => undefined),
  selectRepositoryConnection: vi.fn(async () => undefined),
  setRepositoryEffort: vi.fn(),
  renameThread: vi.fn(async () => undefined),
  setThreadArchived: vi.fn(async () => undefined),
  beginChatgptLogin: vi.fn(async () => ({ kind: "chatgpt", loginId: "login", authUrl: "https://example.test", verificationUrl: null, userCode: null, status: "pending" })),
}));

vi.mock("@/lib/companionState", () => ({
  useCompanionRepositoryState: () => mocks.state,
  useCompanionSelector: (selector: (state: CompanionStateSnapshot) => unknown) => selector(mocks.state!),
  shallowCompanionSelection: () => false,
  companionStore: {
    initialize: mocks.initialize,
    startRuntime: mocks.startRuntime,
    openRepository: mocks.openRepository,
    markScanContext: mocks.markScanContext,
    setDraft: mocks.setDraft,
    retryDraftStorage: mocks.retryDraftStorage,
    newConversation: mocks.newConversation,
    setAttachments: mocks.setAttachments,
    setFocusedFinding: mocks.setFocusedFinding,
    preview: mocks.preview,
    previewSend: mocks.previewSend,
    send: mocks.send,
    stop: mocks.stop,
    setRepositoryConsent: mocks.setRepositoryConsent,
    selectRepositoryConnection: mocks.selectRepositoryConnection,
    setRepositoryEffort: mocks.setRepositoryEffort,
    renameThread: mocks.renameThread,
    setThreadArchived: mocks.setThreadArchived,
    beginChatgptLogin: mocks.beginChatgptLogin,
    getSnapshot: () => mocks.state,
  },
}));

vi.mock("@/lib/companionApi", () => ({
  companionSearchMentions: vi.fn(async () => []),
  companionSourceDetail: vi.fn(async () => { throw new Error("not used"); }),
}));

vi.mock("@/lib/api", () => ({ openExternalUrl: vi.fn() }));
vi.mock("@/components/chat/FixWorkspacePanel", () => ({ FixWorkspacePanel: () => <div>Fix workspace test surface</div> }));

import { companionSearchMentions, companionSourceDetail } from "@/lib/companionApi";
import { RepositoryChat, visibleUserPrompt } from "./RepositoryChat";

const repository: RepoSummary = {
  id: 1, name: "demo", path: "/tmp/demo", sourceType: "local", sourceKey: null,
  remoteUrl: null, remoteHost: null, provider: "local", defaultBranch: "main",
  lastRemoteCommitSha: null, lastRemoteSyncedAt: null, remoteFileCount: null,
  status: "warning", newMedium: 0, newHigh: 1, newCritical: 0,
  openMedium: 0, openHigh: 1, openCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
  lastScannedAt: "2026-08-19T12:00:00Z", lastScanStatus: "completed", lastSuccessfulSourceRevision: "abc",
};

const completedScan = { id: 10, status: "completed", scannedAt: "2026-08-19T12:01:00Z", revision: "abc", evidenceComplete: true };
const scanStatus: RepositoryChatScanStatus = { repositoryId: repository.id, latestCompleted: completedScan, latestAttempt: completedScan };
const reviewedContext: RepositoryChatContext = {
  schemaVersion: 1,
  repository: { id: 1, name: "demo", sourceMode: "local_source", sourceType: "local", approvedRoot: "/tmp/demo", branch: "main", revision: "abc" },
  scan: completedScan, sources: [], attachments: [], omissions: [], untrustedEvidence: [], instructions: [],
  freshness: { state: "matched", message: "Matches the completed scan.", scanRevision: "abc", sourceRevision: "abc", sourceDirty: false },
  bounded: true, estimatedBytes: 400,
};

const thread: CompanionThread = {
  id: 1, repositoryId: 1, provider: "codex", providerThreadId: "provider-1", providerConnectionId: "codex_default",
  title: "Explain critical findings", status: "ready", createdAt: "now", updatedAt: "now", lastOpenedAt: "now", archivedAt: null, contextVersion: 1, lastModelId: "gpt-test",
};

function connection(id: string, driver: "codex" | "grok", modelId: string, modelName: string): ProviderConnectionSnapshot {
  return {
    schemaVersion: 1,
    connection: { schemaVersion: 1, id, driver, displayName: driver === "codex" ? "Codex" : "Grok", accentColor: null, enabled: true, builtIn: true, config: driver === "codex" ? { kind: "codex", binaryPath: null, homePath: null, profile: null, launchArgs: null } : { kind: "grok", binaryPath: null, customModels: [] }, hasServerPassword: false, createdAt: "now", updatedAt: "now" },
    health: "ready", installedVersion: "1", authKind: driver === "codex" ? "chatgpt" : "api_key", accountLabel: driver === "codex" ? "dev@example.test" : null,
    capabilities: { createThread: true, resumeThread: true, interrupt: true, orderedStreaming: true, readOnlyPolicy: true, accountStatus: true, rateLimits: true, tokenUsage: true, reasoningEffort: true, attachments: true },
    upstreamProviders: [{ id: driver === "codex" ? "openai" : "xai", displayName: driver === "codex" ? "OpenAI" : "xAI", connected: true, inferenceBoundary: "external" }],
    models: [{ connectionId: id, driver, upstreamProviderId: driver === "codex" ? "openai" : "xai", inferenceBoundary: "external", id: modelId, displayName: modelName, description: `${modelName} security model`, isDefault: true, hidden: false, reasoningEfforts: ["low"], inputModalities: ["text"], contextWindow: 8192 }],
    checkedAt: "now", message: null, error: null,
  };
}

const codex = connection("codex_default", "codex", "gpt-test", "GPT Test");
const grok = connection("grok_default", "grok", "grok-4.6", "Grok 4.6");

function readyState(repositoryOverrides: Partial<CompanionStateSnapshot["repositories"][number]> = {}): CompanionStateSnapshot {
  return {
    connected: true,
    initializationPhase: "ready",
    runtime: { schemaVersion: 1, phase: "ready", codexPath: "/codex", codexVersion: "1", message: null, startedAt: "now", activeTurns: 0 },
    account: { schemaVersion: 1, signedIn: true, authKind: "chatgpt", planType: "team", accountLabel: "dev@example.test", message: null },
    models: codex.models,
    connections: [codex, grok],
    selectedConnectionId: "codex_default", selectedModel: "gpt-test", selectedEffort: "low", usage: null,
    consent: null,
    login: { phase: "idle", loginId: null, userCode: null, message: null },
    conversationIndex: { schemaVersion: 1, threads: [thread] }, conversationIndexLoading: false,
    repositories: {
      1: {
        repositoryId: 1, loaded: true, threads: [thread], activeThreadId: 1,
        itemsByThread: { 1: [] }, usageByThread: {}, segmentsByThread: {},
        selection: { repositoryId: 1, connectionId: "codex_default", modelId: "gpt-test", effort: "low" },
        selectionModels: codex.models, selectionLoading: false,
        consent: { provider: "fingerprint", disclosureVersion: 1, acceptedAt: null, revokedAt: null, valid: false },
        draft: "", attachments: [], focusedFinding: null, context: null, contextChanged: false, pending: null,
        loading: false, historyUnavailable: false, cached: true, error: null,
        ...repositoryOverrides,
      },
    },
  };
}

describe("RepositoryChat", () => {
  it("protects an unread saved draft and offers an explicit restore retry", async () => {
    mocks.state = readyState({ draftLoaded: false, draftStorageError: "restore" });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(screen.getByText(/saved draft could not be restored/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Explain highest-risk findings." })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Retry draft storage" }));
    expect(mocks.retryDraftStorage).toHaveBeenCalledWith(1);
  });
  it("attributes context to its recorded connection/model and hides Codex quota on another provider", async () => {
    const usage = { inputTokens: 1940, cachedInputTokens: 420, outputTokens: 286, reasoningTokens: 88, totalTokens: 2226, contextWindow: 200000, contextUsed: 2226 };
    mocks.state = readyState({ usageByThread: { 1: usage }, usageAttributionByThread: { 1: { connectionId: "codex_default", modelId: "gpt-test" } } });
    mocks.state.usage = { thread: null, primary: { usedPercent: 42, windowDurationMinutes: 300, resetsAt: null }, secondary: null, lifetimeTokens: 1000, peakDailyTokens: null, supported: true, unavailableReason: null, fetchedAt: "now" };
    const props = { repository, scanStatus, onError: vi.fn() };
    const view = render(<RepositoryChat {...props} />);
    expect(screen.getByRole("button", { name: "Context window 1 percent used" })).toBeInTheDocument();
    mocks.state = { ...mocks.state, repositories: { 1: { ...mocks.state.repositories[1], selection: { repositoryId: 1, connectionId: "grok_default", modelId: "grok-4.6", effort: null }, selectionModels: grok.models } } };
    view.rerender(<RepositoryChat {...props} />);
    expect(screen.queryByRole("button", { name: /Context window/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Previous context reading" }));
    expect(screen.getByText("Last recorded context")).toBeVisible();
    expect(screen.getByText("Codex · gpt-test")).toBeVisible();
    expect(screen.queryByText("42.0% used")).not.toBeInTheDocument();
  });
  it("keeps cached answers readable through provider discovery and failure while blocking send", () => {
    const item = { id: "cached", kind: "assistant_message" as const, status: "completed" as const, text: "Cached security answer", activity: null, citations: [], error: null, createdAt: "now" };
    mocks.state = { ...readyState({ itemsByThread: { 1: [item] }, draft: "Follow up" }), initializationPhase: "checking" };
    const props = { scanStatus, repository, onError: vi.fn() };
    const view = render(<RepositoryChat {...props} />);
    expect(screen.getByText("Cached security answer")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    mocks.state = { ...mocks.state, initializationPhase: "failed" };
    view.rerender(<RepositoryChat {...props} />);
    expect(screen.getByText("Cached security answer")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  });
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(companionSearchMentions).mockReset().mockResolvedValue([]);
    vi.mocked(companionSourceDetail).mockReset().mockRejectedValue(new Error("not used"));
    mocks.startRuntime.mockReset().mockResolvedValue(undefined);
    mocks.state = readyState();
  });

  it("keeps disabled providers stopped and explains their actual availability", () => {
    const item = { id: "cached-disabled", kind: "assistant_message" as const, status: "completed" as const, text: "Saved offline answer", activity: null, citations: [], error: null, createdAt: "now" };
    mocks.state = readyState({ itemsByThread: { 1: [item] }, draft: "Offline draft" });
    mocks.state.connections = [{ ...codex, connection: { ...codex.connection, enabled: false }, health: "disabled" }];
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "disabled", message: "Codex is available and starts only when Chat is used." };
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(screen.getByText("Saved offline answer")).toBeInTheDocument();
    expect(screen.getByText("Codex is disabled. Enable it in provider settings or choose another connection.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(mocks.startRuntime).not.toHaveBeenCalled();
  });

  it("offers provider settings for a disabled connection even if its runtime is unavailable", async () => {
    mocks.state = readyState({ activeThreadId: null, draft: "Keep my draft" });
    mocks.state.connections = [{ ...codex, connection: { ...codex.connection, enabled: false }, health: "disabled" }];
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "unavailable" };
    const onOpenAiConnections = vi.fn();
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} onOpenAiConnections={onOpenAiConnections} />);

    expect(screen.getByRole("heading", { name: "Codex is disabled" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveValue("Keep my draft");
    await userEvent.click(screen.getByRole("button", { name: "Provider settings" }));
    expect(onOpenAiConnections).toHaveBeenCalledOnce();
    expect(mocks.startRuntime).not.toHaveBeenCalled();
  });

  it("appends a starter without losing an existing draft and focuses the composer", async () => {
    mocks.state = readyState({ activeThreadId: null, draft: "Keep my detailed review notes." });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Explain highest-risk findings." }));
    expect(mocks.setDraft).toHaveBeenCalledWith(1, "Keep my detailed review notes.\n\nExplain highest-risk findings.");
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveFocus();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("focuses an empty draft when a starter is chosen without submitting it", async () => {
    mocks.state = readyState({ activeThreadId: null });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Trace vulnerable packages." }));
    expect(mocks.setDraft).toHaveBeenCalledWith(1, "Trace vulnerable packages.");
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveFocus();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it.each(["starting", "signed-out"])("keeps direct provider selection available for an empty %s Codex chat", async (phase) => {
    mocks.state = readyState({ activeThreadId: null, draft: "Keep this draft" });
    if (phase === "starting") mocks.state.runtime = { ...mocks.state.runtime!, phase: "starting" };
    else mocks.state.account = { ...mocks.state.account!, signedIn: false };
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveValue("Keep this draft");
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "AI connection and model" }));
    await userEvent.click(screen.getByRole("button", { name: "Grok models" }));
    await userEvent.click(screen.getByText("Grok 4.6", { exact: true }));
    expect(mocks.selectRepositoryConnection).toHaveBeenCalledWith(1, "grok_default", "grok-4.6", false);
    expect(mocks.send).not.toHaveBeenCalled();
    expect(mocks.beginChatgptLogin).not.toHaveBeenCalled();
  });

  it("leaves a rejected Codex startup and allows an explicit retry", async () => {
    mocks.state = readyState({ activeThreadId: null, threads: [] });
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "disabled", message: "Available" };
    mocks.startRuntime
      .mockImplementationOnce(async () => {
        mocks.state = { ...mocks.state!, runtime: { ...mocks.state!.runtime!, phase: "initializing", message: "Initializing" } };
        throw new Error("Codex startup disconnected");
      })
      .mockImplementationOnce(async () => {
        mocks.state = readyState({ activeThreadId: null, threads: [] });
      });
    const onError = vi.fn();
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={onError} />);

    expect(await screen.findByText("Codex could not start")).toBeInTheDocument();
    expect(screen.getByText("Codex startup disconnected")).toBeInTheDocument();
    expect(screen.queryByText("Starting Codex")).not.toBeInTheDocument();
    expect(onError).toHaveBeenCalledWith("Codex startup disconnected");
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeEnabled());
    expect(mocks.startRuntime).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Codex could not start")).not.toBeInTheDocument();
  });

  it("does not automatically repeat a rejected startup on parent rerenders", async () => {
    mocks.state = readyState({ activeThreadId: null, threads: [] });
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "disabled" };
    mocks.startRuntime.mockRejectedValue(new Error("Start unavailable"));
    const view = render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    await screen.findByText("Codex could not start");

    view.rerender(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    await waitFor(() => expect(mocks.startRuntime).toHaveBeenCalledTimes(1));
  });

  it("shows a recovery error when runtime readiness succeeds but provider setup fails", async () => {
    mocks.state = readyState({ activeThreadId: null, threads: [] });
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "disabled" };
    mocks.startRuntime.mockImplementationOnce(async () => {
      mocks.state = readyState({ activeThreadId: null, threads: [] });
      throw new Error("Account status unavailable");
    });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(await screen.findByText("Account status unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeEnabled();
  });

  it("keeps another provider selectable when Codex cannot start", async () => {
    mocks.state = readyState({ draft: "Keep my draft" });
    mocks.state.runtime = { ...mocks.state.runtime!, phase: "unavailable", message: "Codex is missing" };
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(screen.getByText("Codex could not start")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "AI connection and model" }));
    await userEvent.click(screen.getByRole("button", { name: "Grok models" }));
    await userEvent.click(screen.getByText("Grok 4.6"));
    expect(mocks.selectRepositoryConnection).toHaveBeenCalledWith(1, "grok_default", "grok-4.6", false);
    expect(mocks.setDraft).not.toHaveBeenCalled();
  });

  it("renders a compact header, stable timeline, and composer-owned model control", () => {
    mocks.state = readyState({
      itemsByThread: { 1: [
        { id: "user", kind: "user_message", status: "completed", text: "Explain risk", activity: null, citations: [], error: null, createdAt: "now" },
        { id: "context", kind: "activity", status: "completed", text: "", activity: { kind: "context", label: "Prepared bounded repository context", detail: null, sourceIds: [] }, citations: [], error: null, createdAt: "now" },
        { id: "assistant", kind: "assistant_message", status: "completed", text: "Evidence-backed answer", activity: null, citations: [], error: null, createdAt: "now" },
      ] },
    });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);

    expect(screen.getByText("Explain critical findings")).toBeInTheDocument();
    expect(screen.getByText("Evidence-backed answer")).toBeInTheDocument();
    expect(screen.getByRole("log", { name: "Repository chat messages" }).closest('[data-chat-state]')).toHaveAttribute("data-chat-state", "ready");
    expect(screen.queryByText("Prepared bounded repository context")).not.toBeInTheDocument();
    const modelPicker = screen.getByRole("button", { name: "AI connection and model" });
    expect(modelPicker).toHaveTextContent("GPT Test");
    expect(modelPicker).not.toHaveTextContent("Codex ·");
    expect(modelPicker.querySelector('[data-provider-icon="codex"]')).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toBeInTheDocument();
  });

  it("marks loading, empty, read-only history and contained errors explicitly", () => {
    const view = render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "empty");
    mocks.state = readyState({ loading: true });
    view.rerender(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "loading");
    mocks.state = readyState({
      itemsByThread: { 1: [{ id: "stored", kind: "assistant_message", status: "completed", text: "Stored answer", activity: null, citations: [], error: null, createdAt: "now" }] },
    });
    mocks.state.connections = mocks.state.connections.map(c => ({ ...c, connection: { ...c.connection, enabled: false } }));
    view.rerender(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "read-only");
    expect(screen.getByRole("log", { name: "Repository chat messages" })).toBeInTheDocument();
    expect(screen.getByText("Stored answer")).toBeInTheDocument();
    mocks.state = { ...readyState(), initializationPhase: "failed" };
    view.rerender(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "read-only");
  });

  it("switches ready provider model lists without changing the draft", async () => {
    const user = userEvent.setup();
    mocks.state = readyState({ draft: "Keep my draft" });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    await user.click(screen.getByRole("button", { name: "AI connection and model" }));
    await user.click(screen.getByRole("button", { name: "Grok models" }));
    expect(await screen.findByText("Grok 4.6")).toBeInTheDocument();
    await user.click(screen.getByText("Grok 4.6"));
    expect(mocks.selectRepositoryConnection).toHaveBeenCalledWith(1, "grok_default", "grok-4.6", false);
    expect(mocks.setDraft).not.toHaveBeenCalled();
  });

  it("keeps cached history usable when provider history cannot resume", () => {
    mocks.state = readyState({ historyUnavailable: true, draft: "Continue locally" });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(screen.getByText(/Local conversation restored/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });

  it("renders a compact provider transition marker", () => {
    mocks.state = readyState({ itemsByThread: { 1: [{ id: "transition", kind: "activity", status: "completed", text: "", activity: { kind: "provider_transition", label: "Continued with Grok", detail: "grok-4.6", sourceIds: [] }, citations: [], error: null, createdAt: "now" }] } });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(screen.getByText(/Continued with Grok · grok-4.6/)).toBeInTheDocument();
  });

  it("explains the local visible transcript cache before consent and preserves the draft", async () => {
    mocks.state = readyState({ draft: "Explain finding" });
    const preview = { ...mocks.state.repositories[1], context: reviewedContext };
    mocks.previewSend.mockResolvedValueOnce(preview);
    const user = userEvent.setup();
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Send message" }));
    expect(await screen.findByText(/normalized visible conversation locally/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Accept and send" }));
    await waitFor(() => expect(mocks.setRepositoryConsent).toHaveBeenCalledWith(1, true));
    expect(mocks.send).toHaveBeenCalledWith(1, preview);
  });

  it("asks for a new review when the prepared context changed during a send", async () => {
    mocks.state = readyState({ draft: "Question", consent: { provider: "fingerprint", disclosureVersion: 1, acceptedAt: "now", revokedAt: null, valid: true } });
    const preview = { ...mocks.state.repositories[1], context: reviewedContext };
    mocks.previewSend.mockResolvedValueOnce(preview);
    mocks.send.mockResolvedValueOnce("review-required");
    const user = userEvent.setup();
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Send message" }));
    expect(await screen.findByText(/Repository context changed/)).toBeInTheDocument();
    expect(mocks.send).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Accept and send" }));
    expect(mocks.send).toHaveBeenLastCalledWith(1, preview);
    expect(mocks.setRepositoryConsent).not.toHaveBeenCalled();
  });

  it("supports enter-to-send and remains axe-clean", async () => {
    mocks.state = readyState({ draft: "Question", consent: { provider: "fingerprint", disclosureVersion: 1, acceptedAt: "now", revokedAt: null, valid: true } });
    const view = render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message the security companion" }), { key: "Enter", shiftKey: false });
    expect(mocks.send).toHaveBeenCalledWith(1);
    expect(mocks.preview).not.toHaveBeenCalled();
    expect((await axe.run(view.container)).violations).toEqual([]);
  });

  it("shows a new chat message while its thread is still being prepared", () => {
    const userItem = { id: "local:queued", kind: "user_message" as const, status: "completed" as const, text: "My queued question", activity: null, citations: [], error: null, createdAt: "now" };
    mocks.state = readyState({ activeThreadId: null, pending: { clientTurnId: "queued", threadId: null, userItem, connectionId: "codex_default", phase: "preparing", startedAt: Date.now(), turnId: null, prompt: userItem.text } });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={() => {}} />);
    expect(screen.getByText("My queued question")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Preparing context…");
    expect(screen.getByRole("button", { name: "Preparing message" })).toBeDisabled();
  });

  it("strips bounded context envelopes from visible user messages", () => {
    expect(visibleUserPrompt("Question\n\n<repository_chat_context_v1>\nsecret")).toBe("Question");
  });
  it("closes the context menu, focuses the searchable dialog and returns focus on Escape", async () => {
    vi.mocked(companionSearchMentions).mockResolvedValue([{ kind: "finding", id: "fp", label: "CVE-QA", detail: "Local finding", available: true, unavailableReason: null }]);
    const view = render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add context" }));
    await user.click(screen.getByRole("button", { name: /@finding/ }));
    expect(await screen.findByRole("dialog", { name: "Add finding context" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /@finding/ })).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Search finding" })).toHaveFocus());
    expect(screen.getByRole("button", { name: /CVE-QA.*Local finding/ })).toBeEnabled();
    expect((await axe.run(view.baseElement, { rules: { "color-contrast": { enabled: false } } })).violations).toEqual([]);
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.getByRole("button", { name: "Add context" })).toHaveFocus());
  });

  it("does not reopen a dismissed evidence source when its request finishes", async () => {
    let reject!: (cause: Error) => void;
    vi.mocked(companionSourceDetail).mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
    mocks.state = readyState({ itemsByThread: { 1: [{ id: "answer", kind: "assistant_message", status: "completed", text: "Recorded evidence", activity: null, citations: [{ sourceId: "source", kind: "scan", label: "Scan evidence", stale: false }], error: null, createdAt: "now" }] } });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Scan evidence" }));
    expect(screen.getByRole("dialog", { name: "Evidence source" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await act(async () => reject(new Error("Late unavailable source")));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Late unavailable source")).not.toBeInTheDocument();
  });

  it("keeps the source list open and restores focus to the inspected source", async () => {
    const source = {
      id: "scan:10", kind: "scan" as const, label: "Scan #10", scanId: 10, revision: "abc",
      repositoryRelativePath: null, lineStart: null, lineEnd: null,
      findingFingerprint: null, packageIdentity: null, stale: false,
    };
    mocks.state = readyState({ context: {
      schemaVersion: 1, repository: { id: 1, name: "demo", sourceMode: "local_source",
        sourceType: "local", approvedRoot: "/tmp/demo", branch: "main", revision: "abc" },
      scan: completedScan, sources: [source], attachments: [], omissions: [], untrustedEvidence: [],
      instructions: [], freshness: { state: "matched", message: "Matches the completed scan.",
        scanRevision: "abc", sourceRevision: "abc", sourceDirty: false }, bounded: true, estimatedBytes: 400,
    } });
    vi.mocked(companionSourceDetail).mockResolvedValueOnce({ source, content: null, unavailableReason: "Fixture snippet unavailable" });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    const user = userEvent.setup();
    const inspect = screen.getByRole("button", { name: "Inspect context" });
    await user.click(inspect);
    const context = await screen.findByRole("dialog", { name: "Active repository context" });
    const inspectSource = await within(context).findByRole("button", { name: "Inspect source: Scan #10" });
    await user.click(inspectSource);
    expect(await screen.findByRole("dialog", { name: "Evidence source" })).toBeVisible();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(inspectSource).toHaveFocus());
    expect(context).toBeVisible();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(inspect).toHaveFocus());
    expect(mocks.preview).toHaveBeenCalledTimes(1);
  });

  const findingRequest: FindingChatRequest = {
    repositoryId: 1, fingerprint: "fp", advisoryId: "CVE-2026-1", packageName: "router",
    installedVersion: "1.0", targetVersion: "1.1", scanRunId: 10, mode: "analysis",
  };

  it("replaces the focused finding without removing manual context or draft", async () => {
    mocks.state = readyState({ attachments: [{ kind: "file", id: "app.ts" }], draft: "Keep this draft" });
    const onClearFinding = vi.fn();
    const props = { scanStatus, repository, onClearFinding, onError: vi.fn() };
    const view = render(<RepositoryChat {...props} findingRequest={findingRequest} />);
    expect(mocks.setFocusedFinding).toHaveBeenLastCalledWith(1, "scan:10:fp");
    expect(screen.getByRole("button", { name: /Remove CVE-2026-1.*Scan #10/ })).toBeInTheDocument();
    view.rerender(<RepositoryChat {...props} findingRequest={{ ...findingRequest, fingerprint: "fp-2", advisoryId: "CVE-2026-2", scanRunId: 20 }} />);
    expect(mocks.setFocusedFinding).toHaveBeenLastCalledWith(1, "scan:20:fp-2");
    expect(screen.queryByRole("button", { name: /Remove CVE-2026-1/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove app.ts" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message the security companion" })).toHaveValue("Keep this draft");
    await userEvent.click(screen.getByRole("button", { name: /Remove CVE-2026-2/ }));
    expect(onClearFinding).toHaveBeenCalledOnce();
    expect(mocks.setFocusedFinding).toHaveBeenLastCalledWith(1, null);
    expect(mocks.setAttachments).not.toHaveBeenCalled();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("opens a new draft for a Review handoff instead of reusing an archived conversation", () => {
    mocks.state = readyState({ threads: [{ ...thread, archivedAt: "now" }] });
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} findingRequest={findingRequest} onError={vi.fn()} />);
    expect(mocks.newConversation).toHaveBeenCalledExactlyOnceWith(1);
    expect(mocks.setFocusedFinding).toHaveBeenCalledExactlyOnceWith(1, "scan:10:fp");
  });

  it("opens Fix directly for an actionable finding", async () => {
    mocks.state = readyState();
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} findingRequest={{ ...findingRequest, mode: "fix" }} onError={vi.fn()} />);
    expect(await screen.findByText("Fix workspace test surface")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Fix" })).toHaveAttribute("aria-selected", "true");
    expect(mocks.setFocusedFinding).not.toHaveBeenCalled();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("reports archive state truthfully and contains a failed rename", async () => {
    mocks.state = readyState({ threads: [{ ...thread, archivedAt: "now" }], draft: "Kept draft" });
    mocks.renameThread.mockRejectedValueOnce(new Error("Local rename failed"));
    render(<RepositoryChat scanStatus={scanStatus} repository={repository} onError={vi.fn()} />);
    expect(screen.getByText("Archived")).toBeInTheDocument();
    expect(screen.getByText(/This conversation is archived/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Conversation actions" }));
    const input = screen.getByRole("textbox", { name: "Conversation title" });
    fireEvent.change(input, { target: { value: "New title" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByRole("alert")).toHaveTextContent("Local rename failed");
    expect(screen.getByRole("button", { name: "Rename" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Unarchive" })).toBeEnabled();
  });

});
