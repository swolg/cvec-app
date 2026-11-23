import { Archive, CaretDown, CaretRight, ChatCircleDots, Check, GitBranch, GithubLogo, Plus } from "@phosphor-icons/react";
import { Combobox } from "@base-ui/react/combobox";
import { useMemo, useRef, useState } from "react";

import { AppIcon } from "@/components/ui/app-icon";
import { Button, IconButton } from "@/components/ui/button";
import { SearchField } from "@/components/ui/form-controls";
import type { CompanionThread, RepoSummary } from "@/lib/types";

interface ConversationRailProps {
  repositories: RepoSummary[];
  threads: CompanionThread[];
  selectedRepositoryId: number | null;
  activeThreadId: number | null;
  onSelectRepository: (repositoryId: number) => void;
  onSelectConversation: (repositoryId: number, threadId: number) => void;
  onNewConversation: (repositoryId: number) => void;
}

const INITIAL_VISIBLE_CONVERSATIONS = 5;
const CONVERSATION_PAGE_SIZE = 5;

export function ConversationRail({
  repositories,
  threads,
  selectedRepositoryId,
  activeThreadId,
  onSelectRepository,
  onSelectConversation,
  onNewConversation,
}: ConversationRailProps) {
  const [query, setQuery] = useState("");
  const [showArchived, setShowArchived] = useState(false);
  const [groupExpansion, setGroupExpansion] = useState<Record<string, boolean>>({});
  const [visibleCounts, setVisibleCounts] = useState<Record<number, number>>({});
  const groups = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return repositories.map((repository) => {
      const repositoryMatches = `${repository.name} ${repository.path} ${repository.remoteHost ?? ""}`.toLocaleLowerCase().includes(needle);
      const conversations = threads
        .filter((thread) => thread.repositoryId === repository.id && Boolean(thread.archivedAt) === showArchived)
        .filter((thread) => !needle || repositoryMatches || thread.title.toLocaleLowerCase().includes(needle))
        .sort((a, b) => b.lastOpenedAt.localeCompare(a.lastOpenedAt));
      return { repository, repositoryMatches, conversations };
    }).filter((group) => !needle || group.repositoryMatches || group.conversations.length > 0);
  }, [query, repositories, threads, showArchived]);

  function toggle(repositoryId: number, expanded: boolean) {
    setGroupExpansion((current) => ({ ...current, [`${showArchived}:${repositoryId}`]: !expanded }));
  }

  function showMore(repositoryId: number) {
    setVisibleCounts((current) => ({
      ...current,
      [repositoryId]: (current[repositoryId] ?? INITIAL_VISIBLE_CONVERSATIONS) + CONVERSATION_PAGE_SIZE,
    }));
  }

  return (
    <aside aria-label="Repository conversations" className="app-panel hidden min-h-0 flex-col overflow-hidden rounded-lg border border-hairline bg-surface-1 xl:flex">
      <div className="shrink-0 border-b border-hairline p-3">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h1 className="text-sm font-semibold text-ink">Chat</h1>
            <p className="mt-0.5 text-xs text-ink-subtle">Repository conversations</p>
          </div>
          <IconButton
            aria-label="New conversation"
            disabled={selectedRepositoryId === null}
            size="icon-sm"
            variant="secondary"
            onClick={() => selectedRepositoryId !== null && onNewConversation(selectedRepositoryId)}
          >
            <AppIcon className="size-4" icon={Plus} />
          </IconButton>
        </div>
        <SearchField aria-label="Search repositories and conversations" className="mt-3" placeholder={showArchived ? "Search archived chats" : "Search chats"} value={query} onChange={(event) => setQuery(event.target.value)} />
        <Button aria-label="Show archived conversations" aria-pressed={showArchived} className="mt-2 h-8 w-full justify-start text-xs" size="sm" variant={showArchived ? "secondary" : "ghost"} onClick={() => setShowArchived((value) => !value)}><AppIcon className="size-3.5" icon={Archive} />Archived<span className="ml-auto tabular-nums">{threads.filter((thread) => thread.archivedAt).length}</span></Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]">
        {groups.map(({ repository, conversations }) => {
          const activeRepository = repository.id === selectedRepositoryId;
          const defaultExpanded = showArchived || (activeRepository && activeThreadId !== null);
          const isCollapsed = !query && !(groupExpansion[`${showArchived}:${repository.id}`] ?? defaultExpanded);
          const visibleLimit = visibleCounts[repository.id] ?? INITIAL_VISIBLE_CONVERSATIONS;
          const visibleConversations = query
            ? conversations
            : conversations.slice(0, visibleLimit);
          const activeConversation = conversations.find((thread) => thread.id === activeThreadId);
          if (!query && activeConversation && !visibleConversations.some((thread) => thread.id === activeConversation.id)) {
            visibleConversations.splice(Math.max(0, visibleConversations.length - 1), 1, activeConversation);
          }
          const hiddenConversationCount = conversations.length - visibleConversations.length;
          const nextConversationCount = Math.min(CONVERSATION_PAGE_SIZE, hiddenConversationCount);
          return (
            <section className="border-b border-hairline" key={repository.id}>
              <div className={`flex items-center gap-1 px-2 py-1.5 ${activeRepository ? "bg-surface-2" : ""}`}>
                <IconButton aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${repository.name}`} aria-expanded={!isCollapsed} size="icon-xs" variant="ghost" onClick={() => toggle(repository.id, !isCollapsed)}>
                  <AppIcon className="size-3.5" icon={isCollapsed ? CaretRight : CaretDown} />
                </IconButton>
                <Button className="h-8 min-w-0 flex-1 justify-start gap-2 px-1.5" variant="ghost" onClick={() => onSelectRepository(repository.id)}>
                  <AppIcon className="size-4 shrink-0 text-ink-subtle" icon={repository.sourceType === "github" ? GithubLogo : GitBranch} />
                  <span className="truncate text-sm font-medium">{repository.name}</span>
                  {" "}
                  <span className="ml-auto text-xs tabular-nums text-ink-subtle">{conversations.length}</span>
                </Button>
              </div>
              {!isCollapsed ? (
                <div className="pb-1.5 pl-7 pr-2">
                  {visibleConversations.map((thread) => (
                    <Button
                      aria-current={thread.id === activeThreadId ? "page" : undefined}
                      className={`h-auto w-full justify-start rounded-md px-2 py-2 text-left ${thread.id === activeThreadId ? "bg-row-selected-surface text-ink" : "text-ink-subtle"}`}
                      key={thread.id}
                      variant="ghost"
                      onClick={() => onSelectConversation(repository.id, thread.id)}
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium text-ink">{thread.title}</span>
                        <span className="mt-0.5 block text-xs text-ink-subtle">{relativeTime(thread.lastOpenedAt)}</span>
                      </span>
                    </Button>
                  ))}
                  {!query && hiddenConversationCount > 0 ? (
                    <Button
                      className="h-8 w-full justify-start px-2 text-xs text-ink-subtle"
                      variant="ghost"
                      onClick={() => showMore(repository.id)}
                    >
                      Show {nextConversationCount} more
                      <span className="ml-auto tabular-nums text-ink-subtle">{hiddenConversationCount} remaining</span>
                    </Button>
                  ) : null}
                  {!conversations.length && showArchived ? <p className="px-2 py-2 text-xs text-ink-subtle">No archived conversations.</p> : !conversations.length ? (
                    <Button className="h-8 w-full justify-start px-2 text-xs text-ink-subtle" variant="ghost" onClick={() => onNewConversation(repository.id)}>
                      <AppIcon className="size-3.5" icon={ChatCircleDots} />Start a conversation
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </section>
          );
        })}
        {!groups.length ? <p className="px-4 py-8 text-center text-xs text-ink-subtle">No repository or conversation matches.</p> : null}
      </div>
    </aside>
  );
}

interface ConversationChoice { value: string; label: string; repositoryId: number; threadId: number | null }

export function CompactConversationSelector({
  repositories,
  threads,
  selectedRepositoryId,
  activeThreadId,
  onSelectConversation,
  onNewConversation,
}: ConversationRailProps) {
  const value = activeThreadId ? `thread:${activeThreadId}` : selectedRepositoryId ? `repo:${selectedRepositoryId}` : "";
  const [query, setQuery] = useState("");
  const newDraftFocus = useRef(false);
  const choices = useMemo(() => repositories.flatMap<ConversationChoice>((repository) => [
    { value: `repo:${repository.id}`, label: `${repository.name} · new draft`, repositoryId: repository.id, threadId: null },
    ...threads.filter((thread) => thread.repositoryId === repository.id).sort((a, b) => b.lastOpenedAt.localeCompare(a.lastOpenedAt)).map((thread) => ({
      value: `thread:${thread.id}`, label: `${repository.name} · ${thread.archivedAt ? "Archived: " : ""}${thread.title}`, repositoryId: repository.id, threadId: thread.id,
    })),
  ]), [repositories, threads]);
  const selected = choices.find((choice) => choice.value === value) ?? null;
  const matches = useMemo(() => choices.filter((choice) => choice.label.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())), [choices, query]);
  const visible = matches.slice(0, 200);
  if (!query.trim() && selected && !visible.includes(selected)) visible.splice(Math.max(0, visible.length - 1), 1, selected);
  return (
    <div className="chat-compact-selector flex shrink-0 items-center gap-2 border-b border-hairline bg-surface-2 p-2 xl:hidden">
      <Combobox.Root<ConversationChoice> items={choices} filteredItems={visible} filter={null} value={selected} inputValue={query} onInputValueChange={setQuery} isItemEqualToValue={(a, b) => a.value === b.value}
        onOpenChange={(open) => { if (open) { setQuery(""); newDraftFocus.current = false; } }}
        onValueChange={(next) => {
          if (!next) return;
          newDraftFocus.current = next.threadId === null;
          if (next.threadId === null) onNewConversation(next.repositoryId);
          else onSelectConversation(next.repositoryId, next.threadId);
        }}>
        <Combobox.Trigger aria-label="Repository and conversation" render={<Button className="h-9 min-w-0 flex-1 justify-between gap-2 px-3" variant="secondary" />}>
          <span className="min-w-0 flex-1 truncate text-left"><Combobox.Value placeholder="Choose a repository or conversation" /></span><AppIcon icon={CaretDown} className="size-4 shrink-0" />
        </Combobox.Trigger>
        <Combobox.Portal><Combobox.Positioner align="start" sideOffset={6} className="z-[80] outline-none">
          <Combobox.Popup aria-label="Choose repository and conversation" finalFocus={() => !newDraftFocus.current} className="flex max-h-[var(--available-height)] w-[min(28rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-lg border border-hairline bg-surface-3 text-ink shadow-xl">
            <Combobox.Input aria-label="Search repositories and conversations" placeholder="Search repositories and conversations…" className="m-2 h-9 shrink-0 rounded-md border border-hairline bg-surface-1 px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-primary-focus" />
            <Combobox.Empty className="p-3 text-sm text-ink-subtle">No matching conversation.</Combobox.Empty>
            <Combobox.List aria-label="Conversation results" className="max-h-72 min-h-0 overflow-y-auto p-1.5">{(choice: ConversationChoice) => <Combobox.Item key={choice.value} value={choice} className="relative flex min-h-10 cursor-default items-center rounded-md py-2 pl-8 pr-2 text-sm outline-none data-[highlighted]:bg-row-hover-surface data-[selected]:bg-navigation-active-surface">
              <Combobox.ItemIndicator className="absolute left-2 text-primary"><AppIcon icon={Check} className="size-3.5" /></Combobox.ItemIndicator><span className="min-w-0 truncate">{choice.label}</span>
            </Combobox.Item>}</Combobox.List>
            {matches.length > visible.length ? <p role="status" className="border-t border-hairline p-2 text-xs text-ink-subtle">Showing {visible.length} of {matches.length} matches. Refine your search.</p> : null}
          </Combobox.Popup>
        </Combobox.Positioner></Combobox.Portal>
      </Combobox.Root>
      <IconButton aria-label="New conversation" disabled={selectedRepositoryId === null} size="icon-sm" variant="secondary" onClick={() => selectedRepositoryId !== null && onNewConversation(selectedRepositoryId)}>
        <AppIcon className="size-4" icon={Plus} />
      </IconButton>
    </div>
  );
}

function relativeTime(value: string): string {
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return "Saved locally";
  const seconds = Math.max(0, Math.round((Date.now() - time) / 1000));
  if (seconds < 60) return "Just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return days < 30 ? `${days}d ago` : new Date(time).toLocaleDateString();
}
