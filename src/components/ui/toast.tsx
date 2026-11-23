import { Toast as BaseToast } from "@base-ui/react/toast"
import { CheckCircle, Info, Warning, X, XCircle } from "@phosphor-icons/react"

import { IconButton } from "./button"

type ToastKind = "success" | "error" | "warning" | "info"
type ToastData = { kind?: ToastKind }

const toastManager = BaseToast.createToastManager<ToastData>()

function toast(options: { title: string; description?: string; kind?: ToastKind; timeout?: number }) {
  return toastManager.add({
    title: options.title,
    description: options.description,
    timeout: options.timeout,
    type: options.kind ?? "info",
    priority: options.kind === "error" ? "high" : "low",
    data: { kind: options.kind ?? "info" },
  })
}

const icons = { success: CheckCircle, error: XCircle, warning: Warning, info: Info }

function ToastViewport() {
  const manager = BaseToast.useToastManager<ToastData>()
  return (
    <BaseToast.Portal>
      <BaseToast.Viewport className="fixed bottom-4 right-4 z-[80] flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2 outline-none">
        {manager.toasts.map((item) => {
          const kind = item.data?.kind ?? "info"
          const Icon = icons[kind]
          return (
            <BaseToast.Root key={item.id} toast={item} className="grid grid-cols-[auto_1fr_auto] items-start gap-3 rounded-lg border border-hairline bg-surface-3 p-3 text-ink transition-[opacity,transform] duration-[var(--motion-normal)] data-[ending-style]:translate-y-2 data-[ending-style]:opacity-0 data-[starting-style]:translate-y-2 data-[starting-style]:opacity-0">
              <Icon aria-hidden className="mt-0.5 size-4 text-primary-hover data-[kind=error]:text-severity-critical" data-kind={kind} weight="fill" />
              <BaseToast.Content className="min-w-0 [overflow-wrap:anywhere]">
                <BaseToast.Title className="text-sm font-medium" />
                <BaseToast.Description className="mt-0.5 text-xs text-ink-subtle" />
              </BaseToast.Content>
              <BaseToast.Close render={<IconButton aria-label="Dismiss notification" variant="ghost" size="icon-xs" />}>
                <X aria-hidden />
              </BaseToast.Close>
            </BaseToast.Root>
          )
        })}
      </BaseToast.Viewport>
    </BaseToast.Portal>
  )
}

function ToastProvider({ children }: { children: React.ReactNode }) {
  return (
    <BaseToast.Provider toastManager={toastManager} limit={4} timeout={5000}>
      {children}
      <ToastViewport />
    </BaseToast.Provider>
  )
}

export { ToastProvider, toast }
