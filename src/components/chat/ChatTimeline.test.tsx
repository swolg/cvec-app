import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CompanionTimelineItem } from "@/lib/types";
import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters } from "@/lib/performanceCounters";

import { labelTimelineItems, ChatTimeline } from "./ChatTimeline";

function message(id: string, text: string, kind: "user_message" | "assistant_message" = "assistant_message"): CompanionTimelineItem {
  return { id, kind, status: "completed", text, activity: null, citations: [], error: null, createdAt: "2026-08-20T08:00:00Z" };
}

const baseProps = { repositoryName: "demo", providerLabel: "Codex", segmentProviderLabels: ["Codex"], starters: [] as string[], onStarter: vi.fn(), onCitation: vi.fn() };

describe("ChatTimeline", () => {
  it("opens an empty conversation at the top and does not auto-follow an empty-state resize", () => {
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(1000);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(100);
    const observer = vi.fn(function () { return { observe: vi.fn(), disconnect: vi.fn() }; });
    vi.stubGlobal("ResizeObserver", observer);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    try {
      const view = render(<ChatTimeline {...baseProps} conversationKey="empty-fix-start" items={[]} emptyState={<h3>Describe the fix you need</h3>} />);
      expect(screen.getByRole("log").scrollTop).toBe(0);
      expect(observer).not.toHaveBeenCalled();
      view.rerender(<ChatTimeline {...baseProps} conversationKey="empty-fix-start" items={[message("answer", "Recorded answer")]} />);
      expect(screen.getByRole("log").scrollTop).toBe(1000);
      view.rerender(<ChatTimeline {...baseProps} conversationKey="empty-fix-start" items={[]} emptyState={<h3>Describe the fix you need</h3>} />);
      expect(screen.getByRole("log").scrollTop).toBe(0);
    } finally { vi.unstubAllGlobals(); }
  });
  it("copies only the visible completed answer Markdown without citation transport markers", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    render(<ChatTimeline {...baseProps} items={[message("user", "Question\n\n<repository_chat_context_v1>secret</repository_chat_context_v1>", "user_message"), message("answer", "**Recorded evidence** [source:scan:10]"), { ...message("live", "Still responding"), status: "streaming" }]} />);
    expect(screen.getAllByRole("button", { name: "Copy answer as Markdown" })).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Copy answer as Markdown" }));
    expect(write).toHaveBeenCalledWith("**Recorded evidence**");
  });
  it("keeps completed history memoized across turn start and completion", async () => {
    const history = Array.from({ length: 5 }, (_, index) => message(`steady-${index}`, `Stable completed answer ${index}`));
    const answer = { ...message("active-answer", "Live synthetic answer"), status: "streaming" as const };
    const items = [...history, answer];
    const view = render(<ChatTimeline {...baseProps} conversationKey="turn-boundary-render-test" items={items} />);
    await screen.findByText("Live synthetic answer");
    enablePerformanceCounters();
    try {
      takePerformanceCounters();
      view.rerender(<ChatTimeline {...baseProps} conversationKey="turn-boundary-render-test" items={items} progress={{ label: "Responding…", startedAt: Date.now() }} />);
      expect(takePerformanceCounters().completedItemRenders).toBe(0);
      view.rerender(<ChatTimeline {...baseProps} conversationKey="turn-boundary-render-test" items={[...history, { ...answer, status: "completed" }]} />);
      expect(takePerformanceCounters().completedItemRenders).toBe(1);
    } finally { disablePerformanceCounters(); }
  });

  it("restores each conversation's reading position and opens a new conversation at the end", () => {
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(1000);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(100);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    try {
      const view = render(<ChatTimeline {...baseProps} conversationKey="scroll-review-a" items={[message("a", "First conversation")]} />);
      const first = screen.getByRole("log");
      first.scrollTop = 200;
      fireEvent.scroll(first);
      view.rerender(<ChatTimeline {...baseProps} conversationKey="scroll-review-b" items={[message("b", "Second conversation")]} />);
      expect(screen.getByRole("log").scrollTop).toBe(1000);
      expect(screen.queryByRole("button", { name: "Latest response" })).not.toBeInTheDocument();
      view.rerender(<ChatTimeline {...baseProps} conversationKey="scroll-review-a" items={[message("a", "First conversation")]} />);
      expect(screen.getByRole("log").scrollTop).toBe(200);
    } finally { vi.unstubAllGlobals(); }
  });
  it("keeps an accessible progress indicator through thinking and gaps, and removes it at completion", () => {
    vi.useFakeTimers();
    try {
      const startedAt = Date.now();
      const props = { ...baseProps, items: [message("user", "Question", "user_message")] };
      const view = render(<ChatTimeline {...props} progress={{ label: "Waiting for Codex…", startedAt }} />);
      expect(screen.getByRole("status")).toHaveTextContent("Waiting for Codex…");
      act(() => { vi.advanceTimersByTime(3000); });
      expect(screen.getByTitle("Time since your message was sent")).toHaveTextContent("Elapsed 3s");
      expect(screen.getByRole("status")).not.toHaveTextContent("3s");
      view.rerender(<ChatTimeline {...props} progress={{ label: "Codex is thinking…", startedAt }} />);
      expect(screen.getByRole("status")).toHaveTextContent("Codex is thinking…");
      view.rerender(<ChatTimeline {...props} items={[...props.items, message("step", "First step")]} progress={{ label: "Codex is continuing…", startedAt }} />);
      expect(screen.getByRole("status")).toHaveTextContent("Codex is continuing…");
      expect(screen.getByRole("status").querySelector(".motion-reduce\\:animate-none")).not.toBeNull();
      view.rerender(<ChatTimeline {...props} items={[...props.items, message("answer", "Done")]} />);
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    } finally { vi.useRealTimers(); }
  });

  it("keeps one activity header before the answer and makes its streaming label visually quiet", () => {
    const startedAt = Date.now();
    const user = message("user", "Question", "user_message");
    const step = { ...message("step", "First step"), status: "streaming" as const };
    const progress = { label: "Responding…", providerLabel: "Codex", startedAt, responding: true };
    const view = render(<ChatTimeline {...baseProps} items={[user, step]} progress={progress} />);
    const status = screen.getByRole("status");
    expect(within(status).getByText("Responding…")).toHaveClass("sr-only");
    expect(status.compareDocumentPosition(screen.getByText("First step")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByLabelText("Streaming response")).not.toBeInTheDocument();
    view.rerender(<ChatTimeline {...baseProps} items={[user, { ...step, status: "completed" }]} progress={{ ...progress, label: "Working…", responding: false }} />);
    expect(screen.getByRole("status")).toBe(status);
    expect(within(status).getByText("Working…")).not.toHaveClass("sr-only");
  });

  it("excludes development-feature notices including legacy cached errors from the conversation", () => {
    const warning: CompanionTimelineItem = {
      ...message("warning-1", ""), kind: "warning",
      error: { code: "codex_warning", message: "Under-development features enabled: chronicle.", retryable: false },
    };
    render(<ChatTimeline {...baseProps} items={[
      warning,
      message("answer", "Hey!"),
      { ...warning, id: "legacy", kind: "error", status: "failed" },
      { ...warning, id: "warning-2" },
    ]} />);
    expect(screen.queryByText("Codex configuration warning")).not.toBeInTheDocument();
    expect(screen.queryByText(warning.error!.message)).not.toBeInTheDocument();
    expect(screen.queryByText("Response interrupted")).not.toBeInTheDocument();
    expect(screen.getByText("Hey!")).toBeInTheDocument();
  });

  it("still shows real response interruptions and denied actions", () => {
    render(<ChatTimeline {...baseProps} items={[
      { ...message("offline", ""), kind: "error", status: "failed", error: { code: "offline", message: "Connection ended", retryable: true } },
      { ...message("denied", ""), kind: "error", status: "failed", error: { code: "action_denied", message: "Read-only", retryable: false } },
      { ...message("other-warning", ""), kind: "warning", error: { code: "codex_warning", message: "Provider configuration needs attention", retryable: false } },
    ]} />);
    expect(screen.getByText("Response interrupted")).toBeInTheDocument();
    expect(screen.getByText("Read-only action denied")).toBeInTheDocument();
    expect(screen.getByText("Provider configuration needs attention")).toBeInTheDocument();
  });

  it("filters prepared context, renders transitions, and loads older messages", async () => {
    const items: CompanionTimelineItem[] = Array.from({ length: 201 }, (_, index) => message(`message-${index}`, `Answer ${index}`));
    items.splice(100, 0, { id: "context", kind: "activity", status: "completed", text: "", activity: { kind: "context", label: "Prepared bounded repository context", detail: null, sourceIds: [] }, citations: [], error: null, createdAt: "now" });
    items.splice(150, 0, { id: "transition", kind: "activity", status: "completed", text: "", activity: { kind: "provider_transition", label: "Continued with Grok", detail: "grok-4.6", sourceIds: [] }, citations: [], error: null, createdAt: "now" });
    render(<ChatTimeline {...baseProps} items={items} />);

    expect(screen.queryByText("Prepared bounded repository context")).not.toBeInTheDocument();
    expect(screen.getByText("Continued with Grok · grok-4.6")).toBeInTheDocument();
    expect(screen.queryByText("Answer 0")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Load earlier messages" }));
    expect(screen.getByText("Answer 0")).toBeInTheDocument();
  });

  it("preserves history review position and offers a keyboard-accessible latest control", async () => {
    const view = render(<ChatTimeline {...baseProps} items={[message("assistant", "partial")]} />);
    const log = screen.getByRole("log", { name: "Repository chat messages" });
    Object.defineProperties(log, {
      scrollHeight: { configurable: true, value: 1000 },
      clientHeight: { configurable: true, value: 100 },
    });
    log.scrollTop = 100;
    fireEvent.scroll(log);
    view.rerender(<ChatTimeline {...baseProps} items={[{ ...message("assistant", "partial response"), status: "streaming" }]} />);
    const latest = await screen.findByRole("button", { name: "Latest response" });
    const scrollTo = vi.fn();
    Object.defineProperty(log, "scrollTo", { configurable: true, value: scrollTo });
    await userEvent.click(latest);
    expect(scrollTo).toHaveBeenCalledWith({ top: 1000, behavior: "smooth" });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Latest response" })).not.toBeInTheDocument());
  });

  it("communicates streaming even when animation is reduced", () => {
    render(<ChatTimeline {...baseProps} items={[{ ...message("assistant", "Working"), status: "streaming" }]} />);
    const status = screen.getByLabelText("Streaming response");
    expect(status.querySelector(".motion-reduce\\:animate-none")).not.toBeNull();
  });

  it("keeps historical assistant attribution when the composer changes provider", () => {
    const items: CompanionTimelineItem[] = [
      message("codex-answer", "Codex answer"),
      { id: "transition", kind: "activity", status: "completed", text: "", activity: { kind: "provider_transition", label: "Continued with Grok", detail: null, sourceIds: [] }, citations: [], error: null, createdAt: "now" },
      message("grok-answer", "Grok answer"),
    ];
    render(<ChatTimeline {...baseProps} items={items} providerLabel="Grok" segmentProviderLabels={["Codex", "Grok"]} />);
    const articles = screen.getAllByRole("article");
    expect(within(articles[0]).getByText("Codex")).toBeInTheDocument();
    expect(within(articles[1]).getByText("Grok")).toBeInTheDocument();
  });
});

it("uses explicit segment labels when a history page starts after a provider transition",()=>{
  const items=[message("segment:8:same","Older answer"),message("segment:9:same","Newer answer")];
  expect(labelTimelineItems(items,["Wrong fallback"],"Current",{"segment:8:same":"Grok","segment:9:same":"Codex"}).map((entry)=>entry.providerLabel)).toEqual(["Grok","Codex"]);
});
