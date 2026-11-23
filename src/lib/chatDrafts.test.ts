import { describe, expect, it, vi } from "vitest";
import { ChatDraftStore, type ChatDraftClient } from "./chatDrafts";

function memoryClient(): ChatDraftClient {
  const rows = new Map<string, string>();
  return {
    readDraft: async (repositoryId, scope) => rows.get(JSON.stringify([repositoryId, scope])) ?? null,
    writeDraft: async (repositoryId, scope, text) => { rows.set(JSON.stringify([repositoryId, scope]), text); },
  };
}

describe("local chat drafts", () => {
  it.each([
    ["emoji split by the limit", "A".repeat(65_535) + "👋", "A".repeat(65_535)],
    ["emoji fitting exactly", "A".repeat(65_534) + "👋", "A".repeat(65_534) + "👋"],
    ["ASCII fitting exactly", "A".repeat(65_536), "A".repeat(65_536)],
    ["unpaired units", "before\ud83d middle\udc4b after", "before\ufffd middle\ufffd after"],
  ])("persists and restores well-formed text with %s", async (_label, input, expected) => {
    const client = memoryClient();
    const store = new ChatDraftStore(client);
    await store.save(1, "analysis", input);
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ text: expected, error: null });
    expect(await new ChatDraftStore(client).load(1, "analysis")).toBe(expected);
  });

  it("restores exact text after a new UI session and separates repositories and workspaces", async () => {
    const client = memoryClient();
    const first = new ChatDraftStore(client);
    const text = "  Not sent\n\nGrüße 👋\n";
    await first.save(1, "analysis", text);
    await first.save(2, "analysis", "Other repository");
    await first.save(1, "fix:a", "Workspace A");
    await first.save(1, "fix:b", "Workspace B");
    const reopened = new ChatDraftStore(client);
    expect(await reopened.load(1, "analysis")).toBe(text);
    expect(await reopened.load(2, "analysis")).toBe("Other repository");
    expect(await reopened.load(1, "fix:a")).toBe("Workspace A");
    expect(await reopened.load(1, "fix:b")).toBe("Workspace B");
    await reopened.save(1, "fix:a", "");
    expect(await new ChatDraftStore(client).load(1, "fix:a")).toBe("");
  });

  it("never replaces a newer edit with a delayed restore", async () => {
    let resolve!: (text: string) => void;
    const store = new ChatDraftStore({ ...memoryClient(), readDraft: () => new Promise((done) => { resolve = done; }) });
    const loading = store.load(1, "analysis");
    await store.save(1, "analysis", "Newer text");
    resolve("Old saved text");
    expect(await loading).toBe("Newer text");
    expect(store.getSnapshot(1, "analysis").text).toBe("Newer text");
  });

  it("serializes writes and saves a later clear instead of resurrecting the old prompt", async () => {
    let resolve!: () => void;
    const writeDraft = vi.fn().mockImplementationOnce(() => new Promise<void>((done) => { resolve = done; })).mockResolvedValue(undefined);
    const store = new ChatDraftStore({ readDraft: async () => null, writeDraft });
    const pending = store.save(1, "analysis", "First prompt");
    void store.save(1, "analysis", "Later edit");
    void store.save(1, "analysis", "");
    expect(writeDraft).toHaveBeenCalledTimes(1);
    resolve();
    await pending;
    expect(writeDraft.mock.calls.map((call) => call[2])).toEqual(["First prompt", ""]);
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ text: "", loaded: true, saving: false, error: null });
  });

  describe.each(["analysis", '["fix","workspace-1",null]'])("writer completion in %s", (scope) => {
    it("persists fully awaited saves", async () => {
      const client = memoryClient();
      const store = new ChatDraftStore(client);
      await store.save(1, scope, "First prompt");
      await store.save(1, scope, "Later edit");
      expect(await new ChatDraftStore(client).load(1, scope)).toBe("Later edit");
    });

    it.each([["a new edit", "Later edit"], ["an accepted-task clear", ""]])("persists %s queued after a write completes", async (_label, latest) => {
      const client = memoryClient();
      const writeDraft = vi.fn(client.writeDraft);
      const store = new ChatDraftStore({ ...client, writeDraft });
      const first = store.save(1, scope, "First prompt");
      let next: Promise<void> | undefined;
      queueMicrotask(() => { next = store.save(1, scope, latest); });
      await first;
      await next;
      expect(writeDraft.mock.calls.map((call) => call[2])).toEqual(["First prompt", latest]);
      expect(store.getSnapshot(1, scope)).toMatchObject({ text: latest, saving: false, error: null });
      expect(await new ChatDraftStore(client).load(1, scope)).toBe(latest);
    });
  });

  it("keeps callers and saving state pending until a late write completes", async () => {
    const client = memoryClient();
    let finish!: () => void;
    const gate = new Promise<void>((resolve) => { finish = resolve; });
    const writeDraft = vi.fn(client.writeDraft)
      .mockImplementationOnce(client.writeDraft)
      .mockImplementationOnce(async (...args) => { await gate; await client.writeDraft(...args); });
    const store = new ChatDraftStore({ ...client, writeDraft });
    let firstFinished = false;
    const first = store.save(1, "analysis", "First prompt").then(() => { firstFinished = true; });
    let next: Promise<void> | undefined;
    queueMicrotask(() => { next = store.save(1, "analysis", "Later edit"); });
    try {
      await vi.waitFor(() => expect(writeDraft).toHaveBeenCalledTimes(2), { timeout: 200, interval: 5 });
      expect(firstFinished).toBe(false);
      expect(store.getSnapshot(1, "analysis").saving).toBe(true);
    } finally { finish(); }
    await first;
    await next;
    expect(await new ChatDraftStore(client).load(1, "analysis")).toBe("Later edit");
  });

  it("requires explicit retry when a late write fails", async () => {
    const client = memoryClient();
    const writeDraft = vi.fn(client.writeDraft)
      .mockImplementationOnce(client.writeDraft)
      .mockRejectedValueOnce(new Error("disk unavailable"));
    const store = new ChatDraftStore({ ...client, writeDraft });
    const first = store.save(1, "analysis", "First prompt");
    let next: Promise<void> | undefined;
    queueMicrotask(() => { next = store.save(1, "analysis", "Later edit"); });
    await first;
    await next;
    expect(writeDraft.mock.calls.map((call) => call[2])).toEqual(["First prompt", "Later edit"]);
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ text: "Later edit", saving: false, error: "save" });
    expect(await new ChatDraftStore(client).load(1, "analysis")).toBe("First prompt");
    await store.retry(1, "analysis");
    expect(writeDraft.mock.calls.map((call) => call[2])).toEqual(["First prompt", "Later edit", "Later edit"]);
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ saving: false, error: null });
    expect(await new ChatDraftStore(client).load(1, "analysis")).toBe("Later edit");
  });

  it("registers the writer before notifying saving-state subscribers", async () => {
    const client = memoryClient();
    let finish!: () => void;
    const gate = new Promise<void>((resolve) => { finish = resolve; });
    const writeDraft = vi.fn(client.writeDraft)
      .mockImplementationOnce(async (...args) => { await gate; await client.writeDraft(...args); });
    const store = new ChatDraftStore({ ...client, writeDraft });
    let edited = false;
    let next: Promise<void> | undefined;
    store.subscribe(1, "analysis", () => {
      if (!edited && store.getSnapshot(1, "analysis").saving) {
        edited = true;
        next = store.save(1, "analysis", "Later edit");
      }
    });
    const first = store.save(1, "analysis", "First prompt");
    try { expect(writeDraft).toHaveBeenCalledTimes(1); }
    finally { finish(); }
    await first;
    await next;
    expect(writeDraft.mock.calls.map((call) => call[2])).toEqual(["First prompt", "Later edit"]);
    expect(await new ChatDraftStore(client).load(1, "analysis")).toBe("Later edit");
  });

  it("keeps a failed save editable and retries the latest text", async () => {
    const writeDraft = vi.fn().mockRejectedValueOnce(new Error("disk unavailable")).mockResolvedValue(undefined);
    const store = new ChatDraftStore({ readDraft: async () => null, writeDraft });
    await store.save(1, "analysis", "Keep me");
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ text: "Keep me", loaded: true, error: "save" });
    await store.retry(1, "analysis");
    expect(writeDraft).toHaveBeenLastCalledWith(1, "analysis", "Keep me");
    expect(store.getSnapshot(1, "analysis").error).toBeNull();
  });

  it("does not turn a failed restore into an empty write and allows a retry", async () => {
    const writeDraft = vi.fn();
    const readDraft = vi.fn().mockRejectedValueOnce(new Error("read unavailable")).mockResolvedValue("Original draft");
    const store = new ChatDraftStore({ readDraft, writeDraft });
    await expect(store.load(1, "analysis")).rejects.toThrow("read unavailable");
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ loaded: false, error: "restore" });
    expect(writeDraft).not.toHaveBeenCalled();
    await store.retry(1, "analysis");
    expect(store.getSnapshot(1, "analysis")).toMatchObject({ text: "Original draft", loaded: true, error: null });
  });
});
