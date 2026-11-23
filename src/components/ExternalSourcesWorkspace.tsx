import {
  ArrowRight,
  ArrowSquareOut,
  ArrowsClockwise,
  CaretLeft,
  GlobeHemisphereWest,
  SlidersHorizontal,
  Sparkle,
} from "@phosphor-icons/react";
import { lazy, Suspense, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchField, Select } from "@/components/ui/form-controls";
import { WorkList, type WorkListColumn } from "@/components/ui/work-list";
import { getExternalIntelDetail, openExternalUrl } from "@/lib/api";
import { EvidenceReadsContext } from "@/lib/revisionReadCache";
import type { IntelFeedView } from "@/lib/navigation";
import {
  externalSeverityBadgeVariant,
  formatExternalSeverityLabel,
  formatExternalSourceLabel,
  formatRelativeTimestamp,
} from "@/lib/format";
import { cn } from "@/lib/utils";
import type {
  ExternalIntelDetail,
  ExternalIntelFeed,
  ExternalIntelFeedItem,
  ExternalIntelSummary,
  ExternalIntelTopic,
  ExternalSource,
  ScannedPackageIdentity,
} from "@/lib/types";

const ReactMarkdown = lazy(() => import("react-markdown"));

function openExternalHref(href: string | undefined) {
  if (!href) {
    return;
  }

  void openExternalUrl(href).catch((cause) => {
    console.error("Failed to open external URL", cause);
  });
}

type IntelFilters = IntelFeedView["filters"];
const DEFAULT_FEED_VIEW: IntelFeedView = {
  filters: { search: "", status: "all", severity: "all", source: "all", ecosystem: "all", match: "all", lifecycle: "active", signal: "all" },
  advancedFilters: false,
  sort: { column: "priority", direction: "desc" },
};
const FEED_SORT_LABELS = { priority: "priority", severity: "severity", source: "source" } as const;
const WIDE_INTELLIGENCE_VIEW = "(min-width: 80rem)"; // Tailwind's xl breakpoint
const EMPTY_FEED: ExternalIntelFeedItem[] = [];
type ExternalIntelItem = ExternalIntelFeedItem & ExternalIntelDetail;

interface ExternalSourcesWorkspaceProps {
  feed: ExternalIntelFeed | null;
  feedError: string | null;
  initialFeedView?: IntelFeedView;
  topics: ExternalIntelTopic[];
  summary: ExternalIntelSummary;
  isCompactDetailOpen: boolean;
  isDirectSelection: boolean;
  isLoading: boolean;
  isRefreshing: boolean;
  isSavingTopics: boolean;
  refreshError: string | null;
  selectedKey: string | null;
  onCloseDetail: () => void;
  onFeedViewChange: (view: IntelFeedView) => void;
  onOpenRepository: (repoId: number, pkg: ScannedPackageIdentity) => void;
  onRefresh: () => void;
  onReloadFeed: () => void;
  onSelect: (canonicalKey: string, mode: "preview" | "open") => void;
  onTopicsChange: (topicKeys: string[]) => void;
}

const EXTERNAL_SOURCE_FILTERS: ExternalSource[] = [
  "osv",
  "github",
  "nvd",
  "cisa_kev",
  "vendor",
];

const TOPIC_PRESETS: Array<{
  label: string;
  topicKeys: string[];
}> = [
  {
    label: "Web apps",
    topicKeys: ["npm", "pip", "go", "maven", "nuget", "composer"],
  },
  {
    label: "Cloud/container",
    topicKeys: ["alpine", "ubuntu", "debian", "wolfi", "chainguard", "bitnami"],
  },
  {
    label: "Linux distros",
    topicKeys: ["ubuntu", "debian", "red hat", "suse", "alpine"],
  },
  {
    label: "Dev tooling",
    topicKeys: ["actions", "github_actions_runner", "vscode", "oss-fuzz", "git"],
  },
  {
    label: "Infra products",
    topicKeys: [
      "github_enterprise_server",
      "github_actions_runner",
      "gitlab",
      "jenkins",
      "kubernetes",
      "docker",
      "nginx",
      "postgres",
      "openssl",
    ],
  },
];

export function ExternalSourcesWorkspace({
  feed,
  feedError,
  initialFeedView,
  topics,
  summary,
  isCompactDetailOpen,
  isDirectSelection,
  isLoading,
  isRefreshing,
  isSavingTopics,
  refreshError,
  selectedKey,
  onCloseDetail,
  onFeedViewChange,
  onOpenRepository,
  onRefresh,
  onReloadFeed,
  onSelect,
  onTopicsChange,
}: ExternalSourcesWorkspaceProps) {
  const items = feed?.items ?? EMPTY_FEED;
  const feedFailed = feedError !== null;
  const [feedView, setFeedView] = useState(initialFeedView ?? DEFAULT_FEED_VIEW);
  const {
    filters: {
      search: intelSearch, status: statusFilter, severity: severityFilter, source: sourceFilter,
      ecosystem: ecosystemFilter, match: matchFilter, lifecycle: lifecycleFilter, signal: signalFilter,
    },
    advancedFilters: showAdvancedFilters,
    sort: feedSort,
  } = feedView;
  const [isTopicEditorOpen, setIsTopicEditorOpen] = useState(false);
  const [activeTopicCategory, setActiveTopicCategory] = useState("Packages");
  const [topicSearch, setTopicSearch] = useState("");
  const [isWide, setIsWide] = useState(() => window.matchMedia?.(WIDE_INTELLIGENCE_VIEW)?.matches ?? true);
  const detailTitle = useRef<HTMLHeadingElement>(null);
  const feedPanel = useRef<HTMLDivElement>(null);
  const focusOnBack = useRef(false);
  const watchedTopicKeys = topics
    .filter((topic) => topic.watched)
    .map((topic) => topic.key);

  const normalizedIntelSearch = intelSearch.trim().toLowerCase();
  const ecosystemOptions = useMemo(() => {
    const ecosystems = new Set(items.flatMap((item) => item.ecosystem ? [item.ecosystem] : []));
    if (ecosystemFilter !== "all") ecosystems.add(ecosystemFilter);
    return [...ecosystems].sort((left, right) => left.localeCompare(right));
  }, [items, ecosystemFilter]);
  const stateItems = useMemo(() => items.filter((item) =>
    lifecycleFilter === "all" || Boolean(item.withdrawnAt) === (lifecycleFilter === "withdrawn")
  ), [items, lifecycleFilter]);
  const filteredItems = useMemo(() => stateItems.filter((item) => {
    if (signalFilter === "kev" && !item.isKev) return false;
    if (signalFilter === "ransomware" && !item.isRansomwareKnown) return false;
    if (statusFilter !== "all" && item.relevance !== statusFilter) return false;

    if (!itemMatchesSeverityFilter(item, severityFilter)) {
      return false;
    }

    if (sourceFilter !== "all" && !item.sources.includes(sourceFilter)) {
      return false;
    }

    if (ecosystemFilter !== "all" && item.ecosystem !== ecosystemFilter) {
      return false;
    }

    if (matchFilter !== "all" && item.matchKind !== matchFilter) {
      return false;
    }

    if (normalizedIntelSearch && !itemMatchesSearch(item, normalizedIntelSearch)) {
      return false;
    }

    return true;
  }), [stateItems, signalFilter, statusFilter, severityFilter, sourceFilter, ecosystemFilter, matchFilter, normalizedIntelSearch]);

  const feedColumns: WorkListColumn<ExternalIntelFeedItem>[] = [
    { id: "priority", label: "Advisory", sortable: true, render: (item) => <div className="min-w-0"><p className="line-clamp-2 font-semibold leading-5 text-ink">{item.title}</p><p className="mt-1 truncate text-ink-subtle">{formatPackageLine(item)} · {formatLocalSignal(item)}</p></div> },
    { id: "severity", label: "Risk", sortable: true, className: "w-[8.5rem]", render: (item) => (
      <div className="flex flex-wrap gap-1">
        <Badge variant={externalSeverityBadgeVariant(item.severity)}>{formatExternalSeverityLabel(item.severity)}</Badge>
        {item.withdrawnAt ? <Badge variant="outline">Withdrawn</Badge> : null}
        {item.isKev ? <Badge variant="destructive">KEV</Badge> : null}
        {item.isRansomwareKnown ? <Badge variant="destructive">Ransomware</Badge> : null}
      </div>
    ) },
    { id: "source", label: "Source", sortable: true, className: "w-[8rem]", render: (item) => <div><Badge variant="outline">{formatSourceSummary(item.sources)}</Badge><p className="mt-1 text-ink-subtle">{formatRelativeTimestamp(item.publishedAt)}</p></div> },
  ];
  const orderedFeedItems = useMemo(() => [...filteredItems].sort(
    (left, right) => compareFeedItems(left, right, feedSort.column) * (feedSort.direction === "asc" ? 1 : -1),
  ), [filteredItems, feedSort]);
  const selectedRow = isDirectSelection
    ? items.find((item) => item.canonicalKey === selectedKey)
    : orderedFeedItems.find((item) => item.canonicalKey === selectedKey) ?? orderedFeedItems[0];
  const showCompactDetail = isCompactDetailOpen &&
    (isDirectSelection || selectedRow?.canonicalKey === selectedKey);
  const directSelectionMissing = isDirectSelection && !selectedRow && !isLoading && !feedFailed;

  useEffect(() => {
    onFeedViewChange(feedView);
  }, [feedView, onFeedViewChange]);

  useEffect(() => {
    const query = window.matchMedia?.(WIDE_INTELLIGENCE_VIEW);
    if (!query) return;
    const handleChange = (event: MediaQueryListEvent) => setIsWide(event.matches);
    query.addEventListener("change", handleChange);
    return () => query.removeEventListener("change", handleChange);
  }, []);

  useLayoutEffect(() => {
    if (focusOnBack.current) {
      focusOnBack.current = false;
      (feedPanel.current?.querySelector<HTMLElement>("tr[data-selected]") ?? feedPanel.current)?.focus();
    } else if (showCompactDetail && (isDirectSelection || !isWide)) {
      detailTitle.current?.focus();
    }
  }, [showCompactDetail, isDirectSelection, isWide, selectedKey, selectedRow?.canonicalKey, directSelectionMissing]);
  const topicGroups = groupTopics(topics);
  const topicCategories = topicGroups.map((group) => group.category);
  const watchedTopics = topics.filter((topic) => topic.watched);
  const watchedTopicPreview = watchedTopics.slice(0, 6);
  const suggestedTopicKeys = topics
    .filter((topic) => topic.suggested)
    .map((topic) => topic.key);
  const normalizedTopicSearch = topicSearch.trim().toLowerCase();
  const visibleTopicGroups = normalizedTopicSearch
    ? topicGroups
        .map((group) => ({
          ...group,
          topics: group.topics.filter((topic) =>
            topic.label.toLowerCase().includes(normalizedTopicSearch),
          ),
        }))
        .filter((group) => group.topics.length > 0)
    : topicGroups.filter((group) => group.category === activeTopicCategory);
  const availableTopicKeys = new Set(topics.map((topic) => topic.key));
  const watchedTopicSummary = formatWatchedTopicSummary(watchedTopics);
  const firstFetch = !summary.lastFetchedAt;

  function updateTopics(nextTopicKeys: string[]) {
    onTopicsChange(
      uniqueTopicKeys(nextTopicKeys).filter((key) => availableTopicKeys.has(key)),
    );
  }

  function toggleTopic(topic: ExternalIntelTopic) {
    updateTopics(
      topic.watched
        ? watchedTopicKeys.filter((key) => key !== topic.key)
        : [...watchedTopicKeys, topic.key],
    );
  }

  function applyPreset(nextTopicKeys: string[]) {
    updateTopics(nextTopicKeys);
  }

  function useSuggestedTopics() {
    updateTopics(suggestedTopicKeys);
  }

  function clearTopics() {
    updateTopics([]);
  }

  const presetButtons = TOPIC_PRESETS.map((preset) => ({
    ...preset,
    topicKeys: preset.topicKeys.filter((key) => availableTopicKeys.has(key)),
  })).filter((preset) => preset.topicKeys.length > 0);

  const showTopicEditor =
    isTopicEditorOpen || watchedTopicKeys.length === 0;

  const topicEditorTitle = watchedTopicKeys.length === 0
    ? "Choose topics"
    : "Edit topics";

  const topicEditorActionLabel = isTopicEditorOpen
    ? "Done"
    : "Edit topics";

  const topicEditorAction = () =>
    setIsTopicEditorOpen((current) => !current);


  const hasVisibleTopicGroups = visibleTopicGroups.length > 0;

  const topicEditorSearchPlaceholder =
    normalizedTopicSearch.length > 0
      ? "Search topics"
      : "Search topics";

  const suggestedTopics = topics.filter((topic) => topic.suggested);
  const visibleSuggestedTopics = suggestedTopics.slice(0, 8);

  const emptyTopicEditorMessage =
    normalizedTopicSearch.length > 0
      ? "No topics match that search."
      : "No topics in this category.";

  const hasActiveFilters =
    normalizedIntelSearch.length > 0 ||
    statusFilter !== "all" ||
    severityFilter !== "all" ||
    sourceFilter !== "all" ||
    ecosystemFilter !== "all" ||
    matchFilter !== "all" ||
    signalFilter !== "all" ||
    lifecycleFilter !== "active";
  const fetchLabel = firstFetch ? "Fetch advisories" : "Refresh";

  function updateFilters(update: Partial<IntelFilters>) {
    setFeedView((current) => ({ ...current, filters: { ...current.filters, ...update } }));
  }

  function updateSort(column: IntelFeedView["sort"]["column"]) {
    setFeedView((current) => {
      const direction = current.sort.column === column
        ? current.sort.direction === "asc" ? "desc" : "asc"
        : column === "source" ? "asc" : "desc";
      return { ...current, sort: { column, direction } };
    });
  }

  function returnToFeed() {
    focusOnBack.current = true;
    onCloseDetail();
  }

  return (
    <div className="space-y-3.5">
      <Card className={cn("app-panel rounded-lg border shadow-none", showCompactDetail && "hidden xl:flex")}>
        <CardContent className="grid gap-4 py-4 xl:grid-cols-[minmax(220px,0.8fr)_minmax(360px,1.15fr)_minmax(260px,0.9fr)] xl:items-center">
          <div className="min-w-0">
            <CardTitle className="flex items-center gap-2 text-base font-semibold tracking-tight text-ink">
              <AppIcon className="size-4 text-ink-subtle" icon={GlobeHemisphereWest} />
              <span>Public intel workspace</span>
            </CardTitle>
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-ink-subtle">
              <span>Last refresh {formatRelativeTimestamp(summary.lastFetchedAt)}</span>

            </div>
          </div>

          <div className="app-surface grid grid-cols-2 overflow-hidden rounded-md border sm:grid-cols-4" role="group" aria-label="Advisory counts by local relevance for the selected advisory state">
            <MetricTile label={lifecycleFilter === "active" ? "Active" : lifecycleFilter === "withdrawn" ? "Withdrawn" : "All advisories"} value={stateItems.length} />
            <MetricTile label="Relevant" tone="success" value={stateItems.filter((item) => item.relevance === "relevant").length} />
            <MetricTile label="Triage" tone="warning" value={stateItems.filter((item) => item.relevance === "triage").length} />
            <MetricTile label="External" value={stateItems.filter((item) => item.relevance === "general").length} />
          </div>

          <div className="flex flex-col gap-3 xl:items-end">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
                Watching
              </span>
              {watchedTopicPreview.length > 0 ? (
                <div className="flex flex-wrap gap-1.5" title={watchedTopicSummary}>
                  {watchedTopicPreview.slice(0, 3).map((topic) => (
                    <Badge key={`watching-${topic.key}`} variant="success">
                      {topic.label}
                    </Badge>
                  ))}
                  {watchedTopics.length > 3 ? (
                    <Badge variant="outline">+{watchedTopics.length - 3}</Badge>
                  ) : null}
                </div>
              ) : (
                <span className="text-xs text-ink-subtle">No topics selected</span>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                disabled={isRefreshing || isSavingTopics || watchedTopicKeys.length === 0}
                onClick={onRefresh}
                size="sm"
                type="button"
                variant={watchedTopicKeys.length > 0 ? "default" : "ghost"}
              >
                <AppIcon
                  className={cn("size-4", isRefreshing && "animate-spin")}
                  icon={ArrowsClockwise}
                />
                {isRefreshing ? "Fetching advisories…" : fetchLabel}
              </Button>
              <Button
                disabled={isSavingTopics}
                onClick={topicEditorAction}
                size="sm"
                type="button"
                variant="secondary"
              >
                <AppIcon className="size-4" icon={SlidersHorizontal} />
                {topicEditorActionLabel}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {refreshError !== null ? (
        <Alert variant="warning">
          <AlertTitle>Could not refresh advisories</AlertTitle>
          <AlertDescription>
            <p className="mb-0!">
              {items.length > 0 ? "Showing the last saved advisories. " : ""}
              Try again with {fetchLabel}.
            </p>
            <details className="mt-1">
              <summary className="cursor-pointer font-medium">Technical details</summary>
              <p className="mt-1 max-h-28 overflow-auto [overflow-wrap:anywhere]">{refreshError}</p>
            </details>
          </AlertDescription>
        </Alert>
      ) : null}

      {feedFailed ? (
        <Alert variant="warning">
          <AlertTitle>Could not load saved advisories</AlertTitle>
          <AlertDescription>
            {items.length > 0 ? <p>Showing the last loaded feed.</p> : null}
            <details className="my-1">
              <summary className="cursor-pointer font-medium">Technical details</summary>
              <p className="mt-1 max-h-28 overflow-auto [overflow-wrap:anywhere]">{feedError}</p>
            </details>
            <Button size="sm" variant="secondary" onClick={onReloadFeed}>Reload saved feed</Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {items.length > 0 && watchedTopicKeys.length > 0 ? (
      <Card className={cn("app-panel rounded-lg border shadow-none", showCompactDetail && "hidden xl:flex")}>
        <CardContent className="grid gap-3">
          <div className="flex flex-wrap items-end gap-2">
            <SearchField
              aria-label="Search external intelligence"
              className="h-10"
              containerClassName="min-w-0 flex-[2_1_16rem]"
              onChange={(event) => updateFilters({ search: event.target.value })}
              placeholder="Search package, CVE, GHSA, source..."
              value={intelSearch}
            />
            <FilterSelect
              label="Local relevance"
              onChange={(value) => updateFilters({ status: value as IntelFilters["status"] })}
              options={[
                { label: "All", value: "all" },
                { label: "Relevant", value: "relevant" },
                { label: "Triage", value: "triage" },
                { label: "External", value: "general" },
              ]}
              value={statusFilter}
            />
            <FilterSelect
              label="Severity"
              onChange={(value) => updateFilters({ severity: value as IntelFilters["severity"] })}
              options={[
                { label: "All", value: "all" },
                { label: "High / Critical", value: "high_critical" },
                { label: "Critical", value: "critical" },
                { label: "High", value: "high" },
                { label: "Medium / Low", value: "medium_low" },
              ]}
              value={severityFilter}
            />
            <FilterSelect
              label="Source"
              onChange={(value) => updateFilters({ source: value as IntelFilters["source"] })}
              options={[
                { label: "All", value: "all" },
                ...EXTERNAL_SOURCE_FILTERS.map((source) => ({
                  label: formatExternalSourceLabel(source),
                  value: source,
                })),
              ]}
              value={sourceFilter}
            />
            <FilterSelect
              label="Advisory state"
              onChange={(value) => updateFilters({ lifecycle: value as IntelFilters["lifecycle"] })}
              options={[
                { label: "Active", value: "active" },
                { label: "All", value: "all" },
                { label: "Withdrawn", value: "withdrawn" },
              ]}
              value={lifecycleFilter}
            />
          </div>

          {showAdvancedFilters ? (
            <div className="grid gap-2 border-t border-hairline pt-3 sm:grid-cols-3">
              <FilterSelect
                label="Exploitation signal"
                onChange={(value) => updateFilters({ signal: value as IntelFilters["signal"] })}
                options={[
                  { label: "All", value: "all" },
                  { label: "Known exploited (KEV)", value: "kev" },
                  { label: "Known ransomware use", value: "ransomware" },
                ]}
                value={signalFilter}
              />
              <FilterSelect
                label="Match"
                onChange={(value) => updateFilters({ match: value as IntelFilters["match"] })}
                options={[
                  { label: "All", value: "all" },
                  { label: "Package", value: "package_name" },
                  { label: "Product", value: "product" },
                  { label: "Vendor", value: "vendor" },
                  { label: "Ecosystem", value: "ecosystem" },
                ]}
                value={matchFilter}
              />
              <FilterSelect
                label="Ecosystem"
                onChange={(value) => updateFilters({ ecosystem: value })}
                options={[
                  { label: "All", value: "all" },
                  ...ecosystemOptions.map((ecosystem) => ({
                    label: ecosystem,
                    value: ecosystem,
                  })),
                ]}
                value={ecosystemFilter}
              />
            </div>
          ) : null}

          <div className="flex items-center justify-between gap-2">
            <Button
              aria-expanded={showAdvancedFilters}
              onClick={() => setFeedView((current) => ({ ...current, advancedFilters: !current.advancedFilters }))}
              size="sm"
              type="button"
              variant="toggle"
            >
              {showAdvancedFilters ? "Fewer filters" : signalFilter !== "all" || ecosystemFilter !== "all" || matchFilter !== "all" ? "More filters · active" : "More filters"}
            </Button>
            <Button
              disabled={!hasActiveFilters}
              onClick={() => updateFilters(DEFAULT_FEED_VIEW.filters)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Clear filters
            </Button>
          </div>
        </CardContent>
      </Card>
      ) : null}

      {showTopicEditor ? (
        <Card className={cn("app-panel rounded-lg border shadow-none", showCompactDetail && "hidden xl:flex")}>
          <CardHeader className="flex flex-col gap-3 pb-3 md:flex-row md:items-start md:justify-between">
            <div>
              <CardTitle className="text-base text-ink">
                {topicEditorTitle}
              </CardTitle>
              <CardDescription className="mt-1">
                Start with local suggestions or a preset, then fine-tune the list.
              </CardDescription>
            </div>
            <Button
              className="app-surface rounded-md border px-3 text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
              disabled={isSavingTopics || watchedTopicKeys.length === 0}
              onClick={clearTopics}
              size="sm"
              type="button"
              variant="secondary"
            >
              Clear
            </Button>
          </CardHeader>
          <CardContent className="space-y-4 pt-0">
            {visibleSuggestedTopics.length > 0 ? (
              <section className="space-y-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="px-1 text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
                    Suggested from repos
                  </p>
                  <Button
                    disabled={isSavingTopics}
                    onClick={useSuggestedTopics}
                    size="sm"
                    type="button"
                    variant={watchedTopicKeys.length === 0 ? "default" : "secondary"}
                  >
                    Use local topics
                  </Button>
                </div>
                <div className="flex flex-wrap gap-2">
                  {visibleSuggestedTopics.map((topic) => (
                    <TopicChip
                      isSaving={isSavingTopics}
                      key={`suggested-${topic.key}`}
                      onToggle={() => toggleTopic(topic)}
                      topic={topic}
                    />
                  ))}
                </div>
              </section>
            ) : null}

            <section className="space-y-2">
              <p className="px-1 text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
                Presets
              </p>
              <div className="flex flex-wrap gap-2">
                {presetButtons.map((preset) => (
                  <Button
                    disabled={isSavingTopics}
                    key={preset.label}
                    onClick={() => applyPreset(preset.topicKeys)}
                    type="button"
                    size="xs"
                    variant="secondary"
                  >
                    {preset.label}
                  </Button>
                ))}
              </div>
            </section>

            <section className="space-y-3">
              <div className="flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between">
                <div className="flex flex-wrap gap-2">
                  {topicCategories.map((category) => (
                    <Button
                      className={cn(
                        "px-3",
                        !normalizedTopicSearch &&
                          activeTopicCategory === category &&
                          "bg-[color:var(--surface-4)] text-ink",
                      )}
                      key={category}
                      onClick={() => {
                        setActiveTopicCategory(category);
                        setTopicSearch("");
                      }}
                      type="button"
                      size="xs"
                      variant="secondary"
                    >
                      {category}
                    </Button>
                  ))}
                </div>
                <SearchField
                    aria-label="Search topics"
                    className="h-9"
                    containerClassName="min-w-[220px]"
                    onChange={(event) => setTopicSearch(event.target.value)}
                    placeholder={topicEditorSearchPlaceholder}
                    value={topicSearch}
                  />
              </div>

              {watchedTopics.length > 0 ? (
                <div className="flex flex-wrap gap-2">
                  {watchedTopics.slice(0, 10).map((topic) => (
                    <Badge key={`selected-${topic.key}`} variant="success">
                      {topic.label}
                    </Badge>
                  ))}
                  {watchedTopics.length > 10 ? (
                    <Badge variant="outline">+{watchedTopics.length - 10}</Badge>
                  ) : null}
                </div>
              ) : null}

              {hasVisibleTopicGroups ? (
                <div className="space-y-3">
                  {visibleTopicGroups.map((group) => (
                    <div key={group.category} className="space-y-2">
                      {normalizedTopicSearch ? (
                        <p className="px-1 text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
                          {group.category}
                        </p>
                      ) : null}
                      <div className="flex flex-wrap gap-2">
                        {group.topics.map((topic) => (
                          <TopicChip
                            isSaving={isSavingTopics}
                            key={topic.key}
                            onToggle={() => toggleTopic(topic)}
                            topic={topic}
                          />
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="app-surface rounded-md border border-dashed px-4 py-5 text-xs text-ink-subtle">
                  {emptyTopicEditorMessage}
                </div>
              )}
            </section>
          </CardContent>
        </Card>
      ) : null}

      {watchedTopicKeys.length === 0 && !isDirectSelection ? (
        <Card className="app-panel rounded-lg border shadow-none">
          <CardContent className="pt-6">
            <div className="app-surface rounded-md border border-dashed px-4 py-8 text-center">
              <div className="mx-auto flex size-12 items-center justify-center rounded-full bg-[color:var(--surface-3)] text-[color:var(--ink-subtle)]">
                <AppIcon className="size-5" icon={Sparkle} />
              </div>
              <p className="mt-4 text-sm font-medium text-ink">
                Choose watched topics
              </p>
              <p className="mt-2 text-xs leading-5 text-ink-subtle">
                This intelligence feed stays empty until at least one topic is selected.
              </p>
            </div>
          </CardContent>
        </Card>
      ) : items.length === 0 && !isDirectSelection ? (
        <Card className="app-panel rounded-lg border shadow-none">
          <CardContent className="pt-6">
            <div className="app-surface rounded-md border border-dashed px-4 py-8 text-center">
              <div className="mx-auto flex size-12 items-center justify-center rounded-full bg-[color:var(--surface-3)] text-[color:var(--ink-subtle)]">
                <AppIcon className="size-5" icon={Sparkle} />
              </div>
              <p role="status" className="mt-4 text-sm font-medium text-ink">
                {isLoading ? "Loading advisories…" : feedFailed ? "Saved advisories unavailable" : firstFetch ? "No advisories fetched yet" : "No advisories for these topics"}
              </p>
              {!isLoading && !firstFetch && !feedFailed ? (
                <p className="mt-2 text-xs leading-5 text-ink-subtle">Edit watched topics and refresh to check again.</p>
              ) : null}
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-3.5 xl:grid-cols-[minmax(320px,0.95fr)_minmax(0,1.05fr)]">
          <Card ref={feedPanel} tabIndex={-1} className={cn("app-panel min-w-0 rounded-lg border shadow-none", showCompactDetail && "hidden xl:flex")}>
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <CardTitle className="text-base text-ink">
                    Feed
                  </CardTitle>
                  <CardDescription className="mt-1" role="status">
                    Sorted by {FEED_SORT_LABELS[feedSort.column]} · {filteredItems.length}
                    {filteredItems.length !== stateItems.length ? " of " + stateItems.length : ""}
                    {stateItems.length === 1 ? " advisory shown" : " advisories shown"}
                  </CardDescription>
                </div>
              </div>
            </CardHeader>
            <CardContent className="pt-0">
              {filteredItems.length === 0 ? (
                <div className="app-surface rounded-md border border-dashed px-4 py-5 text-xs text-ink-subtle">
                  No items match the current filters.
                </div>
              ) : (
                <WorkList
                  className="max-h-[36rem] xl:h-[36rem]"
                  items={orderedFeedItems}
                  columns={feedColumns}
                  getKey={(item) => item.canonicalKey}
                  label="External intelligence feed"
                  selectedKey={selectedRow && filteredItems.includes(selectedRow) ? selectedRow.canonicalKey : null}
                  onSelect={onSelect}
                  sort={feedSort}
                  onSort={(column) => updateSort(column as IntelFeedView["sort"]["column"])}
                />
              )}
            </CardContent>
          </Card>

          <Card className={cn("app-panel min-w-0 rounded-lg border shadow-none", !showCompactDetail && "hidden xl:flex")} onKeyDown={(event) => {
            if (event.key === "Escape" && (!isWide || isDirectSelection)) {
              event.preventDefault();
              returnToFeed();
            }
          }}>
            {!isWide || isDirectSelection ? (
              <CardHeader className="pb-3">
                <Button onClick={returnToFeed} size="sm" variant="ghost" className="w-fit">
                  <AppIcon className="size-4" icon={CaretLeft} /> Back to feed
                </Button>
              </CardHeader>
            ) : null}
            <CardContent className={!isWide || isDirectSelection ? "pt-0" : undefined}>
              {selectedRow ? (
                <ScrollArea key={selectedRow.canonicalKey} className="@container/intel-detail xl:h-[36rem] xl:pr-1">
                  <div className="space-y-4">
                    <div>
                      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge
                            variant={externalSeverityBadgeVariant(selectedRow.severity)}
                          >
                            {formatExternalSeverityLabel(selectedRow.severity)}
                          </Badge>
                          {selectedRow.isKev ? <Badge variant="destructive">KEV</Badge> : null}
                          {selectedRow.isRansomwareKnown ? <Badge variant="destructive">Ransomware</Badge> : null}
                          {selectedRow.withdrawnAt ? <Badge variant="outline">Withdrawn</Badge> : null}
                          <RelevanceBadge relevance={selectedRow.relevance} />
                        </div>

                        <SourceBadgeGroup
                          itemKey={`detail-${selectedRow.id}`}
                          limit={4}
                          sources={selectedRow.sources}
                        />
                      </div>

                      <h2 ref={detailTitle} tabIndex={-1} className="mt-3 text-xl font-semibold tracking-tight text-ink focus-visible:rounded-sm focus-visible:outline-primary">
                        {selectedRow.title}
                      </h2>

                      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-ink-subtle">
                        <span>{formatPackageLine(selectedRow)}</span>
                        <span>•</span>
                        <span>{formatAbsoluteTimestamp(selectedRow.publishedAt)}</span>
                      </div>
                    </div>

                    {feed && (isWide || showCompactDetail) ? (
                      <IntelDetailBody
                        key={JSON.stringify([selectedRow.canonicalKey, feed.intelRevision, feed.inventoryRevision])}
                        row={selectedRow}
                        intelRevision={feed.intelRevision}
                        inventoryRevision={feed.inventoryRevision}
                        onReloadFeed={onReloadFeed}
                        onOpenRepository={onOpenRepository}
                      />
                    ) : null}
                  </div>
                </ScrollArea>
              ) : isDirectSelection ? (
                <div className="app-surface rounded-md border border-dashed px-4 py-5">
                  <h2 ref={detailTitle} tabIndex={-1} className="text-sm font-medium text-ink">
                    {isLoading ? "Loading advisory…" : directSelectionMissing ? "Advisory no longer in the saved feed" : "Advisory unavailable"}
                  </h2>
                  {selectedKey ? <p className="mt-1 text-xs text-ink-subtle">{selectedKey}</p> : null}
                </div>
              ) : (
                <div className="app-surface rounded-md border border-dashed px-4 py-5 text-xs text-ink-subtle">
                  Select an item to inspect its details.
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  );
}

function IntelDetailBody({ row, intelRevision, inventoryRevision, onOpenRepository, onReloadFeed }: {
  row: ExternalIntelFeedItem;
  intelRevision: number;
  inventoryRevision: number;
} & Pick<ExternalSourcesWorkspaceProps, "onOpenRepository" | "onReloadFeed">) {
  const cache = useContext(EvidenceReadsContext)?.intel;
  const [detail, setDetail] = useState<ExternalIntelDetail | null>(null);
  const [status, setStatus] = useState<"loading" | "ready" | "stale" | "failed">("loading");
  const [attempt, setAttempt] = useState(0);
  const canonicalKey = row.canonicalKey;

  useEffect(() => {
    let active = true;
    const key = JSON.stringify([canonicalKey, intelRevision, inventoryRevision]);
    const read = () => getExternalIntelDetail(canonicalKey, intelRevision, inventoryRevision);
    const request = cache ? cache.read(key, read) : read();
    void request.then(
      (value) => {
        if (!active) return;
        setDetail(value);
        setStatus(value ? "ready" : "stale");
      },
      () => {
        if (active) setStatus("failed");
      },
    );
    return () => { active = false; };
  }, [canonicalKey, intelRevision, inventoryRevision, attempt, cache]);

  if (status === "loading") {
    return <p role="status" className="text-xs text-ink-subtle">Loading advisory details…</p>;
  }
  if (status === "stale" || status === "failed") {
    const stale = status === "stale";
    return (
      <div className="app-surface flex flex-wrap items-center gap-2 rounded-md border px-4 py-3">
        <p role="status" className="text-xs text-ink-muted">
          {stale ? "The saved feed changed. Reload it to read the latest details." : "Could not load advisory details."}
        </p>
        <Button size="sm" variant="secondary" onClick={() => {
          if (stale) onReloadFeed();
          else {
            setStatus("loading");
            setAttempt((current) => current + 1);
          }
        }}>
          {stale ? "Reload saved feed" : "Retry details"}
        </Button>
      </div>
    );
  }
  if (!detail) return null;

  const selectedItem = { ...row, ...detail };
  return (
    <>
      {selectedItem.withdrawnAt ? (
        <p className="app-surface rounded-md border px-4 py-3 text-xs leading-5 text-ink-muted">
          The source withdrew this advisory. Use current scan results and active advisories to assess exposure.
        </p>
      ) : (
        <IntelSignalCards item={selectedItem} onOpenRepository={onOpenRepository} />
      )}
      <IntelRiskCards item={selectedItem} />
      <AffectedPackages packages={selectedItem.affectedPackages} />

      <section className="space-y-2">
        <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
          Source info
        </h3>
        <div className="app-surface grid gap-x-6 gap-y-3 rounded-md border px-4 py-4 md:grid-cols-2">
          <MetaRow label="Source" value={<ExternalSourceLink item={selectedItem} />} />
          <MetaRow label="Updated" value={formatAbsoluteTimestamp(selectedItem.updatedAt ?? selectedItem.modifiedAt)} />
          {selectedItem.withdrawnAt ? <MetaRow label="Withdrawn" value={formatAbsoluteTimestamp(selectedItem.withdrawnAt)} /> : null}
          <MetaRow label="Aliases" value={<InlineChipList values={selectedItem.aliases} />} />
          <MetaRow label="Product keys" value={<InlineChipList values={selectedItem.productKeys} />} />
          <MetaRow label="Data sources" value={<SourceBadgeGroup itemKey={`meta-${selectedItem.id}`} limit={4} sources={selectedItem.sources} />} />
          <MetaRow label="Advisory ID" value={formatAdvisoryId(selectedItem)} />
        </div>
      </section>

      {selectedItem.kev ? (
        <section className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
            KEV action
          </h3>
          <div className="app-surface grid gap-x-6 gap-y-3 rounded-md border px-4 py-4 md:grid-cols-2">
            <MetaRow label="Date added" value={selectedItem.kev.dateAdded ?? "Not listed"} />
            <MetaRow label="Due date" value={selectedItem.kev.dueDate ?? "Not listed"} />
            <MetaRow label="Ransomware" value={selectedItem.kev.knownRansomwareCampaignUse ?? "Unknown"} />
            <MetaRow label="Catalog" value={selectedItem.kev.catalogVersion ?? "Not listed"} />
            <MetaRow label="Required action" value={selectedItem.kev.requiredAction ?? "Not listed"} />
            <MetaRow label="Notes" value={selectedItem.kev.notes ?? "None"} />
          </div>
        </section>
      ) : null}

      {selectedItem.cwes.length > 0 || selectedItem.references.length > 0 ? (
        <section className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
            Weaknesses and references
          </h3>
          <div className="app-surface grid gap-3 rounded-md border px-4 py-4">
            <MetaRow label="CWEs" value={<InlineChipList values={selectedItem.cwes.map((cwe) => cwe.name ? `${cwe.cweId}: ${cwe.name}` : cwe.cweId)} limit={8} />} />
            <MetaRow label="References" value={<ReferenceLinkList references={selectedItem.references} />} />
          </div>
        </section>
      ) : null}

      {selectedItem.sourceDetails.length > 0 ? (
        <section className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
            Source breakdown
          </h3>
          <div className="app-surface grid gap-2 rounded-md border px-4 py-4">
            {selectedItem.sourceDetails.map((detail) => (
              <div className="flex flex-wrap items-center gap-2 border-t pt-2 first:border-t-0 first:pt-0 [border-color:var(--hairline)]" key={`${detail.source}-${detail.sourceItemId}`}>
                <Badge variant="outline">{formatExternalSourceLabel(detail.source)}</Badge>
                <span className="text-xs font-medium text-ink">{detail.sourceItemId}</span>
                <span className="text-xs text-ink-subtle">{formatExternalSeverityLabel(detail.severity)}</span>
                {detail.cvssScore ? <span className="text-xs text-ink-subtle">CVSS {detail.cvssScore}</span> : null}
                {detail.epssPercentage ? <span className="text-xs text-ink-subtle">EPSS {formatProbability(detail.epssPercentage)}</span> : null}
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="space-y-2">
        <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
          Summary
        </h3>
        <div className="app-surface rounded-md border px-4 py-3">
          <MarkdownSummary value={selectedItem.summary} />
        </div>
      </section>
    </>
  );
}

function AffectedPackages({ packages }: { packages: ExternalIntelDetail["affectedPackages"] }) {
  const [showAll, setShowAll] = useState(false);
  if (!packages.length) return null;
  const visible = showAll ? packages : packages.slice(0, 8);
  return (
    <section className="space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">Affected packages</h3>
      <div className="app-surface grid gap-2 rounded-md border px-4 py-4">
        {visible.map((impact, index) => (
          <div className="grid gap-1 border-t border-hairline pt-2 first:border-t-0 first:pt-0 [overflow-wrap:anywhere]" key={index}>
            <p className="text-xs font-semibold text-ink">
              {[impact.ecosystem, impact.packageName].filter(Boolean).join(" / ") || "Unknown package"}
            </p>
            <p className="text-xs text-ink-subtle">
              Range {impact.vulnerableVersionRange ?? "not listed"} · Patch {impact.firstPatchedVersion ?? "not listed"}
            </p>
            {impact.packagePurl ? <p className="text-xs text-ink-subtle">{impact.packagePurl}</p> : null}
          </div>
        ))}
        {packages.length > 8 ? (
          <Button type="button" variant="ghost" size="sm" className="justify-self-start" aria-expanded={showAll} onClick={() => setShowAll((show) => !show)}>
            {showAll ? "Show fewer affected packages" : `Show all ${packages.length} affected packages`}
          </Button>
        ) : null}
      </div>
    </section>
  );
}

interface MetricTileProps {
  label: string;
  value: number;
  tone?: "default" | "success" | "warning";
}

function MetricTile({ label, value, tone = "default" }: MetricTileProps) {
  return (
    <div className="border-r border-b border-hairline px-4 py-3 last:border-r-0 sm:border-b-0">
      <p
        className={cn(
          "text-xl font-semibold leading-none text-ink",
          tone === "success" && "text-[color:var(--badge-success-fg)]",
          tone === "warning" && "text-[color:var(--badge-warning-fg)]",
        )}
      >
        {value}
      </p>
      <p className="mt-1.5 text-xs font-medium text-ink-subtle">{label}</p>
    </div>
  );
}

interface FilterSelectProps {
  label: string;
  value: string;
  options: Array<{ label: string; value: string }>;
  onChange: (value: string) => void;
}

function FilterSelect({ label, value, options, onChange }: FilterSelectProps) {
  return (
    <Select
      aria-label={label}
      label={label}
      className="h-10 min-w-0"
      containerClassName="min-w-36 flex-1 [&>label]:text-xs [&>label]:text-ink-subtle"
      onValueChange={onChange}
      value={value}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </Select>
  );
}

interface RelevanceBadgeProps {
  relevance: ExternalIntelItem["relevance"];
}

function RelevanceBadge({ relevance }: RelevanceBadgeProps) {
  return (
    <Badge variant={relevanceBadgeVariant(relevance)}>
      {formatRelevanceLabel(relevance)}
    </Badge>
  );
}

interface SourceBadgeGroupProps {
  sources: ExternalSource[];
  itemKey: string;
  limit?: number;
}

function SourceBadgeGroup({ sources, itemKey, limit = 3 }: SourceBadgeGroupProps) {
  const orderedSources = sortExternalSources(sources);
  const visibleSources = orderedSources.slice(0, limit);

  if (visibleSources.length === 0) {
    return <Badge variant="outline">Unknown source</Badge>;
  }

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-1.5 sm:justify-end">
      {visibleSources.map((source) => (
        <Badge key={`${itemKey}-${source}`} variant="outline">
          {formatExternalSourceLabel(source)}
        </Badge>
      ))}
      {orderedSources.length > visibleSources.length ? (
        <Badge variant="outline">+{orderedSources.length - visibleSources.length}</Badge>
      ) : null}
    </div>
  );
}

interface IntelSignalCardsProps {
  item: ExternalIntelItem;
}

function IntelSignalCards({ item, onOpenRepository }: IntelSignalCardsProps & Pick<ExternalSourcesWorkspaceProps, "onOpenRepository">) {
  const [showAllPackages, setShowAllPackages] = useState(false);
  const packages = showAllPackages ? item.localPackages : item.localPackages.slice(0, 5);
  if (item.localPackages.length > 0) {
    return (
      <section className="app-surface space-y-2 rounded-md border px-4 py-3" aria-label="Packages in local scans">
        <h3 className="text-sm font-semibold text-ink">Packages in local scans</h3>
        <p className="text-xs leading-5 text-ink-subtle">
          Package name match. Compare the installed version with the affected range from the advisory.
        </p>
        <ul className="grid gap-2">
          {packages.map(({ repoId, repoName, ecosystem, packageName, package: pkg, affectedVersionRanges }) => (
            <li key={JSON.stringify([repoId, pkg])} className="min-w-0">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                className="h-auto w-full justify-between gap-3 py-2 text-left whitespace-normal [overflow-wrap:anywhere]"
                onClick={() => onOpenRepository(repoId, pkg)}
              >
                <span className="grid min-w-0 gap-0.5">
                  <span className="font-semibold">{repoName}</span>
                  <span className="text-xs text-ink-subtle">{ecosystem} · {packageName} · Installed {pkg.version ?? "version unknown"}</span>
                  <span className="text-xs text-ink-subtle">
                    {affectedVersionRanges?.length ? "Advisory range " + affectedVersionRanges.join(" or ") : "Check affected versions in the source advisory"}
                  </span>
                </span>
                <AppIcon aria-hidden className="size-4 shrink-0" icon={ArrowRight} />
              </Button>
            </li>
          ))}
        </ul>
        {item.localPackages.length > 5 ? (
          <Button type="button" variant="ghost" size="sm" aria-expanded={showAllPackages} onClick={() => setShowAllPackages((show) => !show)}>
            {showAllPackages ? "Show first 5 matches" : "Show all " + item.localPackages.length + " matches (5 shown)"}
          </Button>
        ) : null}
      </section>
    );
  }

  return (
    <section className="grid gap-3 @min-[25rem]/intel-detail:grid-cols-2">
      <SignalCard
        body={buildWhyThisMatters(item)}
        label="Local impact"
        tone="success"
        value={formatLocalSignal(item)}
      />
      <SignalCard
        body={formatRecommendedAction(item)}
        label="Recommended action"
        tone="warning"
        value={item.relevance === "triage" ? "Triage local signal" : "Monitor only"}
      />
    </section>
  );
}

function IntelRiskCards({ item }: IntelSignalCardsProps) {
  if (!item.cvssScore && !item.epssPercentage) return null;
  return (
    <section className="app-surface grid gap-3 rounded-md border px-4 py-3 sm:grid-cols-2" aria-label="Published scores">
      {item.cvssScore ? <MetaRow label="CVSS" value={
        item.cvssScore + (item.cvssVersion ? " · v" + item.cvssVersion : "")
      } /> : null}
      {item.epssPercentage ? <MetaRow label="EPSS" value={
        formatProbability(item.epssPercentage) + (item.epssPercentile ? " · Percentile " + formatProbability(item.epssPercentile) : "")
      } /> : null}
      {item.cvssVector ? <p className="text-xs text-ink-subtle [overflow-wrap:anywhere] sm:col-span-2">{item.cvssVector}</p> : null}
    </section>
  );
}

interface SignalCardProps {
  label: string;
  value: string;
  body: string;
  tone: "success" | "info" | "warning";
}

function SignalCard({ label, value, body, tone }: SignalCardProps) {
  return (
    <div
      className={cn(
        "rounded-md border px-4 py-3",
        tone === "success" && "border-[color:var(--badge-success-bg)] bg-[color:var(--badge-success-bg)]",
        tone === "info" && "border-[color:var(--alert-neutral-border)] bg-[color:var(--alert-neutral-bg)]",
        tone === "warning" && "border-[color:var(--alert-warning-border)] bg-[color:var(--alert-warning-bg)]",
      )}
    >
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full bg-[color:var(--surface-2)]",
            tone === "success" && "text-[color:var(--badge-success-fg)]",
            tone === "info" && "text-[color:var(--alert-neutral-fg)]",
            tone === "warning" && "text-[color:var(--alert-warning-fg)]",
          )}
        >
          <AppIcon className="size-4" icon={Sparkle} />
        </div>
        <div className="min-w-0 [overflow-wrap:anywhere]">
          <p className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-subtle">
            {label}
          </p>
          <p className="mt-2 text-sm font-semibold text-ink">{value}</p>
          <p className="mt-2 text-xs leading-5 text-ink-muted">{body}</p>
        </div>
      </div>
    </div>
  );
}

interface ExternalSourceLinkProps {
  item: ExternalIntelItem;
}

function ExternalSourceLink({ item }: ExternalSourceLinkProps) {
  return (
    <div className="min-w-0 space-y-1">
      <a
        className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
        href={item.url}
        onClick={(event) => {
          event.preventDefault();
          openExternalHref(item.url);
        }}
        rel="noreferrer"
        target="_blank"
      >
        {formatSourceLinkLabel(item)}
        <AppIcon className="size-3" icon={ArrowSquareOut} />
      </a>
      <p className="truncate text-xs text-ink-subtle" title={item.url}>
        {formatUrlHost(item.url)}
      </p>
    </div>
  );
}

interface InlineChipListProps {
  values: string[];
  emptyLabel?: string;
  limit?: number;
}

function InlineChipList({
  values,
  emptyLabel = "None",
  limit = 4,
}: InlineChipListProps) {
  const visibleValues = values.slice(0, limit);

  if (visibleValues.length === 0) {
    return <span className="text-ink-subtle">{emptyLabel}</span>;
  }

  return (
    <div className="flex flex-wrap gap-1.5">
      {visibleValues.map((value) => (
        <Badge className="max-w-full" key={value} title={value} variant="outline">
          <span className="truncate">{value}</span>
        </Badge>
      ))}
      {values.length > visibleValues.length ? (
        <Badge variant="outline">+{values.length - visibleValues.length}</Badge>
      ) : null}
    </div>
  );
}

function ReferenceLinkList({ references }: { references: ExternalIntelItem["references"] }) {
  if (references.length === 0) {
    return <span className="text-ink-subtle">None</span>;
  }

  return (
    <div className="grid gap-1.5">
      {references.slice(0, 8).map((reference) => (
        <a
          className="min-w-0 truncate text-xs font-medium text-primary hover:underline"
          href={reference.url}
          key={reference.url}
          onClick={(event) => {
            event.preventDefault();
            openExternalHref(reference.url);
          }}
          title={reference.url}
        >
          {formatReferenceLabel(reference)}
        </a>
      ))}
      {references.length > 8 ? (
        <span className="text-xs text-ink-subtle">+{references.length - 8} more</span>
      ) : null}
    </div>
  );
}

interface MetaRowProps {
  label: string;
  value: ReactNode;
}

function MetaRow({ label, value }: MetaRowProps) {
  return (
    <div className="grid gap-1 sm:grid-cols-[128px_minmax(0,1fr)] sm:gap-3">
      <p className="text-xs font-medium text-ink-subtle">
        {label}
      </p>
      <div className="min-w-0 text-xs text-ink">{value}</div>
    </div>
  );
}

interface MarkdownSummaryProps {
  value: string;
}

function MarkdownSummary({ value }: MarkdownSummaryProps) {
  return (
    <Suspense
      fallback={
        <p className="text-sm leading-6 text-ink">
          {stripMarkdown(value)}
        </p>
      }
    >
      <ReactMarkdown
        components={{
          h1: ({ children }) => (
            <h4 className="mb-2 mt-4 text-base font-semibold text-ink first:mt-0">
              {children}
            </h4>
          ),
          h2: ({ children }) => (
            <h4 className="mb-2 mt-4 text-base font-semibold text-ink first:mt-0">
              {children}
            </h4>
          ),
          h3: ({ children }) => (
            <h4 className="mb-2 mt-4 text-sm font-semibold text-ink first:mt-0">
              {children}
            </h4>
          ),
          p: ({ children }) => (
            <p className="mb-3 text-sm leading-6 text-ink last:mb-0">
              {children}
            </p>
          ),
          ul: ({ children }) => (
            <ul className="mb-3 list-disc space-y-1.5 pl-5 text-sm leading-6 text-ink">
              {children}
            </ul>
          ),
          ol: ({ children }) => (
            <ol className="mb-3 list-decimal space-y-1.5 pl-5 text-sm leading-6 text-ink">
              {children}
            </ol>
          ),
          li: ({ children }) => <li>{children}</li>,
          a: ({ children, href }) => (
            <a
              className="text-primary underline-offset-4 hover:underline"
              href={href}
              onClick={(event) => {
                event.preventDefault();
                openExternalHref(href);
              }}
              rel="noreferrer"
              target="_blank"
            >
              {children}
            </a>
          ),
          code: ({ children, className }) => {
            const isBlock = Boolean(className);
            if (isBlock) {
              return (
                <code className="block overflow-x-auto whitespace-pre rounded-md border bg-[color:var(--surface-3)] p-3 text-xs leading-5 text-ink">
                  {children}
                </code>
              );
            }

            return (
              <code className="rounded-md border bg-[color:var(--surface-3)] px-1.5 py-0.5 text-xs text-ink">
                {children}
              </code>
            );
          },
          pre: ({ children }) => <pre className="mb-3 overflow-x-auto">{children}</pre>,
          blockquote: ({ children }) => (
            <blockquote className="mb-3 border-l-2 border-[color:var(--hairline)] pl-3 text-sm leading-6 text-ink-subtle">
              {children}
            </blockquote>
          ),
        }}
      >
        {value}
      </ReactMarkdown>
    </Suspense>
  );
}

interface TopicChipProps {
  topic: ExternalIntelTopic;
  isSaving: boolean;
  onToggle: () => void;
}

function TopicChip({ topic, isSaving, onToggle }: TopicChipProps) {
  return (
    <Button
      aria-pressed={topic.watched}
      className={cn(
        "gap-1.5 px-3",
        topic.watched && "bg-[color:var(--surface-4)] text-ink",
      )}
      disabled={isSaving}
      onClick={onToggle}
      type="button"
      size="xs"
      variant="secondary"
    >
      <span>{topic.label}</span>
      <span className="text-ink-subtle">{formatTopicCount(topic.totalCount)}</span>
      {topic.suggested ? (
        <Badge className="h-4 px-1.5 text-xs" variant="success">
          local
        </Badge>
      ) : null}
    </Button>
  );
}

function groupTopics(topics: ExternalIntelTopic[]): Array<{
  category: string;
  topics: ExternalIntelTopic[];
}> {
  const groups = new Map<string, ExternalIntelTopic[]>();
  for (const topic of topics) {
    const group = groups.get(topic.category) ?? [];
    group.push(topic);
    groups.set(topic.category, group);
  }

  return Array.from(groups.entries()).map(([category, groupTopics]) => ({
    category,
    topics: groupTopics,
  }));
}

function uniqueTopicKeys(topicKeys: string[]): string[] {
  return Array.from(new Set(topicKeys));
}

function formatWatchedTopicSummary(topics: ExternalIntelTopic[]): string {
  if (topics.length === 0) {
    return "No topics selected";
  }

  const labels = topics.map((topic) => topic.label);
  if (labels.length <= 4) {
    return labels.join(", ");
  }

  return `${labels.slice(0, 4).join(", ")} + ${labels.length - 4} more`;
}

function stripMarkdown(value: string): string {
  return value
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .trim();
}

function formatTopicCount(value: number): string {
  if (value >= 1000000) {
    return `${(value / 1000000).toFixed(1)}m`;
  }

  if (value >= 1000) {
    return `${Math.round(value / 1000)}k`;
  }

  return String(value);
}

function formatPackageLine(item: ExternalIntelFeedItem): string {
  if (item.packageName && item.ecosystem) {
    return `${item.packageName} • ${item.ecosystem}`;
  }

  if (item.packageName) {
    return item.packageName;
  }

  if (item.ecosystem) {
    return item.ecosystem;
  }

  if (item.productKeys.length > 0) {
    return item.productKeys.slice(0, 2).join(", ");
  }

  return "General advisory";
}

function formatRelevanceLabel(relevance: ExternalIntelItem["relevance"]): string {
  switch (relevance) {
    case "relevant":
      return "Relevant to you";
    case "triage":
      return "Needs triage";
    default:
      return "External advisory";
  }
}

function relevanceBadgeVariant(
  relevance: ExternalIntelItem["relevance"],
): "success" | "warning" | "secondary" {
  switch (relevance) {
    case "relevant":
      return "success";
    case "triage":
      return "warning";
    default:
      return "secondary";
  }
}

function formatLocalSignal(item: ExternalIntelFeedItem): string {
  if (item.relevance === "relevant") {
    if (item.repoCount > 0) {
      return `Package match in ${formatRepoCount(item.repoCount)}`;
    }

    return "Local match";
  }

  if (item.relevance === "triage") {
    if (item.repoCount > 0) {
      return `${formatMatchKindLabel(item.matchKind)} signal in ${formatRepoCount(item.repoCount)}`;
    }

    return `${formatMatchKindLabel(item.matchKind)} signal`;
  }

  return "No local package match";
}

function formatRepoCount(repoCount: number): string {
  return `${repoCount} active repo${repoCount === 1 ? "" : "s"}`;
}

function itemMatchesSeverityFilter(
  item: ExternalIntelFeedItem,
  severityFilter: IntelFilters["severity"],
): boolean {
  switch (severityFilter) {
    case "high_critical":
      return item.severity === "malware" || item.severity === "high" || item.severity === "critical";
    case "critical":
      return item.severity === "malware" || item.severity === "critical";
    case "high":
      return item.severity === "high";
    case "medium_low":
      return item.severity === "medium" || item.severity === "low";
    default:
      return true;
  }
}

function itemMatchesSearch(item: ExternalIntelFeedItem, search: string): boolean {
  const values = [
    item.title,
    item.summary,
    item.packageName,
    item.ecosystem,
    item.canonicalKey,
    ...item.aliases,
    ...item.productKeys,
    ...item.topicKeys,
    ...item.sources.map(formatExternalSourceLabel),
  ];

  return values.some((value) => value?.toLowerCase().includes(search));
}

function formatRecommendedAction(item: ExternalIntelFeedItem): string {
  if (item.relevance === "triage") {
    return "Review the matched product, vendor, or ecosystem signal, then verify exact package/version exposure before remediation.";
  }

  return "Monitor advisory. No local action is suggested unless the package appears in a scan.";
}

function formatAdvisoryId(item: ExternalIntelFeedItem): string {
  if (item.aliases.length > 0) {
    return item.aliases[0];
  }

  return item.canonicalKey;
}

function formatSourceSummary(sources: ExternalSource[]): string {
  const orderedSources = sortExternalSources(sources);

  if (orderedSources.length === 0) {
    return "Unknown source";
  }

  if (orderedSources.length === 1) {
    return formatExternalSourceLabel(orderedSources[0]);
  }

  return `${formatExternalSourceLabel(orderedSources[0])} + ${orderedSources.length - 1} more`;
}

function formatSourceLinkLabel(item: ExternalIntelItem): string {
  if (item.sources.length === 1) {
    return `Open ${formatExternalSourceLabel(item.sources[0])} advisory`;
  }

  return "Open source advisory";
}

function sortExternalSources(sources: ExternalSource[]): ExternalSource[] {
  return [...sources].sort(
    (left, right) => sourceSortIndex(left) - sourceSortIndex(right),
  );
}

function sourceSortIndex(source: ExternalSource): number {
  const index = EXTERNAL_SOURCE_FILTERS.indexOf(source);
  return index === -1 ? Number.MAX_SAFE_INTEGER : index;
}

function formatUrlHost(value: string): string {
  try {
    return new URL(value).hostname.replace(/^www\./, "");
  } catch {
    return value;
  }
}

function formatProbability(value: string): string {
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed)) {
    return value;
  }

  return `${(parsed * 100).toLocaleString(undefined, {
    maximumFractionDigits: 3,
    minimumFractionDigits: parsed > 0 && parsed < 0.01 ? 3 : 0,
  })}%`;
}

function formatReferenceLabel(reference: ExternalIntelItem["references"][number]): string {
  const parts = [reference.source, reference.kind, ...reference.tags.slice(0, 2)]
    .filter((value): value is string => Boolean(value));
  if (parts.length === 0) {
    return formatUrlHost(reference.url);
  }

  return `${formatUrlHost(reference.url)} - ${parts.join(", ")}`;
}

function formatAbsoluteTimestamp(value: string | null): string {
  if (!value) {
    return "Published time unavailable";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function buildWhyThisMatters(item: ExternalIntelFeedItem): string {
  if (item.relevance === "triage") {
    return `Local scan data gives a ${formatMatchKindLabel(item.matchKind).toLowerCase()} signal for this advisory.`;
  }

  return "The latest completed scans have no matching package record.";
}

function formatMatchKindLabel(matchKind: ExternalIntelItem["matchKind"]): string {
  switch (matchKind) {
    case "package_name":
      return "Package";
    case "product":
      return "Product";
    case "vendor":
      return "Vendor";
    case "ecosystem":
      return "Ecosystem";
    default:
      return "Local";
  }
}

function compareFeedItems(left: ExternalIntelFeedItem, right: ExternalIntelFeedItem, column: string): number {
  if (column === "severity") return externalSeverityRank(left.severity) - externalSeverityRank(right.severity);
  if (column === "source") return formatSourceSummary(left.sources).localeCompare(formatSourceSummary(right.sources));
  return (left.isRansomwareKnown ? 4 : left.isKev ? 3 : left.relevance === "relevant" ? 2 : left.relevance === "triage" ? 1 : 0)
    - (right.isRansomwareKnown ? 4 : right.isKev ? 3 : right.relevance === "relevant" ? 2 : right.relevance === "triage" ? 1 : 0)
    || (left.publishedAt ?? "").localeCompare(right.publishedAt ?? "")
    || left.title.localeCompare(right.title);
}

function externalSeverityRank(severity: ExternalIntelItem["severity"]): number {
  return severity === "critical" || severity === "malware" ? 5 : severity === "high" ? 4 : severity === "medium" ? 3 : severity === "low" ? 2 : 1;
}
