import { Profiler, type ReactNode } from "react";
import { recordPerformanceCommit } from "@/lib/performanceCounters";

export function ChatProfiler({ id, children }: { id: "chat-timeline" | "chat-composer"; children: ReactNode }) {
  return import.meta.env.VITE_CVEC_PERF_PROFILER === "on"
    ? <Profiler id={id} onRender={recordPerformanceCommit}>{children}</Profiler>
    : children;
}
