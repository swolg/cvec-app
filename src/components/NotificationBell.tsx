import { Bell, CheckCircle, Info, Trash, WarningCircle, X } from "@phosphor-icons/react";

import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/feedback";
import {
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverDescription,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/overlays";
import { formatRelativeTimestamp } from "@/lib/format";
import type { AppNotification } from "@/lib/types";

interface NotificationBellProps {
  isOpen: boolean;
  notifications: AppNotification[];
  isClearing: boolean;
  onToggle: () => void;
  onClose: () => void;
  onClear: (notificationId: number) => void;
  onClearAll: () => void;
  openTarget?: (notification: AppNotification) => { label: string; open: () => void } | null;
}

export function NotificationBell({ isOpen, notifications, isClearing, onToggle, onClose, onClear, onClearAll, openTarget }: NotificationBellProps) {
  return (
    <Popover open={isOpen} onOpenChange={(open) => { if (open !== isOpen) open ? onToggle() : onClose(); }}>
      <PopoverTrigger
        data-no-window-drag
        render={<IconButton aria-label={`Notifications${notifications.length ? `, ${notifications.length} unread` : ""}`} className="relative overflow-visible" variant="secondary" size="icon-sm" />}
      >
        <AppIcon className="size-4" icon={Bell} />
        {notifications.length > 0 ? (
          <span aria-hidden className="absolute -right-1 -top-1 min-w-4 rounded-full border border-canvas bg-severity-high px-1 text-center text-xs font-medium text-canvas">
            {notifications.length}
          </span>
        ) : null}
      </PopoverTrigger>

      <PopoverContent align="end" className="flex max-h-[var(--available-height)] w-[min(25rem,calc(100vw-2rem))] flex-col overflow-hidden p-0" data-no-window-drag>
        <div className="flex shrink-0 items-start justify-between gap-3 border-b border-hairline p-4">
          <div>
            <PopoverTitle className="flex items-center gap-2 text-base font-medium"><AppIcon className="size-4 text-ink-subtle" icon={Bell} />Notifications</PopoverTitle>
            <PopoverDescription className="mt-1 text-xs text-ink-subtle">Saved alerts and maintenance events you can clear anytime.</PopoverDescription>
          </div>
          <div className="flex items-center gap-1">
            {notifications.length > 0 ? <Button disabled={isClearing} loading={isClearing} onClick={onClearAll} size="xs" type="button" variant="ghost">Clear all</Button> : null}
            <PopoverClose render={<IconButton aria-label="Close notifications" variant="ghost" size="icon-xs" />}><X aria-hidden /></PopoverClose>
          </div>
        </div>

        <div className="max-h-[26rem] min-h-0 overflow-y-auto overscroll-contain p-3" role="region" aria-label="Saved notifications" tabIndex={0}>
          {notifications.length === 0 ? (
            <EmptyState className="min-h-36 border-0 bg-transparent py-6" icon={Bell} title="No notifications" description="Saved scan and maintenance events will appear here." />
          ) : (
            <ul className="space-y-2">
              {notifications.map((notification) => {
                const visual = describeNotification(notification);
                const target = openTarget?.(notification);
                return (
                  <li key={notification.id} className="flex items-start gap-3 rounded-md border border-hairline bg-surface-2 p-3">
                    <span className="flex size-8 shrink-0 items-center justify-center rounded-sm bg-surface-4 text-ink-subtle"><AppIcon className="size-4" icon={visual.icon} weight="fill" /></span>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0"><p className="truncate text-sm font-medium text-ink">{notification.title}</p><p className="mt-1 text-xs text-ink-subtle">{notification.message}</p></div>
                        <IconButton aria-label={`Clear ${notification.title}`} disabled={isClearing} onClick={() => onClear(notification.id)} size="icon-xs" variant="ghost"><Trash aria-hidden /></IconButton>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-2">
                        <Badge variant={visual.badgeVariant}>{visual.badgeLabel}</Badge>
                        {notification.repoName ? <Badge variant="outline">{notification.repoName}</Badge> : null}
                        <time className="text-xs text-ink-subtle" dateTime={notification.createdAt}>{formatRelativeTimestamp(notification.createdAt)}</time>
                      </div>
                      {target ? <Button className="mt-1 px-0" size="xs" variant="ghost" onClick={() => { onClose(); target.open(); }}>{target.label}</Button>
                        : openTarget && notification.repoName ? <p className="mt-2 text-xs text-ink-subtle">Repository no longer available.</p> : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function describeNotification(notification: AppNotification) {
  if (notification.tone === "success") return { icon: CheckCircle, badgeLabel: notification.kind === "db_update" ? "DB update" : "Saved", badgeVariant: "success" as const };
  if (notification.tone === "neutral") return { icon: Info, badgeLabel: notification.kind === "db_update" ? "DB check" : "Info", badgeVariant: "secondary" as const };
  return { icon: WarningCircle, badgeLabel: notification.kind === "scan_alert" ? "Scan alert" : "Attention", badgeVariant: "warning" as const };
}
