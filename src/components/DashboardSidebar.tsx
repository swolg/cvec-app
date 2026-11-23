import {
  ChatCircleDots,
  GearSix,
  GlobeHemisphereWest,
  Kanban,
  ShieldCheckered,
  SquaresFour,
} from "@phosphor-icons/react";

import { AppIcon, type AppIconGlyph } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type DashboardSection =
  | "overview"
  | "review"
  | "chat"
  | "intel"
  | "repositories"
  | "settings";

interface DashboardSidebarProps {
  activeSection: DashboardSection;
  onSelect: (section: DashboardSection) => void;
  onPrepare?: (section:DashboardSection)=>void;
  reviewCount: number;
  favoriteViews?: Array<{ id: string; name: string }>;
  onSelectView?: (viewId: string) => void;
}

const PRIMARY_NAV_ITEMS: Array<{
  id: DashboardSection;
  label: string;
  icon: AppIconGlyph;
}> = [
  {
    id: "overview",
    label: "Home",
    icon: SquaresFour,
  },
  {
    id: "review",
    label: "Review",
    icon: ShieldCheckered,
  },
  {
    id: "chat",
    label: "Chat",
    icon: ChatCircleDots,
  },
  {
    id: "repositories",
    label: "Repositories",
    icon: Kanban,
  },
];

const EXTERNAL_NAV_ITEMS: Array<{
  id: DashboardSection;
  label: string;
  icon: AppIconGlyph;
}> = [
  {
    id: "intel",
    label: "Intelligence",
    icon: GlobeHemisphereWest,
  },
];

const SIDEBAR_ROW_CLASSNAME = "w-full justify-start gap-2 px-3 text-left";

export function DashboardSidebar({
  activeSection,
  onSelect,
  onPrepare,
  reviewCount,
  favoriteViews = [],
  onSelectView,
}: DashboardSidebarProps) {
  function renderNavItem(item: (typeof PRIMARY_NAV_ITEMS)[number]) {
    const active = item.id === activeSection;

    return (
      <Button
        key={item.id}
        aria-label={item.id === "review" && reviewCount > 0 ? `${item.label}, ${reviewCount} need review` : item.label}
        aria-current={active ? "page" : undefined}
        className={cn(
          SIDEBAR_ROW_CLASSNAME,
          active
            ? "border-navigation-active-border bg-navigation-active-surface font-medium text-ink"
            : "text-ink-subtle hover:bg-row-hover-surface hover:text-ink",
        )}
        onClick={() => onSelect(item.id)}
        onPointerEnter={()=>onPrepare?.(item.id)}
        onFocus={()=>onPrepare?.(item.id)}
        variant="ghost"
        type="button"
      >
        <AppIcon className="size-4" icon={item.icon} weight={active ? "fill" : "regular"} />
        <span className="sidebar-item-label min-w-0 flex-1 truncate">{item.label}</span>
        {item.id === "review" && reviewCount > 0 ? (
          <span className="sidebar-item-count rounded-full bg-severity-high/15 px-1.5 py-0.5 text-xs font-medium text-severity-high">
            {reviewCount}
          </span>
        ) : null}
      </Button>
    );
  }

  return (
    <aside className="flex h-full flex-col overflow-auto px-3 pb-3 pt-4">
      <nav className="space-y-0.5">
        {PRIMARY_NAV_ITEMS.map(renderNavItem)}
      </nav>

      <SidebarLabel className="mt-6">External</SidebarLabel>

      <nav className="mt-2.5 space-y-0.5">
        {EXTERNAL_NAV_ITEMS.map(renderNavItem)}
      </nav>

      {favoriteViews.length ? <><SidebarLabel className="mt-6">Saved views</SidebarLabel><nav className="mt-2.5 space-y-0.5">{favoriteViews.map((view) => <Button key={view.id} className={cn(SIDEBAR_ROW_CLASSNAME, "text-ink-subtle hover:bg-row-hover-surface hover:text-ink")} onClick={() => onSelectView?.(view.id)} onPointerEnter={()=>onPrepare?.("review")} onFocus={()=>onPrepare?.("review")} variant="ghost"><AppIcon className="size-4" icon={ShieldCheckered} /><span className="truncate">{view.name}</span></Button>)}</nav></> : null}

      <div className="mt-auto pt-6">
        <SidebarLabel>General</SidebarLabel>

        <nav className="mt-2.5 space-y-0.5">
          {renderNavItem({
            id: "settings",
            label: "Settings",
            icon: GearSix,
          })}
        </nav>
      </div>
    </aside>
  );
}

interface SidebarLabelProps {
  children: string;
  className?: string;
}

function SidebarLabel({ children, className }: SidebarLabelProps) {
  return (
    <p
      className={cn(
        "sidebar-section-label px-2 text-xs font-medium uppercase tracking-wider text-ink-tertiary",
        className,
      )}
    >
      {children}
    </p>
  );
}
