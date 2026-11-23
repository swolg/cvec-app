import * as React from "react"
import { ScrollArea as ScrollAreaPrimitive } from "@base-ui/react/scroll-area"

import { cn } from "@/lib/utils"

function ScrollArea({
  className,
  children,
  ...props
}: ScrollAreaPrimitive.Root.Props) {
  const [isScrolling, setIsScrolling] = React.useState(false)
  const hideScrollbarTimeoutRef = React.useRef<number | null>(null)

  const showScrollbar = React.useCallback(() => {
    setIsScrolling(true)

    if (hideScrollbarTimeoutRef.current !== null) {
      window.clearTimeout(hideScrollbarTimeoutRef.current)
    }

    hideScrollbarTimeoutRef.current = window.setTimeout(() => {
      setIsScrolling(false)
    }, 700)
  }, [])

  React.useEffect(() => {
    return () => {
      if (hideScrollbarTimeoutRef.current !== null) {
        window.clearTimeout(hideScrollbarTimeoutRef.current)
      }
    }
  }, [])

  return (
    <ScrollAreaPrimitive.Root
      data-slot="scroll-area"
      className={cn("group/scroll-area relative", className)}
      {...props}
    >
      <ScrollAreaPrimitive.Viewport
        data-slot="scroll-area-viewport"
        className="size-full rounded-[inherit] transition-[color,box-shadow] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-1"
        onScroll={showScrollbar}
        onTouchMove={showScrollbar}
        onWheelCapture={showScrollbar}
      >
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollBar visible={isScrolling} />
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  )
}

function ScrollBar({
  className,
  orientation = "vertical",
  visible = false,
  ...props
}: ScrollAreaPrimitive.Scrollbar.Props & { visible?: boolean }) {
  return (
    <ScrollAreaPrimitive.Scrollbar
      data-slot="scroll-area-scrollbar"
      data-orientation={orientation}
      orientation={orientation}
      className={cn(
        "flex touch-none p-px transition-[opacity,color] duration-300 select-none group-hover/scroll-area:opacity-100 group-focus-within/scroll-area:opacity-100 data-horizontal:h-2.5 data-horizontal:flex-col data-horizontal:border-t data-horizontal:border-t-transparent data-vertical:h-full data-vertical:w-2.5 data-vertical:border-l data-vertical:border-l-transparent",
        visible ? "pointer-events-auto opacity-100" : "pointer-events-none opacity-0",
        className
      )}
      {...props}
    >
      <ScrollAreaPrimitive.Thumb
        data-slot="scroll-area-thumb"
        className="relative flex-1 rounded-full bg-border"
      />
    </ScrollAreaPrimitive.Scrollbar>
  )
}

export { ScrollArea, ScrollBar }
