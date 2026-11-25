import { useEffect, useMemo, useRef, useState } from "react";
import { Cell, Column, Row, Table, TableBody, TableHeader } from "react-aria-components/Table";
import * as VirtualizerPrimitives from "react-aria-components/Virtualizer";
import { listRepositoryDependencies } from "@/lib/api";
import { severityLabel } from "@/lib/format";
import { SEVERITIES } from "@/lib/types";
import type { DependencyPage, DependencyQuery, RepoPackage, ScannedPackageIdentity, Severity } from "@/lib/types";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { SearchField, Select } from "./ui/form-controls";

const { Virtualizer, TableLayout } = VirtualizerPrimitives as typeof VirtualizerPrimitives & { TableLayout: typeof import("react-aria-components").TableLayout };

type DependencyPosition = { cursor: string | null; previousCursors: (string | null)[] };
type LoadedPage = { key: string; page: DependencyPage; expiresAt: number; bytes: number; scrollTop: number };
type LoadFailure = { key: string; message: string; refreshOverview: boolean };
type DependencySession = {
  affectedOnly: boolean;
  ecosystem: string;
  fixability: string;
  severity: Severity | "all";
  sort: string;
  search: string;
  position: DependencyPosition;
};
const sessions = new Map<string, DependencySession>();
const cachedPages = new Map<string, LoadedPage>();
const pendingPages = new Map<string, Promise<DependencyPage>>();
const FIRST_PAGE: DependencyPosition = { cursor: null, previousCursors: [] };
const PAGE_TTL_MS = 5 * 60 * 1000;
const CACHE_MAX_ENTRIES = 12;
const CACHE_MAX_BYTES = 4 * 1024 * 1024;

function cachedPage(key: string): LoadedPage | undefined {
  const entry = cachedPages.get(key);
  return entry && entry.expiresAt > Date.now() ? entry : undefined;
}

function storePage(key: string, page: DependencyPage, reusable: boolean): LoadedPage {
  const bytes = key.length * 2 + JSON.stringify(page).length * 2;
  const entry = { key, page, expiresAt: Date.now() + PAGE_TTL_MS, bytes, scrollTop: 0 };
  if (reusable && bytes <= CACHE_MAX_BYTES) {
    cachedPages.set(key, entry);
    let total = [...cachedPages.values()].reduce((sum, item) => sum + item.bytes, 0);
    while (cachedPages.size > CACHE_MAX_ENTRIES || total > CACHE_MAX_BYTES) {
      const oldest = cachedPages.entries().next().value;
      if (!oldest) break;
      const [oldestKey, oldestPage] = oldest;
      total -= oldestPage.bytes;
      cachedPages.delete(oldestKey);
    }
  }
  return entry;
}

function readPage(key: string, repoId: number, query: DependencyQuery): Promise<DependencyPage> {
  const pending = pendingPages.get(key);
  if (pending) return pending;
  const request = listRepositoryDependencies(repoId, query).finally(() => pendingPages.delete(key));
  pendingPages.set(key, request);
  return request;
}

interface RepositoryDependenciesProps {
  repoId: number;
  scanRunId: number;
  dependencyRevision: number;
  refreshingInventory?: boolean;
  query: string;
  exactPackage?: ScannedPackageIdentity;
  onClearLinkedPackage?: () => void;
  packageLinkSequence?: number;
  onQueryChange: (query: string) => void;
  onOpenReview: (identity: RepoPackage) => void;
  onRefreshOverview?: () => void;
}

function LinkedDependency({ packageIdentity, onClear }: {
  packageIdentity: ScannedPackageIdentity;
  onClear: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-hairline bg-surface-1 px-3 py-2">
      <p className="min-w-0 text-sm [overflow-wrap:anywhere]">
        <strong>{packageIdentity.name}</strong>{" "}
        <span className="text-ink-subtle">{packageIdentity.version ?? "Unknown version"}</span>
      </p>
      <Button size="sm" variant="secondary" onClick={onClear}>Show all dependencies</Button>
    </div>
  );
}

export function RepositoryDependencies({ repoId, scanRunId, dependencyRevision, refreshingInventory = false, query, exactPackage, onClearLinkedPackage, packageLinkSequence, onQueryChange, onOpenReview, onRefreshOverview }: RepositoryDependenciesProps) {
  const searchField = useRef<HTMLInputElement>(null);
  const wasLinked = useRef(Boolean(exactPackage));
  const sessionKey = `${repoId}:${packageLinkSequence ?? 0}`;
  const [initial] = useState(() => {
    const saved = sessions.get(sessionKey);
    return saved?.search === query ? saved : undefined;
  });
  const [affectedOnly, setAffectedOnly] = useState(initial?.affectedOnly ?? false);
  const [ecosystem, setEcosystem] = useState(initial?.ecosystem ?? "all");
  const [fixability, setFixability] = useState(initial?.fixability ?? "all");
  const [severity, setSeverity] = useState<Severity | "all">(initial?.severity ?? "all");
  const [sort, setSort] = useState(initial?.sort ?? "risk");
  const [search, setSearch] = useState(query);
  const filters = useMemo(() => ({
    search, exactPackage, affectedOnly, ecosystem: ecosystem === "all" ? null : ecosystem,
    severity: severity === "all" ? null : severity,
    fixAvailable: fixability === "all" ? null : fixability === "fixable",
    sort, pageSize: 80, expectedScanRunId: scanRunId,
  }), [search, exactPackage, affectedOnly, ecosystem, severity, fixability, sort, scanRunId]);
  const filterKey = JSON.stringify([sessionKey, dependencyRevision, filters]);
  const [pagination, setPagination] = useState(() => ({
    filterKey,
    position: initial && cachedPage(JSON.stringify([filterKey, initial.position.cursor])) ? initial.position : FIRST_PAGE,
  }));
  const position = pagination.filterKey === filterKey ? pagination.position : FIRST_PAGE;
  const { cursor, previousCursors } = position;
  const requestKey = JSON.stringify([filterKey, cursor]);
  const [result, setResult] = useState<LoadedPage | LoadFailure | undefined>(() => cachedPage(requestKey));
  const [ecosystems, setEcosystems] = useState<string[]>([]);
  const options = result && "page" in result ? result.page.ecosystems : ecosystems;
  const ecosystemOptions = ecosystem === "all" || options.includes(ecosystem) ? options : [...options, ecosystem];
  const pendingSearch = query !== search;
  const current = !pendingSearch && !refreshingInventory && result?.key === requestKey ? result : undefined;
  const loaded = current && "page" in current ? current : undefined;
  const failure = current && "message" in current ? current : undefined;
  const page = loaded?.page;
  const updating = !current;
  const rows = useMemo(() => page?.items.map((pkg, index) => ({ ...pkg, key: `${pkg.packageUrl ?? pkg.name}:${pkg.version}:${index}` })) ?? [], [page]);
  const emptyMessage = exactPackage
    ? "This package version was not found in the latest completed scan."
    : page?.inventoryCount
      ? "No dependencies match these filters."
      : "No dependencies were recorded for this scan.";

  useEffect(() => {
    if (wasLinked.current && !exactPackage) searchField.current?.focus();
    wasLinked.current = Boolean(exactPackage);
  }, [exactPackage]);

  function moveTo(next: DependencyPosition) {
    setPagination({ filterKey, position: next });
  }

  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(query), 150);
    return () => window.clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    // A cursor can only be used with the filters and revision that issued it.
    setPagination((current) => current.filterKey === filterKey ? current : { filterKey, position: FIRST_PAGE });
  }, [filterKey]);
  useEffect(() => {
    if (pendingSearch || refreshingInventory || result?.key === requestKey) return;
    const cached = cachedPage(requestKey);
    if (cached) {
      cachedPages.delete(requestKey);
      cachedPages.set(requestKey, cached);
      setEcosystems(cached.page.ecosystems);
      setResult(cached);
      return;
    }
    let active = true;
    void readPage(requestKey, repoId, { ...filters, cursor })
      .then((value) => {
        if (!active) return;
        if (value.scanRunId !== scanRunId || value.revision < dependencyRevision) {
          throw new Error("DEPENDENCY_CURSOR_STALE");
        }
        setEcosystems(value.ecosystems);
        setResult(storePage(requestKey, value, value.revision === dependencyRevision));
      })
      .catch((cause) => {
        if (!active) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        const stale = message.startsWith("DEPENDENCY_CURSOR_STALE");
        if (stale) cachedPages.delete(JSON.stringify([filterKey, null]));
        if (cursor && stale) {
          setResult(undefined);
          setPagination({ filterKey, position: FIRST_PAGE });
        } else {
          setResult({ key: requestKey, message: stale ? "Scan evidence changed. Refresh this repository." : message, refreshOverview: stale });
        }
      });
    return () => { active = false; };
  }, [repoId, scanRunId, dependencyRevision, filters, filterKey, cursor, pendingSearch, refreshingInventory, requestKey, result]);

  const scroller = useRef<HTMLTableElement>(null);
  useEffect(() => {
    if (!loaded) return;
    const frame = window.requestAnimationFrame(() => {
      if (scroller.current) scroller.current.scrollTop = loaded.scrollTop;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [loaded]);

  const session: DependencySession = {
    affectedOnly, ecosystem, fixability, severity, sort, search: query,
    position: loaded ? position : FIRST_PAGE,
  };
  const latest = useRef({ sessionKey, session });
  latest.current = { sessionKey, session };
  useEffect(() => () => {
    const state = latest.current;
    if (state.sessionKey !== sessionKey) return;
    sessions.delete(sessionKey);
    sessions.set(sessionKey, state.session);
    const oldest = sessions.keys().next().value;
    if (sessions.size > CACHE_MAX_ENTRIES && oldest !== undefined) sessions.delete(oldest);
  }, [sessionKey]);

  return <div className="grid min-w-0 gap-3">
    {exactPackage && onClearLinkedPackage ? <LinkedDependency packageIdentity={exactPackage} onClear={onClearLinkedPackage} /> : null}
    {!exactPackage ? <div className="flex flex-wrap gap-2">
      <SearchField ref={searchField} containerClassName="min-w-0 flex-[2_1_240px]" aria-label="Search dependencies" placeholder="Search package, version, ecosystem…" value={query} onChange={(event)=>onQueryChange(event.target.value)} />
      <Button aria-pressed={affectedOnly} variant="toggle" onClick={()=>setAffectedOnly((value)=>!value)}>Affected only</Button>
      <Select containerClassName="min-w-32 flex-1" aria-label="Filter dependency ecosystem" value={ecosystem} onValueChange={setEcosystem}><option value="all">Any ecosystem</option>{ecosystemOptions.map((kind)=><option key={kind} value={kind}>{kind}</option>)}</Select>
      <Select containerClassName="min-w-32 flex-1" aria-label="Filter fix availability" value={fixability} onValueChange={setFixability}><option value="all">Any fix state</option><option value="fixable">Fix available</option><option value="unlisted">No fix listed</option></Select>
      <Select containerClassName="min-w-32 flex-1" aria-label="Filter dependency severity" value={severity} onValueChange={(value)=>setSeverity(value as Severity|"all")}><option value="all">Any severity</option>{SEVERITIES.map((severity) => <option key={severity} value={severity}>{severityLabel(severity)}</option>)}</Select>
      <Select containerClassName="min-w-32 flex-1" aria-label="Sort dependencies" value={sort} onValueChange={setSort}><option value="risk">Risk</option><option value="name">Name</option><option value="ecosystem">Ecosystem</option></Select>
    </div> : null}
    <p role="status" className={exactPackage && !updating ? "sr-only" : "text-xs text-ink-subtle"}>
      {updating ? "Loading dependencies…" : page && !exactPackage
        ? `${page.inventoryCount} dependencies · ${page.affectedCount} affected · ${page.inventoryCount - page.affectedCount} without current findings`
        : null}
    </p>
    {failure ? <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-severity-critical">
      <p>{failure.message}</p>
      {failure.refreshOverview && onRefreshOverview ? (
        <Button variant="secondary" onClick={() => { setResult(undefined); onRefreshOverview(); }}>Refresh repository</Button>
      ) : <Button variant="secondary" onClick={() => setResult(undefined)}>Retry</Button>}
    </div> : null}
    <div className="h-[min(480px,55vh)] min-h-64 overflow-hidden rounded-lg border border-hairline bg-surface-1" aria-busy={updating}>
      <Virtualizer layout={TableLayout} layoutOptions={{rowHeight:64,headingHeight:38}}>
        <Table ref={scroller} aria-label="Repository dependencies" className="h-full w-full overflow-auto text-left text-xs" onScroll={(event) => { if (loaded) loaded.scrollTop = event.currentTarget.scrollTop; }}>
          <TableHeader><Column id="package" isRowHeader width="1fr" minWidth={170} className="bg-surface-2 px-3 py-2">Package</Column><Column id="version" width={100} className="bg-surface-2 px-2 py-2">Version</Column><Column id="risk" width={100} className="bg-surface-2 px-2 py-2">Risk</Column><Column id="fix" width={100} className="bg-surface-2 px-2 py-2">Fix</Column></TableHeader>
          <TableBody items={rows} renderEmptyState={() => page ? <p className="p-6">{emptyMessage}</p> : null}>
            {(pkg)=><Row id={pkg.key} textValue={`${pkg.name} ${pkg.version??""}`} className="border-b border-hairline bg-surface-1 outline-none data-[focus-visible]:ring-2 data-[focus-visible]:ring-inset data-[focus-visible]:ring-primary">
              <Cell className="min-w-0 px-3 py-2">{pkg.findingCount?<Button aria-label={"View findings for " + pkg.name + (pkg.version ? " " + pkg.version : "")} className="h-auto max-w-full min-w-0 justify-start px-0 text-left" variant="ghost" onClick={()=>onOpenReview(pkg)}><span className="min-w-0"><strong className="block truncate text-sm">{pkg.name}</strong><span className="block truncate text-xs text-ink-subtle">{pkg.packageType??"Unknown ecosystem"}</span></span></Button>:<span className="min-w-0"><strong className="block truncate text-sm">{pkg.name}</strong><span className="block truncate text-xs text-ink-subtle">{pkg.packageType??"Unknown ecosystem"}</span></span>}</Cell>
              <Cell className="truncate px-2">{pkg.version??"Unknown"}</Cell><Cell className="px-2">{pkg.highestSeverity?<Badge variant={pkg.highestSeverity==="CRITICAL"?"destructive":pkg.highestSeverity==="HIGH"?"warning":"secondary"}>{pkg.highestSeverity}</Badge>:<span className="text-ink-subtle">No findings</span>}</Cell><Cell className="px-2">{pkg.fixAvailable?<Badge variant="success">Available</Badge>:<span className="text-ink-subtle">Not listed</span>}</Cell>
            </Row>}
          </TableBody>
        </Table>
      </Virtualizer>
    </div>
    {!exactPackage ? <div className="flex flex-wrap items-center justify-between gap-2"><p className="text-xs text-ink-subtle">{page?`${page.items.length} of ${page.total} matching dependencies · Page ${previousCursors.length+1}`:""}</p><div className="flex gap-2"><Button variant="secondary" disabled={updating||!previousCursors.length} onClick={()=>moveTo({cursor:previousCursors.at(-1)??null,previousCursors:previousCursors.slice(0,-1)})}>Previous</Button><Button variant="secondary" disabled={updating||!page?.nextCursor} onClick={()=>moveTo({cursor:page?.nextCursor??null,previousCursors:[...previousCursors,cursor]})}>Next</Button></div></div> : null}
  </div>;
}
