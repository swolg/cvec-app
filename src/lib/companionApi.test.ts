import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

import {
  companionCachedThread,
  companionConversationIndex,
  companionPreviewContext,
  companionRuntimeStatus,
  companionStartTurn,
  listenCompanionEvents,
} from "./companionApi";
import type { RepositoryChatContext } from "./types";

const context: RepositoryChatContext = {
  schemaVersion: 1,
  repository: { id: 1, name: "demo", sourceType: "local", sourceMode: "local_source", approvedRoot: null, branch: null, revision: null },
  scan: { id: 10, status: "completed", scannedAt: "now", revision: null, evidenceComplete: true },
  freshness: { state: "unknown", sourceRevision: null, scanRevision: null, sourceDirty: null, message: "Unknown" },
  attachments: [],
  sources: [],
  omissions: [],
  untrustedEvidence: [],
  instructions: ["Read-only"],
  estimatedBytes: 512,
  bounded: true,
};

const thread = {
  id: 1,
  repositoryId: 1,
  provider: "codex",
  providerThreadId: "provider-1",
  title: "Security review",
  status: "ready",
  createdAt: "2026-08-21T00:00:00Z",
  updatedAt: "2026-08-21T00:00:00Z",
  lastOpenedAt: "2026-08-21T00:00:00Z",
  archivedAt: null,
  contextVersion: 1,
  lastModelId: "gpt-test",
  providerConnectionId: "codex_default",
} as const;

describe("companionApi", () => {
  beforeEach(() => vi.clearAllMocks());

  it("uses only the typed companion IPC surface", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "companion_runtime_status") return { schemaVersion: 1, phase: "disabled", codexPath: "/codex", codexVersion: "1", message: null, startedAt: null, activeTurns: 0 };
      if (command === "companion_conversation_index") return { schemaVersion: 1, threads: [thread] };
      if (command === "companion_cached_thread") return { schemaVersion: 1, thread, segments: [{ id: 1, threadId: 1, ordinal: 0, provider: "codex", connectionId: "codex_default", providerThreadId: "provider-1", initialModelId: "gpt-test", lastModelId: "gpt-test", status: "ready", createdAt: "2026-08-21T00:00:00Z", updatedAt: "2026-08-21T00:00:00Z", endedAt: null }], items: [], providerHistoryAvailable: true };
      if (command === "companion_preview_context") return context;
      if (command === "companion_start_turn") return { turnId: "turn-1", providerThreadId: "thread-1", acceptedClientTurnId: "client-1" };
      throw new Error(`unexpected ${command}`);
    });
    expect((await companionRuntimeStatus()).phase).toBe("disabled");
    expect((await companionConversationIndex()).threads[0]?.title).toBe("Security review");
    expect((await companionCachedThread(1, 1)).segments[0]?.connectionId).toBe("codex_default");
    await companionPreviewContext({ repositoryId: 1, query: "question", attachments: [] });
    await companionStartTurn({ repositoryId: 1, threadId: 1, connectionId: "codex_default", prompt: "question", model: null, effort: null, context, clientTurnId: "client-1" });
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual([
      "companion_runtime_status",
      "companion_conversation_index",
      "companion_cached_thread",
      "companion_preview_context",
      "companion_start_turn",
    ]);
    expect(JSON.stringify(mocks.invoke.mock.calls)).not.toContain("jsonrpc");
  });

  it("subscribes to one normalized event channel and rejects malformed envelopes", async () => {
    let handler: ((event: { payload: unknown }) => void) | null = null;
    const unlisten = vi.fn();
    mocks.listen.mockImplementation(async (_name, next) => { handler = next; return unlisten; });
    const received = vi.fn();
    const dispose = await listenCompanionEvents(received);
    handler!({ payload: { schemaVersion: 1, repositoryId: 1, threadId: 2, providerThreadId: "provider", connectionId: "codex_default", driver: "codex", modelId: "gpt-test", turnId: "turn", sequence: 1, kind: "item_delta", payload: { itemId: "assistant", text: "hello" } } });
    expect(received).toHaveBeenCalledTimes(1);
    expect(() => handler!({ payload: { jsonrpc: "2.0", method: "raw" } })).toThrow(/Malformed companion event/);
    dispose();
    expect(unlisten).toHaveBeenCalled();
  });
});
