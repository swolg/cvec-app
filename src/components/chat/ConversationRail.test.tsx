import axe from "axe-core";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CompanionThread, RepoSummary } from "@/lib/types";

import { CompactConversationSelector, ConversationRail } from "./ConversationRail";

function repository(id: number, name: string): RepoSummary {
  return {
    id, name, path: `/tmp/${name}`, sourceType: "local", sourceKey: null,
    remoteUrl: null, remoteHost: null, provider: "local", defaultBranch: "main",
    lastRemoteCommitSha: null, lastRemoteSyncedAt: null, remoteFileCount: null,
    status: "clean", newMedium: 0, newHigh: 0, newCritical: 0,
    openMedium: 0, openHigh: 0, openCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
    lastScannedAt: null, lastScanStatus: null, lastSuccessfulSourceRevision: null,
  };
}

function thread(id: number, repositoryId: number, title: string, archivedAt: string | null = null): CompanionThread {
  return {
    id, repositoryId, provider: "codex", providerThreadId: `provider-${id}`, providerConnectionId: "codex_default",
    title, status: "ready", createdAt: "2026-08-20T08:00:00Z", updatedAt: "2026-08-20T08:00:00Z",
    lastOpenedAt: "2026-08-20T08:00:00Z", archivedAt, contextVersion: 1, lastModelId: "gpt-test",
  };
}

describe("ConversationRail", () => {
  it("opens the active group initially and respects a deliberate collapse on rerender", async () => {
    const props = { activeThreadId: 1, repositories: [repository(1, "api")], selectedRepositoryId: 1, threads: [thread(1, 1, "Selected chat")], onNewConversation: vi.fn(), onSelectConversation: vi.fn(), onSelectRepository: vi.fn() };
    const view = render(<ConversationRail {...props} />);
    expect(screen.getByText("Selected chat")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Collapse api" }));
    view.rerender(<ConversationRail {...props} threads={[...props.threads, thread(2, 1, "Another chat")]} />);
    expect(screen.queryByText("Selected chat")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Expand api" }));
    expect(screen.getByText("Selected chat")).toBeVisible();
  });

  it("searches compact history beyond the initial rendering limit", async () => {
    const select = vi.fn();
    const threads = Array.from({ length: 230 }, (_, i) => thread(i + 1, 1, `Conversation ${i + 1}`));
    render(<CompactConversationSelector activeThreadId={1} selectedRepositoryId={1} repositories={[repository(1, "api")]} threads={threads} onNewConversation={vi.fn()} onSelectConversation={select} onSelectRepository={vi.fn()} />);
    await userEvent.click(screen.getByRole("combobox", { name: "Repository and conversation" }));
    expect(await screen.findAllByRole("option")).toHaveLength(200);
    await userEvent.type(screen.getByRole("combobox", { name: "Search repositories and conversations" }), "Conversation 230");
    await userEvent.click(screen.getByRole("option", { name: "api · Conversation 230" }));
    expect(select).toHaveBeenCalledWith(1, 230);
  });
  it("shows five recent conversations per repository and loads older rows in batches", async () => {
    const threads = Array.from({ length: 12 }, (_, index) => ({
      ...thread(index + 1, 1, `Conversation ${index + 1}`),
      lastOpenedAt: `2026-08-20T${String(23 - index).padStart(2, "0")}:00:00Z`,
    }));
    render(
      <ConversationRail
        activeThreadId={1}
        repositories={[repository(1, "api")]}
        selectedRepositoryId={1}
        threads={threads}
        onNewConversation={() => {}}
        onSelectConversation={() => {}}
        onSelectRepository={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: "Collapse api" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Conversation 5")).toBeInTheDocument();
    expect(screen.queryByText("Conversation 6")).not.toBeInTheDocument();
    expect(screen.getByText("7 remaining")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /Show 5 more/ }));
    expect(screen.getByText("Conversation 10")).toBeInTheDocument();
    expect(screen.queryByText("Conversation 11")).not.toBeInTheDocument();
    expect(screen.getByText("2 remaining")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /Show 2 more/ }));
    expect(screen.getByText("Conversation 12")).toBeInTheDocument();
    expect(screen.queryByText(/remaining/)).not.toBeInTheDocument();
  });

  it("keeps the active conversation visible when it is older than the initial batch", () => {
    const threads = Array.from({ length: 8 }, (_, index) => ({
      ...thread(index + 1, 1, `Conversation ${index + 1}`),
      lastOpenedAt: `2026-08-20T${String(23 - index).padStart(2, "0")}:00:00Z`,
    }));
    render(
      <ConversationRail
        activeThreadId={8}
        repositories={[repository(1, "api")]}
        selectedRepositoryId={1}
        threads={threads}
        onNewConversation={() => {}}
        onSelectConversation={() => {}}
        onSelectRepository={() => {}}
      />,
    );

    expect(screen.getByText("Conversation 8")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Conversation \d/ })).toHaveLength(5);
  });

  it("groups searchable conversations by repository and excludes archived rows", async () => {
    const selectConversation = vi.fn();
    const view = render(
      <ConversationRail
        activeThreadId={11}
        repositories={[repository(1, "api"), repository(2, "frontend")]}
        selectedRepositoryId={1}
        threads={[thread(11, 1, "Critical dependency review"), thread(12, 1, "Archived notes", "2026-08-21T08:00:00Z"), thread(21, 2, "Authentication flow")]}
        onNewConversation={() => {}}
        onSelectConversation={selectConversation}
        onSelectRepository={() => {}}
      />,
    );

    expect(screen.getByText("Critical dependency review")).toBeInTheDocument();
    expect(screen.queryByText("Archived notes")).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox", { name: "Search repositories and conversations" }), { target: { value: "authentication" } });
    expect(screen.queryByText("Critical dependency review")).not.toBeInTheDocument();
    await userEvent.click(screen.getByText("Authentication flow"));
    expect(selectConversation).toHaveBeenCalledWith(2, 21);
    expect((await axe.run(view.container, { rules: { "color-contrast": { enabled: false } } })).violations).toEqual([]);
  });

  it("keeps empty repositories actionable and supports collapse plus compact selection", async () => {
    const start = vi.fn();
    const selectRepository = vi.fn();
    const selectConversation = vi.fn();
    const repos = [repository(1, "empty"), repository(2, "api")];
    const threads = [thread(21, 2, "Risk review")];
    const view = render(
      <ConversationRail activeThreadId={null} repositories={repos} selectedRepositoryId={1} threads={threads} onNewConversation={start} onSelectConversation={selectConversation} onSelectRepository={selectRepository} />,
    );
    expect(screen.queryByRole("button", { name: "Start a conversation" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Expand empty" }));
    await userEvent.click(screen.getByRole("button", { name: "Start a conversation" }));
    expect(start).toHaveBeenCalledWith(1);
    await userEvent.click(screen.getByRole("button", { name: "Expand api" }));
    expect(screen.getByText("Risk review")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Collapse api" }));
    expect(screen.queryByText("Risk review")).not.toBeInTheDocument();

    view.rerender(<CompactConversationSelector activeThreadId={null} repositories={repos} selectedRepositoryId={1} threads={threads} onNewConversation={start} onSelectConversation={selectConversation} onSelectRepository={selectRepository} />);
    await userEvent.click(screen.getByRole("combobox", { name: "Repository and conversation" }));
    expect((await axe.run(await screen.findByRole("dialog", { name: "Choose repository and conversation" }), { rules: { "color-contrast": { enabled: false } } })).violations).toEqual([]);
    await userEvent.click(await screen.findByRole("option", { name: "api · Risk review" }));
    expect(selectConversation).toHaveBeenCalledWith(2, 21);
  });
  it("makes archived conversations discoverable and selectable", async () => {
    const select = vi.fn();
    render(<ConversationRail activeThreadId={null} selectedRepositoryId={1} repositories={[repository(1, "api")]} threads={[thread(1, 1, "Active notes"), thread(2, 1, "Archived review", "now")]} onNewConversation={vi.fn()} onSelectConversation={select} onSelectRepository={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Show archived conversations" }));
    expect(screen.queryByText("Active notes")).not.toBeInTheDocument();
    await userEvent.click(screen.getByText("Archived review"));
    expect(select).toHaveBeenCalledWith(1, 2);
    await userEvent.click(screen.getByRole("button", { name: "Collapse api" }));
    expect(screen.queryByText("Archived review")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Expand api" })).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(screen.getByRole("button", { name: "Expand api" }));
    expect(screen.getByText("Archived review")).toBeInTheDocument();
  });

  it("starts a new draft from the compact selector and exposes archived history", async () => {
    const start = vi.fn();
    const selectRepository = vi.fn();
    render(<CompactConversationSelector activeThreadId={2} selectedRepositoryId={1} repositories={[repository(1, "api")]} threads={[thread(2, 1, "Archived review", "now")]} onNewConversation={start} onSelectConversation={vi.fn()} onSelectRepository={selectRepository} />);
    expect(screen.getByRole("combobox")).toHaveTextContent("Archived: Archived review");
    screen.getByRole("combobox").focus();
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.click(await screen.findByRole("option", { name: "api · new draft" }));
    expect(start).toHaveBeenCalledWith(1);
    expect(selectRepository).not.toHaveBeenCalled();
    const trigger = screen.getByRole("combobox", { name: "Repository and conversation" });
    await userEvent.click(trigger);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Search repositories and conversations" })).toHaveFocus());
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(trigger).toHaveFocus());
  });

});
