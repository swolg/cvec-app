import { invoke } from "@tauri-apps/api/core";
import type { ProfilerOnRenderCallback } from "react";

let enabled = false;
const emptyCounts = () => ({ cachedHistoryReads: 0, cachedHistoryHydrations: 0, markdownParses: 0, markdownCacheHits: 0, markdownParsedCharacters: 0, markdownParseDurationMs: 0, completedItemRenders: 0, streamingItemRenders: 0, railRenders: 0, composerRenders: 0, timelineRenders: 0, timelineProjections: 0, modelPickerProjections: 0, composerCommits: 0, timelineCommits: 0, composerDurationMs: 0, timelineDurationMs: 0, completedItemCommits: 0, streamingItemCommits: 0, railCommits: 0, reactActualDurationMs: 0, reactMaxRenderDurationMs: 0 });
let counts = emptyCounts();
export function enablePerformanceCounters() { enabled = true; }
export function disablePerformanceCounters() { enabled = false; counts = emptyCounts(); }
export function performanceCountersEnabled() { return enabled; }
export function countPerformanceRender(key: "completedItemRenders" | "streamingItemRenders" | "railRenders" | "composerRenders" | "timelineRenders" | "timelineProjections" | "modelPickerProjections" | "cachedHistoryReads" | "cachedHistoryHydrations" | "markdownParses" | "markdownCacheHits" | "markdownParsedCharacters" | "markdownParseDurationMs", amount = 1) { if (enabled) counts[key] += amount; }
export const recordPerformanceCommit: ProfilerOnRenderCallback = (id, _phase, duration) => {
  if (!enabled) return;
  const key = id === "streaming-item" ? "streamingItemCommits" : id === "conversation-rail" ? "railCommits" : id === "chat-composer" ? "composerCommits" : id === "chat-timeline" ? "timelineCommits" : "completedItemCommits";
  counts[key]++;
  if (id === "chat-composer" || id === "chat-timeline") {
    counts[id === "chat-composer" ? "composerDurationMs" : "timelineDurationMs"] += duration;
    return;
  }
  counts.reactActualDurationMs += duration;
  counts.reactMaxRenderDurationMs = Math.max(counts.reactMaxRenderDurationMs, duration);
};
export function recordPerformanceSample(sample: Record<string, number | boolean>) {
  if (enabled) void invoke("record_ui_timings", { sample }).catch(() => undefined);
}
export function takePerformanceCounters() { const value = counts; counts = emptyCounts(); return value; }
