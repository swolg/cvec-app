import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_BOARD_PRESENTATION, DEFAULT_TABLE_PRESENTATION } from "@/lib/reviewViews";
import type { ReviewFinding } from "@/lib/types";
import { CvecReviewBoard } from "./CvecReviewBoard";
import { CvecReviewTable } from "./CvecReviewTable";

const items: ReviewFinding[] = [1, 2].map((id) => ({
  repoId: 1, repoName: "fixture", repoSourceType: "local", scanRunId: 10, scannedAt: null,
  finding: {
    fingerprint: `fp-${id}`, advisoryId: `CVE-TEST-${id}`, title: `Finding ${id}`,
    packageName: `package-${id}`, packageVersion: "1.0.0", packageType: "npm", packageUrl: null,
    severity: "HIGH", fixVersion: null, sourceUrl: null, state: "open", intel: null,
    reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null,
  },
  reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null,
  knownExploited: false, ransomwareKnown: false, materialChange: "new", priorityReasons: [],
}));

function selectionMessages() {
  return [...document.querySelectorAll('[data-live-announcer] [role="log"] > div')]
    .map((node) => node.textContent ?? "")
    .filter((text) => /^CVE-TEST-\d+ .* selected\.$/.test(text));
}

describe("Review selection announcements", () => {
  beforeEach(() => {
    // The real React Aria announcer lives outside the rendered component tree.
    document.querySelectorAll('[data-live-announcer] [role="log"]').forEach((log) => log.replaceChildren());
  });

  it.each(["table", "board"] as const)("announces genuine selection changes once across unrelated %s renders", async (surface) => {
    const tablePresentation = DEFAULT_TABLE_PRESENTATION;
    const boardPresentation = DEFAULT_BOARD_PRESENTATION;
    if (tablePresentation.mode !== "table" || boardPresentation.mode !== "board") throw new Error("Unexpected default review presentations");
    const onSelect = vi.fn();
    const shared = { items, total: items.length, hasMore: false, loading: false, onSelect, onLoadMore: vi.fn(), empty: null };
    const renderSurface = (selectedKey: string | null, loading = false) => surface === "table"
      ? <CvecReviewTable {...shared} selectedKey={selectedKey} loading={loading} presentation={tablePresentation} label="Test findings" initialScrollTop={0} onDismiss={vi.fn()} onSort={vi.fn()} onWidthsChange={vi.fn()} onScroll={vi.fn()} />
      : <CvecReviewBoard {...shared} selectedKey={selectedKey} loading={loading} presentation={boardPresentation} groupCounts={{ reviewState: [], severity: [], repository: [], fixAvailability: [] }} pendingKey={null} initialScrollLeft={0} onMove={vi.fn()} onScroll={vi.fn()} onToggleLane={vi.fn()} onLaneOrderChange={vi.fn()} />;

    const view = render(renderSurface(null));
    const first = await screen.findByRole("row", { name: /CVE-TEST-1/ });
    act(() => first.focus());
    expect(first).toHaveFocus();
    view.rerender(renderSurface("1:fp-1"));
    await waitFor(() => expect(selectionMessages()).toHaveLength(1));
    expect(first).toHaveAttribute("aria-selected", "true");

    for (let index = 0; index < 4; index++) {
      view.rerender(renderSurface("1:fp-1", index % 2 === 0));
    }
    expect(selectionMessages()).toHaveLength(1);

    view.rerender(renderSurface("1:fp-2"));
    await waitFor(() => expect(selectionMessages()).toHaveLength(2));
    expect(selectionMessages()[1]).toMatch(/^CVE-TEST-2 /);
    view.rerender(renderSurface(null));
    await waitFor(() => expect(selectionMessages()).toHaveLength(3));
    expect(selectionMessages()[2]).toMatch(/^CVE-TEST-2 .* not selected\.$/);
    view.rerender(renderSurface("1:fp-1"));
    await waitFor(() => expect(selectionMessages()).toHaveLength(4));
    expect(selectionMessages()[3]).toMatch(/^CVE-TEST-1 /);
  });
});
