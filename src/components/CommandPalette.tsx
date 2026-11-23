import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowClockwise, ChatCircleDots, Command, GearSix, House, Kanban, Keyboard, MagnifyingGlass, ShieldCheckered, GlobeHemisphereWest } from "@phosphor-icons/react";

import { AppIcon, type AppIconGlyph } from "./ui/app-icon";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogTitle } from "./ui/overlays";
import { searchLocalEntities } from "@/lib/api";
import type { LocalEntityResult, RepoSummary } from "@/lib/types";
import type { ReviewViewDefinition } from "@/lib/reviewViews";
import type { WorkspaceId } from "@/lib/navigation";

interface CommandItem {
  id: string;
  group: string;
  label: string;
  context?: string;
  icon: AppIconGlyph;
  disabledReason?: string;
  startsScan?: boolean;
  keywords: string;
  execute: () => void;
}
type LocalSearch = { query: string; status: "ready" | "error"; results: LocalEntityResult[] };
const MAX_SEARCH_CHARACTERS = 200;
const entityLabels: Record<LocalEntityResult["kind"], string> = {
  repository: "Repository",
  finding: "Finding",
  package: "Package",
  advisory: "Advisory",
};

export function CommandPalette({ open, onOpenChange, repos, views, scannerReady, scanRunning, onNavigate, onOpenRepo, onOpenEntity, onOpenView, onScanAll, onScanRepo, onShowActivity }: { open: boolean; onOpenChange: (value: boolean) => void; repos: RepoSummary[]; views: ReviewViewDefinition[]; scannerReady: boolean; scanRunning: boolean; onNavigate: (workspace: WorkspaceId) => void; onOpenRepo: (repoId: number) => void; onOpenEntity: (result: LocalEntityResult) => void; onOpenView: (viewId: string) => void; onScanAll: () => void; onScanRepo: (repoId: number) => void; onShowActivity: () => void }) {
  const [query, setQuery] = useState("");
  const [localSearch, setLocalSearch] = useState<LocalSearch | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const activeOptionRef = useRef<HTMLButtonElement>(null);
  const revealKeyboardSelection = useRef(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const needle = query.trim();
  const searchTooLong = Array.from(needle).length > MAX_SEARCH_CHARACTERS;
  const currentSearch = open && needle && localSearch?.query === needle ? localSearch : null;
  const isSearching = open && Boolean(needle) && !searchTooLong && !currentSearch;

  useEffect(() => {
    if (open) return;
    setQuery("");
    setLocalSearch(null);
    setSelectedId(null);
  }, [open]);
  useEffect(() => {
    if (!open || !needle || searchTooLong) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void searchLocalEntities(needle).then(
        (results) => { if (active) setLocalSearch({ query: needle, status: "ready", results }); },
        () => { if (active) setLocalSearch({ query: needle, status: "error", results: [] }); },
      );
    }, 120);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [open, needle, searchTooLong]);

  const commands = useMemo<CommandItem[]>(() => {
    if (!open) return [];
    const destinations: Array<[WorkspaceId, string, AppIconGlyph]> = [["overview", "Open Home", House], ["review", "Open Review", ShieldCheckered], ["chat", "Open Chat", ChatCircleDots], ["repositories", "Open Repositories", Kanban], ["intel", "Open Intelligence", GlobeHemisphereWest], ["settings", "Open Settings", GearSix]];
    return [
      ...destinations.map(([workspace, label, icon]) => ({ id: `nav-${workspace}`, group: "Navigate", label, icon, keywords: `${label} ${workspace}`, execute: () => onNavigate(workspace) })),
      ...views.map((view) => ({ id: `view-${view.id}`, group: "View", label: view.name, context: "Review view", icon: ShieldCheckered, keywords: `${view.name} review view`, execute: () => onOpenView(view.id) })),
      { id: "help-shortcuts", group: "Help", label: "Keyboard shortcuts", context: "Show the shortcut map", icon: Keyboard, keywords: "help keyboard shortcuts command j k enter escape", execute: () => setShortcutsOpen(true) },
      { id: "scan-all", group: "Scan", label: scanRunning ? "Open current scan" : "Scan all repositories", context: scanRunning ? "A scan is already running" : `${repos.length} repositories`, icon: ArrowClockwise, disabledReason: !scanRunning && !scannerReady ? "Scanner setup required" : undefined, startsScan: !scanRunning, keywords: "scan all repositories", execute: scanRunning ? onShowActivity : onScanAll },
      ...repos.map((repo) => ({ id: `scan-${repo.id}`, group: "Scan", label: scanRunning ? `View scan activity for ${repo.name}` : `Scan ${repo.name}`, context: repo.path, icon: ArrowClockwise, disabledReason: !scanRunning && !scannerReady ? "Scanner setup required" : undefined, startsScan: !scanRunning, keywords: `scan repository ${repo.name}`, execute: scanRunning ? onShowActivity : () => onScanRepo(repo.id) })),
    ];
  }, [open, onNavigate, onOpenView, onScanAll, onScanRepo, onShowActivity, repos, scanRunning, scannerReady, views]);

  const normalized = needle.toLowerCase();
  const filteredCommands = commands.filter((command) => !normalized || `${command.label} ${command.context ?? ""} ${command.keywords}`.toLowerCase().includes(normalized));
  const entityCommands: CommandItem[] = (currentSearch?.results ?? []).map((entity) => ({
    id: entity.id, group: entityLabels[entity.kind], label: entity.label, context: entity.context, icon: MagnifyingGlass,
    keywords: "", execute: () => entity.kind === "repository" && entity.repoId
      ? onOpenRepo(entity.repoId) : onOpenEntity(entity),
  }));
  const scanQuery = /^scan(?:\s|$)/.test(normalized);
  const results = scanQuery ? [...filteredCommands, ...entityCommands] : [...entityCommands, ...filteredCommands];
  // A general search can suggest scan commands, but Enter should only start
  // one after deliberate selection or a query beginning with "scan".
  const active = results.find((command) => command.id === selectedId)
    ?? results.find((command) => scanQuery || !command.startsScan);
  const selectedIndex = active ? results.indexOf(active) : -1;
  useEffect(() => {
    if (revealKeyboardSelection.current) {
      revealKeyboardSelection.current = false;
      activeOptionRef.current?.scrollIntoView?.({ block: "nearest" });
    }
  }, [active?.id]);
  const searchMessage = searchTooLong ? "Search supports up to 200 characters."
    : isSearching ? "Searching local data…"
    : currentSearch?.status === "error" ? "Could not search local data. Edit your query to try again."
    : null;

  function execute(command: CommandItem | undefined) { if (!command || command.disabledReason) return; command.execute(); if (command.id !== "help-shortcuts") onOpenChange(false); }

  function handleKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const offset = event.key === "ArrowDown" ? 1 : -1;
      const index = Math.max(0, Math.min(results.length - 1, selectedIndex + offset));
      if (results[index] && index !== selectedIndex) {
        revealKeyboardSelection.current = true;
        setSelectedId(results[index].id);
      }
    } else if (event.key === "Enter") {
      event.preventDefault();
      execute(active);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent initialFocus={inputRef} showClose={false} className="max-w-2xl overflow-hidden p-0">
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <div className="flex items-center gap-3 border-b border-hairline px-4"><AppIcon className="size-4 text-ink-subtle" icon={MagnifyingGlass} /><input ref={inputRef} role="combobox" aria-label="Search commands and local data" aria-controls="command-results" aria-expanded="true" aria-activedescendant={active ? `command-${active.id}` : undefined} className="h-14 min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-tertiary" placeholder="Search cvec or run a command…" value={query} onChange={(event) => { if (event.target.value.trim() !== needle) setLocalSearch(null); setQuery(event.target.value); setSelectedId(null); }} onKeyDown={handleKeyDown} /><kbd className="rounded-sm border border-hairline bg-surface-3 px-1.5 py-0.5 text-xs text-ink-subtle">ESC</kbd></div>
        <p role="status" className={searchMessage ? "px-4 pt-3 text-xs text-ink-subtle" : "sr-only"}>{searchMessage ?? `${results.length} results`}</p>
        <div id="command-results" role="listbox" className="max-h-[min(28rem,65vh)] overflow-auto p-2">
          {results.length ? results.map((command, index) => <Button
            key={command.id} ref={index === selectedIndex ? activeOptionRef : undefined} id={`command-${command.id}`}
            role="option" tabIndex={-1} aria-selected={index === selectedIndex} aria-disabled={Boolean(command.disabledReason)}
            className={`h-auto w-full justify-start gap-3 rounded-md px-3 py-2.5 text-left ${index === selectedIndex ? "bg-row-selected-surface" : ""}`}
            variant="ghost"
            onMouseMove={() => { if (!command.startsScan) setSelectedId(command.id); }}
            onClick={() => execute(command)}
          ><AppIcon className="size-4 shrink-0 text-ink-subtle" icon={command.icon} /><span className="min-w-0 flex-1"><span className="block truncate text-sm font-medium text-ink">{command.label}</span><span className="block truncate text-xs text-ink-subtle">{command.disabledReason ?? command.context ?? command.group}</span></span><span className="text-xs text-ink-tertiary">{command.group}</span></Button>) : searchMessage ? null : <div className="px-4 py-12 text-center text-sm text-ink-subtle">No local results or commands match.</div>}
        </div>
        <div className="flex items-center justify-between border-t border-hairline px-4 py-2 text-xs text-ink-tertiary"><span>↑↓ Navigate · Enter open</span><span className="flex items-center gap-1"><AppIcon className="size-3" icon={Command} />K anywhere</span></div>
      </DialogContent>
      <Dialog open={shortcutsOpen} onOpenChange={setShortcutsOpen}><DialogContent><DialogTitle className="border-b border-hairline px-5 py-4 text-base font-semibold">Keyboard shortcuts</DialogTitle><dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 px-5 py-4 text-sm"><Shortcut keys="⌘/Ctrl K" label="Open command palette" /><Shortcut keys="Option/Alt ←" label="Go back" /><Shortcut keys="↑ ↓ or J K" label="Move through the active work list" /><Shortcut keys="Space" label="Preview the selected finding" /><Shortcut keys="Enter" label="Open the inspector or run a command" /><Shortcut keys="Esc" label="Close the top-most overlay" /></dl><p className="border-t border-hairline px-5 py-3 text-xs text-ink-subtle">Shortcuts pause while you edit text or use menus and form controls.</p></DialogContent></Dialog>
    </Dialog>
  );
}

function Shortcut({ keys, label }: { keys: string; label: string }) { return <><dt><kbd className="rounded-sm border border-hairline bg-surface-3 px-2 py-1 text-xs text-ink">{keys}</kbd></dt><dd className="text-ink-muted">{label}</dd></>; }
