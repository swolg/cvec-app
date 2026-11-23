import * as React from "react"
import { Select as BaseSelect } from "@base-ui/react/select"
import { CaretDown, Check, MagnifyingGlass } from "@phosphor-icons/react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

type FieldMetaProps = {
  label?: React.ReactNode
  description?: React.ReactNode
  error?: React.ReactNode
  disabledReason?: React.ReactNode
  containerClassName?: string
}

const controlClass = "h-11 w-full rounded-md border border-hairline-strong bg-surface-1 px-3 text-sm text-ink outline-none transition-colors placeholder:text-ink-tertiary hover:border-ink-tertiary focus:border-primary-focus focus:ring-2 focus:ring-primary-focus/30 disabled:cursor-not-allowed disabled:bg-surface-2 disabled:text-ink-tertiary"

function FieldMeta({ id, label, description, error, disabledReason, containerClassName, children }: FieldMetaProps & { id: string; children: React.ReactNode }) {
  const supportingText = error ?? disabledReason ?? description
  return (
    <div className={cn("grid gap-1.5 text-sm font-medium text-ink", containerClassName)}>
      {label ? <label htmlFor={id}>{label}</label> : null}
      {children}
      {supportingText ? (
        <span id={`${id}-description`} className={cn("text-xs font-normal text-ink-subtle", error && "text-severity-critical")}>
          {supportingText}
        </span>
      ) : null}
    </div>
  )
}

type InputProps = Omit<React.ComponentProps<"input">, "size"> & FieldMetaProps

const Input = React.forwardRef<HTMLInputElement, InputProps>(({ className, label, description, error, disabledReason, containerClassName, id: providedId, ...props }, ref) => {
  const generatedId = React.useId()
  const id = providedId ?? generatedId
  const describedBy = error || disabledReason || description ? `${id}-description` : undefined
  return (
    <FieldMeta id={id} label={label} description={description} error={error} disabledReason={disabledReason} containerClassName={containerClassName}>
      <input ref={ref} id={id} data-slot="input" className={cn(controlClass, className)} aria-describedby={describedBy} aria-invalid={Boolean(error) || undefined} {...props} />
    </FieldMeta>
  )
})
Input.displayName = "Input"

const SearchField = React.forwardRef<HTMLInputElement, InputProps>(({ className, ...props }, ref) => (
  <div className="relative">
    <MagnifyingGlass aria-hidden className="pointer-events-none absolute left-3 top-1/2 z-10 size-4 -translate-y-1/2 text-ink-subtle" />
    <Input ref={ref} type="search" autoCorrect="off" autoCapitalize="none" spellCheck={false} className={cn("pl-9", className)} {...props} />
  </div>
))
SearchField.displayName = "SearchField"

type SelectProps = Omit<React.ComponentProps<typeof BaseSelect.Root<string>>, "children" | "items" | "onValueChange" | "value" | "defaultValue"> & FieldMetaProps & {
  children: React.ReactNode
  className?: string
  value?: string
  defaultValue?: string
  placeholder?: React.ReactNode
  onValueChange?: (value: string) => void
  "aria-label"?: string
}

type SelectOption = {
  value: string
  label: React.ReactNode
  disabled: boolean
}

function resolveSelectOptions(children: React.ReactNode): SelectOption[] {
  return React.Children.toArray(children).flatMap((child) => {
    if (!React.isValidElement<React.OptionHTMLAttributes<HTMLOptionElement>>(child) || child.type !== "option") return []
    return [{
      value: String(child.props.value ?? child.props.children ?? ""),
      label: child.props.children,
      disabled: Boolean(child.props.disabled),
    }]
  })
}

const Select = React.forwardRef<HTMLButtonElement, SelectProps>(({ className, label, description, error, disabledReason, containerClassName, id: providedId, children, value, defaultValue, placeholder = "Select…", onValueChange, disabled, "aria-label": ariaLabel, ...props }, ref) => {
  const generatedId = React.useId()
  const id = providedId ?? generatedId
  const options = React.useMemo(() => resolveSelectOptions(children), [children])
  const describedBy = error || disabledReason || description ? `${id}-description` : undefined
  return (
    <FieldMeta id={id} label={label} description={description} error={error} disabledReason={disabledReason} containerClassName={containerClassName}>
      <BaseSelect.Root<string>
        {...props}
        defaultValue={defaultValue}
        disabled={disabled}
        id={id}
        items={options.map((option) => ({ label: option.label, value: option.value }))}
        onValueChange={(nextValue) => { if (nextValue !== null) onValueChange?.(nextValue) }}
        value={value}
      >
        <BaseSelect.Trigger
          ref={ref}
          aria-describedby={describedBy}
          aria-invalid={Boolean(error) || undefined}
          aria-label={ariaLabel}
          className={cn(controlClass, "flex items-center justify-between gap-2 pr-3 text-left", className)}
          data-slot="select"
        >
          <BaseSelect.Value className="min-w-0 flex-1 truncate" placeholder={placeholder} />
          <BaseSelect.Icon className="flex shrink-0 text-ink-subtle transition-transform data-[popup-open]:rotate-180">
            <CaretDown aria-hidden className="size-4" />
          </BaseSelect.Icon>
        </BaseSelect.Trigger>
        <BaseSelect.Portal>
          <BaseSelect.Positioner align="start" alignItemWithTrigger={false} className="z-[80] outline-none" sideOffset={6}>
            <BaseSelect.Popup className="max-h-88 min-w-48 origin-[var(--transform-origin)] overflow-hidden rounded-lg border border-hairline bg-surface-3 text-sm text-ink shadow-xl outline-none transition-[opacity,transform] duration-[var(--motion-fast)] data-[ending-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[starting-style]:scale-[0.98] data-[starting-style]:opacity-0">
              <BaseSelect.List className="max-h-88 overflow-y-auto p-1.5">
                {options.map((option) => (
                  <BaseSelect.Item
                    key={option.value}
                    className="relative flex min-h-9 cursor-default items-center rounded-md py-1.5 pl-8 pr-2 text-sm outline-none transition-colors data-[disabled]:pointer-events-none data-[disabled]:opacity-40 data-[highlighted]:bg-row-hover-surface data-[selected]:bg-navigation-active-surface data-[selected]:text-ink"
                    disabled={option.disabled}
                    value={option.value}
                  >
                    <BaseSelect.ItemIndicator className="absolute left-2 flex text-primary">
                      <Check aria-hidden className="size-4" weight="bold" />
                    </BaseSelect.ItemIndicator>
                    <BaseSelect.ItemText className="min-w-0 flex-1 truncate">{option.label}</BaseSelect.ItemText>
                  </BaseSelect.Item>
                ))}
              </BaseSelect.List>
            </BaseSelect.Popup>
          </BaseSelect.Positioner>
        </BaseSelect.Portal>
      </BaseSelect.Root>
    </FieldMeta>
  )
})
Select.displayName = "Select"

type SegmentOption<T extends string> = { value: T; label: React.ReactNode }

function SegmentedControl<T extends string>({ value, options, onValueChange, label }: { value: T; options: readonly SegmentOption<T>[]; onValueChange: (value: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex min-h-11 rounded-md border border-hairline bg-control-surface p-1">
      {options.map((option) => (
        <Button key={option.value} type="button" role="radio" aria-checked={value === option.value} onClick={() => onValueChange(option.value)} className="min-h-9 rounded-sm border border-transparent px-3 text-sm font-medium text-ink-subtle data-[checked=true]:border-navigation-active-border data-[checked=true]:bg-navigation-active-surface data-[checked=true]:text-ink" data-checked={value === option.value} variant="ghost">
          {option.label}
        </Button>
      ))}
    </div>
  )
}

type TabOption<T extends string> = { value: T; label: React.ReactNode }

function Tabs<T extends string>({ value, options, onValueChange, label, idPrefix }: { value: T; options: readonly TabOption<T>[]; onValueChange: (value: T) => void; label: string; idPrefix?: string }) {
  return (
    <div role="tablist" aria-label={label} className="flex min-h-9 flex-wrap gap-4 border-b border-hairline px-1">
      {options.map((option) => {
        const selected = value === option.value
        return (
          <Button
            id={idPrefix ? `${idPrefix}-tab-${option.value}` : undefined}
            aria-controls={idPrefix ? `${idPrefix}-panel-${option.value}` : undefined}
            aria-selected={selected}
            className="-mb-px min-h-9 min-w-max rounded-none border-0 border-b-2 border-transparent px-0.5 text-xs font-medium text-ink-subtle outline-none transition-colors hover:bg-transparent hover:text-ink focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-offset-0 focus-visible:ring-primary-focus data-[selected=true]:border-primary data-[selected=true]:text-ink"
            data-selected={selected}
            key={option.value}
            onClick={() => onValueChange(option.value)}
            onFocus={(event) => event.currentTarget.scrollIntoView?.({ block: "nearest", inline: "nearest" })}
            onKeyDown={(event) => {
              if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.defaultPrevented || event.nativeEvent.isComposing) return;
              const index = options.findIndex((item) => item.value === option.value);
              const next = event.key === "ArrowRight" ? (index + 1) % options.length
                : event.key === "ArrowLeft" ? (index - 1 + options.length) % options.length
                : event.key === "Home" ? 0 : event.key === "End" ? options.length - 1 : -1;
              if (next < 0) return;
              event.preventDefault();
              onValueChange(options[next].value);
              event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus({ preventScroll: true });
            }}
            role="tab"
            size="xs"
            tabIndex={selected ? 0 : -1}
            type="button"
            variant="ghost"
          >
            {option.label}
          </Button>
        )
      })}
    </div>
  )
}

export { Input, SearchField, Select, SegmentedControl, Tabs }
