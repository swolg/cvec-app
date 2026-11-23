import { mergeProps } from "@base-ui/react/merge-props"
import { useRender } from "@base-ui/react/use-render"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"
import { CheckCircle, Info, Warning, WarningCircle, XCircle } from "@phosphor-icons/react"

const badgeVariants = cva(
  "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-full border border-transparent px-2 py-0.5 text-xs font-medium whitespace-nowrap transition-all focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-2.5!",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground [a]:hover:bg-primary/80",
        secondary:
          "bg-secondary text-secondary-foreground [a]:hover:bg-secondary/80",
        success:
          "bg-[color:var(--badge-success-bg)] text-[color:var(--badge-success-fg)] [a]:hover:opacity-90",
        warning:
          "bg-[color:var(--badge-warning-bg)] text-[color:var(--badge-warning-fg)] [a]:hover:opacity-90",
        destructive:
          "bg-destructive/10 text-destructive focus-visible:ring-destructive/20 dark:bg-destructive/20 dark:focus-visible:ring-destructive/40 [a]:hover:bg-destructive/20",
        outline:
          "border-border bg-input/20 text-foreground dark:bg-input/30 [a]:hover:bg-muted [a]:hover:text-muted-foreground",
        ghost:
          "hover:bg-muted hover:text-muted-foreground dark:hover:bg-muted/50",
        link: "text-primary underline-offset-4 hover:underline",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

function Badge({
  className,
  variant = "default",
  render,
  ...props
}: useRender.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return useRender({
    defaultTagName: "span",
    props: mergeProps<"span">(
      {
        className: cn(badgeVariants({ variant }), className),
      },
      props
    ),
    render,
    state: {
      slot: "badge",
      variant,
    },
  })
}

const statusConfig = {
  neutral: { icon: Info, variant: "outline" as const },
  success: { icon: CheckCircle, variant: "success" as const },
  warning: { icon: Warning, variant: "warning" as const },
  critical: { icon: XCircle, variant: "destructive" as const },
  blocked: { icon: WarningCircle, variant: "destructive" as const },
}

function StatusBadge({ status, children, className }: {
  status: keyof typeof statusConfig
  children: React.ReactNode
  className?: string
}) {
  const config = statusConfig[status]
  const Icon = config.icon
  return (
    <Badge className={className} variant={config.variant}>
      <Icon aria-hidden weight="fill" />
      <span>{children}</span>
    </Badge>
  )
}

export { Badge, StatusBadge, badgeVariants }
