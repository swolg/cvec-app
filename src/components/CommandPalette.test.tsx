import axe from "axe-core";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/api", () => ({ searchLocalEntities: vi.fn(async () => []) }));
import { searchLocalEntities } from "@/lib/api";
import { CommandPalette } from "./CommandPalette";
import { BUILT_IN_REVIEW_VIEWS } from "@/lib/reviewViews";
import { EMPTY_REPOSITORY } from "@/test/fixtures";

function renderPalette(overrides: Partial<React.ComponentProps<typeof CommandPalette>> = {}) {
  const props: React.ComponentProps<typeof CommandPalette> = { open: true, onOpenChange: vi.fn(), repos: [EMPTY_REPOSITORY], views: [...BUILT_IN_REVIEW_VIEWS], scannerReady: true, scanRunning: false, onNavigate: vi.fn(), onOpenRepo: vi.fn(), onOpenEntity: vi.fn(), onOpenView: vi.fn(), onScanAll: vi.fn(), onScanRepo: vi.fn(), onShowActivity: vi.fn(), ...overrides };
  return { ...render(<CommandPalette {...props} />), props };
}

describe("CommandPalette", () => {
  beforeEach(() => vi.mocked(searchLocalEntities).mockReset().mockResolvedValue([]));

  it("supports keyboard activation and exposes disabled reasons", async () => {
    const onScanAll = vi.fn();
    renderPalette({ onScanAll, scannerReady: false });
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "scan all" } });
    expect(await screen.findByText("Scanner setup required")).toBeInTheDocument();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onScanAll).not.toHaveBeenCalled();
  });

  it("discards stale async local search results", async () => {
    let resolveFirst!: (value: Awaited<ReturnType<typeof searchLocalEntities>>) => void;
    vi.mocked(searchLocalEntities).mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; })).mockResolvedValueOnce([{ id: "repo:2", kind: "repository", label: "new result", context: "local", repoId: 2, fingerprint: null, advisoryId: null }]);
    renderPalette();
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "old" } });
    await waitFor(() => expect(searchLocalEntities).toHaveBeenCalledWith("old"));
    fireEvent.change(input, { target: { value: "new" } });
    await screen.findByText("new result");
    await act(async () => resolveFirst([{ id: "repo:1", kind: "repository", label: "old result", context: "local", repoId: 1, fingerprint: null, advisoryId: null }]));
    expect(screen.queryByText("old result")).not.toBeInTheDocument();
  });

  it("opens the matching repository before scan commands for the same name", async () => {
    const hit = { id: "repo:1", kind: "repository", label: "cvec", context: "local", repoId: 1, fingerprint: null, advisoryId: null } as const;
    vi.mocked(searchLocalEntities).mockResolvedValueOnce([hit]);
    const { props } = renderPalette();
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "cvec" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onScanRepo).not.toHaveBeenCalled();

    await screen.findByText(hit.label);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onOpenRepo).toHaveBeenCalledWith(1);
    expect(props.onScanRepo).not.toHaveBeenCalled();
  });

  it("keeps a deliberately selected scan command while local results arrive", async () => {
    const hit = { id: "repo:1", kind: "repository", label: "cvec", context: "local", repoId: 1, fingerprint: null, advisoryId: null } as const;
    let resolve!: (hits: typeof hit[]) => void;
    vi.mocked(searchLocalEntities).mockImplementationOnce(() => new Promise((finish) => { resolve = finish; }));
    const { props } = renderPalette();
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "cvec" } });
    fireEvent.keyDown(input, { key: "ArrowDown" });
    const scan = screen.getByText("Scan cvec").closest("[role=option]");
    expect(input).toHaveAttribute("aria-activedescendant", scan?.id);

    await waitFor(() => expect(searchLocalEntities).toHaveBeenCalledWith("cvec"));
    await act(async () => resolve([hit]));
    expect(input).toHaveAttribute("aria-activedescendant", scan?.id);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onScanRepo).toHaveBeenCalledWith(1);
    expect(props.onOpenRepo).not.toHaveBeenCalled();
  });

  it("shows an active scan even when the scanner is unavailable for a new scan", () => {
    const { props } = renderPalette({ scanRunning: true, scannerReady: false });
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "scan all" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onShowActivity).toHaveBeenCalledOnce();
    expect(props.onScanAll).not.toHaveBeenCalled();
  });

  it("clears in-flight results and never opens a previous search match", async () => {
    const hit = { id: "repo:8", kind: "repository", label: "only old repo", context: "local", repoId: 8, fingerprint: null, advisoryId: null } as const;
    let settle!: (items: typeof hit[]) => void;
    vi.mocked(searchLocalEntities).mockResolvedValueOnce([hit])
      .mockImplementationOnce(() => new Promise(resolve => { settle = resolve; }));
    const { props } = renderPalette();
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "old" } });
    await screen.findByText(hit.label);
    fireEvent.change(input, { target: { value: "later" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onOpenRepo).not.toHaveBeenCalled();
    expect(screen.queryByText(hit.label)).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Searching local data");
    await waitFor(() => expect(searchLocalEntities).toHaveBeenCalledWith("later"));
    fireEvent.change(input, { target: { value: "" } });
    await act(async () => settle([hit]));
    expect(screen.queryByText(hit.label)).not.toBeInTheDocument();
    expect(searchLocalEntities).toHaveBeenCalledTimes(2);
  });

  it("shows a failed search separately from an empty result", async () => {
    vi.mocked(searchLocalEntities).mockRejectedValueOnce(new Error("Local query failed"));
    renderPalette();
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "missing" } });
    expect(await screen.findByText(/Could not search local data/)).toBeInTheDocument();
    expect(screen.queryByText(/No local results/)).not.toBeInTheDocument();
  });

  it("distinguishes package destinations in different repositories", async () => {
    const hit = { id: "package:8", kind: "package", label: "fixture-router", context: "api-service", repoId: 8, fingerprint: null, advisoryId: null } as const;
    const other = { ...hit, id: "package:9", context: "web-client", repoId: 9 };
    vi.mocked(searchLocalEntities).mockResolvedValueOnce([hit, other]);
    const { props } = renderPalette();
    fireEvent.change(screen.getByRole("combobox"), { target: { value: hit.label } });
    const results = await screen.findAllByRole("option", { name: /fixture-router/ });
    expect(results).toHaveLength(2);
    const [result, otherResult] = results;
    expect(within(result).getByText("api-service")).toBeVisible();
    expect(within(otherResult).getByText("web-client")).toBeVisible();
    fireEvent.click(result);
    expect(props.onOpenEntity).toHaveBeenCalledWith(hit);
  });

  it("reveals the keyboard-selected option without moving focus out of search", () => {
    const { props } = renderPalette();
    const input = screen.getByRole("combobox", { name: "Search commands and local data" });
    input.focus();
    const review = screen.getByRole("option", { name: /Open Review/ });
    const reveal = vi.fn();
    review.scrollIntoView = reveal;
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-activedescendant", review.id);
    expect(review).toHaveAttribute("aria-selected", "true");
    expect(review).toHaveAttribute("tabindex", "-1");
    expect(reveal).toHaveBeenCalledWith({ block: "nearest" });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(props.onNavigate).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onNavigate).toHaveBeenCalledWith("review");
  });

  it("shows shortcut help and has no serious accessibility violations", async () => {
    const { container } = renderPalette();
    fireEvent.click(screen.getByText("Keyboard shortcuts"));
    expect(await screen.findByRole("heading", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(results.violations.filter((violation) => violation.impact === "critical" || violation.impact === "serious")).toEqual([]);
  });
});
