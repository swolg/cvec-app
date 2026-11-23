import * as React from "react"
import { CircleNotch, Info } from "@phosphor-icons/react"

import { cn } from "@/lib/utils"

function Spinner({ className, label = "Loading" }: { className?: string; label?: string }) {
  return (
    <span role="status" className={cn("inline-flex items-center gap-2 text-sm text-ink-muted", className)}>
      <CircleNotch aria-hidden className="size-4 animate-spin" />
      <span className="sr-only">{label}</span>
    </span>
  )
}

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return <div aria-hidden data-slot="skeleton" className={cn("animate-pulse rounded-sm bg-surface-3", className)} {...props} />
}

function EmptyState({ icon: Icon = Info, title, description, action, className }: { icon?: React.ComponentType<{ className?: string; "aria-hidden"?: boolean }>; title: React.ReactNode; description?: React.ReactNode; action?: React.ReactNode; className?: string }) {
  return (
    <div data-slot="empty-state" className={cn("flex min-h-48 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-hairline bg-surface-1 px-6 py-10 text-center", className)}>
      <span className="flex size-10 items-center justify-center rounded-md bg-surface-3 text-ink-subtle"><Icon aria-hidden className="size-5" /></span>
      <div className="space-y-1"><h3 className="text-sm font-medium text-ink">{title}</h3>{description ? <p className="max-w-md text-sm text-ink-subtle">{description}</p> : null}</div>
      {action}
    </div>
  )
}

export { EmptyState, Skeleton, Spinner }
