import { Check, Copy } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";

export function CopyButton({ text, label, className, showLabel = false }: { text: string | (() => string); label: string; className?: string; showLabel?: boolean }) {
  const [state, setState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const source = useRef(text);
  source.current = text;
  const request = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => { setState("idle"); return () => { request.current += 1; clearTimeout(timer.current); }; }, [text, label]);
  const read = () => typeof source.current === "function" ? source.current() : source.current;

  async function copy() {
    const sequence = ++request.current;
    clearTimeout(timer.current);
    const value = read();
    setState("copying");
    try {
      if (!navigator.clipboard?.writeText) throw new Error("Clipboard unavailable");
      await navigator.clipboard.writeText(value);
      if (sequence !== request.current) return;
      setState(read() === value ? "copied" : "idle");
      timer.current = setTimeout(() => { if (sequence === request.current) setState("idle"); }, 2000);
    } catch {
      if (sequence === request.current) setState(read() === value ? "failed" : "idle");
    }
  }

  return <span className={`inline-flex flex-wrap items-center gap-2 ${className ?? ""}`}>
    <Button aria-label={label} title={label} size="xs" variant="ghost" disabled={state === "copying"} onClick={() => void copy()}>
      <AppIcon className="size-3.5" icon={state === "copied" ? Check : Copy} />{state === "copied" ? "Copied" : state === "failed" ? "Retry copy" : showLabel ? label : "Copy"}
    </Button>
    {state === "copied" || state === "failed" ? <span role="status" className={state === "failed" ? "text-xs text-status-warning-foreground" : "sr-only"}>{state === "copied" ? "Copied to clipboard." : "Copy failed. Select the text to copy manually, or retry."}</span> : null}
  </span>;
}
