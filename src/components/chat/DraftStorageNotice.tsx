import { Button } from "@/components/ui/button";

export function DraftStorageNotice({ error, onRetry }: { error: "restore" | "save" | null | undefined; onRetry: () => void }) {
  if (!error) return null;
  return <div role="status" className="mb-2 flex flex-wrap items-center gap-2 text-xs text-status-high">
    <span>{error === "restore" ? "Your saved draft could not be restored. Retry before editing." : "Your draft could not be saved locally. Keep this window open and retry."}</span>
    <Button size="xs" variant="secondary" onClick={onRetry}>Retry draft storage</Button>
  </div>;
}
