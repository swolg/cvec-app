import "react-grid-layout/css/styles.css";
import { scannerIssue } from "@/lib/grypeStatus";
import "react-resizable/css/styles.css";

import {
  ArrowCounterClockwise,
  ArrowsOutCardinal,
  Check,
  Plus,
  SlidersHorizontal,
  Trash,
} from "@phosphor-icons/react";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import {
  Responsive,
  WidthProvider,
  type Layout,
  type ResponsiveLayouts,
} from "react-grid-layout/legacy";

import { AppIcon } from "@/components/ui/app-icon";
import { Button, IconButton } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { CRITICAL_TO_REVIEW_VIEW, OTHER_TO_REVIEW_VIEW } from "@/lib/reviewViews";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/overlays";

import {
  DASHBOARD_WIDGETS,
  widgetNeedsIntel,
  createDefaultDashboardState,
  createWidgetLayout,
  getWidgetDefinition,
  getWidgetTitle,
} from "./dashboardRegistry";
import {
  loadDashboardState,
  saveDashboardState,
} from "./dashboardStorage";
import { DashboardWidgetRenderer } from "./DashboardWidgets";
import type {
  DashboardDataContext,
  DashboardLayoutItem,
  DashboardState,
  DashboardWidgetConfig,
  DashboardWidgetKind,
} from "./types";

const ResponsiveGridLayout = WidthProvider(Responsive);

type GridLayout = Layout;
type GridLayouts = ResponsiveLayouts;

const BREAKPOINTS = { lg: 1200, md: 996, sm: 768, xs: 480, xxs: 0 };
const COLUMNS = { lg: 12, md: 10, sm: 6, xs: 4, xxs: 1 };
const BREAKPOINT_KEYS = Object.keys(COLUMNS);

interface DashboardOverviewProps {
  context: DashboardDataContext;
  onIntelDemandChange?:(needed:boolean)=>void;
  renderHeader: (actions: ReactNode) => ReactNode;
}

export function DashboardOverview({ context, renderHeader,onIntelDemandChange }: DashboardOverviewProps) {
  const defaultState = useMemo(() => createDefaultDashboardState(), []);
  const [dashboardState, setDashboardState] = useState<DashboardState>(() =>
    loadDashboardState(defaultState),
  );
  useEffect(()=>{onIntelDemandChange?.(dashboardState.widgets.some(widget=>widgetNeedsIntel(widget.kind)));},[dashboardState.widgets,onIntelDemandChange]);
  const [isEditing, setIsEditing] = useState(false);
  const [isPickerOpen, setIsPickerOpen] = useState(false);
  const [activeBreakpoint, setActiveBreakpoint] = useState("lg");
  const [layoutAnnouncement, setLayoutAnnouncement] = useState("");
  const hasMountedRef = useRef(false);
  const editBaseline = useRef<DashboardState | null>(null);
  const gestureBaseline = useRef<DashboardState | null>(null);
  const [history, setHistory] = useState<DashboardState[]>([]);

  useEffect(() => {
    if (!hasMountedRef.current) {
      hasMountedRef.current = true;
      return;
    }

    if (isEditing) return;

    const saveTimeoutId = window.setTimeout(() => {
      saveDashboardState(dashboardState);
    }, 250);

    return () => window.clearTimeout(saveTimeoutId);
  }, [dashboardState, isEditing]);

  const layouts = useMemo(
    () => withWidgetConstraints(dashboardState),
    [dashboardState],
  );

  function handleLayoutChange(_current: GridLayout, nextLayouts: GridLayouts) {
    const next = normalizeLayouts(nextLayouts);
    setDashboardState((current) =>
      JSON.stringify(current.layouts) === JSON.stringify(next)
        ? current
        : { ...current, layouts: next },
    );
  }

  function beginEditing() {
    if (editBaseline.current) return;
    editBaseline.current = dashboardState;
    setHistory([]);
    setIsEditing(true);
    setLayoutAnnouncement("");
  }

  function rememberLayout() {
    setHistory((previous) => [...previous.slice(-49), dashboardState]);
  }

  function handleGestureStop(nextLayout: GridLayout) {
    const previous = gestureBaseline.current;
    gestureBaseline.current = null;
    if (!previous) return;
    const before = previous.layouts[activeBreakpoint] ?? [];
    const changed = nextLayout.some((item) => {
      const old = before.find((candidate) => candidate.i === item.i);
      return !old || old.x !== item.x || old.y !== item.y || old.w !== item.w || old.h !== item.h;
    });
    if (changed) setHistory((items) => [...items.slice(-49), previous]);
  }

  function handleUndo() {
    const previous = history.at(-1);
    if (!previous) return;
    setDashboardState(previous);
    setHistory((items) => items.slice(0, -1));
    setLayoutAnnouncement("Last dashboard change undone.");
  }

  function finishEditing(cancel: boolean) {
    const next = cancel ? editBaseline.current ?? dashboardState : dashboardState;
    setDashboardState(next);
    saveDashboardState(next);
    editBaseline.current = null;
    setHistory([]);
    setIsEditing(false);
    setIsPickerOpen(false);
  }

  function handleAddWidget(kind: DashboardWidgetKind) {
    beginEditing();
    rememberLayout();
    const widget = DASHBOARD_WIDGETS[kind].create();
    setDashboardState((current) => addWidgetToState(current, widget));
    setIsPickerOpen(false);
    setIsEditing(true);
  }

  function handleRemoveWidget(widgetId: string) {
    rememberLayout();
    setDashboardState((current) => ({
      ...current,
      widgets: current.widgets.filter((widget) => widget.id !== widgetId),
      layouts: Object.fromEntries(
        Object.entries(current.layouts).map(([breakpoint, items]) => [
          breakpoint,
          items.filter((item) => item.i !== widgetId),
        ]),
      ),
    }));
  }

  function handleReset() {
    rememberLayout();
    setDashboardState(defaultState);
    setIsPickerOpen(false);
  }

  function handleLayoutKeyDown(
    widget: DashboardWidgetConfig,
    event: KeyboardEvent<HTMLButtonElement>,
  ) {
    if (!event.key.startsWith("Arrow")) return;
    event.preventDefault();
    const items = layouts[activeBreakpoint] ?? [];
    const item = items.find((candidate) => candidate.i === widget.id);
    if (!item) return;
    const columns = COLUMNS[activeBreakpoint as keyof typeof COLUMNS] ?? COLUMNS.lg;
    const adjusted = adjustLayoutItem(item, event.key, event.shiftKey, columns);
    if (item.x === adjusted.x && item.y === adjusted.y && item.w === adjusted.w && item.h === adjusted.h) return;
    rememberLayout();
    setDashboardState((current) => ({
      ...current,
      layouts: {
        ...current.layouts,
        [activeBreakpoint]: items.map((candidate) => candidate.i === widget.id ? adjusted : candidate),
      },
    }));

    const action = event.shiftKey ? "Resized" : "Moved";
    setLayoutAnnouncement(`${action} ${getWidgetTitle(widget)} with ${event.key}.`);
  }

  const canArrange = isEditing && activeBreakpoint !== "xxs";
  const headerActions = isEditing ? (
    <div
      aria-label="Dashboard editing controls"
      className="flex flex-wrap items-center gap-2"
      role="group"
    >
      <span className="mr-1 hidden text-xs font-medium text-ink-subtle 2xl:inline">
        Editing dashboard
      </span>
      <Button
        onClick={() => setIsPickerOpen(true)}
        size="sm"
        type="button"
        variant="secondary"
      >
        <AppIcon className="size-4" icon={Plus} />
        Add widget
      </Button>
      <Button disabled={!history.length} onClick={handleUndo} size="sm" type="button" variant="secondary">
        <AppIcon className="size-4" icon={ArrowCounterClockwise} />
        Undo
      </Button>
      <Button onClick={handleReset} size="sm" type="button" variant="ghost">
        <AppIcon className="size-4" icon={ArrowCounterClockwise} />
        Reset layout
      </Button>
      <div className="flex items-center gap-2">
        <Button onClick={() => finishEditing(true)} size="sm" type="button" variant="ghost">
          Cancel
        </Button>
        <Button onClick={() => finishEditing(false)} size="sm" type="button">
          <AppIcon className="size-4" icon={Check} />
          Done
        </Button>
      </div>
    </div>
  ) : (
    <Popover>
      <PopoverTrigger
        render={<Button size="sm" type="button" variant="secondary" />}
      >
        <AppIcon className="size-4" icon={SlidersHorizontal} />
        Customize
      </PopoverTrigger>
      <PopoverContent align="end" className="w-56 p-1.5">
        <PopoverTitle className="sr-only">Customize dashboard</PopoverTitle>
        <Button
          className="w-full justify-start"
          onClick={() => setIsPickerOpen(true)}
          type="button"
          variant="ghost"
        >
          <AppIcon className="size-4" icon={Plus} />
          Add widget
        </Button>
        <Button
          className="w-full justify-start"
          onClick={beginEditing}
          type="button"
          variant="ghost"
        >
          <AppIcon className="size-4" icon={SlidersHorizontal} />
          Edit layout
        </Button>
      </PopoverContent>
    </Popover>
  );

  return (
    <>
      {renderHeader(headerActions)}
      <div className="dashboard-overview min-h-0 flex-1">
        <NeedsAttention context={context} />

        {isEditing ? (
          <>
            <p className="mb-3 text-xs leading-5 text-ink-muted" id="dashboard-layout-keyboard-help">
              {canArrange
                ? "Drag a widget header to move it, or its bottom corner to resize. On the move button, use arrow keys to move and Shift + arrows to resize."
                : "Widen the window to move or resize widgets. You can still add or remove them here."}
              {" "}Changes are saved when you select Done.
            </p>
            <p aria-live="polite" className="sr-only">
              {layoutAnnouncement}
            </p>
          </>
        ) : null}

        <ResponsiveGridLayout
          breakpoints={BREAKPOINTS}
          className={cn("dashboard-grid", isEditing && "dashboard-grid-editing")}
          cols={COLUMNS}
          compactType="vertical"
          containerPadding={[0, 0]}
          draggableHandle=".dashboard-drag-handle"
          draggableCancel=".dashboard-widget-control-remove"
          isBounded
          isDraggable={canArrange}
          isResizable={canArrange}
          layouts={layouts}
          margin={[14, 14]}
          onBreakpointChange={setActiveBreakpoint}
          onLayoutChange={handleLayoutChange}
          onDragStart={() => { gestureBaseline.current = dashboardState; }}
          onDragStop={handleGestureStop}
          onResizeStart={() => { gestureBaseline.current = dashboardState; }}
          onResizeStop={handleGestureStop}
          preventCollision={false}
          resizeHandles={["se"]}
          rowHeight={isEditing ? 92 : 72}
          style={isEditing ? { backgroundSize: `calc((100% + 14px) / ${COLUMNS[activeBreakpoint as keyof typeof COLUMNS]}) 106px` } : undefined}
          useCSSTransforms
        >
          {dashboardState.widgets.map((widget) => (
            <div className="dashboard-grid-item" key={widget.id}>
              <DashboardWidgetFrame
                isEditing={isEditing}
                canArrange={canArrange}
                onLayoutKeyDown={(event) => handleLayoutKeyDown(widget, event)}
                onRemove={() => handleRemoveWidget(widget.id)}
                widget={widget}
              >
                <DashboardWidgetRenderer context={context} widget={widget} />
              </DashboardWidgetFrame>
            </div>
          ))}
        </ResponsiveGridLayout>

        {isPickerOpen ? (
          <DashboardWidgetPicker
            widgets={dashboardState.widgets}
            onAdd={handleAddWidget}
            onClose={() => setIsPickerOpen(false)}
          />
        ) : null}
      </div>
    </>
  );
}

export function NeedsAttention({ context }: { context: DashboardDataContext }) {
  const summary = context.snapshot.reviewSummary;
  const attention = [
    { label: summary.unreviewedCritical === 1 ? "Critical finding" : "Critical findings", value: summary.unreviewedCritical, view: CRITICAL_TO_REVIEW_VIEW, tone: "text-severity-critical" },
    { label: summary.unreviewedOther === 1 ? "Other finding" : "Other findings", value: summary.unreviewedOther, view: OTHER_TO_REVIEW_VIEW, tone: "text-severity-high" },
    { label: summary.failedScans === 1 ? "Failed scan" : "Failed scans", value: summary.failedScans, view: null, tone: "text-severity-critical" },
  ];
  const total = summary.needsReview + summary.failedScans;
  const hasScanResults = Boolean(context.snapshot.workspaceStatus.lastSuccessfulScanAt);
  const needsSource = context.snapshot.repos.length === 0;
  const scanIssue = scannerIssue(context.snapshot.workspaceStatus, context.snapshot.versionInfo.grypeDb);
  const action = context.isScanning ? { label: "View scan", run: context.onScanAll }
    : needsSource ? { label: "Choose repository folder", run: context.onChooseFolder }
    : summary.needsReview ? { label: "Review queue", run: () => context.onOpenReview("needs-review") }
    : scanIssue && context.onOpenScannerSettings ? { label: "Open scanner settings", run: context.onOpenScannerSettings }
    : summary.failedScans ? { label: "Inspect failed scans", run: context.onOpenFailedScans }
    : !hasScanResults && !scanIssue ? { label: "Run first scan", run: context.onScanAll }
    : null;
  return (
    <section className="mb-3 rounded-lg border border-hairline bg-surface-1 px-4 py-3" aria-labelledby="needs-attention-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="needs-attention-title" className="text-sm font-semibold text-ink">
            {context.isScanning ? "Scan in progress" : needsSource ? "Add your first repository" : !total && scanIssue ? "Prepare to scan" : "Needs attention"}
          </h2>
          <p className="mt-0.5 text-xs text-ink-subtle">
            {total ? `${total} item${total === 1 ? "" : "s"} to review or retry.` : needsSource ? "Choose the folder containing your local Git repositories." : scanIssue ?? (hasScanResults ? "No findings need review." : "Your repositories are ready for their first scan.")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {action ? <Button size="sm" onClick={action.run}>{action.label}</Button> : null}
          {summary.needsReview > 0 && scanIssue && context.onOpenScannerSettings && !context.isScanning && !needsSource
            ? <Button size="sm" variant="secondary" onClick={context.onOpenScannerSettings}>Open scanner settings</Button> : null}
        </div>
      </div>
      {total ? (
        <div className="mt-3 grid grid-cols-3 gap-px overflow-hidden rounded-md border border-hairline bg-hairline">
          {attention.map((item) => (
            <Button
              type="button"
              key={item.label}
              aria-label={item.value + " " + item.label.toLowerCase()}
              disabled={item.value === 0}
              className="h-auto min-w-0 items-start whitespace-normal rounded-none border-0 bg-surface-2 px-2 py-2.5 text-left hover:bg-row-hover-surface disabled:opacity-100 sm:px-3"
              variant="ghost"
              onClick={() => item.view ? context.onOpenReview(item.view) : context.onOpenFailedScans()}
            >
              <span>
                <span className={`block text-lg font-semibold ${item.value ? item.tone : "text-ink-subtle"}`}>{item.value}</span>
                <span className="text-xs text-ink-subtle">{item.label}</span>
              </span>
            </Button>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function DashboardWidgetFrame({
  children,
  isEditing,
  canArrange,
  onLayoutKeyDown,
  onRemove,
  widget,
}: {
  children: ReactNode;
  isEditing: boolean;
  canArrange: boolean;
  onLayoutKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
  onRemove: () => void;
  widget: DashboardWidgetConfig;
}) {
  const title = getWidgetTitle(widget);

  return (
    <div className="dashboard-widget-frame h-full">
      {isEditing ? (
        <div className={cn("dashboard-widget-controls", canArrange && "dashboard-drag-handle")}>
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink-muted">{title}</span>
          <IconButton
            aria-describedby="dashboard-layout-keyboard-help"
            aria-label={`Arrange ${title}`}
            className="dashboard-widget-control"
            disabled={!canArrange}
            onKeyDown={onLayoutKeyDown}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <AppIcon className="size-3.5" icon={ArrowsOutCardinal} />
          </IconButton>
          <IconButton
            aria-label={`Remove ${title}`}
            className="dashboard-widget-control dashboard-widget-control-remove"
            onClick={onRemove}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <AppIcon className="size-3.5" icon={Trash} />
          </IconButton>
        </div>
      ) : null}
      <div className="dashboard-widget-content">{children}</div>
    </div>
  );
}

function DashboardWidgetPicker({
  widgets,
  onAdd,
  onClose,
}: {
  widgets: DashboardWidgetConfig[];
  onAdd: (kind: DashboardWidgetKind) => void;
  onClose: () => void;
}) {
  const localKinds = new Set<DashboardWidgetKind>([
    "summary_metric",
    "workspace_status",
    "risk_trend",
    "repository_table",
    "risk_quadrant",
  ]);
  const groups = [
    { label: "Local operations", widgets: Object.values(DASHBOARD_WIDGETS).filter((widget) => localKinds.has(widget.kind)) },
    { label: "External intelligence", widgets: Object.values(DASHBOARD_WIDGETS).filter((widget) => !localKinds.has(widget.kind)) },
  ];

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-2xl" data-no-window-drag>
        <DialogHeader>
          <DialogTitle data-slot="dialog-title">Add widget</DialogTitle>
          <DialogDescription data-slot="dialog-description">
            Choose a panel, then position it in edit mode.
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[70vh] space-y-5 overflow-auto p-5">
          {groups.map((group) => (
            <section key={group.label} aria-labelledby={`widget-group-${group.label.replaceAll(" ", "-")}`}>
              <h3 id={`widget-group-${group.label.replaceAll(" ", "-")}`} className="mb-2 text-xs font-medium uppercase tracking-wider text-ink-subtle">{group.label}</h3>
              <div className="grid gap-2 sm:grid-cols-2">
                {group.widgets.map((widget) => (
                  <Button className="h-auto min-h-24 min-w-0 items-start justify-start gap-3 whitespace-normal bg-surface-1 px-3 py-3 text-left" key={widget.kind} onClick={() => onAdd(widget.kind)} type="button" variant="outline">
                    <span className="flex size-8 shrink-0 items-center justify-center rounded-sm bg-surface-3 text-ink-subtle"><AppIcon className="size-4" icon={widget.icon} /></span>
                    <span className="min-w-0 flex-1 break-words"><span className="block text-sm font-medium text-ink">{widget.title}</span>{" "}<span className="mt-1 block text-xs leading-5 text-ink-subtle">{widget.description}</span>{widgets.some((item) => item.kind === widget.kind) ? <span className="mt-2 block text-xs text-ink-muted">{" "}{widgets.filter((item) => item.kind === widget.kind).length} on dashboard · Add another</span> : null}</span>
                  </Button>
                ))}
              </div>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function addWidgetToState(
  state: DashboardState,
  widget: DashboardWidgetConfig,
): DashboardState {
  const layouts = Object.fromEntries(
    BREAKPOINT_KEYS.map((breakpoint) => {
      const existingItems = state.layouts[breakpoint] ?? [];
      const y = existingItems.reduce(
        (max, item) => Math.max(max, item.y + item.h),
        0,
      );
      return [breakpoint, [...existingItems, createWidgetLayout(widget, y)]];
    }),
  );

  return {
    ...state,
    widgets: [...state.widgets, widget],
    layouts,
  };
}

function withWidgetConstraints(state: DashboardState): GridLayouts {
  const definitionsById = new Map(
    state.widgets.map((widget) => [widget.id, getWidgetDefinition(widget)]),
  );

  return Object.fromEntries(
    Object.entries(state.layouts).map(([breakpoint, items]) => [
      breakpoint,
      items.map((item) => {
        const definition = definitionsById.get(item.i);
        const columns = COLUMNS[breakpoint as keyof typeof COLUMNS] ?? COLUMNS.lg;
        const minW = Math.min(definition?.minW ?? item.minW ?? 2, columns);
        const minH = definition?.minH ?? item.minH ?? 2;
        const w = Math.min(columns, Math.max(item.w, minW));
        return {
          ...item,
          x: Math.min(Math.max(0, item.x), columns - w),
          w,
          h: Math.max(item.h, minH),
          minW,
          minH,
        };
      }),
    ]),
  );
}

function normalizeLayouts(layouts: GridLayouts): Record<string, DashboardLayoutItem[]> {
  return Object.fromEntries(
    Object.entries(layouts).map(([breakpoint, items]) => [
      breakpoint,
      (items ?? []).map((item) => ({
        i: item.i,
        x: item.x,
        y: item.y,
        w: item.w,
        h: item.h,
        minW: item.minW,
        minH: item.minH,
      })),
    ]),
  );
}

export function adjustLayoutItem(
  item: DashboardLayoutItem,
  key: string,
  resize: boolean,
  columns: number,
): DashboardLayoutItem {
  const minW = Math.min(item.minW ?? 1, columns);
  const minH = item.minH ?? 1;

  if (resize) {
    const nextW =
      key === "ArrowLeft"
        ? Math.max(minW, item.w - 1)
        : key === "ArrowRight"
          ? Math.min(columns - item.x, item.w + 1)
          : item.w;
    const nextH =
      key === "ArrowUp"
        ? Math.max(minH, item.h - 1)
        : key === "ArrowDown"
          ? item.h + 1
          : item.h;
    return { ...item, w: nextW, h: nextH };
  }

  return {
    ...item,
    x:
      key === "ArrowLeft"
        ? Math.max(0, item.x - 1)
        : key === "ArrowRight"
          ? Math.min(columns - item.w, item.x + 1)
          : item.x,
    y:
      key === "ArrowUp"
        ? Math.max(0, item.y - 1)
        : key === "ArrowDown"
          ? item.y + 1
          : item.y,
  };
}
