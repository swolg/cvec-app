import { useEffect, useRef, type KeyboardEvent, type ReactNode } from "react";
import { CaretDown, CaretUp } from "@phosphor-icons/react";

import { AppIcon } from "./app-icon";
import { Button } from "./button";
import { cn } from "@/lib/utils";

export interface WorkListColumn<T> {
  id: string;
  label: string;
  className?: string;
  sortable?: boolean;
  render: (item: T) => ReactNode;
}

interface WorkListProps<T> {
  items: T[];
  columns: WorkListColumn<T>[];
  getKey: (item: T) => string;
  selectedKey: string | null;
  onSelect: (key: string, mode: "preview" | "open") => void;
  sort?: { column: string; direction: "asc" | "desc" } | null;
  onSort?: (column: string) => void;
  label: string;
  empty?: ReactNode;
  className?: string;
  onDismiss?: () => void;
  initialScrollTop?: number;
  onScrollTopChange?: (value: number) => void;
}

function keyOwnedByControl(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return Boolean(target.closest("input, textarea, select, [contenteditable=true], [role=menu], [role=listbox], button, a"));
}

export function WorkList<T>({ items, columns, getKey, selectedKey, onSelect, sort, onSort, label, empty, className, onDismiss, initialScrollTop = 0, onScrollTopChange }: WorkListProps<T>) {
  const rowRefs = useRef(new Map<string, HTMLTableRowElement>());
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => { if (containerRef.current) containerRef.current.scrollTop = initialScrollTop; }, []);

  useEffect(() => {
    if (selectedKey) rowRefs.current.get(selectedKey)?.scrollIntoView?.({ block: "nearest" });
  }, [selectedKey]);

  function handleKey(event: KeyboardEvent<HTMLTableRowElement>, index: number, key: string) {
    if (event.defaultPrevented || event.nativeEvent.isComposing || keyOwnedByControl(event.target)) return;
    const previous = event.key === "ArrowUp" || event.key.toLowerCase() === "k";
    const next = event.key === "ArrowDown" || event.key.toLowerCase() === "j";
    if (previous || next) {
      event.preventDefault();
      const nextIndex = Math.min(items.length - 1, Math.max(0, index + (next ? 1 : -1)));
      const nextKey = getKey(items[nextIndex]);
      onSelect(nextKey, "preview");
      rowRefs.current.get(nextKey)?.focus();
    } else if (event.key === " ") {
      event.preventDefault();
      onSelect(key, "preview");
    } else if (event.key === "Enter") {
      event.preventDefault();
      onSelect(key, "open");
    } else if (event.key === "Escape" && onDismiss) {
      event.preventDefault();
      onDismiss();
    }
  }

  if (items.length === 0) return <>{empty}</>;

  return (
    <div ref={containerRef} onScroll={(event) => onScrollTopChange?.(event.currentTarget.scrollTop)} className={cn("min-h-0 overflow-auto rounded-md border border-hairline bg-surface-1", className)}>
      <table className="w-full table-fixed text-left text-xs" aria-label={label}>
        <thead className="sticky top-0 z-10 bg-control-surface text-xs uppercase tracking-[0.12em] text-ink-subtle">
          <tr>
            {columns.map((column) => {
              const direction = sort?.column === column.id ? sort.direction : undefined;
              return (
                <th key={column.id} className={cn("border-b border-hairline px-3 py-2", column.className)} aria-sort={direction ? (direction === "asc" ? "ascending" : "descending") : undefined}>
                  {column.sortable ? (
                    <Button type="button" size="xs" variant="ghost" className="h-8 px-1 text-left" onClick={() => onSort?.(column.id)}>
                      {column.label}
                      {direction ? <AppIcon className="size-3" icon={direction === "asc" ? CaretUp : CaretDown} /> : null}
                    </Button>
                  ) : column.label}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {items.map((item, index) => {
            const key = getKey(item);
            const selected = selectedKey === key;
            return (
              <tr
                aria-selected={selected}
                className={cn("cursor-pointer border-t border-hairline outline-none transition-colors hover:bg-row-hover-surface focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus", selected && "bg-row-selected-surface")}
                data-selected={selected || undefined}
                key={key}
                onClick={() => onSelect(key, "open")}
                onKeyDown={(event) => handleKey(event, index, key)}
                ref={(node) => { if (node) rowRefs.current.set(key, node); else rowRefs.current.delete(key); }}
                tabIndex={selected || (!selectedKey && index === 0) ? 0 : -1}
              >
                {columns.map((column) => <td key={column.id} className={cn("px-3 py-2.5 align-top", column.className)}>{column.render(item)}</td>)}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
