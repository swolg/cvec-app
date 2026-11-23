import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/companionApi", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/companionApi")>(),
  companionScanStatus: vi.fn(),
}));
vi.mock("@/components/RepositoryChat", () => ({
  RepositoryChat: ({ repository, contextStatus, findingRequest }: { repository: { name: string }; contextStatus?: string; findingRequest?: { fingerprint: string } }) => (
    <div data-chat-state={contextStatus} aria-label="Repository chat messages">{repository.name} chat {findingRequest?.fingerprint}</div>
  ),
}));

import { companionScanStatus } from "@/lib/companionApi";
import type { RepositoryChatScanStatus, RepoSummary } from "@/lib/types";
import { ChatWorkspace } from "./ChatWorkspace";

const repo = (id: number, name: string): RepoSummary => ({
  id, name, path: `/tmp/${name}`, sourceType: "local", sourceKey: null,
  remoteUrl: null, remoteHost: null, provider: "local", defaultBranch: "main",
  lastRemoteCommitSha: null, lastRemoteSyncedAt: null, remoteFileCount: null,
  status: "warning", newMedium: 0, newHigh: 0, newCritical: 0,
  openMedium: 0, openHigh: id, openCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
  lastScannedAt: "2026-08-20T08:00:00Z", lastScanStatus: "completed", lastSuccessfulSourceRevision: "abc",
});

const emptyScan = { id: null, status: null, scannedAt: null, revision: null, evidenceComplete: false };
const scanStatus = (repositoryId: number): RepositoryChatScanStatus => ({
  repositoryId, latestCompleted: emptyScan, latestAttempt: emptyScan,
});

describe("ChatWorkspace", () => {
  beforeEach(() => { vi.mocked(companionScanStatus).mockReset(); });

  it("keeps Chat first-class while making repository scope explicit and interactive", async () => {
    const first = repo(1, "api");
    const second = repo(2, "frontend");
    vi.mocked(companionScanStatus).mockImplementation(async (id) => scanStatus(id));
    const onSelectRepo = vi.fn();
    const view = render(<ChatWorkspace repos={[first, second]} selectedRepoId={first.id} findingRequest={{ repositoryId: first.id, fingerprint: "fp-1", advisoryId: "CVE-2026-1", packageName: "fixture", installedVersion: "1", targetVersion: "2", scanRunId: 10, mode: "analysis" }} onSelectRepo={onSelectRepo} onOpenReview={() => {}} onError={() => {}} />);

    expect(await screen.findByLabelText("Repository chat messages")).toHaveTextContent("api chat fp-1");
    expect(screen.getAllByRole("heading", { name: "Chat" }).length).toBeGreaterThan(0);
    expect(screen.getByRole("combobox", { name: "Repository and conversation" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^frontend 0$/ }));
    expect(onSelectRepo).toHaveBeenCalledWith(second.id);
    view.rerender(<ChatWorkspace repos={[first, second]} selectedRepoId={second.id} onSelectRepo={onSelectRepo} onOpenReview={() => {}} onError={() => {}} />);
    expect(await screen.findByLabelText("Repository chat messages")).toHaveTextContent("frontend chat");
  });

  it("selects the first repository when Chat opens without a current scope", () => {
    const first = repo(1, "api");
    const onSelectRepo = vi.fn();
    render(<ChatWorkspace repos={[first]} selectedRepoId={null} onSelectRepo={onSelectRepo} onOpenReview={() => {}} onError={() => {}} />);
    expect(onSelectRepo).toHaveBeenCalledWith(first.id);
  });

  it("marks an empty workspace and keeps a pending repository distinct from a failed request", async () => {
    const first = repo(1, "api");
    let finish!: (value: RepositoryChatScanStatus) => void;
    vi.mocked(companionScanStatus).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const props = { onSelectRepo: vi.fn(), onOpenReview: vi.fn(), onError: vi.fn() };
    const view = render(<ChatWorkspace repos={[]} selectedRepoId={null} {...props} />);
    expect(view.container.querySelector('[data-chat-workspace]')).toHaveAttribute("data-chat-state", "empty");
    view.rerender(<ChatWorkspace repos={[first]} selectedRepoId={first.id} {...props} />);
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "loading");
    await act(async () => finish(scanStatus(first.id)));
    expect(await screen.findByLabelText("Repository chat messages")).toHaveTextContent("api chat");
    let fail!: (reason: Error) => void;
    vi.mocked(companionScanStatus).mockImplementation(() => new Promise((_resolve, reject) => { fail = reject; }));
    view.rerender(<ChatWorkspace repos={[repo(2, "next")]} selectedRepoId={2} {...props} />);
    expect(screen.getByLabelText("Repository chat messages")).toHaveTextContent("next chat");
    await act(async () => fail(new Error("offline fixture")));
    expect(screen.getByLabelText("Repository chat messages")).toHaveTextContent("next chat");
    expect(view.container.querySelector('[data-chat-state]')).toHaveAttribute("data-chat-state", "error");
  });

  it("reloads scan status when evidence changes and ignores an earlier request", async () => {
    const first = { ...repo(1, "api"), evidenceRevision: 1 };
    const updated = { ...first, evidenceRevision: 2, lastScanStatus: "completed" as const };
    let resolveEarlier!: (value: RepositoryChatScanStatus) => void;
    vi.mocked(companionScanStatus)
      .mockImplementationOnce(() => new Promise(resolve => { resolveEarlier = resolve; }))
      .mockResolvedValueOnce(scanStatus(first.id));
    const onError = vi.fn();
    const props = { selectedRepoId: first.id, onSelectRepo: vi.fn(), onOpenReview: vi.fn(), onError };
    const view = render(<ChatWorkspace repos={[first]} {...props} />);
    view.rerender(<ChatWorkspace repos={[updated]} {...props} />);
    await screen.findByText("api chat");
    await act(async () => resolveEarlier(scanStatus(2)));
    expect(companionScanStatus).toHaveBeenCalledTimes(2);
    expect(screen.getByLabelText("Repository chat messages")).toHaveAttribute("data-chat-state", "ready");
    expect(onError).not.toHaveBeenCalled();
  });
});
