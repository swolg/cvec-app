import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { fixSnapshot } from "@/test/fixWorkspaceFixture";
import { createFixWorkspace, listArchivedFixWorkspaces, parseFixSnapshot, parseFixWorkspace, setFixWorkspaceArchived, startFixWorkspace, stopFixWorkspace } from "./fixWorkspaceApi";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

describe("fix workspace contract and scope", () => {
  beforeEach(() => vi.clearAllMocks());

  it("requires versioned, bounded evidence and rejects unknown or malformed fields", () => {
    const snapshot = fixSnapshot();
    expect(parseFixSnapshot(snapshot)).toEqual(snapshot);
    expect(() => parseFixWorkspace({ ...snapshot.workspace, schemaVersion: 4 })).toThrow();
    expect(() => parseFixWorkspace({ ...snapshot.workspace, status: "published" })).toThrow();
    expect(() => parseFixSnapshot({ ...snapshot, diff: { ...snapshot.diff, digest: "unknown" } })).toThrow();
    expect(() => parseFixSnapshot({ ...snapshot, diff: { ...snapshot.diff, files: [{ path: "x", patch: "x".repeat(256 * 1024 + 1), truncated: false }] } })).toThrow();
    expect(() => parseFixWorkspace({ ...snapshot.workspace, token: "private" })).toThrow();
  });

  it("validates lifecycle versions and bounds archive pages with exact IPC scope", async () => {
    const snapshot = fixSnapshot();
    const archived = { ...snapshot, workspace: { ...snapshot.workspace, schemaVersion: 3, archivedAt: "2026-11-10T00:00:00Z" } };
    expect(parseFixSnapshot(archived).workspace.archivedAt).toBeTruthy();
    expect(() => parseFixWorkspace({ ...archived.workspace, schemaVersion: 2 })).toThrow();
    expect(() => parseFixWorkspace({ ...archived.workspace, status: "working" })).toThrow();
    vi.mocked(invoke).mockResolvedValueOnce(archived);
    await setFixWorkspaceArchived(1, snapshot.workspace.id, true);
    expect(invoke).toHaveBeenLastCalledWith("fix_workspace_set_archived", { repositoryId: 1, workspaceId: snapshot.workspace.id, archived: true });
    vi.mocked(invoke).mockResolvedValueOnce({ entries: [], nextCursor: null });
    await listArchivedFixWorkspaces(1, "cursor");
    expect(invoke).toHaveBeenLastCalledWith("fix_workspace_archived", { repositoryId: 1, cursor: "cursor" });
    const entry = { id: snapshot.workspace.id, repositoryId: 1, branch: snapshot.workspace.branch, title: "Saved task", archivedAt: "now" };
    vi.mocked(invoke).mockResolvedValueOnce({ entries: Array(21).fill(entry), nextCursor: null });
    await expect(listArchivedFixWorkspaces(1)).rejects.toThrow();
  });

  it("represents a missing worktree with saved evidence and no invented diff", () => {
    const snapshot = fixSnapshot();
    snapshot.diff = null;
    snapshot.sourceError = "Worktree missing";
    expect(parseFixSnapshot(snapshot).diff).toBeNull();
  });

  it("passes explicit creation confirmation and exact repository/workspace scope", async () => {
    const snapshot = fixSnapshot();
    vi.mocked(invoke).mockResolvedValueOnce(snapshot).mockResolvedValue(undefined);
    await createFixWorkspace(1, "finding-a");
    expect(invoke).toHaveBeenCalledWith("fix_workspace_create", { repositoryId: 1, findingFingerprint: "finding-a", confirmed: true });
    await startFixWorkspace(1, snapshot.workspace.id, { prompt: "Fix package", model: "model-a", effort: null, connectionId: "grok_default", connectionUpdatedAt: "now" });
    expect(invoke).toHaveBeenCalledWith("fix_workspace_start", { repositoryId: 1, workspaceId: snapshot.workspace.id, request: { connectionId: "grok_default", connectionUpdatedAt: "now", prompt: "Fix package", model: "model-a", effort: null, consent: true } });
    await stopFixWorkspace(1, snapshot.workspace.id);
    expect(invoke).toHaveBeenCalledWith("fix_workspace_stop", { repositoryId: 1, workspaceId: snapshot.workspace.id });
  });
});
