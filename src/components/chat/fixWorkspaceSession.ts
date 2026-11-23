import { companionClient } from "@/lib/companionApi";
import { ChatDraftStore, useChatDraftState } from "@/lib/chatDrafts";

// Drafts use the same profile-local persistence as Analysis. Selection remains
// UI-only; restoring a draft never starts a provider or submits a task.
const drafts = new ChatDraftStore(companionClient);
export type FixWorkspaceView = "conversation" | "diff" | "checks" | "rescan";
const selections = new Map<string, string>();
const views = new Map<string, FixWorkspaceView>();
const LIMIT = 128;

function retain<T>(map: Map<string, T>, key: string, value: T) {
  map.delete(key);
  map.set(key, value);
  while (map.size > LIMIT) map.delete(map.keys().next().value!);
}

export function fixDraftKey(repositoryId: number, workspaceId: string | null, finding?: string) {
  return JSON.stringify([repositoryId, workspaceId, workspaceId ? null : finding ?? null]);
}

function draftScope(key: string): [number, string] {
  const [repositoryId, workspaceId, finding] = JSON.parse(key) as [number, string | null, string | null];
  return [repositoryId, JSON.stringify(["fix", workspaceId, finding])];
}
export function readFixDraft(key: string) { return drafts.getSnapshot(...draftScope(key)).text ?? undefined; }
export function loadFixDraft(key: string) { return drafts.load(...draftScope(key)); }
export function saveFixDraft(key: string, value: string) { return drafts.save(...draftScope(key), value); }
export function retryFixDraft(key: string) { return drafts.retry(...draftScope(key)); }
export function useFixDraftState(key: string) { return useChatDraftState(drafts, ...draftScope(key)); }
export function readFixSelection(repositoryId: number, finding?: string) { return selections.get(JSON.stringify([repositoryId, finding ?? null])); }
export function saveFixSelection(repositoryId: number, finding: string | undefined, id: string) { retain(selections, JSON.stringify([repositoryId, finding ?? null]), id); }
export function readFixWorkspaceView(repositoryId: number, workspaceId: string): FixWorkspaceView {
  return views.get(fixDraftKey(repositoryId, workspaceId)) ?? "conversation";
}
export function saveFixWorkspaceView(repositoryId: number, workspaceId: string, view: FixWorkspaceView) {
  retain(views, fixDraftKey(repositoryId, workspaceId), view);
}
export function clearFixWorkspaceSession() {
  drafts.clearMemory();
  selections.clear();
  views.clear();
}
