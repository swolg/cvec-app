import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CompanionThread, RepositoryChatScanStatus, RepoSummary } from "@/lib/types";

import { ConversationHeader } from "./ConversationHeader";

const repository: RepoSummary = {
  id: 1, name: "api", path: "/tmp/api", sourceType: "local", sourceKey: null,
  remoteUrl: null, remoteHost: null, provider: "local", defaultBranch: "main",
  lastRemoteCommitSha: null, lastRemoteSyncedAt: null, remoteFileCount: null,
  status: "clean", newMedium: 0, newHigh: 0, newCritical: 0, openMedium: 0, openHigh: 0, openCritical: 0,
  resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0, lastScannedAt: null, lastScanStatus: null, lastSuccessfulSourceRevision: null,
};
const completedScan = { id: 42, status: "completed", scannedAt: "2026-08-19T12:01:00Z", revision: "abc", evidenceComplete: true };
const scanStatus: RepositoryChatScanStatus = { repositoryId: repository.id, latestCompleted: completedScan, latestAttempt: completedScan };
const thread: CompanionThread = { id: 10, repositoryId: 1, provider: "codex", providerThreadId: "provider", providerConnectionId: "codex_default", title: "Risk review", status: "ready", createdAt: "now", updatedAt: "now", lastOpenedAt: "now", archivedAt: null, contextVersion: 1, lastModelId: "gpt" };

describe("ConversationHeader", () => {
  it("keeps the completed scan visible after a newer attempt fails", () => {
    const latestAttempt = { ...completedScan, id: 43, status: "failed", evidenceComplete: false };
    render(<ConversationHeader context={null} contextChanged={false} scanStatus={{ ...scanStatus, latestAttempt }} repository={repository} thread={thread} onArchive={() => {}} onInspectContext={() => {}} onRename={() => {}} />);
    expect(screen.getByText("Scan #42")).toBeVisible();
    expect(screen.getByText("Latest scan failed")).toHaveAttribute("title", "Scan #43 failed");
  });

  it("keeps hierarchy and security scope compact while making unknown alignment actionable", async () => {
    const inspect = vi.fn();
    render(<ConversationHeader context={null} contextChanged={false} scanStatus={scanStatus} repository={repository} thread={thread} onArchive={() => {}} onInspectContext={inspect} onRename={() => {}} />);
    expect(screen.getByText("api")).toBeInTheDocument();
    expect(screen.getByText("Risk review")).toBeInTheDocument();
    expect(screen.getByText("Read-only")).toBeInTheDocument();
    expect(screen.getByText("Scan #42")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Inspect context" }));
    expect(inspect).toHaveBeenCalledOnce();
  });

  it("contains rename and archive in one conversation action surface", async () => {
    const rename = vi.fn();
    const archive = vi.fn();
    const view = render(<ConversationHeader context={null} contextChanged={false} scanStatus={scanStatus} repository={repository} thread={thread} onArchive={archive} onInspectContext={() => {}} onRename={rename} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Conversation actions" }));
    const title = screen.getByRole("textbox", { name: "Conversation title" });
    await user.clear(title);
    await user.type(title, "Dependency plan");
    await user.click(screen.getByRole("button", { name: "Rename" }));
    expect(rename).toHaveBeenCalledWith("Dependency plan");

    view.unmount();
    render(<ConversationHeader context={null} contextChanged={false} scanStatus={scanStatus} repository={repository} thread={thread} onArchive={archive} onInspectContext={() => {}} onRename={rename} />);
    await user.click(screen.getByRole("button", { name: "Conversation actions" }));
    await user.click(screen.getByRole("button", { name: "Archive" }));
    expect(archive).toHaveBeenCalledWith(true);
  });
});
