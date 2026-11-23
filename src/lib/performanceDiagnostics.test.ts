import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock("@tauri-apps/api/core", () => native);
vi.mock("./companionState", () => ({ companionStore: { getPerformanceMetrics: () => ({ historyCacheEntries: 0, historyCacheBytes: 0, deltaBufferRoutes: 0 }) } }));
import { installPerformanceDiagnostics } from "./performanceDiagnostics";
import { performanceCountersEnabled } from "./performanceCounters";

let cleanup: (() => void) | undefined;
const samples = () => native.invoke.mock.calls.filter(([command]) => command === "record_ui_timings").map(([, args]) => args.sample as Record<string, number | boolean>);
const byAction = (id: number) => samples().filter(sample => sample.uiAction === id);
const tick = (ms = 40) => vi.advanceTimersByTimeAsync(ms);

beforeEach(() => {
  vi.useFakeTimers();
  native.invoke.mockReset().mockImplementation(async (command: string) => command === "get_build_identity" ? { trace: true } : undefined);
  native.isTauri.mockReturnValue(true);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, width: 100, height: 100, top: 0, left: 0, right: 100, bottom: 100, toJSON: () => ({}) });
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => window.setTimeout(() => callback(performance.now()), 16));
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(handle => window.clearTimeout(handle));
  document.body.innerHTML = '<div id="root"><div class="review-workspace"><div role="row" data-key="1:next">Next row</div></div><aside data-inspector-finding-key="1:old" data-inspector-detail-state="ready" data-inspector-remediation-state="ready"><button data-inspector-target-key="1:next">Next finding</button></aside></div>';
});

afterEach(() => { cleanup?.(); cleanup = undefined; document.body.innerHTML = ""; vi.useRealTimers(); vi.restoreAllMocks(); });

describe("opt-in performance diagnostics", () => {
  it("waits for the requested Inspector identity and measures details separately", async () => {
    cleanup = await installPerformanceDiagnostics();
    document.querySelector<HTMLButtonElement>("button")!.click();
    await tick();
    expect(byAction(7)).toHaveLength(0); expect(byAction(8)).toHaveLength(0);
    const pane = document.querySelector<HTMLElement>("aside")!;
    pane.dataset.inspectorFindingKey = "1:next"; pane.dataset.inspectorDetailState = "loading"; pane.dataset.inspectorRemediationState = "loading";
    await tick();
    expect(byAction(7)).toHaveLength(1); expect(byAction(8)).toHaveLength(0);
    pane.dataset.inspectorDetailState = "ready"; pane.dataset.inspectorRemediationState = "ready";
    await tick();
    expect(byAction(8)).toHaveLength(1);
    expect(Number(byAction(8)[0].inspectorDetailsReadyMs)).toBeGreaterThan(Number(byAction(7)[0].inspectorReadyMs));
    expect(JSON.stringify(samples())).not.toContain("1:next");
  });

  it("records failed details instead of a successful ready time", async () => {
    cleanup = await installPerformanceDiagnostics(); document.querySelector<HTMLButtonElement>("button")!.click();
    const pane = document.querySelector<HTMLElement>("aside")!;
    pane.dataset.inspectorFindingKey = "1:next"; pane.dataset.inspectorDetailState = "error";
    await tick();
    expect(byAction(8)).toMatchObject([{ uiAction: 8, uiTransitionFailed: true }]);
  });

  it("discards a superseded target while its paint callbacks are pending", async () => {
    cleanup = await installPerformanceDiagnostics();
    const button = document.querySelector<HTMLButtonElement>("button")!, pane = document.querySelector<HTMLElement>("aside")!;
    button.click(); pane.dataset.inspectorFindingKey = "1:next"; await tick(16);
    button.dataset.inspectorTargetKey = "1:third"; button.click();
    await tick(); expect(byAction(7)).toHaveLength(0);
    pane.dataset.inspectorFindingKey = "1:third"; await tick();
    expect(byAction(7)).toHaveLength(1); expect(byAction(8)).toHaveLength(1);
  });

  it("times out an Inspector that never matches the requested finding", async () => {
    cleanup = await installPerformanceDiagnostics(); document.querySelector<HTMLButtonElement>("button")!.click();
    await tick(10_050);
    expect(byAction(7)).toMatchObject([{ uiAction: 7, uiTransitionTimeout: true }]);
    expect(byAction(8)).toMatchObject([{ uiAction: 8, uiTransitionTimeout: true }]);
  });

  it("installs once and removes observers, listeners, timers and frame callbacks", async () => {
    cleanup = await installPerformanceDiagnostics(); expect(await installPerformanceDiagnostics()).toBe(cleanup);
    expect(native.invoke.mock.calls.filter(([command]) => command === "get_build_identity")).toHaveLength(1);
    document.querySelector<HTMLButtonElement>("button")!.click(); cleanup!(); cleanup = undefined;
    expect(performanceCountersEnabled()).toBe(false); expect(vi.getTimerCount()).toBe(0);
    const count = native.invoke.mock.calls.length;
    document.querySelector<HTMLButtonElement>("button")!.click(); await tick(6000);
    expect(native.invoke).toHaveBeenCalledTimes(count);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("does not collect diagnostics outside an opted-in native run", async () => {
    native.invoke.mockResolvedValueOnce({ trace: false });
    expect(await installPerformanceDiagnostics()).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0); expect(performanceCountersEnabled()).toBe(false);
  });

  it("keeps a new diagnostics session alive if a stale disposer runs again",async()=>{
    const old=await installPerformanceDiagnostics();old!();
    cleanup=await installPerformanceDiagnostics();old!();
    expect(performanceCountersEnabled()).toBe(true);
    document.querySelector<HTMLButtonElement>("button")!.click();await tick(10_050);
    expect(byAction(8)).toMatchObject([{uiAction:8,uiTransitionTimeout:true}]);
  });

  const chatFixture = (state = "loading", content = "") => {
    document.body.innerHTML = `<div id="root"><aside><button>Chat</button></aside><section data-active-workspace="chat"><div data-chat-workspace><div data-chat-state="${state}">${content}</div></div></section></div>`;
    return document.querySelector<HTMLElement>('[data-chat-state]')!;
  };
  const openChat = () => document.querySelector<HTMLButtonElement>('button')!.click();
  const timeline = '<div role="log" aria-label="Repository chat messages">Local fixture history</div>';

  it("ignores unrelated textareas and the obsolete label, then waits for the real scoped timeline", async () => {
    const state = chatFixture("ready", '<textarea disabled>private fixture</textarea><div aria-label="Conversation timeline"></div>');
    cleanup = await installPerformanceDiagnostics(); openChat(); await tick();
    expect(byAction(3)).toHaveLength(0);
    state.insertAdjacentHTML("beforeend", timeline); await tick();
    expect(byAction(3)).toMatchObject([{ uiAction: 3, uiRoute: 3, uiReadyState: 1 }]);
    expect(JSON.stringify(samples())).not.toContain("private fixture");
  });

  it("requires the active chat route and rechecks a delayed load after two frames", async () => {
    const state = chatFixture("loading", timeline);
    const route = document.querySelector<HTMLElement>('[data-active-workspace]')!;
    route.dataset.activeWorkspace = "repositories";
    cleanup = await installPerformanceDiagnostics(); openChat(); await tick();
    state.dataset.chatState = "ready"; await tick();
    expect(byAction(3)).toHaveLength(0);
    route.dataset.activeWorkspace = "chat"; await tick(16);
    state.dataset.chatState = "loading"; await tick();
    expect(byAction(3)).toHaveLength(0);
    state.dataset.chatState = "ready"; await tick();
    expect(byAction(3)).toHaveLength(1);
  });

  it.each([
    ["empty", "", 2],
    ["read-only", timeline + '<textarea disabled></textarea>', 3],
    ["read-only", "contained", 3],
  ])("accepts the explicit %s state without requiring a send-enabled composer", async (name, content, code) => {
    const state = chatFixture(name, content === "contained" ? "Connect a provider" : content);
    if (content === "contained") state.setAttribute("data-chat-contained-state", "");
    cleanup = await installPerformanceDiagnostics(); openChat(); await tick();
    expect(byAction(3)).toMatchObject([{ uiReadyState: code }]);
  });

  it("does not count contained errors or an unresolved loading timeout as ready", async () => {
    const state = chatFixture();
    cleanup = await installPerformanceDiagnostics(); openChat();
    state.dataset.chatState = "error"; await tick();
    expect(byAction(3)).toMatchObject([{ uiTransitionFailed: true, uiReadyState: 4 }]);
    expect(byAction(3)[0]).not.toHaveProperty("workspaceReadyMs");
    state.dataset.chatState = "loading"; openChat(); await tick(10_050);
    expect(byAction(3)[1]).toMatchObject({ uiTransitionTimeout: true });
    expect(byAction(3)[1]).not.toHaveProperty("workspaceReadyMs");
  });

  it("uses a single clock anchor, distinct sequences and exact duration boundaries for all outcomes", async () => {
    const state = chatFixture("empty");
    cleanup = await installPerformanceDiagnostics();
    await tick(100); openChat(); await tick();
    state.dataset.chatState = "error"; openChat(); await tick();
    state.dataset.chatState = "loading"; openChat(); await tick(10_050);
    const anchor = samples().filter(sample => sample.sessionTimeOriginMs !== undefined);
    expect(anchor).toHaveLength(1);
    expect(Number(anchor[0].sessionTimeOriginMs)).toBeGreaterThan(0);
    const transitions = byAction(3);
    expect(transitions.map(t => t.uiTransitionId)).toEqual([1, 2, 3]);
    for (const t of transitions) {
      expect(t.uiRoute).toBe(3);
      expect(Number(t.uiStartedElapsedMs)).toBeGreaterThanOrEqual(100);
      expect(Number(t.uiSettledElapsedMs)).toBeGreaterThanOrEqual(Number(t.uiStartedElapsedMs));
      if (t.workspaceReadyMs !== undefined) expect(Number(t.uiSettledElapsedMs) - Number(t.uiStartedElapsedMs)).toBe(t.workspaceReadyMs);
      expect(Object.values(t).every(v => typeof v === "number" || typeof v === "boolean")).toBe(true);
    }
  });
});
