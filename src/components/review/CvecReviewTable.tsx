import { useEffect, useMemo, useRef, type KeyboardEvent, type MouseEvent, type ReactNode, type UIEvent } from "react";
import { Cell, Column, ColumnResizer, ResizableTableContainer, Row, Table, TableBody, TableHeader } from "react-aria-components/Table";
import * as VirtualizerPrimitives from "react-aria-components/Virtualizer";

import { formatRelativeTimestamp } from "@/lib/format";
import { findingDisplayTitle, projectReviewLanes, reviewFindingKey } from "@/lib/reviewProjection";
import { REVIEW_FIELD_REGISTRY, type ReviewFieldId, type ReviewPresentation } from "@/lib/reviewViews";
import type { ReviewFinding, ReviewSort } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const { Virtualizer, TableLayout } = VirtualizerPrimitives as typeof VirtualizerPrimitives & { TableLayout: typeof import("react-aria-components").TableLayout };

export function CvecReviewTable({ items, total, hasMore, loading, presentation, selectedKey, label, empty, initialScrollTop, onLoadMore, onSelect, onDismiss, onSort, onWidthsChange, onScroll }: { items: ReviewFinding[]; total: number; hasMore: boolean; loading: boolean; presentation: Extract<ReviewPresentation, { mode: "table" }>; selectedKey: string | null; label: string; empty: ReactNode; initialScrollTop: number; onLoadMore: () => void; onSelect: (key: string, mode: "preview" | "open") => void; onDismiss: () => void; onSort: (sort: ReviewSort, direction: "asc" | "desc") => void; onWidthsChange: (widths: Partial<Record<ReviewFieldId, number>>) => void; onScroll: (top: number) => void }) {
  const rootRef = useRef<HTMLDivElement>(null);
  const openFrameRef = useRef<number | null>(null);
  // React Aria uses selection identity to announce changes, even for equal keys.
  const selectedKeys = useMemo(() => new Set(selectedKey ? [selectedKey] : []), [selectedKey]);
  const fields = presentation.fields;
  const sort = presentation.sorts[0];
  const sortedItems = items;
  const groups = useMemo(() => presentation.grouping === "none" ? [] : projectReviewLanes(sortedItems, presentation.grouping), [sortedItems, presentation.grouping]);
  const displayedItems = groups.length ? groups.flatMap((lane) => lane.items) : sortedItems;
  const groupLabels = useMemo(() => new Map(groups.filter((lane) => lane.items[0]).map((lane) => [reviewFindingKey(lane.items[0]), `${lane.label} · ${lane.items.length}`])), [groups]);
  const rowHeight = (presentation.density === "compact" ? 52 : 72) + (presentation.grouping === "none" ? 0 : 18);
  useEffect(() => {
    const frame = window.requestAnimationFrame(() => { const scroller = rootRef.current?.querySelector<HTMLElement>(".react-aria-ResizableTableContainer, .react-aria-Virtualizer"); if (scroller) scroller.scrollTop = initialScrollTop; });
    return () => window.cancelAnimationFrame(frame);
  }, []);
  useEffect(() => () => {
    if (openFrameRef.current !== null) window.cancelAnimationFrame(openFrameRef.current);
  }, []);
  useEffect(() => {
    if (!selectedKey) return;
    const row = [...(rootRef.current?.querySelectorAll<HTMLElement>("[data-key]") ?? [])].find((node) => node.dataset.key === selectedKey);
    row?.scrollIntoView?.({ block: "nearest" });
  }, [selectedKey]);
  const table = (
      <Table aria-label={label} aria-busy={loading} selectionMode="single" selectionBehavior="replace" selectedKeys={selectedKeys} sortDescriptor={sort ? { column: sortToField(sort.field), direction: sort.direction === "asc" ? "ascending" : "descending" } : undefined} onSortChange={(descriptor) => onSort(fieldToSort(String(descriptor.column)), descriptor.direction === "ascending" ? "asc" : "desc")} onSelectionChange={(keys) => { if (keys === "all") return; const key = [...keys][0]; if (key != null) onSelect(String(key), "preview"); }} className="w-full min-w-full table-fixed text-left text-xs outline-none">
        <TableHeader columns={fields}>{(field) => { const definition = REVIEW_FIELD_REGISTRY[field]; const isFluidFinding = field === "finding" && presentation.widths[field] == null; return <Column id={field} isRowHeader={field === "finding"} allowsSorting={sortableField(field)} width={isFluidFinding ? "1fr" : presentation.widths[field] ?? definition.defaultWidth} minWidth={definition.min} maxWidth={isFluidFinding ? undefined : definition.max} className="sticky top-0 z-10 border-b border-hairline bg-control-surface px-3 py-2 text-left text-xs font-semibold uppercase tracking-[.12em] text-ink-subtle"><span>{definition.label}</span><ColumnResizer aria-label={`Resize ${definition.label}`} className="absolute right-0 top-1/2 h-5 w-2 -translate-y-1/2 cursor-col-resize rounded data-[focus-visible]:ring-2 data-[resizing]:bg-primary" /></Column>; }}</TableHeader>
        <TableBody items={displayedItems} renderEmptyState={() => <div className="p-6">{empty}</div>}>{(item) => <Row id={reviewFindingKey(item)} textValue={[item.finding.advisoryId, findingDisplayTitle(item.finding) ?? item.finding.packageName, item.repoName].join(" ")} columns={fields} className={({ isSelected, isFocusVisible }) => cn("cursor-default border-b border-hairline bg-surface-1 outline-none hover:bg-row-hover-surface", groupLabels.has(reviewFindingKey(item)) && "border-t border-t-hairline-strong", isSelected && "bg-row-selected-surface", isFocusVisible && "ring-2 ring-inset ring-primary-focus")} style={{ height: rowHeight }}>{(field) => <Cell className={cn("min-w-0 px-3 align-middle outline-none", presentation.density === "compact" ? "py-1.5" : "py-2.5")}>{field === "finding" && groupLabels.has(reviewFindingKey(item)) ? <p className="mb-1 text-xs font-semibold uppercase tracking-[.1em] text-ink-tertiary">{groupLabels.get(reviewFindingKey(item))}</p> : null}{renderField(field, item, fields.includes("package"))}</Cell>}</Row>}</TableBody>
      </Table>
  );
  const virtualized = items.length > 40;
  const content = <ResizableTableContainer className={cn("min-h-0 flex-1", virtualized ? "overflow-hidden" : "overflow-auto")} onResizeEnd={(widths) => onWidthsChange(Object.fromEntries([...widths].filter((entry): entry is [ReviewFieldId, number] => typeof entry[0] === "string" && typeof entry[1] === "number")))}>{virtualized ? <Virtualizer layout={TableLayout} layoutOptions={{ rowHeight, headingHeight: 42 }}>{table}</Virtualizer> : table}</ResizableTableContainer>;
  const handleScroll = (event: UIEvent<HTMLDivElement>) => { const target = event.target; if (target instanceof HTMLElement && target.scrollHeight > target.clientHeight) onScroll(target.scrollTop); };
  const scheduleOpen = (key: string) => {
    if (openFrameRef.current !== null) window.cancelAnimationFrame(openFrameRef.current);
    openFrameRef.current = window.requestAnimationFrame(() => {
      openFrameRef.current = null;
      onSelect(key, "open");
    });
  };
  const handleOpenKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" || !(event.target instanceof HTMLElement) || event.target.closest("input, textarea, select, button, a, [contenteditable=true]")) return;
    const key = event.target.closest<HTMLElement>("[role=row][data-key]")?.dataset.key ?? selectedKey;
    if (!key) return;
    event.preventDefault();
    event.stopPropagation();
    scheduleOpen(key);
  };
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLElement && event.target.closest("input, textarea, select, button, a, [contenteditable=true]")) return;
    if (event.key === "Escape") { event.preventDefault(); onDismiss(); return; }
    const delta = event.key.toLowerCase() === "j" ? 1 : event.key.toLowerCase() === "k" ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const nextKey = adjacentReviewKey(displayedItems, selectedKey, delta);
    if (nextKey) onSelect(nextKey, "preview");
  };
  const rowKeyFromClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!(event.target instanceof HTMLElement) || event.target.closest("button, a, input, select, textarea, [role=separator]")) return null;
    return event.target.closest<HTMLElement>("[role=row][data-key]")?.dataset.key ?? null;
  };
  const handleClick = (event: MouseEvent<HTMLDivElement>) => { const key = rowKeyFromClick(event); if (key) scheduleOpen(key); };
  return <div ref={rootRef} className="review-result-scroller flex h-full min-h-0 flex-col overflow-hidden rounded-md border border-hairline bg-surface-1" onClickCapture={handleClick} onScrollCapture={handleScroll} onKeyDownCapture={handleOpenKeyDown} onKeyDown={handleKeyDown}>{content}{hasMore ? <div className="flex shrink-0 items-center justify-between gap-3 border-t border-hairline bg-surface-1 px-3 py-2"><p className="text-xs text-ink-subtle"><span className="font-medium text-ink">{items.length}</span> of {total} findings loaded</p><Button size="sm" variant="secondary" disabled={loading} onClick={onLoadMore}>{loading ? "Loading…" : "Load more findings"}</Button></div> : null}</div>;
}

export function adjacentReviewKey(items: ReviewFinding[], selectedKey: string | null, delta: -1 | 1): string | null {
  if (!items.length) return null;
  const index = items.findIndex((item) => reviewFindingKey(item) === selectedKey);
  if(index<0)return reviewFindingKey(delta===1?items[0]:items[items.length-1]);
  const next = items[Math.min(items.length - 1, Math.max(0, index + delta))];
  return next ? reviewFindingKey(next) : null;
}

function sortableField(field: ReviewFieldId): boolean { return field === "finding" || field === "severity" || field === "repository" || field === "package" || field === "scannedAt"; }
function fieldToSort(field: string): ReviewSort { return field === "severity" ? "severity" : field === "repository" ? "repository" : field === "package" ? "package" : field === "scannedAt" ? "recent" : "priority"; }
function sortToField(sort: ReviewSort): ReviewFieldId { return sort === "recent" ? "scannedAt" : sort === "priority" ? "finding" : sort; }
function renderField(field: ReviewFieldId, item: ReviewFinding, separatePackageColumn: boolean): ReactNode {
  const finding = item.finding;
  if (field === "severity") return <Badge variant={finding.severity === "CRITICAL" ? "destructive" : finding.severity === "HIGH" ? "warning" : "secondary"}>{finding.severity}</Badge>;
  const title = field === "finding" ? findingDisplayTitle(finding) ?? (separatePackageColumn ? null : finding.packageName) : null;
  if (field === "finding") return <div className="min-w-0"><p className="truncate font-semibold text-ink">{finding.advisoryId}</p>{title ? <p className="mt-0.5 truncate text-ink-subtle">{title}</p> : null}</div>;
  if (field === "repository") return <span className="block truncate font-medium text-ink">{item.repoName}</span>;
  if (field === "package") return <span className="block truncate text-ink">{finding.packageName} <span className="text-ink-subtle">{finding.packageVersion}</span></span>;
  if (field === "fix") return finding.fixVersion ? <Badge variant="success">{finding.fixVersion}</Badge> : <span className="text-ink-tertiary">No fix</span>;
  if (field === "reviewState") return <Badge variant={item.reviewState === "unreviewed" ? "secondary" : "outline"}>{item.reviewState}</Badge>;
  if (field === "scannerState") return <Badge variant="outline">{finding.state}</Badge>;
  return <span className="text-ink-subtle">{item.scannedAt ? formatRelativeTimestamp(item.scannedAt) : "—"}</span>;
}
