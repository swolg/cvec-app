import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CODEX_CONNECTION, fixConnection } from "@/test/fixProviderFixture";
import { EMPTY_REPOSITORY } from "@/test/fixtures";
import { fixSnapshot } from "@/test/fixWorkspaceFixture";
import type { FixSnapshot, FixWorkspace } from "@/lib/types";

// Observe retained records, rather than only the rendered active-workspace list.
const retainedRefs = vi.hoisted(() => new Set<{ current: unknown }>());
vi.mock("react", async (original) => {
  const actual = await original<typeof import("react")>();
  return { ...actual, useRef: (initial: unknown) => {
    const ref = actual.useRef(initial);
    if (initial instanceof Map) retainedRefs.add(ref);
    return ref;
  } };
});
const mocks = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn(), create: vi.fn(), start: vi.fn(), stop: vi.fn(), check: vi.fn(), rescan: vi.fn(), listen: vi.fn(), archived: vi.fn(), setArchived: vi.fn() }));
vi.mock("@/lib/companionApi", () => ({ companionClient: { readDraft: async () => null, writeDraft: async () => undefined } }));
vi.mock("@/lib/fixWorkspaceApi", () => ({
  listFixWorkspaces: mocks.list, getFixWorkspace: mocks.get, createFixWorkspace: mocks.create, startFixWorkspace: mocks.start, stopFixWorkspace: mocks.stop,
  checkFixWorkspace: mocks.check, rescanFixWorkspace: mocks.rescan, listenFixWorkspaces: mocks.listen, listArchivedFixWorkspaces: mocks.archived, setFixWorkspaceArchived: mocks.setArchived,
  fixWorkspaceBusy: (workspace: FixWorkspace | null) => workspace && ["working", "testing", "scanning"].includes(workspace.status),
}));
import { FixWorkspacePanel } from "./FixWorkspacePanel";
import { clearFixWorkspaceSession, fixDraftKey, saveFixDraft } from "./fixWorkspaceSession";

const props = { repository: EMPTY_REPOSITORY, model: "fixture-model", modelLabel: "Fixture model", effort: "high", providerReady: true, connectionId: "codex_default", connections: [CODEX_CONNECTION] };
let snapshot: FixSnapshot;
let listener: (workspace: FixWorkspace) => void;

beforeEach(() => {
  vi.resetAllMocks();
  retainedRefs.clear();
  clearFixWorkspaceSession();
  snapshot = fixSnapshot();
  mocks.list.mockResolvedValue([snapshot.workspace]);
  mocks.get.mockImplementation(async () => snapshot);
  mocks.create.mockResolvedValue(snapshot);
  mocks.start.mockResolvedValue(undefined);
  mocks.stop.mockResolvedValue(undefined);
  mocks.check.mockResolvedValue(snapshot);
  mocks.rescan.mockResolvedValue(snapshot);
  mocks.archived.mockResolvedValue({ entries: [], nextCursor: null });
  mocks.setArchived.mockImplementation(async (_repo, _id, archived) => ({ ...snapshot, workspace: { ...snapshot.workspace, schemaVersion: 3, revision: snapshot.workspace.revision + 1, archivedAt: archived ? "2026-11-10T00:00:00Z" : null }, actions: archived ? [] : snapshot.actions }));
  mocks.listen.mockImplementation(async (callback) => { listener = callback; return vi.fn(); });
});

describe("Provider fix workspace", () => {

  it("does not resurrect evicted archived workspaces from late older events", async () => {
    const original = fixSnapshot();
    const next = { ...fixSnapshot(), workspace: { ...fixSnapshot().workspace, id: "b".repeat(24) } };
    mocks.create.mockResolvedValue(next);
    mocks.get.mockImplementation(async (_repo, id: string) => id === next.workspace.id ? next : original);
    render(<FixWorkspacePanel {...props} />);
    await screen.findByRole("textbox", { name: "Fix task" });
    await userEvent.click(screen.getByRole("button", { name: "Archive workspace" }));
    await userEvent.click(screen.getByRole("button", { name: "Archive and keep files" }));
    await screen.findByRole("button", { name: "Restore workspace" });
    await userEvent.click(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ }));
    await userEvent.click(screen.getByRole("button", { name: "Create branch and worktree" }));
    await screen.findByRole("button", { name: "Archive workspace" });
    await act(async () => listener(original.workspace));
    expect(screen.getByText("1/20 active workspaces")).toBeInTheDocument();
    for (const ref of retainedRefs) {
      if (ref.current instanceof Map) expect(ref.current.has(original.workspace.id)).toBe(false);
    }
  });

  it("bounds full records through repeated archive/create and reloads archived drafts and evidence", async () => {
    let current = fixSnapshot();
    const saved = new Map<string, FixSnapshot>();
    const initialId = current.workspace.id;
    current.workspace.messages = [{ role: "assistant", text: "Durable archived explanation" }];
    mocks.list.mockResolvedValue([current.workspace]);
    mocks.get.mockImplementation(async (_repo, id: string) => saved.get(id) ?? current);
    mocks.setArchived.mockImplementation(async (_repo, _id, archived) => {
      current = { ...current, workspace: { ...current.workspace, schemaVersion: 3, revision: current.workspace.revision + 1, archivedAt: archived ? "2026-11-10T00:00:00Z" : null } };
      saved.set(current.workspace.id, current);
      return current;
    });
    mocks.create.mockImplementation(async () => {
      current = { ...fixSnapshot(), workspace: { ...fixSnapshot().workspace, id: (saved.size + 10).toString(16).padStart(24, "0") } };
      return current;
    });
    render(<FixWorkspacePanel {...props} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Retained original draft" } });
    // Exercise the full eviction loop without repeated pointer simulation delays.
    for (let step = 0; step < 24; step++) {
      fireEvent.click(screen.getByRole("button", { name: "Archive workspace" }));
      fireEvent.click(screen.getByRole("button", { name: "Archive and keep files" }));
      await screen.findByRole("button", { name: "Restore workspace" });
      fireEvent.click(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ }));
      fireEvent.click(screen.getByRole("button", { name: "Create branch and worktree" }));
      await screen.findByRole("button", { name: "Archive workspace" });
      expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("");
    }
    const recordMaps = [...retainedRefs].map((ref) => ref.current).filter((value): value is Map<string, FixWorkspace> => value instanceof Map && [...value.values()].some((item) => item?.worktreePath));
    expect(recordMaps.length).toBeGreaterThan(0);
    for (const records of recordMaps) expect([...records.values()].filter((value) => value.archivedAt)).toHaveLength(0);
    await act(async () => {
      for (const value of saved.values()) listener({ ...value.workspace, archivedAt: null, revision: value.workspace.revision - 1 });
    });
    for (const records of recordMaps) expect(records.size).toBe(1);
    expect(screen.getByText("1/20 active workspaces")).toBeInTheDocument();
    // Eviction is independent of the durable archive and its unsent draft.
    mocks.archived.mockResolvedValue({ entries: [{ id: initialId, repositoryId: 1, branch: saved.get(initialId)!.workspace.branch, title: "Original archived workspace", archivedAt: "2026-11-10T00:00:00Z" }], nextCursor: null });
    await userEvent.click(screen.getByRole("button", { name: "Browse archived fixes" }));
    await userEvent.click(await screen.findByRole("button", { name: /Original archived workspace/ }));
    await screen.findByText("Durable archived explanation");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Retained original draft");
    for (const records of recordMaps) expect([...records.values()].filter((value) => value.archivedAt)).toHaveLength(1);
    expect(mocks.start).not.toHaveBeenCalled();
  }, 15_000);

  it("archives without losing the draft or evidence and restores before accepting new work", async () => {
    snapshot.workspace.messages = [{ role: "assistant", text: "Retained fix explanation" }];
    render(<FixWorkspacePanel {...props} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    fireEvent.change(input, { target: { value: "Unsent local draft" } });
    await userEvent.click(screen.getByRole("button", { name: "Archive workspace" }));
    expect(mocks.setArchived).not.toHaveBeenCalled();
    expect(screen.getByText(/Archiving does not delete files/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Archive and keep files" }));
    await screen.findByText(/Archived · saved evidence is read-only/);
    expect(mocks.setArchived).toHaveBeenCalledWith(1, snapshot.workspace.id, true);
    expect(input).toHaveValue("Unsent local draft");
    expect(input).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(screen.getByText("Retained fix explanation")).toBeInTheDocument();
    expect(screen.getByText("0/20 active workspaces")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: /^Rescan/ }));
    expect(screen.getByRole("button", { name: "Rescan worktree" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Restore workspace" }));
    await screen.findByRole("button", { name: "Archive workspace" });
    expect(mocks.setArchived).toHaveBeenLastCalledWith(1, snapshot.workspace.id, false);
    await userEvent.click(screen.getByRole("tab", { name: "Conversation" }));
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Unsent local draft");
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeEnabled();
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("keeps the active workspace and draft when archiving cannot be saved", async () => {
    mocks.setArchived.mockRejectedValueOnce(new Error("Disk full: archive not saved"));
    render(<FixWorkspacePanel {...props} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    fireEvent.change(input, { target: { value: "Retain after failure" } });
    await userEvent.click(screen.getByRole("button", { name: "Archive workspace" }));
    await userEvent.click(screen.getByRole("button", { name: "Archive and keep files" }));
    await screen.findByText("Disk full: archive not saved");
    expect(screen.getByRole("button", { name: "Archive workspace" })).toBeEnabled();
    expect(input).toHaveValue("Retain after failure");
    expect(screen.getByText("1/20 active workspaces")).toBeInTheDocument();
  });

  it("keeps non-submodule source editable while explaining unavailable verification", async () => {
    snapshot.verificationWarning = "Submodules cannot yet be verified safely.";
    snapshot.actions = [];
    render(<FixWorkspacePanel {...props} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Fix root source" } });
    expect(screen.getByText("Verification unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeEnabled();
    await userEvent.click(screen.getByRole("tab", { name: /^Rescan/ }));
    expect(screen.getByRole("button", { name: "Rescan worktree" })).toBeDisabled();
    expect(mocks.rescan).not.toHaveBeenCalled();
  });

  it("requires a model from the current connection before source sharing", async () => {
    render(<FixWorkspacePanel {...props} model={null} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Task" } });
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(screen.getByText("Choose an available model before starting a fix.")).toBeInTheDocument();
    expect(mocks.start).not.toHaveBeenCalled();
  });
  it("creates the branch/worktree only after the concrete creation disclosure", async () => {
    mocks.list.mockResolvedValue([]);
    await saveFixDraft(fixDraftKey(1, null, "finding-a"), "Draft for this finding's new workspace");
    render(<FixWorkspacePanel {...props} findingFingerprint="finding-a" />);
    await screen.findByText("Fix in an isolated checkout");
    await userEvent.click(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ }));
    expect(mocks.create).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Create branch and worktree" }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith(1, "finding-a", undefined));
    expect(mocks.start).not.toHaveBeenCalled();
    await screen.findByText("Worktree ready");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Draft for this finding's new workspace");
  });

  it("keeps the draft when sharing is cancelled and sends only after explicit authorization", async () => {
    render(<FixWorkspacePanel {...props} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    fireEvent.change(input, { target: { value: "Update the vulnerable dependency" } });
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    expect(mocks.start).not.toHaveBeenCalled();
    expect(screen.getByText(/additional source selected by the agent/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel and keep draft" }));
    expect(input).toHaveValue("Update the vulnerable dependency");
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    await userEvent.click(screen.getByRole("button", { name: "Allow and start fix" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledWith(1, snapshot.workspace.id, { prompt: "Update the vulnerable dependency", model: "fixture-model", effort: "high", connectionId: "codex_default", connectionUpdatedAt: "now" }));
  });

  it("shows actual test failure/output, stale evidence, diff and isolated rescan", async () => {
    snapshot.diff!.files = [{ path: "package.json", patch: "-old\n+new", truncated: false }];
    snapshot.workspace.checks = [{ action: "npm:test:run", status: "failed", exitCode: 1, output: "Assertion failed", digest: "d".repeat(64), stale: false, finishedAt: "now" }];
    snapshot.workspace.rescan = { findingCount: 1, findings: [{ fingerprint: "fp", advisoryId: "CVE-fixture", packageName: "demo", version: "2", packageType: "npm", packageUrl: null, fixVersion: "3", severity: "HIGH" }], notDetected: 2, digest: "c".repeat(64), stale: false, scannedAt: "now", scannerVersions: "Fixture scanners" };
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText("package.json");
    expect(screen.getByText("Worktree ready")).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Fix workspace" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "Checks: 1 failed · stale" }));
    expect(screen.getByRole("tab", { name: /^Checks/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText(/failed \(exit 1\) · stale/)).toBeInTheDocument();
    expect(screen.getByText("Assertion failed")).toBeInTheDocument();
    expect(screen.getByText("Assertion failed").closest("details")).toHaveAttribute("open");
    expect(screen.getByText(/1 finding detected · 2 baseline findings not detected/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: /^Checks/ }));
    await userEvent.click(screen.getByRole("button", { name: "Run check" }));
    await waitFor(() => expect(mocks.check).toHaveBeenCalledWith(1, snapshot.workspace.id, "npm:test:run"));
    await userEvent.click(screen.getByRole("tab", { name: /^Rescan/ }));
    await userEvent.click(screen.getByRole("button", { name: "Rescan worktree" }));
    await waitFor(() => expect(mocks.rescan).toHaveBeenCalledWith(1, snapshot.workspace.id));
  });

  it("exposes Stop during work and ignores updates belonging to other repositories", async () => {
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText("Worktree ready");
    act(() => listener({ ...snapshot.workspace, repositoryId: 2, activity: "Unrelated" }));
    expect(screen.queryByText("Unrelated")).not.toBeInTheDocument();
    act(() => listener({ ...snapshot.workspace, status: "working", activity: "Updating files" }));
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.stop).toHaveBeenCalledWith(1, snapshot.workspace.id);
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
  });

  it("waits for cancellation to settle before enabling new tasks or local checks", async () => {
    mocks.list.mockResolvedValue([snapshot.workspace, { ...snapshot.workspace, id: "eeeeeeee-other", createdAt: "2025-01-01T00:00:00Z" }]);
    let resolveStop!: () => void;
    mocks.stop.mockImplementationOnce(() => new Promise<void>((resolve) => { resolveStop = resolve; }));
    render(<FixWorkspacePanel {...props} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Next focused task" } });
    act(() => listener({ ...snapshot.workspace, revision: 1, status: "working" }));
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    snapshot.workspace = { ...snapshot.workspace, revision: 2, status: "interrupted", activity: "Local actions stopped." };
    act(() => listener(snapshot.workspace));
    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Fix workspace" })).toBeDisabled();
    await userEvent.click(screen.getByRole("tab", { name: /^Checks/ }));
    expect(screen.getByRole("button", { name: "Run check" })).toBeDisabled();
    await act(async () => resolveStop());
    await waitFor(() => expect(screen.getByRole("button", { name: "Run check" })).toBeEnabled());
    expect(mocks.start).not.toHaveBeenCalled();
    expect(mocks.check).not.toHaveBeenCalled();
  });

  it("distinguishes unavailable source and evidence freshness from a verified empty diff", async () => {
    const verifiedDiff = snapshot.diff!;
    snapshot.diff = null;
    snapshot.sourceError = "Git could not inspect the worktree";
    snapshot.actions = [];
    snapshot.workspace.messages = [{ role: "assistant", text: "Saved fix explanation" }];
    snapshot.workspace.checks = [{ action: "npm:test:run", status: "passed", exitCode: 0, output: "Passed", digest: verifiedDiff.digest, stale: false, finishedAt: "now" }];
    snapshot.workspace.rescan = { findingCount: 0, findings: [], notDetected: null, digest: verifiedDiff.digest, stale: false, scannedAt: "now", scannerVersions: "Fixture scanners" };
    render(<FixWorkspacePanel {...props} providerReady={false} />);
    await screen.findByText("Saved fix explanation");
    expect(screen.getByText("Worktree unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    await userEvent.click(screen.getByRole("tab", { name: "Checks: passed · freshness unavailable" }));
    expect(screen.getByText(/passed \(exit 0\) · freshness unavailable/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Run check" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "Rescan: 0 findings · freshness unavailable" }));
    expect(screen.getByText(/0 findings detected.*freshness unavailable/)).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Use the changes" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Rescan worktree" })).toBeDisabled();
    await userEvent.click(screen.getByRole("tab", { name: "Diff unavailable" }));
    expect(screen.getByRole("heading", { name: "Diff unavailable" })).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Diff · 0 files" })).not.toBeInTheDocument();
    mocks.get.mockResolvedValueOnce({ ...snapshot, diff: verifiedDiff, sourceError: null });
    await userEvent.click(screen.getByRole("button", { name: "Refresh diff and check freshness" }));
    expect(await screen.findByRole("heading", { name: "Diff · 0 files" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Checks: passed" })).toBeVisible();
    expect(screen.getByRole("tab", { name: "Rescan: 0 findings" })).toBeVisible();
  });

  it("only suggests applying a fix after a current comparable rescan", async () => {
    const current = fixSnapshot();
    const fresh = {
      findingCount: 0,
      findings: [],
      notDetected: 1,
      targetDetected: false,
      digest: current.diff!.digest,
      stale: false,
      scannedAt: "2026-11-12T00:00:00Z",
      scannerVersions: "Syft fixture · Grype fixture",
    };
    let refreshed: FixSnapshot = { ...current, workspace: { ...current.workspace, rescan: { ...fresh, notDetected: null, targetDetected: null } } };
    mocks.get.mockImplementation(async () => refreshed);
    render(<FixWorkspacePanel {...props} />);
    await userEvent.click(await screen.findByRole("tab", { name: /^Rescan/ }));
    expect(screen.getByText(/could not be compared with this worktree/)).toBeVisible();
    expect(screen.getByRole("heading", { name: "Worktree files" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Copy branch" })).toHaveTextContent("Copy branch");
    expect(screen.getByRole("button", { name: "Copy worktree path" })).toHaveTextContent("Copy worktree path");
    expect(screen.queryByRole("heading", { name: "Use the changes" })).not.toBeInTheDocument();

    refreshed = { ...refreshed, workspace: { ...refreshed.workspace, rescan: fresh } };
    mocks.rescan.mockImplementation(async () => refreshed);
    await userEvent.click(screen.getByRole("button", { name: "Rescan worktree" }));
    expect(await screen.findByRole("heading", { name: "Use the changes" })).toBeVisible();
    mocks.rescan.mockRejectedValueOnce(new Error("Incomplete npm inventory: restore the lockfile in apps/database and retry."));
    await userEvent.click(screen.getByRole("button", { name: "Rescan worktree" }));
    expect(await screen.findByText("Incomplete worktree inventory")).toBeVisible();
    expect(screen.getByText("restore the lockfile in apps/database and retry.")).toBeVisible();
    expect(screen.queryByText("Scanner setup required")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Use the changes" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Rescan worktree" }));
    expect(await screen.findByRole("heading", { name: "Use the changes" })).toBeVisible();
    refreshed = { ...refreshed, diff: { ...refreshed.diff!, digest: "changed-files" } };
    await userEvent.click(screen.getByRole("tab", { name: /^Diff/ }));
    await userEvent.click(screen.getByRole("button", { name: "Refresh diff and check freshness" }));
    await userEvent.click(screen.getByRole("tab", { name: /^Rescan:.*stale/ }));
    expect(screen.getByText("Files changed after this scan. Rescan the worktree to verify the current files.")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Use the changes" })).not.toBeInTheDocument();
  });

  it("does not submit nonlocal repositories and exposes accessible controls", async () => {
    const { container } = render(<FixWorkspacePanel {...props} repository={{ ...EMPTY_REPOSITORY, sourceType: "github" }} providerReady={false} />);
    await screen.findByText("Worktree ready");
    expect(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    const result = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(result.violations).toEqual([]);
  });

  it("discards a late snapshot after switching to another repository", async () => {
    let resolveOld!: (value: FixSnapshot) => void;
    const oldSnapshot = new Promise<FixSnapshot>((resolve) => { resolveOld = resolve; });
    const next = fixSnapshot();
    next.workspace = { ...next.workspace, id: "e".repeat(24), repositoryId: 2, activity: "Second repository workspace" };
    mocks.list.mockImplementation(async (repositoryId) => repositoryId === 1 ? [snapshot.workspace] : [next.workspace]);
    mocks.get.mockImplementation(async (repositoryId) => repositoryId === 1 ? oldSnapshot : next);
    const { rerender } = render(<FixWorkspacePanel {...props} key={1} />);
    await waitFor(() => expect(mocks.get).toHaveBeenCalledWith(1, snapshot.workspace.id));
    rerender(<FixWorkspacePanel {...props} key={2} repository={{ ...EMPTY_REPOSITORY, id: 2 }} />);
    await screen.findByText("Second repository workspace");
    await act(async () => resolveOld({ ...snapshot, workspace: { ...snapshot.workspace, activity: "Stale first repository" } }));
    expect(screen.queryByText("Stale first repository")).not.toBeInTheDocument();
    expect(screen.getByText("Second repository workspace")).toBeInTheDocument();
  });

  it("ignores older events and an older snapshot for the same workspace", async () => {
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText("Worktree ready");
    let resolveOld!: (value: FixSnapshot) => void;
    mocks.get.mockImplementationOnce(() => new Promise<FixSnapshot>((resolve) => { resolveOld = resolve; }));
    await userEvent.click(screen.getByRole("tab", { name: /^Diff/ }));
    await userEvent.click(screen.getByRole("button", { name: "Refresh diff and check freshness" }));
    act(() => listener({ ...snapshot.workspace, revision: 3, status: "working", activity: "Current fix is running" }));
    act(() => listener({ ...snapshot.workspace, revision: 2, status: "ready", activity: "Stale completion" }));
    await act(async () => resolveOld({ ...snapshot, workspace: { ...snapshot.workspace, revision: 1, activity: "Stale snapshot" } }));
    expect(screen.getByText("Current fix is running")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
    expect(screen.queryByText("Stale completion")).not.toBeInTheDocument();
    expect(screen.queryByText("Stale snapshot")).not.toBeInTheDocument();
  });

  it.each(["initial load", "workspace switch"])("keeps current state and Stop visible when an event overtakes the first snapshot during %s", async (stage) => {
    const target = fixSnapshot();
    target.workspace = { ...target.workspace, id: "e".repeat(24), revision: 1, activity: "Old workspace state" };
    let resolveSnapshot!: (value: FixSnapshot) => void;
    const delayed = new Promise<FixSnapshot>((resolve) => { resolveSnapshot = resolve; });
    mocks.list.mockResolvedValue(stage === "initial load" ? [target.workspace] : [snapshot.workspace, target.workspace]);
    mocks.get.mockImplementation(async (_repositoryId, workspaceId) => workspaceId === target.workspace.id ? delayed : snapshot);
    render(<FixWorkspacePanel {...props} />);
    if (stage === "workspace switch") {
      await screen.findByText("Worktree ready");
      screen.getByRole("combobox", { name: "Fix workspace" }).focus();
      await userEvent.keyboard("{ArrowDown}");
      await userEvent.click(await screen.findByRole("option", { name: /eeeeeeee/ }));
    }
    await waitFor(() => expect(mocks.get).toHaveBeenCalledWith(1, target.workspace.id));
    const current: FixWorkspace = { ...target.workspace, revision: 3, status: "working", activity: "Current task is running", messages: [{ role: "assistant", text: "Latest provider progress" }] };
    act(() => listener(current));
    await act(async () => resolveSnapshot(target));
    expect(await screen.findByRole("button", { name: "Stop" })).toBeEnabled();
    expect(screen.getByText("Current task is running")).toBeInTheDocument();
    expect(screen.getByText("Latest provider progress")).toBeInTheDocument();
    expect(screen.queryByText("Old workspace state")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.stop).toHaveBeenCalledWith(1, target.workspace.id);
    expect(mocks.start).not.toHaveBeenCalled();
    expect(mocks.create).not.toHaveBeenCalled();
  });

  it("keeps task submission disabled and explains a failed event subscription", async () => {
    mocks.listen.mockRejectedValueOnce(new Error("Fixture event subscription failed."));
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText("Worktree ready");
    expect(screen.getByText("Workspace updates unavailable")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Fix task" }), { target: { value: "Focused fix" } });
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("waits for the saved workspace inventory before allowing creation", async () => {
    let resolveList!: (value: FixWorkspace[]) => void;
    mocks.list.mockImplementationOnce(() => new Promise<FixWorkspace[]>((resolve) => { resolveList = resolve; }));
    render(<FixWorkspacePanel {...props} />);
    expect(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ })).toBeDisabled();
    await act(async () => resolveList([]));
    expect(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ })).toBeEnabled();
  });

  it.each(["inventory", "snapshot"])("can retry an initial %s failure without creating another workspace", async (stage) => {
    if (stage === "inventory") mocks.list.mockRejectedValueOnce(new Error("Temporary inventory failure"));
    else mocks.get.mockRejectedValueOnce(new Error("Temporary snapshot failure"));
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText(`Temporary ${stage} failure`);
    await userEvent.click(screen.getByRole("button", { name: "Retry loading workspaces" }));
    await screen.findByRole("textbox", { name: "Fix task" });
    expect(screen.queryByText(`Temporary ${stage} failure`)).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(mocks.create).not.toHaveBeenCalled();
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("keeps a newer draft when an old submission resolves after finding navigation", async () => {
    snapshot.workspace.findingFingerprint = "finding-a";
    let finishStart!: () => void;
    mocks.start.mockImplementationOnce(() => new Promise<void>((resolve) => { finishStart = resolve; }));
    const { rerender } = render(<FixWorkspacePanel {...props} findingFingerprint="finding-a" />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Original submitted task" } });
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    await userEvent.click(screen.getByRole("button", { name: "Allow and start fix" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledTimes(1));
    snapshot.workspace = { ...snapshot.workspace, revision: 2, status: "ready" };
    rerender(<FixWorkspacePanel {...props} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    await waitFor(() => expect(input).toBeEnabled());
    fireEvent.change(input, { target: { value: "New unsent task after navigation" } });
    await act(async () => finishStart());
    expect(input).toHaveValue("New unsent task after navigation");
  });

  it("does not reuse an unrelated finding workspace for a newly selected finding", async () => {
    snapshot.workspace.findingFingerprint = "previous-finding";
    render(<FixWorkspacePanel {...props} findingFingerprint="new-finding" />);
    await screen.findByText("Fix in an isolated checkout");
    expect(mocks.get).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: /^(New fix workspace|Create isolated fix|Create another fix)$/ }));
    await userEvent.click(screen.getByRole("button", { name: "Create branch and worktree" }));
    expect(mocks.create).toHaveBeenCalledWith(1, "new-finding", undefined);
  });

  it("reloads the matching workspace when the selected finding changes in the same repository", async () => {
    snapshot.workspace.findingFingerprint = "finding-a";
    const next = fixSnapshot();
    next.workspace = { ...next.workspace, id: "e".repeat(24), findingFingerprint: "finding-b", activity: "Finding B workspace" };
    mocks.list.mockResolvedValue([snapshot.workspace, next.workspace]);
    mocks.get.mockImplementation(async (_repositoryId, workspaceId) => workspaceId === next.workspace.id ? next : snapshot);
    const { rerender } = render(<FixWorkspacePanel {...props} findingFingerprint="finding-a" />);
    await screen.findByText("Worktree ready");
    rerender(<FixWorkspacePanel {...props} findingFingerprint="finding-b" />);
    await screen.findByText("Finding B workspace");
    expect(mocks.get).toHaveBeenLastCalledWith(1, next.workspace.id);
  });

  it("does not show a late Stop error in another finding workspace", async () => {
    snapshot.workspace.findingFingerprint = "finding-a";
    snapshot.workspace.status = "working";
    const next = fixSnapshot();
    next.workspace = { ...next.workspace, id: "e".repeat(24), findingFingerprint: "finding-b", activity: "Finding B workspace" };
    mocks.list.mockResolvedValue([snapshot.workspace, next.workspace]);
    mocks.get.mockImplementation(async (_repositoryId, workspaceId) => workspaceId === next.workspace.id ? next : snapshot);
    let rejectStop!: (cause: Error) => void;
    mocks.stop.mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { rejectStop = reject; }));
    const { rerender } = render(<FixWorkspacePanel {...props} findingFingerprint="finding-a" />);
    await userEvent.click(await screen.findByRole("button", { name: "Stop" }));
    rerender(<FixWorkspacePanel {...props} findingFingerprint="finding-b" />);
    await screen.findByText("Finding B workspace");
    await act(async () => rejectStop(new Error("Old workspace cancellation failed")));
    expect(screen.queryByText("Old workspace cancellation failed")).not.toBeInTheDocument();
    expect(screen.getByText("Finding B workspace")).toBeInTheDocument();
  });
  it("restores unsent workspace drafts after remount and keeps other workspace drafts separate", async () => {
    const next = fixSnapshot();
    next.workspace = { ...next.workspace, id: "e".repeat(24), activity: "Second workspace" };
    mocks.list.mockResolvedValue([snapshot.workspace, next.workspace]);
    mocks.get.mockImplementation(async (_repo, id) => id === next.workspace.id ? next : snapshot);
    const view = render(<FixWorkspacePanel {...props} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Keep draft A" } });
    const user = userEvent.setup();
    screen.getByRole("combobox", { name: "Fix workspace" }).focus();
    await user.keyboard("{ArrowDown}");
    await user.click(await screen.findByRole("option", { name: /eeeeeeee/ }));
    await screen.findByText("Second workspace");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("");
    fireEvent.change(screen.getByRole("textbox", { name: "Fix task" }), { target: { value: "Keep draft B" } });
    view.unmount();
    render(<FixWorkspacePanel {...props} />);
    await screen.findByText("Second workspace");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Keep draft B");
    screen.getByRole("combobox", { name: "Fix workspace" }).focus();
    await user.keyboard("{ArrowDown}");
    await user.click(await screen.findByRole("option", { name: /aaaaaaaa/ }));
    await screen.findByText("Worktree ready");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Keep draft A");
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("renders Markdown and reaches evidence using keyboard tabs without scrolling past messages", async () => {
    snapshot.workspace.messages = [{ role: "assistant", text: "**Actual result**\n\n- Tests passed\n- Review the diff" }];
    render(<FixWorkspacePanel {...props} />);
    await waitFor(() => expect(screen.getByText("Actual result").tagName).toBe("STRONG"));
    const tab = screen.getByRole("tab", { name: "Conversation" });
    tab.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: /^Diff/ })).toHaveFocus();
    expect(screen.getByRole("button", { name: "Refresh diff and check freshness" })).toBeVisible();
    const rescanTab = screen.getByRole("tab", { name: "Rescan" });
    const revealTab = vi.fn();
    rescanTab.scrollIntoView = revealTab;
    await userEvent.keyboard("{End}");
    expect(rescanTab).toHaveFocus();
    expect(revealTab).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
    expect(screen.getByRole("button", { name: "Rescan worktree" })).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "Fix task" })).not.toBeInTheDocument();
  });

  it("keeps non-ASCII prompts below the actual backend byte limit before consent", async () => {
    render(<FixWorkspacePanel {...props} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    fireEvent.change(input, { target: { value: "界".repeat(11_000) } });
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(screen.getByText(/32,000 UTF-8 bytes/)).toBeVisible();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("clears a submitted draft on success but preserves it on a rejected start", async () => {
    const view = render(<FixWorkspacePanel {...props} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Focused edit" } });
    mocks.start.mockRejectedValueOnce(new Error("Start was rejected"));
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    await userEvent.click(screen.getByRole("button", { name: "Allow and start fix" }));
    await screen.findByText("Start was rejected");
    expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue("Focused edit");
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    await userEvent.click(screen.getByRole("button", { name: "Allow and start fix" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Fix task" })).toHaveValue(""));
    view.unmount();
    render(<FixWorkspacePanel {...props} />);
    expect(await screen.findByRole("textbox", { name: "Fix task" })).toHaveValue("");
  });

});


describe("provider-aware fix workflow", () => {
  it("discloses and submits the selected Grok connection without a Codex fallback", async () => {
    render(<FixWorkspacePanel {...props} connectionId="grok_default" connections={[CODEX_CONNECTION, fixConnection("grok", "grok_default")]} model="grok-model" modelLabel="Grok model" />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Fix this dependency" } });
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    expect(screen.queryByRole("combobox", { name: "Reasoning effort" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Allow Grok to work on this fix?" })).toBeInTheDocument();
    expect(screen.getByText(/including messages from other providers/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Allow and start fix" }));
    await waitFor(() => expect(mocks.start).toHaveBeenCalledWith(1, snapshot.workspace.id, { prompt: "Fix this dependency", model: "grok-model", effort: null, connectionId: "grok_default", connectionUpdatedAt: "now" }));
  });

  it("invalidates an open source-sharing confirmation when the connection changes", async () => {
    const connections = [CODEX_CONNECTION, fixConnection("grok", "grok_default")];
    const { rerender } = render(<FixWorkspacePanel {...props} connections={connections} />);
    const input = await screen.findByRole("textbox", { name: "Fix task" });
    fireEvent.change(input, { target: { value: "Keep this unsent task" } });
    await userEvent.click(screen.getByRole("button", { name: "Send fix task" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    rerender(<FixWorkspacePanel {...props} connections={connections} connectionId="grok_default" />);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(input).toHaveValue("Keep this unsent task");
    expect(mocks.start).not.toHaveBeenCalled();
  });

  it("retains provider and model labels from historical messages", async () => {
    snapshot.workspace.messages = [
      { role: "assistant", text: "Previous Codex answer", provider: { connectionId: "codex_default", driver: "codex", displayName: "Codex", model: "codex-model" } },
      { role: "assistant", text: "Previous Grok answer", provider: { connectionId: "grok_default", driver: "grok", displayName: "Grok", model: "grok-model" } },
    ];
    render(<FixWorkspacePanel {...props} />);
    expect(await screen.findByText("Codex · codex-model")).toBeInTheDocument();
    expect(screen.getByText("Grok · grok-model")).toBeInTheDocument();
  });

  it("keeps local results available but refuses existing-server OpenCode fixes", async () => {
    const openCode = fixConnection("open_code", "opencode_default");
    openCode.connection.config = { kind: "open_code", mode: "existing_loopback", serverUrl: "http://127.0.0.1:4096" };
    render(<FixWorkspacePanel {...props} connectionId="opencode_default" connections={[openCode]} />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Fix task" }), { target: { value: "Fix" } });
    expect(screen.getByRole("button", { name: "Send fix task" })).toBeDisabled();
    expect(screen.getByText(/Existing servers cannot enforce/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: /^Checks/ }));
    expect(screen.getByRole("button", { name: "Run check" })).toBeEnabled();
  });
});
