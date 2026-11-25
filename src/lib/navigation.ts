export type WorkspaceId = "overview" | "review" | "chat" | "intel" | "repositories" | "settings";
export type LegacyWorkspaceId = WorkspaceId | "inbox";

export interface IntelFeedView {
  filters: {
    search: string;
    status: "all" | ExternalIntelRelevance;
    severity: "all" | "high_critical" | "critical" | "high" | "medium_low";
    source: "all" | ExternalSource;
    ecosystem: string;
    match: "all" | ExternalIntelMatchKind;
    lifecycle: "active" | "all" | "withdrawn";
    signal: "all" | "kev" | "ransomware";
  };
  advancedFilters: boolean;
  sort: { column: "priority" | "severity" | "source"; direction: "asc" | "desc" };
}

export interface IntelSelection {
  canonicalKey: string | null;
  detailOpen: boolean;
  direct: boolean;
}

export interface WorkspaceContext {
  viewId?: string;
  repoId?: number;
  chatFinding?: FindingChatRequest;
  entityId?: string;
  selectedKey?: string;
  scrollTop?: number;
  reviewQuery?: ReviewQuery;
  reviewLoadedRows?: number;
  reviewPresentation?: ReviewPresentation;
  inspectorPinned?: boolean;
  inspectorKey?: string;
  focusKey?: string;
  boardScrollLeft?: number;
  repositorySection?: "overview" | "dependencies" | "history" | "artifacts" | "chat" | "settings";
  repositoryScrollAnchor?: string;
  repositoryQuery?:string;
  repositorySort?:"attention"|"freshness"|"changes"|"name";
  repositoryPackageQuery?:string;
  repositoryExactPackage?: ScannedPackageIdentity;
  repositoryPackageLinkSequence?: number;
  repositoryDetailOpen?: boolean;
  repositoryFocusOnReturn?: boolean;
  repositoryFailedOnly?: boolean;
  repositoryScanRunId?: number;
  ephemeralReviewContext?: boolean;
  remediationGroup?: ReviewOpenRequest["remediationGroup"];
  intelFeedView?: IntelFeedView;
  intelSelection?: IntelSelection;
}

export interface NavigationEntry {
  workspace: WorkspaceId;
  context: WorkspaceContext;
}

export function normalizeWorkspace(value: LegacyWorkspaceId): WorkspaceId {
  return value === "inbox" ? "review" : value;
}

export class WorkspaceHistory {
  private entries: NavigationEntry[] = [];
  private contexts = new Map<WorkspaceId, WorkspaceContext>();

  push(entry: NavigationEntry, { preserveCurrent = false }: { preserveCurrent?: boolean } = {}): void {
    const normalized = { workspace: normalizeWorkspace(entry.workspace), context: cloneContext(entry.context) };
    const current = this.entries.at(-1);
    if (!preserveCurrent && current?.workspace === normalized.workspace) {
      this.entries[this.entries.length - 1] = normalized;
    } else {
      this.entries.push(normalized);
    }
    this.contexts.set(normalized.workspace, cloneContext(normalized.context));
  }

  replaceContext(workspace: WorkspaceId, context: WorkspaceContext): void {
    const next = cloneContext(context);
    this.contexts.set(workspace, next);
    const current = this.entries.at(-1);
    if (current?.workspace === workspace) this.entries[this.entries.length - 1] = { workspace, context: next };
  }

  back(): NavigationEntry | null {
    if (!this.canGoBack) return null;
    this.entries.pop();
    const entry = this.entries[this.entries.length - 1];
    const context = cloneContext(entry.context);
    this.contexts.set(entry.workspace, context);
    return { workspace: entry.workspace, context: cloneContext(context) };
  }

  get canGoBack(): boolean {
    return this.entries.length > 1;
  }

  context(workspace: WorkspaceId): WorkspaceContext {
    return cloneContext(this.contexts.get(workspace) ?? {});
  }
}

function cloneContext(context: WorkspaceContext): WorkspaceContext {
  return typeof structuredClone === "function" ? structuredClone(context) : JSON.parse(JSON.stringify(context)) as WorkspaceContext;
}
import type { ExternalIntelMatchKind, ExternalIntelRelevance, ExternalSource, FindingChatRequest, ReviewOpenRequest, ReviewQuery, ScannedPackageIdentity } from "./types";

import type { ReviewPresentation } from "./reviewViews";
