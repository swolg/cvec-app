import { useCallback, useSyncExternalStore } from "react";
import { limitDraftText } from "./chatDraftText";

export interface ChatDraftClient {
  readDraft(repositoryId: number, scope: string): Promise<string | null>;
  writeDraft(repositoryId: number, scope: string, text: string): Promise<void>;
}

export interface ChatDraftState {
  text: string | null;
  loaded: boolean;
  saving: boolean;
  error: "restore" | "save" | null;
}

interface Entry {
  state: ChatDraftState;
  revision: number;
  dirty: boolean;
  reading?: Promise<string | null>;
  writing?: Promise<void>;
  listeners: Set<() => void>;
}

// The backend stores these in the current profile's database. Never use a
// provider, credentials, or WebView storage shared by different QA profiles.
export class ChatDraftStore {
  private readonly entries = new Map<string, Entry>();
  constructor(private readonly client: ChatDraftClient) {}

  private entry(repositoryId: number, scope: string): Entry {
    const key = JSON.stringify([repositoryId, scope]);
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { state: { text: null, loaded: false, saving: false, error: null }, revision: 0, dirty: false, listeners: new Set() };
      this.entries.set(key, entry);
    }
    return entry;
  }

  getSnapshot(repositoryId: number, scope: string): ChatDraftState {
    return this.entry(repositoryId, scope).state;
  }

  subscribe(repositoryId: number, scope: string, listener: () => void): () => void {
    const entry = this.entry(repositoryId, scope);
    entry.listeners.add(listener);
    return () => entry.listeners.delete(listener);
  }

  private update(entry: Entry, change: Partial<ChatDraftState>) {
    entry.state = { ...entry.state, ...change };
    entry.listeners.forEach((listener) => listener());
  }

  private trim() {
    for (const [key, entry] of this.entries) {
      if (this.entries.size <= 128) break;
      if (!entry.listeners.size && !entry.reading && !entry.writing && !entry.dirty) this.entries.delete(key);
    }
  }

  async load(repositoryId: number, scope: string): Promise<string | null> {
    const entry = this.entry(repositoryId, scope);
    if (entry.state.loaded) return entry.state.text;
    if (entry.reading) return entry.reading;
    const revision = entry.revision;
    entry.reading = this.client.readDraft(repositoryId, scope).then((text) => {
      // A slow restore must never replace a newer edit or a cleared draft.
      if (entry.revision === revision) this.update(entry, { text, loaded: true, error: null });
      return entry.state.text;
    }).catch((cause) => {
      if (entry.revision !== revision) return entry.state.text;
      this.update(entry, { error: "restore" });
      throw cause;
    }).finally(() => { entry.reading = undefined; this.trim(); });
    return entry.reading;
  }

  save(repositoryId: number, scope: string, text: string): Promise<void> {
    const entry = this.entry(repositoryId, scope);
    entry.revision += 1;
    entry.dirty = true;
    this.update(entry, { text: limitDraftText(text), loaded: true });
    return this.write(repositoryId, scope, entry);
  }

  private write(repositoryId: number, scope: string, entry: Entry): Promise<void> {
    if (entry.writing) return entry.writing;
    // One writer per draft, coalescing edits that arrive during a write.
    // In particular, an older save cannot resurrect a submitted/cleared draft.
    entry.writing = (async () => {
      while (entry.dirty) {
        const revision = entry.revision;
        await this.client.writeDraft(repositoryId, scope, entry.state.text ?? "");
        entry.dirty = revision !== entry.revision;
      }
      this.update(entry, { error: null });
    })().catch(() => { this.update(entry, { error: "save" }); })
      .finally(() => {
        entry.writing = undefined;
        // A save may arrive after the drain loop but before this handler.
        // Chain its writer so pending callers await it; failures still need retry.
        if (entry.dirty && entry.state.error === null) return this.write(repositoryId, scope, entry);
        this.update(entry, { saving: false });
        this.trim();
      });
    // Subscribers may save another edit when they observe the saving state.
    this.update(entry, { saving: true });
    return entry.writing;
  }

  async retry(repositoryId: number, scope: string): Promise<void> {
    const entry = this.entry(repositoryId, scope);
    if (!entry.state.loaded) { await this.load(repositoryId, scope); return; }
    if (entry.dirty) await this.write(repositoryId, scope, entry);
  }

  // Clears only the UI cache; persisted drafts are retained.
  clearMemory(): void { this.entries.clear(); }
}

export function useChatDraftState(store: ChatDraftStore, repositoryId: number, scope: string) {
  const subscribe = useCallback((listener: () => void) => store.subscribe(repositoryId, scope, listener), [store, repositoryId, scope]);
  const snapshot = useCallback(() => store.getSnapshot(repositoryId, scope), [store, repositoryId, scope]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
