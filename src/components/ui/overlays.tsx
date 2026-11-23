import * as React from "react"
import { Dialog as BaseDialog } from "@base-ui/react/dialog"
import { Popover as BasePopover } from "@base-ui/react/popover"
import { Tooltip as BaseTooltip } from "@base-ui/react/tooltip"
import { X } from "@phosphor-icons/react"

import { cn } from "@/lib/utils"
import { IconButton } from "./button"

const Dialog = BaseDialog.Root
const DialogTrigger = BaseDialog.Trigger
const DialogClose = BaseDialog.Close
const DialogTitle = BaseDialog.Title
const DialogDescription = BaseDialog.Description

const DialogContent = React.forwardRef<HTMLDivElement, React.ComponentProps<typeof BaseDialog.Popup> & { positioned?: boolean; fullScreen?: boolean; showClose?: boolean }>(
  ({ className, children, positioned = false, fullScreen = false, showClose = true, ...props }, ref) => (
    <BaseDialog.Portal>
      <BaseDialog.Backdrop className="fixed inset-0 z-50 bg-[var(--overlay)] transition-opacity duration-[var(--motion-normal)] data-[closed]:pointer-events-none data-[ending-style]:pointer-events-none data-[ending-style]:opacity-0 data-[starting-style]:opacity-0" />
      <BaseDialog.Viewport className={cn("fixed inset-0 z-50 flex data-[closed]:pointer-events-none data-[ending-style]:pointer-events-none", fullScreen ? "p-0" : "p-4", positioned ? "items-stretch justify-end" : "items-center justify-center")}>
        <BaseDialog.Popup
          ref={ref}
          className={cn(
            "relative max-h-[calc(100dvh-2rem)] w-full overflow-auto rounded-lg border border-hairline bg-surface-2 text-ink outline-none transition-[opacity,transform] duration-[var(--motion-normal)] data-[ending-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[starting-style]:scale-[0.98] data-[starting-style]:opacity-0",
            fullScreen ? "h-full max-h-none max-w-none rounded-none border-0" : positioned ? "h-full max-w-2xl" : "max-w-lg",
            className,
          )}
          {...props}
        >
          {children}
          {showClose ? (
            <BaseDialog.Close render={<IconButton aria-label="Close dialog" variant="ghost" size="icon-sm" className="absolute right-3 top-3" />}>
              <X aria-hidden />
            </BaseDialog.Close>
          ) : null}
        </BaseDialog.Popup>
      </BaseDialog.Viewport>
    </BaseDialog.Portal>
  ),
)
DialogContent.displayName = "DialogContent"

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("space-y-1 border-b border-hairline px-5 py-4 pr-14 [&_[data-slot=dialog-title]]:text-base [&_[data-slot=dialog-title]]:font-medium [&_[data-slot=dialog-description]]:text-sm [&_[data-slot=dialog-description]]:text-ink-subtle", className)} {...props} />
}

function DialogBody({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("px-5 py-4", className)} {...props} />
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex flex-wrap justify-end gap-2 border-t border-hairline px-5 py-4", className)} {...props} />
}

const Popover = BasePopover.Root
const PopoverTrigger = BasePopover.Trigger
const PopoverClose = BasePopover.Close
const PopoverTitle = BasePopover.Title
const PopoverDescription = BasePopover.Description

const PopoverContent = React.forwardRef<HTMLDivElement, React.ComponentProps<typeof BasePopover.Popup> & { side?: React.ComponentProps<typeof BasePopover.Positioner>["side"]; align?: React.ComponentProps<typeof BasePopover.Positioner>["align"]; sideOffset?: number }>(
  ({ className, side = "bottom", align = "center", sideOffset = 8, ...props }, ref) => (
    <BasePopover.Portal>
      <BasePopover.Positioner side={side} align={align} sideOffset={sideOffset} className="z-[60] outline-none">
        <BasePopover.Popup ref={ref} className={cn("w-80 origin-[var(--transform-origin)] rounded-lg border border-hairline bg-surface-3 p-3 text-sm text-ink outline-none transition-[opacity,transform] duration-[var(--motion-fast)] data-[ending-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[starting-style]:scale-[0.98] data-[starting-style]:opacity-0", className)} {...props} />
      </BasePopover.Positioner>
    </BasePopover.Portal>
  ),
)
PopoverContent.displayName = "PopoverContent"

const TooltipProvider = BaseTooltip.Provider
const Tooltip = BaseTooltip.Root
const TooltipTrigger = BaseTooltip.Trigger

const TooltipContent = React.forwardRef<HTMLDivElement, React.ComponentProps<typeof BaseTooltip.Popup> & { sideOffset?: number }>(
  ({ className, sideOffset = 6, ...props }, ref) => (
    <BaseTooltip.Portal>
      <BaseTooltip.Positioner sideOffset={sideOffset} className="z-[70]">
        <BaseTooltip.Popup ref={ref} className={cn("max-w-xs rounded-sm border border-hairline bg-surface-4 px-2 py-1.5 text-xs text-ink", className)} {...props} />
      </BaseTooltip.Positioner>
    </BaseTooltip.Portal>
  ),
)
TooltipContent.displayName = "TooltipContent"

export {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverDescription,
  PopoverTitle,
  PopoverTrigger,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
}
