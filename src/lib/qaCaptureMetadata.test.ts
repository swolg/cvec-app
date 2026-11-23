import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => native);
import { installQaCaptureMetadata, readQaViewport } from "./qaCaptureMetadata";

let dispose: (() => void) | undefined;
const qa = { trace: true, bundleIdentifier: "dev.cvec.performance-qa" };
const records = () => native.invoke.mock.calls.filter(([name]) => name === "record_qa_capture_viewport").map(([, args]) => args.sample);

beforeEach(() => {
  vi.useFakeTimers();
  native.invoke.mockReset().mockImplementation(async (name, args) => name === "get_qa_capture_geometry" ? { sequence: args.sequence } : undefined);
});
afterEach(() => { dispose?.(); dispose = undefined; vi.useRealTimers(); vi.restoreAllMocks(); });

describe("QA capture geometry sampling", () => {
  it("does no sampling in production or without trace", () => {
    expect(installQaCaptureMetadata({ trace: true, bundleIdentifier: "dev.cvec.app" })).toBeUndefined();
    expect(installQaCaptureMetadata({ trace: false, bundleIdentifier: qa.bundleIdentifier })).toBeUndefined();
    expect(native.invoke).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("retains signed screen coordinates, DPR and missing visual viewport without reading content", () => {
    vi.spyOn(window, "screenX", "get").mockReturnValue(-1900);
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(2);
    document.body.innerHTML = "<p>private repository content</p>";
    const sample = readQaViewport();
    expect(sample.screenXCss).toBe(-1900);
    expect(sample.devicePixelRatio).toBe(2);
    expect(sample.visual).toBeNull();
    expect(JSON.stringify(sample)).not.toContain("private");
  });

  it("records bracketing viewport times and distinct native sequence numbers", async () => {
    dispose = installQaCaptureMetadata(qa);
    await vi.advanceTimersByTimeAsync(2000);
    expect(records().map(r => r.sequence)).toEqual([1, 2]);
    expect(records().every(r => r.nativeAvailable && r.after.epochMs >= r.before.epochMs)).toBe(true);
    dispose!(); expect(vi.getTimerCount()).toBe(0);
  });

  it("bounds a stalled native request, skips overlapping polls and stops after one unavailable record", async () => {
    native.invoke.mockImplementation((name) => name === "get_qa_capture_geometry" ? new Promise(() => undefined) : Promise.resolve());
    dispose = installQaCaptureMetadata(qa);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(native.invoke.mock.calls.filter(([name]) => name === "get_qa_capture_geometry")).toHaveLength(1);
    expect(records()).toHaveLength(1); expect(records()[0].nativeAvailable).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("discards late replies after disposal and never records a newer session from an old disposer", async () => {
    let resolve!: (value: { sequence: number }) => void;
    native.invoke.mockImplementation((name) => name === "get_qa_capture_geometry" ? new Promise(r => { resolve = r; }) : Promise.resolve());
    const old = installQaCaptureMetadata(qa)!; old(); old();
    resolve({ sequence: 1 }); await vi.advanceTimersByTimeAsync(100);
    expect(records()).toHaveLength(0); expect(vi.getTimerCount()).toBe(0);
  });

  it("stops on sequence mismatch or recording rejection without retry", async () => {
    native.invoke.mockResolvedValue({ sequence: 77 });
    dispose = installQaCaptureMetadata(qa); await vi.advanceTimersByTimeAsync(6000);
    expect(records()).toHaveLength(1); expect(records()[0].nativeAvailable).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    dispose!();
    native.invoke.mockClear().mockImplementation(async (name, args) => {
      if (name === "record_qa_capture_viewport") throw new Error("unavailable");
      return { sequence: args.sequence };
    });
    dispose = installQaCaptureMetadata(qa); await vi.advanceTimersByTimeAsync(6000);
    expect(records()).toHaveLength(1); expect(vi.getTimerCount()).toBe(0);
  });
});
