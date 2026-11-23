import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowSquareOut, ArrowUp, CaretLeft, CaretRight, Check, Clock, Copy, DotsThree, Funnel, House, PencilSimple, FloppyDisk, ShieldCheckered, Star, StarHalf, Trash, X } from "@phosphor-icons/react";

import { getReviewFindingDetail, getFindingRemediation, listReviewFindings, markFindingReviewed, openFindingManifest, returnFindingToReview, snoozeFinding } from "@/lib/api";
import { BUILT_IN_REVIEW_VIEWS, DASHBOARD_REVIEW_VIEWS, DEFAULT_TABLE_PRESENTATION, allReviewViews, createCustomView, deleteCustomView, reorderFavoriteView, reviewQuery, setDefaultView, toggleFavoriteView, updateCustomView, updateReviewSort, type ReviewPresentation, type ReviewViewDefinition, type ReviewViewsState } from "@/lib/reviewViews";
import { findingDisplayTitle, isStaleReviewEvidence, reviewActionBasis, reviewFindingKey, transitionReviewFinding } from "@/lib/reviewProjection";
import { reviewSessionCache } from "@/lib/reviewPageCache";
import { remediationProjectionCache } from "@/lib/remediationProjectionCache";
import { useReviewController } from "@/hooks/useReviewController";
import type { FindingChatRequest, FindingRemediationProjection, ReviewFinding, ReviewFilters, ReviewQuery, ReviewSort, ReviewState, ReviewUpdate, Severity } from "@/lib/types";
import { formatRelativeTimestamp } from "@/lib/format";
import { AppIcon } from "./ui/app-icon";
import { Badge } from "./ui/badge";
import { Button, IconButton } from "./ui/button";
import { EmptyState, Spinner } from "./ui/feedback";
import { Input, SearchField, Select } from "./ui/form-controls";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "./ui/overlays";
import { WorkList, type WorkListColumn } from "./ui/work-list";
import { FindingInspectorBody } from "./FindingInspectorBody";
import type { WorkspaceContext } from "@/lib/navigation";
import { CvecReviewTable } from "./review/CvecReviewTable";
import { CvecReviewBoard } from "./review/CvecReviewBoard";
import { ReviewPresentationControls } from "./review/ReviewPresentationControls";

const REVIEW_SESSION_CACHE_ROWS = 2000;

interface ReviewWorkspaceProps {
  viewsState: ReviewViewsState;
  onViewsStateChange: (state: ReviewViewsState) => void;
  initialViewId?: string;
  initialContext?: WorkspaceContext;
  dataRevision?: number;
  reviewRevision?: number;
  onContextChange?: (context: WorkspaceContext) => void;
  onReviewStateChange?: (update: ReviewUpdate | null) => Promise<void> | void;
  scannerReady: boolean;
  onOpenRepo: (repoId: number) => void;
  onOpenUrl: (url: string) => void;
  onContinueInChat?: (finding: FindingChatRequest) => void;
  onError: (message: string) => void;
}

export function ReviewWorkspace({ viewsState, onViewsStateChange, initialViewId, initialContext, dataRevision = 0, reviewRevision = 0, onContextChange, onReviewStateChange, scannerReady, onOpenRepo, onOpenUrl, onContinueInChat, onError }: ReviewWorkspaceProps) {
  const inspectorHeading = useRef<HTMLHeadingElement>(null);
  const [entryViewId] = useState(initialContext?.viewId ?? initialViewId);
  const entryRepoId = initialContext?.reviewQuery?.filters.repoIds[0];
  const views = useMemo<ReviewViewDefinition[]>(() => {
    const base = allReviewViews(viewsState);
    const attentionView = DASHBOARD_REVIEW_VIEWS.find((view) => view.id === entryViewId);
    if (attentionView) return [...base, attentionView];
    if (!entryViewId) return base;
    const [kind, rawIds] = entryViewId.split(":");
    if (!rawIds || !["scan", "scan-resolved", "scan-resolution"].includes(kind)) return base;
    const runIds = rawIds.split(",").map(Number).filter((value) => Number.isSafeInteger(value) && value > 0);
    if (!runIds.length) return base;
    if (kind === "scan-resolution") {
      if (runIds.length !== 1) return base;
      const runId = runIds[0];
      return [...base, {
        id: entryViewId,
        name: `Resolved in scan #${runId}`,
        query: reviewQuery({
          resolvedInScanRunId: runId,
          scannerStates: ["resolved"],
          repoIds: entryRepoId === undefined ? [] : [entryRepoId],
        }),
        presentation: { ...structuredClone(DEFAULT_TABLE_PRESENTATION), fields: ["severity", "finding", "package", "repository"] },
        builtIn: true,
      }];
    }
    const resolved = kind === "scan-resolved";
    return [...base, {
      id: entryViewId,
      name: resolved ? "Resolved in this scan" : "Scan changes",
      query: resolved
        ? reviewQuery({ scanResolvedRunIds: runIds, scannerStates: ["resolved"] }, "recent")
        : reviewQuery({ scanChangeRunIds: runIds, scannerStates: ["new", "open"] }),
      presentation: structuredClone(resolved ? DEFAULT_TABLE_PRESENTATION : BUILT_IN_REVIEW_VIEWS[0].presentation),
      builtIn: true,
    }];
  }, [entryViewId, entryRepoId, viewsState]);
  const restoredViewId = initialContext?.viewId;
  const initialResolvedViewId = restoredViewId && views.some((view) => view.id === restoredViewId) ? restoredViewId : initialViewId && views.some((view) => view.id === initialViewId) ? initialViewId : viewsState.defaultViewId;
  const initialActiveView = views.find((view) => view.id === initialResolvedViewId) ?? BUILT_IN_REVIEW_VIEWS[0];
  const initialQuery = initialContext?.reviewQuery ?? initialActiveView.query;
  const initialGroup = initialContext?.remediationGroup;
  const { viewId, setViewId, query, setQuery, presentation, setPresentation, items, total, groupCounts, nextCursor, loading, setLoading, selectedKey, setSelectedKey, focusKey, setFocusKey, inspectorKey, setInspectorKey, inspectorPinned, setInspectorPinned, scrollTop, scrollAnchors, applyPage, applyLocalReviewFinding, captureTableScroll, captureBoardScroll } = useReviewController({ viewId: initialResolvedViewId, query: initialGroup ? { ...initialQuery, filters: { ...initialQuery.filters, packageIdentity: initialGroup } } : initialQuery, presentation: initialContext?.reviewPresentation ?? initialActiveView.presentation, selectedKey: initialContext?.selectedKey, scrollTop: initialContext?.scrollTop, inspectorPinned:initialContext?.inspectorPinned,inspectorKey:initialContext?.inspectorKey,focusKey:initialContext?.focusKey,boardScrollLeft:initialContext?.boardScrollLeft });
  const activeView = views.find((view) => view.id === viewId) ?? BUILT_IN_REVIEW_VIEWS[0];
  const [searchValue, setSearchValue] = useState(initialContext?.reviewQuery?.search ?? activeView.query.search ?? "");
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [snoozeOpen, setSnoozeOpen] = useState(false);
  const [snoozeUntil, setSnoozeUntil] = useState(defaultSnoozeValue());
  const [snoozeNote, setSnoozeNote] = useState("");
  const [mutating, setMutating] = useState(false);
  const [pendingMoveKey, setPendingMoveKey] = useState<string | null>(null);
  const [requestedItemUnavailable, setRequestedItemUnavailable] = useState(false);
  const [snoozeTarget, setSnoozeTarget] = useState<ReviewFinding | null>(null);
  const loadedViewId = useRef(initialResolvedViewId);
  const ephemeralSelection = useRef(initialContext?.ephemeralReviewContext && initialContext.inspectorPinned === undefined
    ? initialContext.selectedKey ?? null
    : null);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const searchField = useRef<HTMLInputElement>(null);
  const inspectorOriginKey = useRef(initialContext?.focusKey ?? initialContext?.selectedKey ?? null);
  const presentationCache = useRef<Partial<Record<"table" | "board", ReviewPresentation>>>({ [initialActiveView.presentation.mode]: structuredClone(initialActiveView.presentation) });
  const searchTimer = useRef<number | null>(null);
  const pageCache = useMemo(() => reviewSessionCache(dataRevision), [dataRevision]);
  const loadSequence = useRef(0);
  const activeLoad = useRef<number | null>(null);
  const cacheRevision = useRef(dataRevision);
  const mutationInFlight = useRef(false);
  const deferredReload = useRef(false);
  const [reloadRevision, setReloadRevision] = useState(0);
  const appliedPage = useRef<{ query: ReviewQuery; dataRevision: number; reloadRevision: number; revision: number } | null>(null);
  const loadedEvidenceRevision = appliedPage.current?.dataRevision ?? dataRevision;
  const [reviewConflict, setReviewConflict] = useState<"refreshing" | "refreshed" | null>(null);
  const refreshingEvidence = loadedEvidenceRevision !== dataRevision || reviewConflict === "refreshing";
  const [loadError, setLoadError] = useState(false);
  const [unavailableInspection, setUnavailableInspection] = useState<ReviewQuery | null>(null);
  const currentSelection = useRef({ selectedKey, inspectorKey, inspectorPinned, loadedRows: items.length });
  currentSelection.current = { selectedKey, inspectorKey, inspectorPinned, loadedRows: items.length };
  const initialLoadedRows = useRef(initialContext?.reviewLoadedRows ?? 0);
  const selected = selectedKey ? items.find((item) => reviewKey(item) === (inspectorKey ?? selectedKey)) ?? null : null;
  const selectedIndex = selected ? items.indexOf(selected) : -1;
  const currentSnoozeTarget = snoozeTarget && items.find((item) => reviewFindingKey(item) === reviewFindingKey(snoozeTarget));
  const snoozeTargetIsCurrent = Boolean(snoozeTarget && currentSnoozeTarget
    && currentSnoozeTarget.finding.state !== "resolved"
    && JSON.stringify(reviewActionBasis(currentSnoozeTarget)) === JSON.stringify(reviewActionBasis(snoozeTarget)));
  const snoozeTimeIsValid = new Date(snoozeUntil).getTime() > Date.now();
  const remediationGroup = query.filters.packageIdentity ? initialGroup : undefined;
  const activeFilterCount = ["severities", "reviewStates", "fixAvailable"].filter((key) => {
    const field = key as "severities" | "reviewStates" | "fixAvailable";
    return JSON.stringify(query.filters[field]) !== JSON.stringify(activeView.query.filters[field]);
  }).length;
  const scope = query.filters;
  const scanIds = scope.resolvedInScanRunId ? [scope.resolvedInScanRunId] : [...new Set([
    ...scope.scanRunIds, ...(scope.scanChangeRunIds ?? []), ...(scope.scanResolvedRunIds ?? []),
  ])];
  function dropScope(changes: Partial<ReviewFilters>) {
    setRequestedItemUnavailable(false);
    setInspectorPinned(false);
    setQuery((current) => ({ ...current, cursor: null, filters: { ...current.filters, ...changes } }));
  }

  const severityFilter = query.filters.severities.join(",");
  const filtersChanged = JSON.stringify(query.filters) !== JSON.stringify(activeView.query.filters);
  const presentationDirty = JSON.stringify(presentation) !== JSON.stringify(activeView.presentation);
  const reviewSurfaceV2 = import.meta.env.VITE_CVEC_REVIEW_SURFACE_V2 !== "legacy";
  const isWorkflowBoard = presentation.mode === "board" && presentation.grouping === "reviewState" && !scope.reviewStates.length;
  const reviewStateRestricted = viewId === "needs-review";

  useEffect(() => {
    if (loadedViewId.current === viewId) return;
    loadedViewId.current = viewId;
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    searchTimer.current = null;
    const view = views.find((entry) => entry.id === viewId) ?? BUILT_IN_REVIEW_VIEWS[0];
    const nextPresentation = structuredClone(view.presentation);
    presentationCache.current = { [nextPresentation.mode]: nextPresentation };
    setQuery({ ...view.query, cursor: null });
    setSearchValue(view.query.search ?? "");
    setPresentation((current) => samePresentation(current, nextPresentation) ? current : nextPresentation);
  }, [viewId, views, setQuery, setPresentation]);
  useEffect(() => {
    if (cacheRevision.current !== dataRevision) {
      pageCache.clear();
      cacheRevision.current = dataRevision;
    }
    if (mutationInFlight.current) {
      deferredReload.current = true;
      return;
    }
    // The page can reconcile expired snoozes and publish its own revision.
    const current = appliedPage.current;
    if (current?.query === query && current.dataRevision === dataRevision
      && current.reloadRevision === reloadRevision && current.revision >= reviewRevision) return;
    void load(false);
  }, [query, dataRevision, reviewRevision, reloadRevision]);
  useEffect(() => () => {
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    loadSequence.current += 1;
  }, [pageCache]);
  const latestContext=useRef<WorkspaceContext>({});
  const contextSink=useRef(onContextChange);contextSink.current=onContextChange;
  latestContext.current={viewId,reviewQuery:query,reviewLoadedRows:items.length,reviewPresentation:presentation,selectedKey:selectedKey??undefined,focusKey:focusKey??undefined,inspectorPinned,inspectorKey:inspectorKey??undefined,scrollTop,boardScrollLeft:scrollAnchors.current.board.offset,ephemeralReviewContext:initialContext?.ephemeralReviewContext,remediationGroup};
  useEffect(()=>{contextSink.current?.(latestContext.current);},[query,presentation,items.length,selectedKey,focusKey,inspectorPinned,inspectorKey,scrollTop,viewId]);
  useEffect(()=>()=>{contextSink.current?.({...latestContext.current,scrollTop:scrollAnchors.current.table.offset,boardScrollLeft:scrollAnchors.current.board.offset});},[]);
  useEffect(() => { if (!selectedKey && inspectorKey) setInspectorKey(null); }, [inspectorKey, selectedKey, setInspectorKey]);
  useEffect(() => {
    const key = ephemeralSelection.current;
    if (!key || loading || loadError) return;
    ephemeralSelection.current = null;
    const requested = items.find((item) => reviewKey(item) === key);
    if (requested) {
      setSelectedKey(key);
      setInspectorKey(key);
      setInspectorPinned(true);
      setRequestedItemUnavailable(false);
    } else if (remediationGroup && items[0]) {
      const fallbackKey = reviewKey(items[0]);
      setSelectedKey(fallbackKey);
      setInspectorKey(fallbackKey);
      setInspectorPinned(true);
      setRequestedItemUnavailable(true);
    } else {
      setRequestedItemUnavailable(true);
    }
  }, [remediationGroup, items, loading, loadError, setInspectorKey, setInspectorPinned, setSelectedKey]);

  async function load(append: boolean, force = false, preserveContext = !appliedPage.current || appliedPage.current.query === query) {
    if (mutationInFlight.current) {
      deferredReload.current = true;
      return;
    }
    const requestedQuery = { ...query, filters: { ...query.filters, packageIdentity: query.filters.packageIdentity ?? null }, cursor: append ? nextCursor : null, pageSize: remediationGroup ? 200 : query.pageSize };
    const sequence = ++loadSequence.current;
    activeLoad.current = null;
    setLoadError(false);
    const selection = currentSelection.current;
    const selectionKey = selection.inspectorKey ?? selection.selectedKey;
    const loadedRows = selection.loadedRows || initialLoadedRows.current;
    const restoreSelection = ephemeralSelection.current && !remediationGroup
      ? undefined
      : selectionKey ?? undefined;
    const restore = !append && preserveContext && (loadedRows || restoreSelection)
      ? { loadedRows, preserveSelection: restoreSelection }
      : undefined;
    if (!append && !force) {
      const cached = pageCache.get(requestedQuery);
      const desiredRows = Math.min(restore?.loadedRows ?? 0, REVIEW_SESSION_CACHE_ROWS);
      if (cached && cached.revision >= reviewRevision &&
          (!restore?.preserveSelection || cached.items.some((item) => reviewKey(item) === restore.preserveSelection)) &&
          (!cached.nextCursor || cached.items.length >= desiredRows)) {
        appliedPage.current = { query, dataRevision, reloadRevision, revision: cached.revision };
        initialLoadedRows.current = 0;
        applyPage(cached, false);
        setLoading(false);
        return;
      }
    }
    activeLoad.current = sequence;
    setLoading(true);
    try {
      const page = await (restore ? listReviewFindings(requestedQuery, restore) : listReviewFindings(requestedQuery));
      if (sequence !== loadSequence.current) return;
      const latest = currentSelection.current;
      const inspectedKey = latest.inspectorKey ?? latest.selectedKey;
      if (!append && preserveContext && latest.inspectorPinned && inspectedKey &&
          !page.items.some((item) => reviewKey(item) === inspectedKey)) {
        setInspectorPinned(false);
        setUnavailableInspection(query);
      }
      appliedPage.current = { query, dataRevision, reloadRevision, revision: page.revision };
      initialLoadedRows.current = 0;
      if (append && items.length + page.items.length <= REVIEW_SESSION_CACHE_ROWS) pageCache.set(requestedQuery, { ...page, items: [...items, ...page.items] });
      else if (!append) pageCache.set(requestedQuery, page);
      applyPage(page, append);
      if (!append) setReviewConflict((current) => current === "refreshing" ? "refreshed" : current);
      if (page.reviewSummary) void publishReviewUpdate({ reviewSummary: page.reviewSummary, revision: page.revision });
    } catch (cause) {
      if (sequence === loadSequence.current) {
        if (append && String(cause).includes("REVIEW_CURSOR_STALE")) { pageCache.clear(); await load(false, true, true); }
        else setLoadError(true);
      }
    } finally { if (sequence === loadSequence.current) { activeLoad.current = null; setLoading(false); } }
  }

  function beginMutation() {
    // Invalidate reads already in flight as well as deferring new refreshes.
    if (activeLoad.current !== null) deferredReload.current = true;
    activeLoad.current = null;
    loadSequence.current += 1;
    pageCache.clear();
    mutationInFlight.current = true;
    setReviewConflict(null);
    setMutating(true);
  }

  function finishMutation() {
    mutationInFlight.current = false;
    setMutating(false);
    if (deferredReload.current) {
      deferredReload.current = false;
      pageCache.clear();
      setReloadRevision((current) => current + 1);
    }
  }

  async function publishReviewUpdate(update: ReviewUpdate | null) {
    try {
      await onReviewStateChange?.(update);
    } catch (cause) {
      onError(String(cause));
    }
  }

  async function mutate(action: () => Promise<ReviewUpdate | null>) {
    if (mutationInFlight.current || refreshingEvidence) return;
    beginMutation();
    try {
      const update = await action();
      setSnoozeOpen(false);
      setSnoozeTarget(null);
      deferredReload.current = true;
      await publishReviewUpdate(update);
    } catch (cause) {
      handleReviewFailure(cause);
    } finally {
      finishMutation();
    }
  }

  function handleReviewFailure(cause: unknown) {
    if (isStaleReviewEvidence(cause)) {
      setReviewConflict("refreshing");
      deferredReload.current = true;
      return;
    }
    onError(String(cause));
  }

  function updateViews(next: ReviewViewsState) { onViewsStateChange(next); }
  function updateSearch(value: string) {
    setSearchValue(value);
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    if (!value.trim()) {
      searchTimer.current = null;
      setQuery((current) => current.search ? { ...current, search: null, cursor: null } : current);
      return;
    }
    searchTimer.current = window.setTimeout(() => {
      searchTimer.current = null;
      setQuery((current) => ({ ...current, search: value.trim(), cursor: null }));
    }, 180);
  }

  function resetQuery() {
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    searchTimer.current = null;
    setSearchValue(activeView.query.search ?? "");
    setQuery({ ...activeView.query, cursor: null });
  }
  function handleSave() { try { const next = createCustomView(viewsState, saveName, query, presentation); updateViews(next); setSaveOpen(false); setSaveName(""); setViewId(next.customViews.at(-1)!.id); } catch (cause) { onError(String(cause)); } }
  function handleSelect(key: string, mode: "preview" | "open") {
    if (mode === "open") setUnavailableInspection(null);
    if (mode === "open") {
      inspectorOriginKey.current = key;
      startRemediationPerformanceQa();
    }
    setSelectedKey(key);
    setFocusKey(key);
    setInspectorKey(key);
    // React Aria can emit a selection preview after the row-open action for the
    // same pointer gesture. A preview may update focus, but must not undo an
    // inspector that the gesture already opened.
    if (mode === "open") setInspectorPinned(true);
  }
  function closeInspector() {
    setInspectorPinned(false);
    setSelectedKey(null);
    setInspectorKey(null);
  }
  function restoreInspectorFocus() {
    // A restored inspector has no mounted trigger for the dialog's default
    // focus return. Resolve its original finding by identity in either surface.
    const rows = [...(workspaceRef.current?.querySelectorAll<HTMLElement>("[role=row][data-key], [role=row][data-review-drag-key]") ?? [])];
    for (const key of [inspectorOriginKey.current, currentSelection.current.selectedKey]) {
      const row = rows.find((row) => row.dataset.key === key || row.dataset.reviewDragKey === key);
      if (row) return row;
    }
    return searchField.current ?? true;
  }
  function moveSelection(delta: -1 | 1) { const item = items[Math.min(items.length - 1, Math.max(0, selectedIndex + delta))]; if (item) { const key = reviewKey(item); setSelectedKey(key); setFocusKey(key); setInspectorKey(key); setInspectorPinned(true); } }
  function renameActiveView() { const name = window.prompt("View name", activeView.name); if (!name || activeView.builtIn) return; try { updateViews(updateCustomView(viewsState, activeView.id, { name })); } catch (cause) { onError(String(cause)); } }
  function updateReviewState(value: ReviewState | "all") {
    setQuery((current) => ({ ...current, cursor: null, filters: { ...current.filters, reviewStates: value === "all" ? [] : [value] } }));
  }
  async function handleBoardMove(item: ReviewFinding, state: ReviewState) {
    if (query.filters.resolvedInScanRunId !== undefined || state === item.reviewState || mutationInFlight.current || refreshingEvidence) return;
    if (state === "snoozed") { openSnooze(item); return; }
    const evidence = reviewActionBasis(item);
    await commitBoardMove(item, state, () => state === "reviewed"
      ? markFindingReviewed(item.repoId, item.finding.fingerprint, evidence)
      : returnFindingToReview(item.repoId, item.finding.fingerprint, evidence));
  }
  async function commitBoardMove(item: ReviewFinding, state: ReviewState, action: () => Promise<ReviewUpdate | null>, details: { reviewedAt?: string | null; snoozeUntil?: string | null; reviewNote?: string | null } = {}) {
    const key = reviewFindingKey(item);
    const optimistic = transitionReviewFinding(item, state, details);
    beginMutation();
    setPendingMoveKey(key);
    setSelectedKey(key);
    applyLocalReviewFinding(optimistic, item.reviewState);
    try {
      await publishReviewUpdate(await action());
    } catch (cause) {
      applyLocalReviewFinding(item, state);
      handleReviewFailure(cause);
    } finally {
      setPendingMoveKey(null);
      finishMutation();
    }
  }
  function openSnooze(item: ReviewFinding) {
    setSnoozeTarget(item);
    setSnoozeUntil(defaultSnoozeValue());
    setSnoozeNote("");
    setSnoozeOpen(true);
  }
  async function confirmSnooze() {
    if (!snoozeTarget || !snoozeTargetIsCurrent || refreshingEvidence || mutationInFlight.current) return;
    const date = new Date(snoozeUntil);
    if (!(date.getTime() > Date.now())) return;
    const { repoId, finding } = snoozeTarget;
    const until = date.toISOString();
    const saveSnooze = () => snoozeFinding(repoId, finding.fingerprint, until, snoozeNote, reviewActionBasis(snoozeTarget));
    if (!isWorkflowBoard) {
      await mutate(saveSnooze);
      return;
    }
    setSnoozeOpen(false);
    setSnoozeTarget(null);
    await commitBoardMove(snoozeTarget, "snoozed", saveSnooze, { snoozeUntil: until, reviewNote: snoozeNote.trim() || null });
  }
  function changePrimarySort(sort:ReviewSort){
    const direction=sort==="repository"||sort==="package"?"asc":"desc";
    const sorts=[{field:sort,direction}] as {field:ReviewSort;direction:"asc"|"desc"}[];
    setQuery((current) => updateReviewSort(current, sorts));
    setPresentation((current)=>current.mode==="table"?{...current,sorts}:current);
  }
  function updatePresentation(next: ReviewPresentation) {
    presentationCache.current[presentation.mode] = presentation;
    const resolved = next.mode === presentation.mode ? next : presentationCache.current[next.mode] ?? next;
    presentationCache.current[resolved.mode] = resolved;
    setPresentation(resolved);
    if (resolved.mode === "table") setQuery((current) => updateReviewSort(current, resolved.sorts));
  }

  const conflictNotice = reviewConflict === "refreshed"
    ? <p role="status" className="text-xs text-ink-subtle">The finding changed before this action was saved. Review the current evidence before trying again.</p>
    : null;
  const evidenceFeedback = refreshingEvidence ? (
    loadError ? (
      <div className="flex flex-wrap items-center gap-2">
        <p role="alert" className="text-xs text-ink-muted">Finding evidence could not be refreshed.</p>
        <Button size="sm" variant="secondary" onClick={() => void load(false, true)}>Retry loading evidence</Button>
      </div>
    ) : <p role="status" className="text-xs text-ink-subtle">Updating finding evidence…</p>
  ) : conflictNotice;
  const emptyState = loadError ? (
    <EmptyState icon={ShieldCheckered} title="Findings could not be loaded" description="Retry to load the saved findings for this view." action={<Button onClick={() => void load(false, true)}>Retry loading findings</Button>} />
  ) : (
    <ReviewEmpty
      searching={Boolean(query.search)}
      filtersChanged={filtersChanged}
      activeViewId={activeView.id}
      resolvedInScanRunId={query.filters.resolvedInScanRunId}
      onClearSearch={() => { updateSearch(""); searchField.current?.focus(); }}
      onResetFilters={() => { resetQuery(); searchField.current?.focus(); }}
      onAllOpen={() => setViewId("all-open")}
    />
  );

  const columns: WorkListColumn<ReviewFinding>[] = [
    { id: "severity", label: "Severity", sortable: true, className: "w-[7.5rem]", render: (item) => <Badge variant={severityVariant(item.finding.severity)}>{item.finding.severity}</Badge> },
    { id: "finding", label: "Finding", render: (item) => <div className="min-w-0"><p className="truncate font-semibold text-ink">{item.finding.advisoryId}</p><p className="mt-1 truncate text-ink-subtle">{item.finding.title ?? item.finding.packageName}</p><div className="mt-1.5 flex flex-wrap gap-1">{item.priorityReasons.slice(0, 2).map((reason) => <span key={reason} className="rounded-sm bg-surface-3 px-1.5 py-0.5 text-xs text-ink-subtle">{reason}</span>)}</div></div> },
    { id: "repository", label: "Repository", sortable: true, className: "w-[22%]", render: (item) => <div><p className="truncate font-medium text-ink">{item.repoName}</p><p className="truncate text-ink-subtle">{item.finding.packageName} {item.finding.packageVersion}</p></div> },
    { id: "status", label: "Status", className: "w-[10rem]", render: (item) => <div className="flex flex-wrap gap-1"><Badge variant={item.reviewState === "unreviewed" ? "secondary" : "outline"}>{item.reviewState}</Badge><Badge variant="outline">{item.finding.state}</Badge>{item.finding.fixVersion ? <Badge variant="success">Fix</Badge> : null}{item.snoozeUntil ? <p className="w-full text-ink-tertiary">Returns {formatRelativeTimestamp(item.snoozeUntil)}</p> : item.scannedAt ? <p className="w-full text-ink-tertiary">Scanned {formatRelativeTimestamp(item.scannedAt)}</p> : null}</div> },
  ];

  return (
    <div ref={workspaceRef} className="review-workspace flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-hairline pb-3">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5" role="navigation" aria-label="Review views">{views.map((view) => <Button key={view.id} size="sm" aria-current={view.id === viewId ? "true" : undefined} variant={view.id === viewId ? "secondary" : "ghost"} onClick={() => setViewId(view.id)}><span className="max-w-36 truncate">{view.name}</span>{view.builtIn ? null : viewsState.favoriteIds.includes(view.id) ? <AppIcon className="size-3 text-primary" icon={Star} weight="fill" /> : null}</Button>)}</div>
        <div className="flex items-center gap-1"><span className="text-xs text-ink-subtle">{total} finding{total === 1 ? "" : "s"}</span><Popover><PopoverTrigger render={<IconButton aria-label="View options" size="icon-sm" variant="ghost" />}><AppIcon className="size-4" icon={DotsThree} weight="bold" /></PopoverTrigger><PopoverContent align="end" className="w-60 p-1.5"><PopoverTitle className="sr-only">View options</PopoverTitle>{!activeView.builtIn ? <Button className="w-full justify-start" variant="ghost" onClick={() => updateViews(updateCustomView(viewsState, activeView.id, { query, presentation }))}><AppIcon className="size-4" icon={FloppyDisk} />Update saved view</Button> : null}<Button className="w-full justify-start" variant="ghost" onClick={() => { const next = createCustomView(viewsState, `${activeView.name} copy`, query, presentation); updateViews(next); setViewId(next.customViews.at(-1)!.id); }}><AppIcon className="size-4" icon={Copy} />Save copy</Button><Button className="w-full justify-start" variant="ghost" onClick={() => { resetQuery(); setPresentation(structuredClone(activeView.presentation)); }}><AppIcon className="size-4" icon={ArrowDown} />Reset changes</Button>{allReviewViews(viewsState).some((view) => view.id === activeView.id) ? <Button className="w-full justify-start" variant="ghost" onClick={() => updateViews(setDefaultView(viewsState, activeView.id))}><AppIcon className="size-4" icon={House} />{viewsState.defaultViewId === activeView.id ? "Default view" : "Set as default"}</Button> : null}{!activeView.builtIn ? <><Button className="w-full justify-start" variant="ghost" onClick={renameActiveView}><AppIcon className="size-4" icon={PencilSimple} />Rename</Button><Button className="w-full justify-start" variant="ghost" onClick={() => updateViews(toggleFavoriteView(viewsState, activeView.id))}><AppIcon className="size-4" icon={viewsState.favoriteIds.includes(activeView.id) ? Star : StarHalf} />{viewsState.favoriteIds.includes(activeView.id) ? "Remove favorite" : "Favorite"}</Button>{viewsState.favoriteIds.includes(activeView.id) ? <div className="grid grid-cols-2 gap-1"><Button variant="ghost" onClick={() => updateViews(reorderFavoriteView(viewsState, activeView.id, -1))}><AppIcon className="size-4" icon={ArrowUp} />Earlier</Button><Button variant="ghost" onClick={() => updateViews(reorderFavoriteView(viewsState, activeView.id, 1))}><AppIcon className="size-4" icon={ArrowDown} />Later</Button></div> : null}<Button className="w-full justify-start text-severity-critical" variant="ghost" onClick={() => { if (window.confirm(`Delete ${activeView.name}?`)) { updateViews(deleteCustomView(viewsState, activeView.id)); setViewId("needs-review"); } }}><AppIcon className="size-4" icon={Trash} />Delete view</Button></> : null}</PopoverContent></Popover></div>
      </div>

      <div data-testid="review-toolbar" className="grid min-w-0 gap-2 xl:grid-cols-[minmax(220px,1fr)_auto_180px_auto_auto]">
        <SearchField ref={searchField} aria-label="Search review findings" placeholder="Search CVE, package, repository…" value={searchValue} onChange={(event) => updateSearch(event.target.value)} />
        <div className="flex min-w-0 flex-wrap items-center gap-2 xl:contents">
          <Popover><PopoverTrigger render={<Button className="shrink-0" variant="secondary" />}><AppIcon className="size-4" icon={Funnel} />Filters{activeFilterCount ? <Badge variant="secondary">{activeFilterCount}</Badge> : null}</PopoverTrigger><PopoverContent align="end" className="grid w-72 gap-3"><PopoverTitle className="font-semibold">Filter Review</PopoverTitle><Select label="Severity" value={severityFilter || "all"} onValueChange={(value) => {
            const severities = value === "all" ? [] : value.split(",") as Severity[];
            setQuery((current) => ({ ...current, cursor: null, filters: { ...current.filters, severities } }));
          }}><option value="all">Any severity</option>{query.filters.severities.length > 1 ? <option value={severityFilter}>{query.filters.severities.map((severity) => severity[0] + severity.slice(1).toLowerCase()).join(" and ")}</option> : null}<option value="CRITICAL">Critical</option><option value="HIGH">High</option><option value="MEDIUM">Medium</option></Select><Select label="Review state" value={query.filters.reviewStates[0] ?? "all"} disabled={reviewStateRestricted} disabledReason={reviewStateRestricted ? "Snoozed findings return to this queue at their scheduled time." : undefined} onValueChange={(value) => updateReviewState(value as ReviewState | "all")}><option value="all">Any review state</option><option value="unreviewed">Unreviewed</option><option value="reviewed">Reviewed</option><option value="snoozed">Snoozed</option></Select><Select label="Fix availability" value={query.filters.fixAvailable === null ? "all" : String(query.filters.fixAvailable)} onValueChange={(value) => setQuery((current) => ({ ...current, cursor: null, filters: { ...current.filters, fixAvailable: value === "all" ? null : value === "true" } }))}><option value="all">Any fix state</option><option value="true">Fix available</option><option value="false">No fix listed</option></Select></PopoverContent></Popover>
          <Select containerClassName="min-w-40 flex-1 xl:min-w-0" aria-label="Sort review findings" value={query.sort} onValueChange={(value) => changePrimarySort(value as ReviewSort)}><option value="priority">Priority</option><option value="severity">Severity</option><option value="recent">Recent</option><option value="repository">Repository</option><option value="package">Package</option></Select>
          {reviewSurfaceV2 ? <ReviewPresentationControls presentation={presentation} dirty={presentationDirty} onChange={updatePresentation} onReset={() => updatePresentation(structuredClone(activeView.presentation))} /> : null}
          <Button className="shrink-0" variant="secondary" onClick={() => { setSaveName(`${activeView.name} copy`); setSaveOpen(true); }}><AppIcon className="size-4" icon={FloppyDisk} />Save view</Button>
        </div>
      </div>

      {!scannerReady && !total && !loading ? <p className="text-xs text-ink-subtle">Prepare the scanner in Settings to check for new findings.</p> : null}
      {loadError && items.length > 0 ? (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-hairline bg-surface-2 px-3 py-2">
          <p role="status" className="text-xs text-ink-muted">Findings could not be refreshed. Showing the last loaded results.</p>
          <Button size="xs" variant="secondary" onClick={() => void load(false, true)}>Retry loading findings</Button>
        </div>
      ) : null}
      {scope.repoIds.length || scope.packages.length || scope.packageIdentity || scanIds.length ? (
        <div aria-label="Active context filters" role="group" className="flex flex-wrap items-center gap-2 rounded-md border border-hairline bg-surface-2 px-3 py-2 text-xs text-ink-subtle">
          {scope.repoIds.map((repoId) => <ScopeFilter key={repoId}
            label={"Repository " + (items.find((item) => item.repoId === repoId)?.repoName ?? "#" + repoId)}
            onClear={() => dropScope({ repoIds: scope.repoIds.filter((id) => id !== repoId), fingerprint: undefined })}
          />)}
          {scope.packages.map((pkg) => <ScopeFilter key={pkg} label={"Package " + pkg}
            onClear={() => dropScope({ packages: scope.packages.filter((name) => name !== pkg) })}
          />)}
          {scope.packageIdentity ? <ScopeFilter
            label={["Package " + scope.packageIdentity.packageName, scope.packageIdentity.installedVersion, scope.packageIdentity.fixVersion ? "→ " + scope.packageIdentity.fixVersion : null].filter(Boolean).join(" ")}
            onClear={() => dropScope({ packageIdentity: null, fingerprint: undefined })}
          /> : null}
          {scanIds.length ? <ScopeFilter label={scanIds.map((id) => "Scan #" + id).join(", ")}
            onClear={() => viewId.startsWith("scan") ? setViewId("all-open") : dropScope({
              scanRunIds: [], scanChangeRunIds: [], scanResolvedRunIds: [], resolvedInScanRunId: undefined, fingerprint: undefined,
            })}
          /> : null}
        </div>
      ) : null}
      {requestedItemUnavailable ? <div role="status" className="rounded-md border border-hairline bg-surface-2 px-3 py-2 text-xs text-ink-subtle">The requested finding is no longer available in this result. The valid repository and package context is still applied.</div> : null}
      {unavailableInspection === query ? <p role="status" className="text-xs text-ink-subtle">The inspected finding is no longer in the loaded results.</p> : null}
      {!inspectorPinned && !snoozeOpen ? conflictNotice : null}
      <div className="review-result-region min-h-0 min-w-0 flex-1 overflow-hidden">
        <div className="relative flex h-full min-h-0 flex-col overflow-hidden">{loading&&items.length?<p role="status" className="shrink-0 text-xs text-ink-subtle">Updating review…</p>:null}{loading && !items.length ? <div className="flex h-64 items-center justify-center"><Spinner label="Loading review queue" /></div> : reviewSurfaceV2 ? presentation.mode === "table" ? <CvecReviewTable items={items} total={total} hasMore={Boolean(nextCursor)} loading={loading} presentation={presentation} selectedKey={selectedKey} label={`${activeView.name} findings`} initialScrollTop={scrollAnchors.current.table.offset} onLoadMore={() => void load(true)} onSelect={handleSelect} onDismiss={() => { setInspectorPinned(false); setSelectedKey(null); }} onScroll={captureTableScroll} onSort={(sort, direction) => { setQuery((current) => updateReviewSort(current, [{ field: sort, direction }])); setPresentation((current) => current.mode === "table" ? { ...current, sorts: [{ field: sort, direction }] } : current); }} onWidthsChange={(widths) => setPresentation((current) => current.mode === "table" ? { ...current, widths } : current)} empty={emptyState} /> : <CvecReviewBoard items={items} total={total} groupCounts={groupCounts} hasMore={Boolean(nextCursor)} loading={loading} presentation={presentation} selectedKey={selectedKey} pendingKey={pendingMoveKey} initialScrollLeft={scrollAnchors.current.board.offset} onSelect={handleSelect} onMove={handleBoardMove} readOnly={!isWorkflowBoard} onLoadMore={() => void load(true)} onScroll={captureBoardScroll} onToggleLane={(laneId) => setPresentation((current) => current.mode === "board" ? { ...current, collapsedLaneIds: current.collapsedLaneIds.includes(laneId) ? current.collapsedLaneIds.filter((id) => id !== laneId) : [...current.collapsedLaneIds, laneId] } : current)} onLaneOrderChange={(laneOrder) => setPresentation((current) => current.mode === "board" ? { ...current, laneOrder } : current)} empty={emptyState} /> : <WorkList items={items} columns={columns} getKey={reviewKey} label={`${activeView.name} findings`} selectedKey={selectedKey} onSelect={handleSelect} onDismiss={() => { setInspectorPinned(false); setSelectedKey(null); }} initialScrollTop={scrollTop} onScrollTopChange={captureTableScroll} sort={{ column: query.sort === "repository" ? "repository" : query.sort === "severity" ? "severity" : "", direction: "desc" }} onSort={(column) => setQuery((current) => ({ ...current, sort: column as ReviewSort, cursor: null }))} empty={emptyState} />}{nextCursor && !reviewSurfaceV2 ? <Button className="mt-2 w-full shrink-0" variant="ghost" onClick={() => void load(true)} disabled={loading}>Load more</Button> : null}</div>
      </div>

      {selected && inspectorPinned ? <Dialog open onOpenChange={(open) => { if (!open) closeInspector(); }}>
        <DialogContent initialFocus={inspectorHeading} finalFocus={restoreInspectorFocus} positioned showClose={false} className="w-[min(44rem,calc(100vw-1rem))] max-w-none overflow-hidden p-0 shadow-2xl">
          <FindingInspector
            dataRevision={loadedEvidenceRevision}
            item={selected}
            resolvedInScanRunId={query.filters.resolvedInScanRunId}
            mutating={mutating || refreshingEvidence}
            evidenceFeedback={snoozeOpen ? null : evidenceFeedback}
            onClose={closeInspector}
            headingRef={inspectorHeading}
            onContinueInChat={onContinueInChat}
            nextKey={items[selectedIndex + 1] ? reviewFindingKey(items[selectedIndex + 1]) : undefined}
            previousKey={items[selectedIndex - 1] ? reviewFindingKey(items[selectedIndex - 1]) : undefined}
            onNext={selectedIndex < items.length - 1 ? () => moveSelection(1) : undefined}
            onPrevious={selectedIndex > 0 ? () => moveSelection(-1) : undefined}
            onOpenRepo={onOpenRepo}
            onOpenUrl={onOpenUrl}
            onReview={() => void mutate(() => markFindingReviewed(selected.repoId, selected.finding.fingerprint, reviewActionBasis(selected)))}
            onReturn={() => void mutate(() => returnFindingToReview(selected.repoId, selected.finding.fingerprint, reviewActionBasis(selected)))}
            onSnooze={() => openSnooze(selected)}
          />
        </DialogContent>
      </Dialog> : null}
      <Dialog open={saveOpen} onOpenChange={setSaveOpen}><DialogContent><DialogHeader><DialogTitle>Save Review view</DialogTitle></DialogHeader><DialogBody><Input autoFocus label="View name" value={saveName} onChange={(event) => setSaveName(event.target.value)} /></DialogBody><DialogFooter><Button variant="ghost" onClick={() => setSaveOpen(false)}>Cancel</Button><Button onClick={handleSave}>Save view</Button></DialogFooter></DialogContent></Dialog>
      <Dialog open={snoozeOpen} onOpenChange={(open) => { setSnoozeOpen(open); if (!open) setSnoozeTarget(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Snooze finding</DialogTitle>
            {snoozeTarget ? <DialogDescription>{snoozeTarget.finding.advisoryId} · {snoozeTarget.repoName}</DialogDescription> : null}
          </DialogHeader>
          <DialogBody className="grid gap-3">
            {refreshingEvidence ? evidenceFeedback : !snoozeTargetIsCurrent
              ? <p role="status" className="text-xs text-ink-subtle">The finding changed. Reopen it to snooze current evidence.</p>
              : conflictNotice}
            <Input label="Return at" type="datetime-local" value={snoozeUntil} onChange={(event) => setSnoozeUntil(event.target.value)} error={snoozeTimeIsValid ? undefined : "Choose a return time in the future."} />
            <Input label="Note (optional)" maxLength={500} value={snoozeNote} onChange={(event) => setSnoozeNote(event.target.value)} />
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => { setSnoozeOpen(false); setSnoozeTarget(null); }}>Cancel</Button>
            <Button disabled={mutating || refreshingEvidence || !snoozeTargetIsCurrent || !snoozeTimeIsValid} onClick={() => void confirmSnooze()}>Snooze</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function FindingInspector({ item, headingRef, resolvedInScanRunId, dataRevision, mutating, evidenceFeedback, onClose, onReview, onReturn, onSnooze, onOpenRepo, onOpenUrl, onContinueInChat, onPrevious, onNext, previousKey, nextKey }: { item: ReviewFinding; headingRef: React.Ref<HTMLHeadingElement>; resolvedInScanRunId?: number; dataRevision: number; mutating: boolean; evidenceFeedback: ReactNode; onClose: () => void; onReview: () => void; onReturn: () => void; onSnooze: () => void; onOpenRepo: (repoId: number) => void; onOpenUrl: (url: string) => void; onContinueInChat?: (finding: FindingChatRequest) => void; onPrevious?: () => void; onNext?: () => void; previousKey?: string; nextKey?: string }) {
  const [detail, setDetail] = useState<ReviewFinding | null>(null);
  const [detailFailure, setDetailFailure] = useState<{ key: string; message: string } | null>(null);
  const detailKey = `${item.repoId}:${item.finding.fingerprint}:${item.scanRunId}`;
  const detailRequestKey = JSON.stringify([detailKey, dataRevision, reviewActionBasis(item).riskBasis, resolvedInScanRunId]);
  const [loadedDetailKey, setLoadedDetailKey] = useState("");
  const detailError = detailFailure?.key === detailRequestKey ? detailFailure.message : null;
  const finding = loadedDetailKey === detailRequestKey && detail ? detail.finding : item.finding;
  useEffect(() => {
    let active = true;
    setDetailFailure(null);
    if (item.finding.intel || !item.scanRunId) return;
    void Promise.resolve().then(() => getReviewFindingDetail(item.repoId, item.finding.fingerprint, item.scanRunId!, resolvedInScanRunId))
      .then((value) => {
        if (value.repoId !== item.repoId || value.finding.fingerprint !== item.finding.fingerprint || value.scanRunId !== item.scanRunId) throw new Error("Finding details belong to a different evidence identity.");
        if (active) { setDetail(value); setLoadedDetailKey(detailRequestKey); }
      })
      .catch(() => { if (active) setDetailFailure({ key: detailRequestKey, message: "Advisory details could not be loaded. Reopen this finding to retry." }); });
    return () => { active = false; };
  }, [detailRequestKey]);
  const sourceUrl = finding.intel?.url ?? finding.sourceUrl;
  const historical = resolvedInScanRunId !== undefined;
  const fixPlanEnabled = !historical && isFixPlanEnabled(import.meta.env.VITE_CVEC_FIX_PLAN, window.location.search, import.meta.env.DEV);
  const [remediation, setRemediation] = useState<FindingRemediationProjection | null>(null);
  const [remediationLoading, setRemediationLoading] = useState(false);
  const [remediationError, setRemediationError] = useState<string | null>(null);
  const [retryTick, setRetryTick] = useState(0);
  const remediationRequestKey = `${detailRequestKey}:${retryTick}`;
  const [settledRemediationKey, setSettledRemediationKey] = useState("");
  const requestSequence = useRef(0);
  useEffect(() => { recordRemediationPerformanceQa("qaInspectorFeedbackMs"); }, [finding.fingerprint]);
  useEffect(() => { if (remediation) recordRemediationPerformanceQa("qaRemediationDetailMs"); }, [remediation]);
  useEffect(() => {
    const sequence = ++requestSequence.current;
    const scanRunId = item.scanRunId;
    setRemediation(null);
    setRemediationError(null);
    if (!fixPlanEnabled) {
      setRemediationLoading(false);
      setSettledRemediationKey(remediationRequestKey);
      return;
    }
    if (!scanRunId) {
      setRemediationLoading(false);
      setRemediationError("Remediation evidence is not available for this finding. Run a new scan.");
      setSettledRemediationKey(remediationRequestKey);
      return;
    }
    const key = { repoId: item.repoId, fingerprint: finding.fingerprint, evidenceScanRunId: scanRunId };
    const cached = remediationProjectionCache.get(key);
    if (cached) {
      setRemediation(cached);
      setRemediationLoading(false);
      setSettledRemediationKey(remediationRequestKey);
      return;
    }
    setRemediationLoading(true);
    void remediationProjectionCache.getOrLoad(key, () => getFindingRemediation(item.repoId, finding.fingerprint, scanRunId))
      .then((projection) => {
        if (requestSequence.current !== sequence) return;
        setRemediation(projection);
      })
      .catch(() => {
        if (requestSequence.current !== sequence) return;
        setRemediationError("Local fix evidence could not be loaded.");
      })
      .finally(() => {
        if (requestSequence.current === sequence) { setRemediationLoading(false); setSettledRemediationKey(remediationRequestKey); }
      });
    return () => { requestSequence.current += 1; };
  }, [remediationRequestKey, fixPlanEnabled]);
  const currentRemediation = settledRemediationKey === remediationRequestKey && remediation?.repoId === item.repoId && remediation.fingerprint === finding.fingerprint && remediation.evidenceScanRunId === item.scanRunId ? remediation : null;
  const detailState = item.finding.intel || !item.scanRunId || loadedDetailKey === detailRequestKey ? "ready" : detailError ? "error" : "loading";
  const remediationState = !fixPlanEnabled ? "disabled" : !item.scanRunId ? "unavailable" : settledRemediationKey !== remediationRequestKey || remediationLoading ? "loading" : currentRemediation ? "ready" : "error";
  const targetVersion = currentRemediation?.scannerReportedTarget ?? finding.fixVersion;
  const canPlanFix = !historical && finding.state !== "resolved" && item.repoSourceType === "local"
    && Boolean(targetVersion && item.scanRunId);
  const canFix = canPlanFix && currentRemediation && currentRemediation.evidenceQuality !== "stale"
    && (currentRemediation.verificationState === "still_detected" || currentRemediation.verificationState === "reappeared");
  const preparingHandoff = canPlanFix && remediationState === "loading";
  const title = findingDisplayTitle(finding) ?? finding.advisoryId;
  const chatAction = onContinueInChat ? {
    label: preparingHandoff ? "Checking fix evidence…" : canFix ? "Fix in Chat" : "Analyze in Chat",
    loading: preparingHandoff || mutating,
    analysisReason: canFix || preparingHandoff ? undefined
      : historical || finding.state === "resolved" ? "Inspect the scan that recorded this historical finding."
      : item.repoSourceType === "github" ? "A fix needs a local checkout. You can analyze the linked repository’s scan here."
      : item.repoSourceType !== "local" ? "A fix needs a local Git checkout."
      : !targetVersion ? "The scanner has no fixed version for this package."
      : "A current scan with verifiable fix evidence is needed to start a fix.",
    onOpen: () => onContinueInChat({
      repositoryId: item.repoId,
      fingerprint: finding.fingerprint,
      advisoryId: finding.advisoryId,
      packageName: finding.packageName,
      installedVersion: finding.packageVersion,
      targetVersion,
      scanRunId: item.scanRunId,
      mode: canFix ? "fix" : "analysis",
    }),
  } : undefined;
  return (
    <aside aria-label={`Finding ${finding.advisoryId}`} data-inspector-finding-key={reviewFindingKey(item)} data-inspector-detail-state={detailState} data-inspector-remediation-state={remediationState} className="flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-hairline bg-surface-1">
      <div className="border-b border-hairline px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle">{item.repoName}</p>
            <DialogTitle ref={headingRef} tabIndex={-1} className="mt-1 break-words text-lg font-semibold text-ink">{title}</DialogTitle>
            <div className="mt-2 flex flex-wrap gap-1.5"><Badge variant={severityVariant(finding.severity)}>{finding.severity}</Badge>{title.toUpperCase().includes(finding.advisoryId.toUpperCase()) ? null : <Badge variant="outline">{finding.advisoryId}</Badge>}{item.knownExploited ? <Badge variant="warning">KEV</Badge> : null}{item.ransomwareKnown ? <Badge variant="destructive">Ransomware</Badge> : null}</div>
          </div>
          <div className="flex items-center gap-1"><IconButton aria-label="Previous finding" data-inspector-target-key={previousKey} disabled={!onPrevious} variant="ghost" onClick={onPrevious}><AppIcon className="size-4" icon={CaretLeft} /></IconButton><IconButton aria-label="Next finding" data-inspector-target-key={nextKey} disabled={!onNext} variant="ghost" onClick={onNext}><AppIcon className="size-4" icon={CaretRight} /></IconButton><IconButton aria-label="Close inspector" variant="ghost" onClick={onClose}><AppIcon className="size-4" icon={X} /></IconButton></div>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4 [scrollbar-gutter:stable]">
        {evidenceFeedback ? <div className="mb-3">{evidenceFeedback}</div> : null}
        {detailError ? <p role="alert" className="mb-3 text-sm text-severity-critical">{detailError}</p> : null}
        <FindingInspectorBody
          finding={finding}
          scanResolution={historical ? { runId: resolvedInScanRunId, previousRunId: item.scanRunId } : undefined}
          priorityReasons={historical ? undefined : item.priorityReasons}
          reviewState={historical ? undefined : item.reviewState}
          snoozeUntil={historical ? undefined : item.snoozeUntil}
          onOpenUrl={onOpenUrl}
          remediation={currentRemediation}
          remediationLoading={remediationState === "loading"}
          remediationError={settledRemediationKey === remediationRequestKey ? remediationError : null}
          onRetryRemediation={() => setRetryTick((value) => value + 1)}
          onOpenManifest={currentRemediation && item.scanRunId ? () => openFindingManifest(item.repoId, finding.fingerprint, item.scanRunId!) : undefined}
          chatAction={chatAction}
          fixPlanEnabled={fixPlanEnabled}
        />
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 border-t border-hairline bg-surface-1 p-3">{item.reviewState === "unreviewed" && finding.state !== "resolved" ? <Button disabled={mutating} variant="secondary" onClick={onReview}><AppIcon className="size-4" icon={Check} />Mark reviewed</Button> : finding.state !== "resolved" ? <Button disabled={mutating} variant="secondary" onClick={onReturn}>Return to review</Button> : null}{finding.state !== "resolved" ? <Button disabled={mutating} variant="secondary" onClick={onSnooze}><AppIcon className="size-4" icon={Clock} />Snooze</Button> : null}<Button variant="ghost" onClick={() => onOpenRepo(item.repoId)}>Open repo</Button>{sourceUrl ? <Button variant="ghost" onClick={() => onOpenUrl(sourceUrl)}><AppIcon className="size-4" icon={ArrowSquareOut} />Advisory</Button> : null}</div>
    </aside>
  );
}

export function isFixPlanEnabled(flag: string | undefined, search: string, development: boolean): boolean {
  if (flag === "off") return false;
  if (!development) return true;
  const qaParams = new URLSearchParams(search);
  return !(qaParams.has("qa") && qaParams.get("fixPlan") === "off");
}

function startRemediationPerformanceQa() {
  if (!isRemediationPerformanceQa()) return;
  const root = document.documentElement;
  root.dataset.qaInspectorStartedAt = String(performance.now());
  root.dataset.qaInspectorFeedbackMs = "";
  root.dataset.qaRemediationDetailMs = "";
  root.dataset.qaMaxLongTaskMs = "0";
}

function recordRemediationPerformanceQa(attribute: "qaInspectorFeedbackMs" | "qaRemediationDetailMs") {
  if (!isRemediationPerformanceQa()) return;
  const root = document.documentElement;
  const startedAt = Number(root.dataset.qaInspectorStartedAt);
  if (!Number.isFinite(startedAt) || root.dataset[attribute]) return;
  root.dataset[attribute] = (performance.now() - startedAt).toFixed(2);
}

function isRemediationPerformanceQa() {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get("qa") === "large";
}

function reviewKey(item: ReviewFinding) { return `${item.repoId}:${item.finding.fingerprint}`; }
function ReviewEmpty({ searching, filtersChanged, activeViewId, resolvedInScanRunId, onClearSearch, onResetFilters, onAllOpen }: {
  searching: boolean;
  filtersChanged: boolean;
  activeViewId: string;
  resolvedInScanRunId?: number;
  onClearSearch: () => void;
  onResetFilters: () => void;
  onAllOpen: () => void;
}) {
  if (searching || filtersChanged) {
    return (
      <EmptyState
        icon={ShieldCheckered}
        title="No matching findings"
        description={searching ? "Try a different advisory ID, package, or repository name." : "No findings match the current filters."}
        action={searching ? (
          <Button variant="secondary" onClick={onClearSearch}>Clear search</Button>
        ) : (
          <Button variant="secondary" onClick={onResetFilters}>Reset filters</Button>
        )}
      />
    );
  }

  if (resolvedInScanRunId !== undefined) {
    return <EmptyState icon={ShieldCheckered} title="No recorded resolutions in this scan" />;
  }

  if (activeViewId.startsWith("scan-resolved:")) {
    return <EmptyState
      icon={ShieldCheckered}
      title="No current resolutions from these scans"
      description="This view shows resolutions from the latest completed scan of each repository."
    />;
  }

  const reviewComplete = activeViewId === "needs-review";
  return (
    <EmptyState
      icon={ShieldCheckered}
      title={reviewComplete ? "Review complete" : "No findings in this view"}
      description={reviewComplete ? "Nothing currently needs review. Open findings remain available in All open." : "Change filters or select a different view."}
      action={reviewComplete ? <Button variant="secondary" onClick={onAllOpen}>View all open</Button> : undefined}
    />
  );
}
function severityVariant(severity: Severity): "destructive" | "warning" | "secondary" { return severity === "CRITICAL" ? "destructive" : severity === "HIGH" ? "warning" : "secondary"; }
function defaultSnoozeValue() { const date = new Date(Date.now() + 24 * 60 * 60 * 1000); const offset = date.getTimezoneOffset() * 60_000; return new Date(date.getTime() - offset).toISOString().slice(0, 16); }
function ScopeFilter({ label, onClear }: { label: string; onClear: () => void }) {
  return <Button size="xs" variant="outline" onClick={onClear} aria-label={"Remove filter " + label} className="h-auto min-h-7 max-w-full whitespace-normal text-left">
    <span className="break-words">{label}</span><AppIcon icon={X} className="size-3.5" />
  </Button>;
}
function samePresentation(left: ReviewPresentation, right: ReviewPresentation): boolean { return JSON.stringify(left) === JSON.stringify(right); }
