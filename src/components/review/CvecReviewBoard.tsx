import { useEffect, useMemo, useRef, useState, type DragEvent as ReactDragEvent, type MouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode, type UIEvent } from "react";
import { createPortal } from "react-dom";
import { Button as AriaButton } from "react-aria-components/Button";
import { GridList, GridListItem } from "react-aria-components/GridList";
import { ListLayout, Virtualizer } from "react-aria-components/Virtualizer";
import { useDragAndDrop, type DragAndDropOptions } from "react-aria-components/useDragAndDrop";
import { CaretDown, CaretRight, CheckCircle, DotsSixVertical, DotsThree, GitBranch, GithubLogo } from "@phosphor-icons/react";

import { formatRelativeTimestamp } from "@/lib/format";
import { findingDisplayTitle, projectReviewLanes, reorderLanes, reviewFindingKey, type ReviewLane } from "@/lib/reviewProjection";
import type { ReviewPresentation } from "@/lib/reviewViews";
import type { ReviewFinding, ReviewGroupCounts, ReviewState } from "@/lib/types";
import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/overlays";
import { cn } from "@/lib/utils";

const CVEC_DRAG_TYPE = "application/x-cvec-review-finding";
const CVEC_REVIEW_STATE_DRAG_TYPE = "application/x-cvec-review-finding-state";
const POINTER_DRAG_THRESHOLD = 6;
const REVIEW_POINTER_PHASE_ATTRIBUTE = "data-review-pointer-phase";
type ReviewPointerPhase = "armed-card" | "armed-handle" | "dragging";
type PointerReviewDrag = {
  key: string;
  sourceReviewState: ReviewState;
  pointerId: number;
  startX: number;
  startY: number;
  offsetX: number;
  offsetY: number;
  previewWidth: number;
  active: boolean;
  hoveredLaneId: string | null;
  origin: "card" | "handle";
  originElement: HTMLElement;
  originWasDraggable: boolean;
  laneTargets: Array<{ laneId: string; left: number; right: number; top: number; bottom: number }>;
};
type PointerReviewPreview = { key: string; x: number; y: number; offsetX: number; offsetY: number; width: number };
const ROOT_ONLY_DROP_TARGET_DELEGATE: NonNullable<DragAndDropOptions<ReviewFinding>["dropTargetDelegate"]> = {
  getDropTargetFromPoint: (_x, _y, isValidDropTarget) => {
    const target = { type: "root" } as const;
    return isValidDropTarget(target) ? target : null;
  },
};

function reviewStateDragType(state: ReviewState): string {
  return `${CVEC_REVIEW_STATE_DRAG_TYPE}-${state}`;
}

function setReviewPointerPhase(phase: ReviewPointerPhase | null) {
  if (typeof document === "undefined") return;
  if (phase) document.documentElement.setAttribute(REVIEW_POINTER_PHASE_ATTRIBUTE, phase);
  else document.documentElement.removeAttribute(REVIEW_POINTER_PHASE_ATTRIBUTE);
}

function restoreNativeCardDrag(drag: PointerReviewDrag) {
  drag.originElement.draggable = drag.originWasDraggable;
}

export function createReviewDragItem(key: string, item: ReviewFinding): Record<string, string> {
  return {
    [CVEC_DRAG_TYPE]: key,
    [reviewStateDragType(item.reviewState)]: item.reviewState,
    "text/plain": key,
  };
}

export function createReviewLaneDropOperation(lane: ReviewLane): NonNullable<DragAndDropOptions<ReviewFinding>["getDropOperation"]> {
  return (target, types, allowedOperations) => {
    if (!lane.mutable || !lane.reviewState || target.type !== "root") return "cancel";
    if (!types.has(CVEC_DRAG_TYPE) || !allowedOperations.includes("move")) return "cancel";
    return types.has(reviewStateDragType(lane.reviewState)) ? "cancel" : "move";
  };
}

export function canAcceptPointerReviewDrop(lane: ReviewLane, drag: Pick<PointerReviewDrag, "sourceReviewState"> | null): boolean {
  if (!lane.mutable || !lane.reviewState) return false;
  return Boolean(drag && drag.sourceReviewState !== lane.reviewState);
}

export function CvecReviewBoard({ items, total, groupCounts, hasMore, loading, presentation, selectedKey, empty, pendingKey, initialScrollLeft, readOnly = false, onSelect, onMove, onToggleLane, onLaneOrderChange, onLoadMore, onScroll }: { items: ReviewFinding[]; total: number; groupCounts: ReviewGroupCounts | null; hasMore: boolean; loading: boolean; presentation: Extract<ReviewPresentation, { mode: "board" }>; selectedKey: string | null; empty: ReactNode; pendingKey: string | null; initialScrollLeft: number; readOnly?: boolean; onSelect: (key: string, mode: "preview" | "open") => void; onMove: (item: ReviewFinding, state: ReviewState) => Promise<void> | void; onToggleLane: (laneId: string) => void; onLaneOrderChange: (ids: string[]) => void; onLoadMore: () => void; onScroll: (left: number) => void }) {
  const boardRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const pointerDragRef = useRef<PointerReviewDrag | null>(null);
  const pointerPreviewElementRef = useRef<HTMLDivElement>(null);
  const pointerPreviewFrameRef = useRef<number | null>(null);
  const pointerOpenFrameRef = useRef<number | null>(null);
  const latestPointerPositionRef = useRef<{ x: number; y: number } | null>(null);
  const suppressCardClickUntilRef = useRef(0);
  const settleTimerRef = useRef<number | null>(null);
  const [activeDropLaneId, setActiveDropLaneId] = useState<string | null>(null);
  const [settledLaneId, setSettledLaneId] = useState<string | null>(null);
  const [pointerPreview, setPointerPreview] = useState<PointerReviewPreview | null>(null);
  const [pointerAnnouncement, setPointerAnnouncement] = useState("");
  const lanes = useMemo(() => {
    const projected = projectReviewLanes(items, presentation.grouping, groupCounts);
    return reorderLanes(readOnly ? projected.map((lane) => ({ ...lane, mutable: false })) : projected, presentation.laneOrder);
  }, [groupCounts, items, presentation.grouping, presentation.laneOrder, readOnly]);
  const byKey = useMemo(() => new Map(items.map((item) => [reviewFindingKey(item), item])), [items]);
  const lanesById = useMemo(() => new Map(lanes.map((lane) => [lane.id, lane])), [lanes]);
  const cardLayout = useMemo(() => reviewCardLayout(presentation), [presentation]);
  const usesFixedLaneRail = presentation.grouping === "repository";
  const laneTemplate = useMemo(() => lanes.map((lane) => presentation.collapsedLaneIds.includes(lane.id) ? "3.5rem" : usesFixedLaneRail ? "20rem" : "minmax(10rem, 1fr)").join(" "), [lanes, presentation.collapsedLaneIds, usesFixedLaneRail]);
  const settleDropLane = (laneId: string) => {
    setActiveDropLaneId(null);
    setSettledLaneId(laneId);
    if (settleTimerRef.current !== null) window.clearTimeout(settleTimerRef.current);
    settleTimerRef.current = window.setTimeout(() => { setSettledLaneId(null); settleTimerRef.current = null; }, 180);
  };
  useEffect(() => { if (railRef.current) railRef.current.scrollLeft = initialScrollLeft; }, []);
  useEffect(() => () => {
    if (settleTimerRef.current !== null) window.clearTimeout(settleTimerRef.current);
    if (pointerPreviewFrameRef.current !== null) window.cancelAnimationFrame(pointerPreviewFrameRef.current);
    if (pointerOpenFrameRef.current !== null) window.cancelAnimationFrame(pointerOpenFrameRef.current);
  }, []);
  useEffect(() => {
    const board = boardRef.current;
    if (!board || readOnly) return;
    const restoreOriginFocus = (drag: PointerReviewDrag) => requestAnimationFrame(() => {
      const currentCard = [...board.querySelectorAll<HTMLElement>("[role=row][data-review-drag-key]")]
        .find((candidate) => candidate.dataset.reviewDragKey === drag.key);
      if (currentCard) currentCard.focus();
      else if (drag.originElement.isConnected) drag.originElement.focus();
    });
    const releasePointer = (drag: PointerReviewDrag) => {
      try {
        if (board.hasPointerCapture?.(drag.pointerId)) board.releasePointerCapture(drag.pointerId);
      } catch {
        // The WebView may release capture before pointercancel/blur reaches us.
      }
    };
    const clearPointerDrag = () => {
      const drag = pointerDragRef.current;
      pointerDragRef.current = null;
      setReviewPointerPhase(null);
      latestPointerPositionRef.current = null;
      if (pointerPreviewFrameRef.current !== null) {
        window.cancelAnimationFrame(pointerPreviewFrameRef.current);
        pointerPreviewFrameRef.current = null;
      }
      setPointerPreview(null);
      setActiveDropLaneId(null);
      if (drag) {
        restoreNativeCardDrag(drag);
        releasePointer(drag);
      }
      return drag;
    };
    const laneAtPoint = (clientX: number, clientY: number, drag: PointerReviewDrag) => {
      const target = drag.laneTargets.find((candidate) => clientX >= candidate.left && clientX <= candidate.right && clientY >= candidate.top && clientY <= candidate.bottom);
      const lane = target ? lanesById.get(target.laneId) ?? null : null;
      return lane && canAcceptPointerReviewDrop(lane, drag) ? lane : null;
    };
    const captureLaneTargets = () => [...board.querySelectorAll<HTMLElement>("[data-review-lane-id]")].map((laneElement) => {
      const laneBounds = laneElement.getBoundingClientRect();
      return { laneId: laneElement.dataset.reviewLaneId ?? "", left: laneBounds.left, right: laneBounds.right, top: laneBounds.top, bottom: laneBounds.bottom };
    }).filter((targetLane) => targetLane.laneId.length > 0);
    const schedulePreviewPosition = (clientX: number, clientY: number) => {
      latestPointerPositionRef.current = { x: clientX, y: clientY };
      if (pointerPreviewFrameRef.current !== null) return;
      pointerPreviewFrameRef.current = window.requestAnimationFrame(() => {
        pointerPreviewFrameRef.current = null;
        const drag = pointerDragRef.current;
        const position = latestPointerPositionRef.current;
        const preview = pointerPreviewElementRef.current;
        if (!drag?.active || !position || !preview) return;
        preview.style.transform = `translate3d(${position.x - drag.offsetX}px, ${position.y - drag.offsetY}px, 0)`;
      });
    };
    const updateDestination = (clientX: number, clientY: number, drag: PointerReviewDrag) => {
      const lane = laneAtPoint(clientX, clientY, drag);
      if (drag.hoveredLaneId === lane?.id) return lane;
      drag.hoveredLaneId = lane?.id ?? null;
      setActiveDropLaneId(lane?.id ?? null);
      setPointerAnnouncement(lane ? `Release to move to ${lane.label}.` : "Move cancelled unless released over an eligible lane.");
      return lane;
    };
    const handlePointerMove = (event: PointerEvent) => {
      const drag = pointerDragRef.current;
      if (!drag || event.pointerId !== drag.pointerId) return;
      if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < POINTER_DRAG_THRESHOLD) return;
      event.preventDefault();
      if (!drag.active) {
        drag.active = true;
        setReviewPointerPhase("dragging");
        try {
          board.setPointerCapture?.(event.pointerId);
        } catch {
          // Window listeners still own the drag if WKWebView rejects pointer capture.
        }
        drag.laneTargets = captureLaneTargets();
        suppressCardClickUntilRef.current = performance.now() + 500;
        setPointerAnnouncement(`Moving ${byKey.get(drag.key)?.finding.advisoryId ?? "finding"}.`);
        setPointerPreview({ key: drag.key, x: event.clientX, y: event.clientY, offsetX: drag.offsetX, offsetY: drag.offsetY, width: drag.previewWidth });
      }
      updateDestination(event.clientX, event.clientY, drag);
      schedulePreviewPosition(event.clientX, event.clientY);
    };
    const handlePointerUp = (event: PointerEvent) => {
      const current = pointerDragRef.current;
      if (!current || event.pointerId !== current.pointerId) return;
      const drag = clearPointerDrag();
      if (!drag) return;
      if (!drag.active) {
        if (drag.origin === "handle") {
          restoreOriginFocus(drag);
          return;
        }
        suppressCardClickUntilRef.current = performance.now() + 500;
        if (pointerOpenFrameRef.current !== null) window.cancelAnimationFrame(pointerOpenFrameRef.current);
        pointerOpenFrameRef.current = window.requestAnimationFrame(() => {
          pointerOpenFrameRef.current = null;
          onSelect(drag.key, "open");
        });
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      const lane = laneAtPoint(event.clientX, event.clientY, drag);
      const item = byKey.get(drag.key);
      if (!lane?.reviewState || !item || !canAcceptPointerReviewDrop(lane, drag)) {
        setPointerAnnouncement("Move cancelled.");
        restoreOriginFocus(drag);
        return;
      }
      settleDropLane(lane.id);
      setPointerAnnouncement(`${item.finding.advisoryId} move requested for ${lane.label}.`);
      void onMove(item, lane.reviewState);
    };
    const cancelPointerDrag = () => {
      const drag = clearPointerDrag();
      if (!drag) return;
      if (drag.active) setPointerAnnouncement("Move cancelled.");
      restoreOriginFocus(drag);
    };
    const handlePointerCancel = (event: PointerEvent) => {
      if (pointerDragRef.current?.pointerId === event.pointerId) cancelPointerDrag();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !pointerDragRef.current) return;
      event.preventDefault();
      cancelPointerDrag();
    };
    window.addEventListener("pointermove", handlePointerMove, { capture: true, passive: false });
    window.addEventListener("pointerup", handlePointerUp, true);
    window.addEventListener("pointercancel", handlePointerCancel, true);
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("blur", cancelPointerDrag);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove, true);
      window.removeEventListener("pointerup", handlePointerUp, true);
      window.removeEventListener("pointercancel", handlePointerCancel, true);
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("blur", cancelPointerDrag);
      const drag = pointerDragRef.current;
      pointerDragRef.current = null;
      setReviewPointerPhase(null);
      if (drag) {
        restoreNativeCardDrag(drag);
        releasePointer(drag);
      }
    };
  }, [byKey, lanesById, onMove, onSelect, readOnly]);
  if (!items.length) return <>{empty}</>;
  const moveLane = (index: number, direction: -1 | 1) => { const target = Math.min(lanes.length - 1, Math.max(0, index + direction)); if (target === index) return; const ids = lanes.map((lane) => lane.id); [ids[index], ids[target]] = [ids[target], ids[index]]; onLaneOrderChange(ids); };
  const setDropLaneActive = (laneId: string, active: boolean) => setActiveDropLaneId((current) => active ? laneId : current === laneId ? null : current);
  const finishDrag = () => setActiveDropLaneId(null);
  const handlePointerDownCapture = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (readOnly || pointerDragRef.current) return;
    const target = event.target instanceof Element ? event.target : null;
    const handle = target?.closest<HTMLElement>("[data-review-pointer-handle]") ?? null;
    const interactiveTarget = target?.closest("button, a, input, select, textarea, [contenteditable='true']") ?? null;
    if ((interactiveTarget && !handle) || presentation.grouping !== "reviewState" || !event.isPrimary || event.button !== 0 || (event.pointerType !== "mouse" && event.pointerType !== "pen")) return;
    const card = target?.closest<HTMLElement>("[role='row'][data-review-drag-key]") ?? null;
    const key = card?.dataset.reviewDragKey ?? null;
    const item = key ? byKey.get(key) : null;
    if (!card || !item || pendingKey === key) return;
    const originWasDraggable = card.draggable;
    card.draggable = false;
    setReviewPointerPhase(handle ? "armed-handle" : "armed-card");
    const bounds = card.getBoundingClientRect();
    const previewWidth = Math.min(320, Math.max(160, bounds.width));
    pointerDragRef.current = {
      key: reviewFindingKey(item),
      sourceReviewState: item.reviewState,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      offsetX: Math.min(previewWidth - 16, Math.max(16, event.clientX - bounds.left)),
      offsetY: Math.min(bounds.height - 16, Math.max(16, event.clientY - bounds.top)),
      previewWidth,
      active: false,
      hoveredLaneId: null,
      origin: handle ? "handle" : "card",
      originElement: card,
      originWasDraggable,
      laneTargets: [],
    };
  };
  const handleNativeDragStartCapture = (event: ReactDragEvent<HTMLDivElement>) => {
    if (pointerDragRef.current) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  const handleBoardClickCapture = (event: MouseEvent<HTMLDivElement>) => {
    if (suppressCardClickUntilRef.current === 0 || performance.now() > suppressCardClickUntilRef.current) return;
    suppressCardClickUntilRef.current = 0;
    event.preventDefault();
    event.stopPropagation();
  };
  const previewItem = pointerPreview ? byKey.get(pointerPreview.key) ?? null : null;
  const board = <div ref={boardRef} className="flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-hairline bg-surface-2" aria-label="Review board" role="region" onClickCapture={handleBoardClickCapture} onPointerDownCapture={handlePointerDownCapture} onDragStartCapture={handleNativeDragStartCapture}><div ref={railRef} data-testid="review-board-rail" tabIndex={0} aria-label={usesFixedLaneRail ? "Repository lanes. Scroll horizontally to see more repositories." : "Board lanes"} className="review-board-rail min-h-0 min-w-0 flex-1 overflow-x-auto overflow-y-hidden p-2 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus" onScroll={(event: UIEvent<HTMLDivElement>) => onScroll(event.currentTarget.scrollLeft)}><div data-testid="review-board-lanes" className={cn("grid h-full min-w-full gap-2", usesFixedLaneRail && "w-max snap-x snap-proximity")} style={{ gridTemplateColumns: laneTemplate }}>{lanes.map((lane, index) => <BoardLane key={lane.id} lane={lane} allItems={byKey} cardLayout={cardLayout} selectedKey={selectedKey} pendingKey={pendingKey} pointerDraggingKey={pointerPreview?.key ?? null} previewMagnetized={activeDropLaneId !== null} isMagneticTarget={activeDropLaneId === lane.id} isSettling={settledLaneId === lane.id} collapsed={presentation.collapsedLaneIds.includes(lane.id)} onSelect={onSelect} onMove={onMove} onDragEnd={finishDrag} onDropTargetChange={setDropLaneActive} onDropCommit={settleDropLane} onToggle={() => onToggleLane(lane.id)} onMoveLane={(direction) => moveLane(index, direction)} canMoveEarlier={index > 0} canMoveLater={index < lanes.length - 1} />)}</div></div>{hasMore ? <div className="flex shrink-0 items-center justify-between gap-3 border-t border-hairline bg-surface-1 px-3 py-2"><p className="text-xs text-ink-subtle"><span className="font-medium text-ink">{items.length}</span> of {total} findings loaded</p><Button size="sm" variant="secondary" disabled={loading} onClick={onLoadMore}>{loading ? "Loading…" : "Load more findings"}</Button></div> : null}<span className="sr-only" aria-live="polite">{pointerAnnouncement}</span></div>;
  const preview = pointerPreview && previewItem && typeof document !== "undefined" ? createPortal(<div ref={pointerPreviewElementRef} data-testid="review-pointer-drag-preview" aria-hidden className="pointer-events-none fixed left-0 top-0 z-[100] will-change-transform" style={{ width: pointerPreview.width, transform: `translate3d(${pointerPreview.x - pointerPreview.offsetX}px, ${pointerPreview.y - pointerPreview.offsetY}px, 0)` }}><ReviewCardPreview item={previewItem} layout={cardLayout} magnetized={activeDropLaneId !== null} /></div>, document.body) : null;
  return <>{board}{preview}</>;
}

function BoardLane({ lane, allItems, cardLayout, selectedKey, pendingKey, pointerDraggingKey, previewMagnetized, isMagneticTarget, isSettling, collapsed, onSelect, onMove, onDragEnd, onDropTargetChange, onDropCommit, onToggle, onMoveLane, canMoveEarlier, canMoveLater }: { lane: ReviewLane; allItems: Map<string, ReviewFinding>; cardLayout: ReviewCardLayout; selectedKey: string | null; pendingKey: string | null; pointerDraggingKey: string | null; previewMagnetized: boolean; isMagneticTarget: boolean; isSettling: boolean; collapsed: boolean; onSelect: (key: string, mode: "preview" | "open") => void; onMove: (item: ReviewFinding, state: ReviewState) => Promise<void> | void; onDragEnd: () => void; onDropTargetChange: (laneId: string, active: boolean) => void; onDropCommit: (laneId: string) => void; onToggle: () => void; onMoveLane: (direction: -1 | 1) => void; canMoveEarlier: boolean; canMoveLater: boolean }) {
  const [announcement, setAnnouncement] = useState("");
  const moveDroppedItem = async (key: string) => {
    if (!lane.mutable || !lane.reviewState) return;
    const item = allItems.get(key);
    if (!item || item.reviewState === lane.reviewState) { setAnnouncement("Move cancelled."); return; }
    onDropCommit(lane.id);
    setAnnouncement(`Moving ${item.finding.advisoryId} to ${lane.label}.`);
    await onMove(item, lane.reviewState);
    setAnnouncement(`${item.finding.advisoryId} move requested for ${lane.label}.`);
  };
  const dragAndDrop = useDragAndDrop<ReviewFinding>({
    getItems: (keys) => [...keys].map((key) => {
      const itemKey = String(key);
      const item = allItems.get(itemKey);
      return item ? createReviewDragItem(itemKey, item) : { "text/plain": itemKey };
    }),
    getAllowedDropOperations: () => ["move"],
    onDragEnd,
    renderDragPreview: (dragItems) => {
      const key = dragItems[0]?.[CVEC_DRAG_TYPE];
      const item = key ? allItems.get(key) : null;
      return item ? <div className="w-80 max-w-[calc(100vw-2rem)]"><ReviewCardPreview item={item} layout={cardLayout} magnetized={previewMagnetized} /></div> : <div className="rounded-lg border border-navigation-active-border bg-surface-1 px-3 py-2 shadow-2xl">{key}</div>;
    },
    acceptedDragTypes: [CVEC_DRAG_TYPE],
    dropTargetDelegate: ROOT_ONLY_DROP_TARGET_DELEGATE,
    getDropOperation: createReviewLaneDropOperation(lane),
    onDropEnter: () => onDropTargetChange(lane.id, true),
    onDropExit: () => onDropTargetChange(lane.id, false),
    onDrop: async (event) => {
      if (!lane.mutable || !lane.reviewState) return;
      const dropped = event.items.find((item) => item.kind === "text" && item.types.has(CVEC_DRAG_TYPE));
      if (!dropped || dropped.kind !== "text") { setAnnouncement("Unsupported external drop rejected."); return; }
      const key = await dropped.getText(CVEC_DRAG_TYPE);
      await moveDroppedItem(key);
    },
  });
  return (
    <section
      data-review-lane-id={lane.id}
      data-magnetic-target={isMagneticTarget || undefined}
      data-drop-settling={isSettling || undefined}
      className={cn(
        "flex h-full min-w-0 snap-start flex-col overflow-hidden rounded-lg border border-hairline bg-surface-1 transition-[border-color,box-shadow,transform] duration-150 ease-out motion-reduce:transform-none motion-reduce:transition-none",
        isMagneticTarget && "-translate-y-0.5 scale-[1.005] border-primary shadow-lg ring-2 ring-primary-focus/50",
        isSettling && "scale-[0.995] border-primary shadow-md ring-2 ring-primary-focus/30",
      )}
      aria-label={lane.label + ", " + lane.totalCount + (lane.totalCount === 1 ? " finding" : " findings")}
    >
      <div className={cn("flex h-10 shrink-0 items-center gap-1 border-b border-hairline bg-control-surface px-2 transition-colors motion-reduce:transition-none", isMagneticTarget && "border-primary bg-row-selected-surface")}>
        <button type="button" onClick={onToggle} className="flex min-h-8 min-w-0 flex-1 items-center gap-1.5 rounded px-1 text-left text-sm font-semibold text-ink outline-none focus-visible:ring-2 focus-visible:ring-primary-focus" aria-expanded={!collapsed} aria-label={`${collapsed ? "Expand" : "Collapse"} ${lane.label}`}>
          <AppIcon className="size-3.5 shrink-0 text-ink-subtle" icon={collapsed ? CaretRight : CaretDown} />
          <span className={cn("truncate", collapsed && "sr-only")}>{lane.label}</span>
        </button>
        {!collapsed ? (
          <>
            <Badge variant="outline">{lane.totalCount}</Badge>
            <LaneOptions laneLabel={lane.label} canMoveEarlier={canMoveEarlier} canMoveLater={canMoveLater} onMoveLane={onMoveLane} onCollapse={onToggle} />
          </>
        ) : null}
      </div>
      {!collapsed ? (
        <>
          {!lane.mutable ? <p className="border-b border-hairline px-3 py-1.5 text-xs font-medium text-ink-tertiary">Read-only grouping</p> : null}
          <LaneGrid
            lane={lane}
            cardLayout={cardLayout}
            selectedKey={selectedKey}
            pendingKey={pendingKey}
            pointerDraggingKey={pointerDraggingKey}
            isMagneticTarget={isMagneticTarget}
            dragAndDropHooks={dragAndDrop.dragAndDropHooks}
            onSelect={onSelect}
            onMove={onMove}
          />
        </>
      ) : null}
      <span className="sr-only" aria-live="polite">{announcement}</span>
    </section>
  );
}

function LaneOptions({ laneLabel, canMoveEarlier, canMoveLater, onMoveLane, onCollapse }: { laneLabel: string; canMoveEarlier: boolean; canMoveLater: boolean; onMoveLane: (direction: -1 | 1) => void; onCollapse: () => void }) {
  return <Popover><PopoverTrigger render={<IconButton aria-label={`${laneLabel} lane options`} size="icon-xs" variant="ghost" />}><AppIcon className="size-4" icon={DotsThree} weight="bold" /></PopoverTrigger><PopoverContent align="end" className="w-48 p-1.5"><PopoverTitle className="sr-only">{laneLabel} lane options</PopoverTitle><Button className="w-full justify-start" size="sm" variant="ghost" disabled={!canMoveEarlier} onClick={() => onMoveLane(-1)}>Move lane left</Button><Button className="w-full justify-start" size="sm" variant="ghost" disabled={!canMoveLater} onClick={() => onMoveLane(1)}>Move lane right</Button><Button className="w-full justify-start" size="sm" variant="ghost" onClick={onCollapse}>Collapse lane</Button></PopoverContent></Popover>;
}

function LaneEmpty({ lane, isDropTarget = false }: { lane: ReviewLane; isDropTarget?: boolean }) {
  const hasUnloadedFindings = lane.totalCount > 0;
  const label = hasUnloadedFindings ? `${lane.totalCount} findings in this lane` : lane.mutable ? `Drop into ${lane.label}` : "No findings";
  return (
    <div className={cn(
      "flex min-h-16 flex-col items-center justify-center rounded-md border border-dashed border-hairline-strong bg-surface-2 p-3 text-center transition-[background-color,border-color,box-shadow]",
      isDropTarget && "border-primary bg-row-selected-surface ring-2 ring-inset ring-primary-focus",
    )}>
      <p className="text-sm font-medium text-ink">{isDropTarget ? `Release to move to ${lane.label}` : label}</p>
      {hasUnloadedFindings && !isDropTarget ? <p className="mt-1 text-xs text-ink-subtle">Load more to reveal cards.</p> : null}
    </div>
  );
}

function LaneGrid({ lane, cardLayout, selectedKey, pendingKey, pointerDraggingKey, isMagneticTarget, dragAndDropHooks, onSelect, onMove }: {
  lane: ReviewLane;
  cardLayout: ReviewCardLayout;
  selectedKey: string | null;
  pendingKey: string | null;
  pointerDraggingKey: string | null;
  isMagneticTarget: boolean;
  dragAndDropHooks: ReturnType<typeof useDragAndDrop<ReviewFinding>>["dragAndDropHooks"];
  onSelect: (key: string, mode: "preview" | "open") => void;
  onMove: (item: ReviewFinding, state: ReviewState) => Promise<void> | void;
}) {
  const validKeys = useMemo(() => new Set(lane.items.map(reviewFindingKey)), [lane.items]);
  // Keep unchanged selections from being re-announced on lane updates.
  const selectedKeys = useMemo(() => new Set(selectedKey ? [selectedKey] : []), [selectedKey]);
  const virtualized = lane.items.length > 40;
  const grid = (
    <GridList
      aria-label={`${lane.label} findings`}
      items={lane.items}
      dependencies={[cardLayout, lane.mutable, pendingKey, pointerDraggingKey, virtualized]}
      selectionMode="single"
      selectionBehavior="replace"
      selectedKeys={selectedKeys}
      dragAndDropHooks={lane.mutable ? dragAndDropHooks : undefined}
      renderEmptyState={({ isDropTarget }) => <LaneEmpty lane={lane} isDropTarget={isDropTarget || isMagneticTarget} />}
      className={({ isDropTarget }) => cn(
        "h-full min-w-0 w-full overflow-x-hidden overflow-y-auto rounded-md outline-none transition-[background-color,box-shadow]",
        (isDropTarget || isMagneticTarget) && "bg-row-selected-surface ring-2 ring-inset ring-primary-focus",
      )}
      onSelectionChange={(keys) => {
        if (keys === "all") return;
        const key = [...keys][0];
        if (key != null) onSelect(String(key), "preview");
      }}
    >
      {(item) => {
        const key = reviewFindingKey(item);
        const isPending = pendingKey === key;
        const isPointerDragging = pointerDraggingKey === key;
        return (
          <GridListItem
            id={key}
            data-review-drag-key={key}
            textValue={item.finding.advisoryId + " " + (findingDisplayTitle(item.finding) ?? item.finding.packageName)}
            onAction={() => onSelect(key, "open")}
            style={{ height: cardLayout.height }}
            render={(rowProps, { isDragging }) => (
              <ReviewCardRow
                rowProps={rowProps}
                title={reviewCardTitle(item)}
                previewTitle={!isPending && !isPointerDragging && !isDragging}
              />
            )}
            className={({ isSelected, isFocusVisible, isDropTarget, isDragging }) => {
              const isMoving = isDragging || isPointerDragging;
              return cn(
                "group/card relative flex flex-col overflow-hidden rounded-lg border border-hairline bg-surface-1 outline-none transition-[background-color,border-color,box-shadow,opacity,transform] hover:border-hairline-strong hover:bg-row-hover-surface",
                !virtualized && "mb-2",
                cardLayout.padding,
                isSelected && !isMoving && "review-board-card-selected",
                isFocusVisible && "ring-2 ring-primary-focus ring-offset-1",
                isDropTarget && "border-primary bg-row-selected-surface ring-2 ring-primary-focus",
                isMoving ? "cursor-grabbing border-dashed border-primary bg-row-hover-surface shadow-inner" : isPending ? "cursor-wait opacity-60" : "cursor-pointer",
              );
            }}
          >
            {({ isDragging }) => isDragging || isPointerDragging ? (
              <div aria-hidden className="flex h-full items-center justify-center">
                <span className="rounded-full border border-hairline-strong bg-surface-1 px-2.5 py-1 text-xs font-medium text-ink-subtle">Moving…</span>
              </div>
            ) : (
              <ReviewCardContent
                item={item}
                layout={cardLayout}
                actions={lane.mutable ? (
                  <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/card:opacity-100 group-focus-within/card:opacity-100">
                    <CardActions item={item} onMove={onMove} />
                    <div data-review-pointer-handle data-review-drag-key={key} className={cn("rounded", isPending ? "cursor-wait" : "cursor-grab active:cursor-grabbing")} onClick={(event) => event.stopPropagation()}>
                      <AriaButton
                        slot="drag"
                        data-review-drag-key={key}
                        aria-label={`Drag ${item.finding.advisoryId}`}
                        isDisabled={isPending}
                        isPending={isPending}
                        className={({ isPending }) => cn(
                          "flex size-8 shrink-0 cursor-grab items-center justify-center rounded text-ink-tertiary outline-none active:cursor-grabbing hover:bg-row-hover-surface hover:text-ink-subtle focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus",
                          isPending && "cursor-wait opacity-45",
                        )}
                        onClick={(event) => event.stopPropagation()}
                      >
                        <AppIcon className="size-4" icon={DotsSixVertical} />
                      </AriaButton>
                    </div>
                  </div>
                ) : undefined}
              />
            )}
          </GridListItem>
        );
      }}
    </GridList>
  );
  const handleClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!(event.target instanceof HTMLElement) || event.target.closest("button, a, input, select, textarea")) return;
    const key = event.target.closest<HTMLElement>("[role=row][data-key]")?.dataset.key;
    if (key && validKeys.has(key)) onSelect(key, "open");
  };
  return (
    <div className="relative min-h-0 flex-1 overflow-hidden p-2" onClickCapture={handleClick}>
      {isMagneticTarget && lane.items.length > 0 ? (
        <div aria-hidden className="pointer-events-none absolute inset-x-3 top-3 z-20 flex justify-center">
          <span className="rounded-full border border-primary bg-surface-1/95 px-3 py-1.5 text-xs font-semibold text-primary shadow-md">Release → {lane.label}</span>
        </div>
      ) : null}
      {virtualized ? <Virtualizer layout={ListLayout} layoutOptions={{ rowHeight: cardLayout.height, gap: 8 }}>{grid}</Virtualizer> : grid}
    </div>
  );
}

function ReviewCardRow({ rowProps, title, previewTitle }: { rowProps: React.ComponentProps<"div">; title: string; previewTitle: boolean }) {
  return (
    <Tooltip disabled={!previewTitle} onOpenChange={(open, details) => {
      if (!open) return;
      const heading = details.trigger?.querySelector<HTMLElement>("[data-review-card-title]");
      if (!heading || heading.scrollHeight <= heading.clientHeight) details.cancel();
    }}>
      <TooltipTrigger render={<div {...rowProps} />} />
      <TooltipContent aria-hidden="true" className="max-w-sm [overflow-wrap:anywhere]">{title}</TooltipContent>
    </Tooltip>
  );
}

function reviewCardLayout(presentation: Extract<ReviewPresentation, { mode: "board" }>) {
  const fields = presentation.cardFields;
  const compact = presentation.density === "compact";
  const showPackage = fields.includes("package");
  const showFix = fields.includes("fix");
  const extraFields = fields.filter((field) => field === "reviewState" || field === "scannerState" || field === "scannedAt");
  // Allow one or two full title lines and reserve room for every enabled field.
  // Cards, the virtualizer, and the drag preview share the same height.
  const baseHeight = compact ? 84 : 112;
  const footerHeight = showPackage || showFix ? (compact ? 24 : 26) : 0;
  return {
    density: presentation.density,
    padding: compact ? "px-2.5 py-1.5" : "p-2.5",
    height: baseHeight + extraFields.length * 24 + footerHeight,
    extraFields,
    showRepository: fields.includes("repository"),
    showPackage,
    showFix,
  };
}

type ReviewCardLayout = ReturnType<typeof reviewCardLayout>;

function ReviewCardPreview({ item, layout, magnetized }: { item: ReviewFinding; layout: ReviewCardLayout; magnetized: boolean }) {
  return (
    <div
      aria-label={`Moving ${item.finding.advisoryId}`}
      data-magnetized={magnetized || undefined}
      style={{ height: layout.height }}
      className={cn(
        "flex w-full origin-center cursor-grabbing flex-col overflow-hidden rounded-lg border border-navigation-active-border bg-surface-1 shadow-2xl ring-1 ring-primary-focus/40 transition-[border-color,box-shadow,transform] duration-150 ease-out motion-reduce:transform-none motion-reduce:transition-none",
        layout.padding,
        magnetized ? "scale-[0.985] border-primary shadow-xl ring-2 ring-primary-focus" : "scale-[1.02] rotate-[0.5deg]",
      )}
    >
      <ReviewCardContent item={item} layout={layout} />
    </div>
  );
}

function ReviewCardContent({ item, layout, actions }: { item: ReviewFinding; layout: ReviewCardLayout; actions?: ReactNode }) {
  const severity = item.finding.severity;
  return (
    <>
      <div className="flex h-8 shrink-0 items-center justify-between gap-1">
        <Badge variant={severity === "CRITICAL" ? "destructive" : severity === "HIGH" ? "warning" : "secondary"}>{severity}</Badge>
        {actions}
      </div>
      <CardFinding item={item} density={layout.density} showRepository={layout.showRepository} />
      {layout.extraFields.map((field) => <CardField key={field} field={field} item={item} />)}
      <CardFooter item={item} showPackage={layout.showPackage} showFix={layout.showFix} />
    </>
  );
}

function CardActions({ item, onMove }: { item: ReviewFinding; onMove: (item: ReviewFinding, state: ReviewState) => Promise<void> | void }) {
  const actions: { state: ReviewState; label: string }[] = [{ state: "unreviewed", label: "Move to Needs review" }, { state: "reviewed", label: "Move to Reviewed" }, { state: "snoozed", label: "Snooze…" }];
  return <div onClick={(event) => event.stopPropagation()} onPointerDown={(event) => event.stopPropagation()}><Popover><PopoverTrigger render={<IconButton aria-label={`Actions for ${item.finding.advisoryId}`} size="icon-xs" variant="ghost" />}><AppIcon className="size-4" icon={DotsThree} weight="bold" /></PopoverTrigger><PopoverContent align="end" className="w-52 p-1.5"><PopoverTitle className="sr-only">Actions for {item.finding.advisoryId}</PopoverTitle>{actions.filter((action) => action.state !== item.reviewState).map((action) => <Button key={action.state} className="w-full justify-start" size="sm" variant="ghost" onClick={() => void onMove(item, action.state)}>{action.label}</Button>)}</PopoverContent></Popover></div>;
}

function CardFinding({ item, density, showRepository }: { item: ReviewFinding; density: "compact" | "comfortable"; showRepository: boolean }) {
  const title = reviewCardTitle(item);
  const showAdvisoryId = title !== item.finding.advisoryId;
  return <div className="shrink-0 overflow-hidden">
    <p data-review-card-title className={cn("break-words text-sm font-semibold leading-5 text-ink", density === "compact" ? "line-clamp-1" : "line-clamp-2")}>{title}</p>
    {showAdvisoryId || showRepository ? <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs leading-4 text-ink-subtle">
      {showAdvisoryId ? <span className={cn("shrink-0 truncate font-medium text-ink-muted", showRepository ? "max-w-3/5" : "max-w-full")} title={item.finding.advisoryId}>{item.finding.advisoryId}</span> : null}
      {showRepository ? <>{showAdvisoryId ? <span aria-hidden className="text-ink-tertiary">·</span> : null}<span className="flex min-w-0 items-center gap-1"><AppIcon aria-label={reviewRepositorySourceLabel(item.repoSourceType)} className="size-3.5 text-ink-tertiary" data-repository-source={item.repoSourceType} data-testid="review-card-repository-icon" icon={reviewRepositoryIcon(item.repoSourceType)} role="img" /><span className="truncate">{item.repoName}</span></span></> : null}
    </div> : null}
  </div>;
}

function reviewCardTitle(item: ReviewFinding): string {
  return findingDisplayTitle(item.finding) ?? item.finding.advisoryId;
}

function reviewRepositoryIcon(sourceType: ReviewFinding["repoSourceType"]) {
  return sourceType === "github" ? GithubLogo : GitBranch;
}

function reviewRepositorySourceLabel(sourceType: ReviewFinding["repoSourceType"]): string {
  if (sourceType === "github") return "GitHub repository";
  if (sourceType === "local") return "Local repository";
  return "Repository";
}

function CardField({ field, item }: { field: ReviewCardLayout["extraFields"][number]; item: ReviewFinding }) {
  if (field === "reviewState") return <div className="mt-1 flex h-5 shrink-0 items-center"><Badge variant="outline">{item.reviewState}</Badge></div>;
  if (field === "scannerState") return <div className="mt-1 flex h-5 shrink-0 items-center"><Badge variant="outline">{item.finding.state}</Badge></div>;
  return <p className="mt-1 shrink-0 text-xs leading-5 text-ink-tertiary">{item.scannedAt ? formatRelativeTimestamp(item.scannedAt) : "Not scanned"}</p>;
}

function CardFooter({ item, showPackage, showFix }: { item: ReviewFinding; showPackage: boolean; showFix: boolean }) {
  if (!showPackage && !showFix) return null;
  const { packageName, packageVersion, fixVersion } = item.finding;
  return (
    <p className="mt-auto flex min-w-0 shrink-0 items-center gap-1.5 border-t border-hairline pt-0.5 text-xs leading-4 text-ink-muted">
      {showPackage ? (
        <>
          <span className="min-w-0 max-w-28 truncate font-medium text-ink">{packageName}</span>
          <span className="min-w-0 truncate"><span className="sr-only">Installed </span>{packageVersion ?? "unknown"}</span>
        </>
      ) : null}
      {showFix ? fixVersion ? (
        <>
          {showPackage ? <span className="shrink-0 text-ink-tertiary" aria-hidden>→</span> : null}
          <span className="min-w-0 truncate font-medium text-status-success"><span className="sr-only">Fixed in </span>{fixVersion}</span>
          <span className="ml-auto inline-flex shrink-0 items-center text-status-success" title="Fix available">
            <AppIcon aria-hidden className="size-3.5" icon={CheckCircle} weight="fill" />
            <span className="sr-only">Fix available</span>
          </span>
        </>
      ) : <span className="truncate text-ink-tertiary">No fix listed</span> : null}
    </p>
  );
}
