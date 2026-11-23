import { invoke } from "@tauri-apps/api/core";

export function readQaViewport() {
  const visual = window.visualViewport;
  return {
    epochMs: Date.now(),
    innerWidthCss: window.innerWidth, innerHeightCss: window.innerHeight,
    outerWidthCss: window.outerWidth, outerHeightCss: window.outerHeight,
    screenXCss: window.screenX, screenYCss: window.screenY,
    devicePixelRatio: window.devicePixelRatio,
    visual: visual ? {
      widthCss: visual.width, heightCss: visual.height,
      offsetLeftCss: visual.offsetLeft, offsetTopCss: visual.offsetTop,
      pageLeftCss: visual.pageLeft, pageTopCss: visual.pageTop, scale: visual.scale,
    } : null,
  };
}

/** Sample only; no screenshot, focus, resize, routing, or lifecycle control. */
export function installQaCaptureMetadata(identity: { trace: boolean; bundleIdentifier?: string }) {
  if (!identity.trace || identity.bundleIdentifier !== "dev.cvec.performance-qa") return;
  let stopped = false;
  let pending = false;
  let sequence = 0;
  let timeout: number | undefined;
  let interval: number | undefined;
  const stop = () => {
    stopped = true;
    if (interval !== undefined) window.clearInterval(interval);
    if (timeout !== undefined) window.clearTimeout(timeout);
  };
  const collect = async () => {
    if (stopped || pending) return;
    pending = true;
    const id = ++sequence;
    const before = readQaViewport();
    let nativeAvailable = false;
    try {
      const native = await Promise.race([
        invoke<{ sequence: number }>("get_qa_capture_geometry", { sequence: id }),
        new Promise<never>((_, reject) => {
          timeout = window.setTimeout(() => reject(new Error("Geometry sampling timeout")), 1000);
        }),
      ]);
      nativeAvailable = native?.sequence === id;
    } catch { /* One explicit unavailable record, then stop; never retry a fault. */ }
    finally { if (timeout !== undefined) window.clearTimeout(timeout); timeout = undefined; }
    if (stopped) return;
    const sample = { sequence: id, before, after: readQaViewport(), nativeAvailable };
    try { await invoke("record_qa_capture_viewport", { sample }); }
    catch { stop(); }
    if (!nativeAvailable) stop();
    pending = false;
  };
  interval = window.setInterval(() => { void collect(); }, 2000);
  void collect();
  return stop;
}
