import type { ProviderConnectionSnapshot, ProviderDriverKind } from "@/lib/types";

export function fixConnection(driver: ProviderDriverKind, id: string): ProviderConnectionSnapshot {
  return {
    schemaVersion: 1,
    connection: { schemaVersion: 1, id, driver, displayName: driver === "codex" ? "Codex" : driver === "grok" ? "Grok" : "OpenCode", accentColor: null, enabled: true, builtIn: true, config: driver === "codex" ? { kind: "codex" } : driver === "grok" ? { kind: "grok" } : { kind: "open_code", mode: "managed_local" }, hasServerPassword: false, createdAt: "now", updatedAt: "now" },
    health: "ready", installedVersion: driver === "open_code" ? "2.0.32" : "1.0.0", authKind: "none", accountLabel: null,
    capabilities: { createThread: true, resumeThread: true, interrupt: true, orderedStreaming: true, readOnlyPolicy: true, accountStatus: false, rateLimits: false, tokenUsage: true, reasoningEffort: true, attachments: false },
    upstreamProviders: [], models: [{ id: driver === "grok" ? "grok-model" : "fixture-model", displayName: "Fixture model", description: null, isDefault: true, hidden: false, reasoningEfforts: ["low", "high"], inputModalities: ["text"], contextWindow: null, connectionId: id, driver, upstreamProviderId: null, inferenceBoundary: "external" }], checkedAt: "now", message: null, error: null,
  };
}
export const CODEX_CONNECTION = fixConnection("codex", "codex_default");
