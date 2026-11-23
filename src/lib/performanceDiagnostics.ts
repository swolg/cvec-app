import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters, recordPerformanceSample } from "./performanceCounters";
import { companionStore } from "./companionState";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { installQaCaptureMetadata } from "./qaCaptureMetadata";

let activeCleanup: (() => void) | undefined;

export async function installPerformanceDiagnostics(): Promise<(() => void) | undefined> {
  if (!isTauri()) return;
  if (activeCleanup) return activeCleanup;
  const identity = await invoke<{ trace: boolean; bundleIdentifier?: string }>("get_build_identity");
  if (!identity.trace) return;
  if (activeCleanup) return activeCleanup;
  enablePerformanceCounters();
  const disposeCaptureMetadata = installQaCaptureMetadata(identity);
  const started = performance.now();
  // Epoch bridge for external capture times; every elapsed value below uses
  // this WebView clock, independently of the native heartbeat clock.
  recordPerformanceSample({ sessionTimeOriginMs: performance.timeOrigin + started });
  let last = started;
  let stopped = false;
  const scheduled = new Set<number>();
  function raf(callback: FrameRequestCallback) {
    const handle = requestAnimationFrame((time) => {
      scheduled.delete(handle);
      if (!stopped) callback(time);
    });
    scheduled.add(handle);
  }
  const afterPaint = (callback: () => void) => raf(() => raf(callback));
  const visibleNode = (selector: string) => [...document.querySelectorAll<HTMLElement>(selector)].find(node => node.getBoundingClientRect().height > 0);
  const chatState = () => {
    const workspace = visibleNode('[data-active-workspace="chat"] [data-chat-workspace]');
    if (!workspace) return;
    return [workspace, ...workspace.querySelectorAll<HTMLElement>('[data-chat-state]')]
      .find(node => node.dataset.chatState && node.getBoundingClientRect().height > 0);
  };
  const chatReady = () => {
    const node = chatState();
    if (!node) return false;
    const state = node.dataset.chatState;
    if (state === "empty") return true;
    if (state !== "ready" && state !== "read-only") return false;
    // An explicit connection/read-only state can be usable without history.
    // A regular conversation must expose its real timeline, never any textarea.
    return node.hasAttribute("data-chat-contained-state") ||
      [...node.querySelectorAll<HTMLElement>('[role="log"][aria-label="Repository chat messages"]')]
        .some(timeline => timeline.getBoundingClientRect().height > 0);
  };
  const chatStates: Record<string, number> = { ready: 1, empty: 2, "read-only": 3, error: 4 };
  const chatReadyState = () => chatStates[chatState()?.dataset.chatState ?? ""];
  type Transition = { at: number; sequence: number; check: () => boolean; failed?: () => boolean; readyState?: () => number | undefined; key: string; settling: boolean };
  let transitionSequence = 0;
  const pending = new Map<number, Transition>();
  const begin = (id: number, key: string, check: () => boolean, failed?: () => boolean, readyState?: () => number | undefined) => {
    pending.set(id, { at: performance.now(), sequence: ++transitionSequence, check, failed, readyState, key, settling: false });
    inspect();
  };
  function finish(id: number, entry: Transition, outcome: "ready" | "failed" | "timeout") {
    const now = performance.now();
    pending.delete(id);
    const state = entry.readyState?.();
    recordPerformanceSample({
      uiAction: id, uiTransitionId: entry.sequence, uiRoute: id <= 4 ? id : 1,
      uiStartedElapsedMs: entry.at - started, uiSettledElapsedMs: now - started,
      ...(state !== undefined ? { uiReadyState: state } : {}),
      ...(outcome === "ready" ? { [entry.key]: now - entry.at } :
        outcome === "failed" ? { uiTransitionFailed: true } : { uiTransitionTimeout: true }),
    });
  }
  function inspect() {
    for (const [id, entry] of pending) {
      if (entry.failed?.()) {
        finish(id, entry, "failed"); continue;
      }
      if (performance.now() - entry.at > 10_000) {
        finish(id, entry, "timeout"); continue;
      }
      if (entry.settling || !entry.check()) continue;
      entry.settling = true;
      afterPaint(() => {
        if (pending.get(id) !== entry) return;
        if (!entry.check()) { entry.settling = false; return; }
        finish(id, entry, "ready");
      });
    }
  }
  const observer = new MutationObserver(inspect);
  observer.observe(document.getElementById("root") ?? document.body, {
    childList: true, subtree: true, attributes: true,
    attributeFilter: ["aria-busy", "aria-selected", "data-active-workspace", "data-chat-state", "data-inspector-finding-key", "data-inspector-detail-state", "data-inspector-remediation-state"],
  });
  function inspectorTransition(target: Element | null) {
    const button = target?.closest<HTMLButtonElement>("button");
    if (button?.disabled) return;
    const row = target?.closest<HTMLElement>('[data-review-drag-key],[role="row"][data-key]');
    const expectedKey = button?.dataset.inspectorTargetKey ?? (row?.closest(".review-workspace") ? row.dataset.reviewDragKey ?? row.dataset.key : undefined);
    if (!expectedKey) return;
    const pane = () => [...document.querySelectorAll<HTMLElement>("aside[data-inspector-finding-key]")].find(node => node.dataset.inspectorFindingKey === expectedKey && node.getBoundingClientRect().height > 0);
    // Action 7 measures the matching shell, action 8 the settled matching data.
    // Keys stay in DOM only; numeric diagnostics never log finding identities.
    begin(7, "inspectorReadyMs", () => Boolean(pane()));
    begin(8, "inspectorDetailsReadyMs", () => {
      const node = pane();
      return node?.dataset.inspectorDetailState === "ready" && ["ready", "disabled"].includes(node.dataset.inspectorRemediationState ?? "");
    }, () => {
      const node = pane();
      return node?.dataset.inspectorDetailState === "error" || ["error", "unavailable"].includes(node?.dataset.inspectorRemediationState ?? "");
    });
  }
  const onClick = (event: MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    const button = target?.closest("button");
    const text = button?.textContent?.trim() ?? "";
    if (button?.closest("aside") && text.startsWith("Review")) begin(1, "workspaceReadyMs", () => Boolean(document.querySelector('.review-result-scroller,[aria-label="Review board"]')));
    else if (button?.closest("aside") && text === "Home") begin(2, "workspaceReadyMs", () => [...document.querySelectorAll("h1")].some(node => node.textContent === "Overview"));
    else if (button?.closest("aside") && text === "Chat") begin(3, "workspaceReadyMs", chatReady, () => chatState()?.dataset.chatState === "error", chatReadyState);
    else if (button?.closest("aside") && text === "Repositories") begin(4, "workspaceReadyMs", () => [...document.querySelectorAll("h1")].some(node => node.textContent === "Repositories"));
    else if (text === "Board") begin(5, "presentationReadyMs", () => Boolean(visibleNode('[aria-label="Review board"]')));
    else if (text === "Table") begin(6, "presentationReadyMs", () => Boolean(visibleNode('.review-result-scroller [role="grid"]')));
    else inspectorTransition(target);
  };
  const onKeyDown = (event: KeyboardEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    if (["Enter", " "].includes(event.key) && !target?.closest("input,textarea,select,[contenteditable=true]")) inspectorTransition(target);
  };
  document.addEventListener("click", onClick, { capture: true, passive: true });
  document.addEventListener("keydown", onKeyDown, { capture: true, passive: true });

  const readyObserver = new MutationObserver(() => {
    if (!document.querySelector("aside button")) return;
    readyObserver.disconnect();
    afterPaint(() => recordPerformanceSample({ firstAppPaintMs: performance.now() - started }));
  });
  readyObserver.observe(document.getElementById("root") ?? document.body, { childList: true, subtree: true });
  let frames: number[] = [];
  let interactions: number[] = [];
  let longTasks = 0;
  let longTaskMax = 0;
  const supported = typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes.includes("longtask");
  const longTaskObserver = supported ? new PerformanceObserver(entries => {
    for (const entry of entries.getEntries()) { longTasks++; longTaskMax = Math.max(longTaskMax, entry.duration); }
  }) : null;
  longTaskObserver?.observe({ type: "longtask", buffered: true });
  const percentile = (samples: number[], p: number) => samples.length ? samples[Math.min(samples.length - 1, Math.ceil(samples.length * p) - 1)] : 0;
  const interaction = (event: Event) => {
    const received = performance.now();
    const at = event.timeStamp < performance.timeOrigin ? event.timeStamp : event.timeStamp - performance.timeOrigin;
    afterPaint(() => { if (interactions.length < 1000) interactions.push(performance.now() - (at > 0 && received - at < 1000 ? Math.min(received, at) : received)); });
  };
  document.addEventListener("pointerdown", interaction, { passive: true });
  document.addEventListener("click", interaction, { passive: true, capture: true });
  document.addEventListener("keydown", interaction, { passive: true });
  function frame(now: number) {
    if (pending.size) inspect();
    if (frames.length < 1000) frames.push(now - last);
    last = now; raf(frame);
  }
  const onVisibility = () => { last = performance.now(); };
  document.addEventListener("visibilitychange", onVisibility);
  raf(frame);
  const interval = window.setInterval(() => {
    frames.sort((a, b) => a - b); interactions.sort((a, b) => a - b);
    const heap = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize;
    const sample = {
      reactProfiling: import.meta.env.VITE_CVEC_PERF_PROFILER === "on", ...takePerformanceCounters(), ...companionStore.getPerformanceMetrics(),
      visible: document.visibilityState === "visible", elapsedMs: performance.now() - started,
      frames: frames.length, frameP50Ms: percentile(frames, .5), frameP95Ms: percentile(frames, .95), frameP99Ms: percentile(frames, .99), frameMaxMs: frames.at(-1) ?? 0,
      longTaskSupported: supported, longTasks, longTaskMaxMs: longTaskMax,
      interactionCount: interactions.length, interactionP95Ms: percentile(interactions, .95), interactionMaxMs: interactions.at(-1) ?? 0,
      reviewRows: document.querySelectorAll(".review-result-scroller [role=row][data-key]").length,
      dependencyRows: document.querySelectorAll('[aria-label="Repository dependencies"] [role=row][data-key]').length,
      messageNodes: document.querySelectorAll("[data-message-id]").length, domNodes: document.getElementsByTagName("*").length,
      ...(heap ? { heapBytes: heap } : {}),
    };
    document.documentElement.dataset.performanceSample = JSON.stringify(sample);
    recordPerformanceSample(sample);
    frames = []; interactions = []; longTasks = 0; longTaskMax = 0;
  }, 2000);
  activeCleanup = () => {
    if (stopped) return;
    stopped = true;
    for (const handle of scheduled) cancelAnimationFrame(handle);
    scheduled.clear(); pending.clear();
    window.clearInterval(interval);
    disposeCaptureMetadata?.();
    observer.disconnect(); readyObserver.disconnect(); longTaskObserver?.disconnect();
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("keydown", onKeyDown, true);
    document.removeEventListener("pointerdown", interaction);
    document.removeEventListener("click", interaction, true);
    document.removeEventListener("keydown", interaction);
    document.removeEventListener("visibilitychange", onVisibility);
    disablePerformanceCounters();
    activeCleanup = undefined;
  };
  return activeCleanup;
}
