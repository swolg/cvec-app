import { useEffect, useMemo, useRef, useState } from "react";

import { canonicalReviewFindings, nearestEligibleKey, reviewFindingKey, transitionReviewGroupCounts, type ReviewScrollAnchors } from "@/lib/reviewProjection";
import type { ReviewPresentation } from "@/lib/reviewViews";
import type { ReviewFinding, ReviewGroupCounts, ReviewPage, ReviewQuery } from "@/lib/types";

export function useReviewController(initial: { viewId: string; query: ReviewQuery; presentation: ReviewPresentation; selectedKey?: string; scrollTop?: number; inspectorPinned?: boolean; inspectorKey?: string; focusKey?: string; boardScrollLeft?: number }) {
  const [viewId, setViewId] = useState(initial.viewId);
  const [query, setQuery] = useState(initial.query);
  const [presentation, setPresentation] = useState<ReviewPresentation>(() => structuredClone(initial.presentation));
  const [items, setItems] = useState<ReviewFinding[]>([]);
  const [total, setTotal] = useState(0);
  const [groupCounts, setGroupCounts] = useState<ReviewGroupCounts | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedKey, setSelectedKey] = useState<string | null>(initial.selectedKey ?? null);
  const [focusKey, setFocusKey] = useState<string | null>(initial.focusKey ?? initial.selectedKey ?? null);
  const [inspectorKey, setInspectorKey] = useState<string | null>(initial.inspectorKey ?? (initial.inspectorPinned ? initial.selectedKey : null) ?? null);
  const [inspectorPinned, setInspectorPinned] = useState(initial.inspectorPinned ?? false);
  const [scrollTop, setScrollTop] = useState(initial.scrollTop ?? 0);
  const scrollCommitTimer = useRef<number | null>(null);
  const scrollAnchors = useRef<ReviewScrollAnchors>({ table: { key: initial.selectedKey ?? null, offset: initial.scrollTop ?? 0 }, board: { key: initial.selectedKey ?? null, offset: initial.boardScrollLeft ?? 0 } });
  const orderedKeys = useMemo(() => items.map(reviewFindingKey), [items]);

  function applyPage(page: ReviewPage, append: boolean) {
    setItems((current) => {
      const next = canonicalReviewFindings(append ? [...current, ...page.items] : page.items);
      setSelectedKey((key) => { if (!key) { setFocusKey(null); return null; } const eligible = nearestEligibleKey(current, next, key); setFocusKey(eligible); return eligible; });
      setInspectorKey((key) => key ? nearestEligibleKey(current, next, key) : null);
      return next;
    });
    setTotal(page.total);
    setGroupCounts(page.groupCounts);
    setNextCursor(page.nextCursor);
  }
  function applyLocalReviewFinding(nextItem: ReviewFinding, previousReviewState: ReviewFinding["reviewState"]) {
    const key = reviewFindingKey(nextItem);
    setItems((current) => current.map((item) => reviewFindingKey(item) === key ? nextItem : item));
    setGroupCounts((current) => transitionReviewGroupCounts(current, previousReviewState, nextItem.reviewState));
  }
  useEffect(() => () => { if (scrollCommitTimer.current !== null) window.clearTimeout(scrollCommitTimer.current); }, []);

  function captureTableScroll(offset: number) {
    scrollAnchors.current.table = { key: selectedKey, offset };
    if (scrollCommitTimer.current !== null) window.clearTimeout(scrollCommitTimer.current);
    scrollCommitTimer.current = window.setTimeout(() => { setScrollTop(offset); scrollCommitTimer.current = null; }, 120);
  }
  function captureBoardScroll(offset: number) { scrollAnchors.current.board = { key: selectedKey, offset }; }

  return { viewId, setViewId, query, setQuery, presentation, setPresentation, items, total, groupCounts, nextCursor, loading, setLoading, selectedKey, setSelectedKey, focusKey, setFocusKey, inspectorKey, setInspectorKey, inspectorPinned, setInspectorPinned, scrollTop, scrollAnchors, orderedKeys, applyPage, applyLocalReviewFinding, captureTableScroll, captureBoardScroll };
}
