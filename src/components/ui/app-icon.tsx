import type { Icon, IconProps } from "@phosphor-icons/react"

import { cn } from "@/lib/utils"

interface AppIconProps extends Omit<IconProps, "children"> {
  icon: Icon
}

function AppIcon({ className, color = "currentColor", icon: IconComponent, weight = "regular", ...props }: AppIconProps) {
  return <IconComponent className={cn("shrink-0", className)} color={color} weight={weight} {...props} />
}

export { AppIcon }
export type { Icon as AppIconGlyph }
