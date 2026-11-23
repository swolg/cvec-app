import { ArrowDown, ArrowUp, Columns, SlidersHorizontal, SquaresFour } from "@phosphor-icons/react";

import { REVIEW_FIELD_REGISTRY, REVIEW_GROUPING_REGISTRY, type ReviewDensity, type ReviewFieldId, type ReviewGrouping, type ReviewPresentation, type ReviewPresentationMode } from "@/lib/reviewViews";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/form-controls";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "@/components/ui/overlays";

export function ReviewPresentationControls({ presentation, dirty, onChange, onReset }: { presentation: ReviewPresentation; dirty: boolean; onChange: (value: ReviewPresentation) => void; onReset: () => void }) {
  const setMode = (mode: ReviewPresentationMode) => {
    if (mode === presentation.mode) return;
    onChange(mode === "table"
      ? { mode: "table", fields: ["severity", "finding", "repository", "reviewState", "scannerState"], widths: {}, density: presentation.density, grouping: "none", sorts: [{ field: "priority", direction: "desc" }] }
      : { mode: "board", grouping: presentation.grouping === "none" ? "reviewState" : presentation.grouping, cardFields: ["finding", "severity", "repository", "package", "fix"], density: presentation.density, laneOrder: [], collapsedLaneIds: [] });
  };
  return (
    <div className="flex items-center gap-1">
      <div className="inline-flex rounded-md border border-hairline bg-control-surface p-0.5" aria-label="Presentation" role="group">
        <Button size="xs" variant={presentation.mode === "board" ? "secondary" : "ghost"} aria-pressed={presentation.mode === "board"} onClick={() => setMode("board")}><AppIcon className="size-4" icon={SquaresFour} />Board</Button>
        <Button size="xs" variant={presentation.mode === "table" ? "secondary" : "ghost"} aria-pressed={presentation.mode === "table"} onClick={() => setMode("table")}><AppIcon className="size-4" icon={Columns} />Table</Button>
      </div>
      <Popover>
        <PopoverTrigger render={<Button size="sm" variant="secondary" />}><AppIcon className="size-4" icon={SlidersHorizontal} />Customize{dirty ? <span className="size-1.5 rounded-full bg-primary" aria-label="Unsaved changes" /> : null}</PopoverTrigger>
        <PopoverContent align="end" className="grid max-h-[min(34rem,80vh)] w-80 gap-3 overflow-y-auto">
          <PopoverTitle className="font-semibold">Customize {presentation.mode}</PopoverTitle>
          <Select label="Density" value={presentation.density} onValueChange={(value) => onChange({ ...presentation, density: value as ReviewDensity })}><option value="comfortable">Comfortable</option><option value="compact">Compact</option></Select>
          <Select label="Group by" value={presentation.grouping} onValueChange={(value) => { const grouping = value as ReviewGrouping; if (presentation.mode === "table") onChange({ ...presentation, grouping }); else if (grouping !== "none") onChange({ ...presentation, grouping }); }}>{Object.entries(REVIEW_GROUPING_REGISTRY).filter(([id]) => presentation.mode === "table" || id !== "none").map(([id, value]) => <option key={id} value={id}>{value.label}{value.mutable ? " · movable" : ""}</option>)}</Select>
          <fieldset className="grid gap-1">
            <legend className="mb-1 text-xs font-semibold uppercase tracking-[.1em] text-ink-subtle">{presentation.mode === "table" ? "Columns" : "Card fields"}</legend>
            {Object.entries(REVIEW_FIELD_REGISTRY).map(([id, field]) => {
              const fieldId = id as ReviewFieldId;
              const values = presentation.mode === "table" ? presentation.fields : presentation.cardFields;
              const required = Boolean(field.required || (presentation.mode === "board" && fieldId === "severity"));
              const checked = required || values.includes(fieldId);
              const position = values.indexOf(fieldId);
              const commit = (next: ReviewFieldId[]) => onChange(presentation.mode === "table" ? { ...presentation, fields: next } : { ...presentation, cardFields: next });
              const move = (direction: -1 | 1) => { const next = [...values]; const target = Math.min(next.length - 1, Math.max(0, position + direction)); if (position < 0 || target === position) return; [next[position], next[target]] = [next[target], next[position]]; commit(next); };
              return <div key={id} className="flex min-h-9 items-center gap-1 rounded px-2 text-sm text-ink hover:bg-row-hover-surface"><label className="flex min-w-0 flex-1 items-center justify-between gap-2"><span>{field.label}{required ? " · required" : ""}</span><input type="checkbox" checked={checked} disabled={required} onChange={(event) => commit(event.target.checked ? [...values, fieldId] : values.filter((value) => value !== fieldId))} /></label>{checked && position >= 0 ? <><Button size="xs" variant="ghost" aria-label={`Move ${field.label} earlier`} disabled={position === 0} onClick={() => move(-1)}><AppIcon className="size-3" icon={ArrowUp} /></Button><Button size="xs" variant="ghost" aria-label={`Move ${field.label} later`} disabled={position === values.length - 1} onClick={() => move(1)}><AppIcon className="size-3" icon={ArrowDown} /></Button></> : null}</div>;
            })}
          </fieldset>
          {presentation.mode === "table" ? <TableSortControls presentation={presentation} onChange={onChange} /> : null}
          <Button variant="ghost" onClick={onReset} disabled={!dirty}>Reset presentation</Button>
        </PopoverContent>
      </Popover>
    </div>
  );
}

function TableSortControls({ presentation, onChange }: { presentation: Extract<ReviewPresentation, { mode: "table" }>; onChange: (value: ReviewPresentation) => void }) {
  const fields = ["priority", "severity", "recent", "repository", "package"] as const;
  const update = (index: number, value: Partial<(typeof presentation.sorts)[number]>) => onChange({ ...presentation, sorts: presentation.sorts.map((sort, sortIndex) => sortIndex === index ? { ...sort, ...value } : sort) });
  const unused = fields.find((field) => !presentation.sorts.some((sort) => sort.field === field));
  return <fieldset className="grid gap-2"><legend className="text-xs font-semibold uppercase tracking-[.1em] text-ink-subtle">Sorting</legend>{presentation.sorts.map((sort, index) => <div key={sort.field} className="grid grid-cols-[1fr_7rem_auto] items-end gap-1"><Select label={index === 0 ? "Primary sort" : `Then by ${index}`} value={sort.field} onValueChange={(value) => update(index, { field: value as typeof sort.field })}>{fields.map((field) => <option key={field} value={field} disabled={presentation.sorts.some((entry, entryIndex) => entryIndex !== index && entry.field === field)}>{field}</option>)}</Select><Select label="Direction" value={sort.direction} onValueChange={(value) => update(index, { direction: value as "asc" | "desc" })}><option value="desc">Descending</option><option value="asc">Ascending</option></Select><Button size="xs" variant="ghost" aria-label={`Remove sort ${index + 1}`} disabled={index === 0} onClick={() => onChange({ ...presentation, sorts: presentation.sorts.filter((_, sortIndex) => sortIndex !== index) })}>Remove</Button></div>)}<Button size="xs" variant="ghost" disabled={!unused || presentation.sorts.length >= 3} onClick={() => unused && onChange({ ...presentation, sorts: [...presentation.sorts, { field: unused, direction: "desc" }] })}>Add secondary sort</Button></fieldset>;
}
