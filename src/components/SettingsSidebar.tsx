import { ArrowLeft, Cpu, SlidersHorizontal } from "@phosphor-icons/react";

import { AppIcon, type AppIconGlyph } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type SettingsPage = "general" | "providers";

interface SettingsSidebarProps {
  activePage: SettingsPage;
  onBack: () => void;
  onSelect: (page: SettingsPage) => void;
}

const SETTINGS_ITEMS: Array<{ id: SettingsPage; label: string; icon: AppIconGlyph }> = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "providers", label: "AI Providers", icon: Cpu },
];

export function SettingsSidebar({ activePage, onBack, onSelect }: SettingsSidebarProps) {
  return (
    <aside className="flex h-full flex-col overflow-auto px-3 pb-3 pt-4">
      <Button
        aria-label="Back to cvec"
        className="w-full justify-start gap-2 px-3 text-ink-subtle hover:bg-row-hover-surface hover:text-ink"
        onClick={onBack}
        variant="ghost"
      >
        <AppIcon className="size-4" icon={ArrowLeft} />
        <span>Back to cvec</span>
      </Button>

      <p className="mt-6 px-2 text-xs font-medium uppercase tracking-wider text-ink-tertiary">
        Settings
      </p>
      <nav aria-label="Settings" className="mt-2.5 space-y-0.5">
        {SETTINGS_ITEMS.map((item) => {
          const active = item.id === activePage;
          return (
            <Button
              key={item.id}
              aria-current={active ? "page" : undefined}
              className={cn(
                "w-full justify-start gap-2 px-3 text-left",
                active
                  ? "border-navigation-active-border bg-navigation-active-surface font-medium text-ink"
                  : "text-ink-subtle hover:bg-row-hover-surface hover:text-ink",
              )}
              onClick={() => onSelect(item.id)}
              variant="ghost"
            >
              <AppIcon className="size-4" icon={item.icon} weight={active ? "fill" : "regular"} />
              <span className="min-w-0 flex-1 truncate">{item.label}</span>
            </Button>
          );
        })}
      </nav>

      <p className="mt-auto px-2 pt-6 text-xs leading-5 text-ink-tertiary">
        Settings and provider connections stay on this device.
      </p>
    </aside>
  );
}
