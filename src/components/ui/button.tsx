import { Button as ButtonPrimitive } from "@base-ui/react/button"
import { CircleNotch } from "@phosphor-icons/react"
import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "group/button inline-flex shrink-0 cursor-pointer items-center justify-center rounded-md border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-[color,background-color,border-color,transform] duration-[var(--motion-fast)] outline-none select-none focus-visible:ring-2 focus-visible:ring-primary-focus focus-visible:ring-offset-2 focus-visible:ring-offset-canvas active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-45 aria-invalid:border-destructive aria-invalid:ring-destructive/30 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary-hover",
        outline:
          "border-hairline-strong bg-surface-1 text-ink hover:bg-surface-2 aria-expanded:bg-surface-3",
        secondary:
          "border-hairline bg-surface-2 text-ink hover:bg-surface-3 aria-expanded:bg-surface-3",
        toggle:
          "border-hairline bg-surface-1 text-ink-muted hover:bg-surface-2 hover:text-ink aria-expanded:bg-row-selected-surface aria-expanded:text-ink aria-pressed:bg-row-selected-surface aria-pressed:text-ink",
        ghost:
          "text-ink-muted hover:bg-surface-2 hover:text-ink aria-expanded:bg-surface-3",
        destructive:
          "bg-destructive/10 text-destructive hover:bg-destructive/20 focus-visible:border-destructive/40 focus-visible:ring-destructive/20 dark:bg-destructive/20 dark:hover:bg-destructive/30 dark:focus-visible:ring-destructive/40",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-10 gap-2 px-3.5",
        xs: "h-8 gap-1.5 rounded-sm px-2.5 text-xs",
        sm: "h-9 gap-1.5 px-3 text-xs",
        lg: "h-11 gap-2 px-5",
        icon: "size-10",
        "icon-xs": "size-8 rounded-sm [&_svg:not([class*='size-'])]:size-3.5",
        "icon-sm": "size-9 [&_svg:not([class*='size-'])]:size-3.5",
        "icon-lg": "size-11 [&_svg:not([class*='size-'])]:size-5",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  loading = false,
  loadingLabel = "Working",
  children,
  disabled,
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants> & {
  loading?: boolean
  loadingLabel?: string
}) {
  return (
    <ButtonPrimitive
      data-slot="button"
      data-loading={loading || undefined}
      className={cn(buttonVariants({ variant, size, className }))}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {loading ? <CircleNotch aria-hidden className="animate-spin" /> : null}
      {loading ? <span className="sr-only">{loadingLabel}</span> : null}
      {loading ? " " : null}
      {children}
    </ButtonPrimitive>
  )
}

type IconButtonProps = Omit<React.ComponentProps<typeof Button>, "children" | "size"> & {
  "aria-label": string
  children?: React.ReactNode
  size?: "icon" | "icon-xs" | "icon-sm" | "icon-lg"
}

function IconButton({ size = "icon", ...props }: IconButtonProps) {
  return <Button data-slot="icon-button" size={size} {...props} />
}

export { Button, IconButton, buttonVariants }
