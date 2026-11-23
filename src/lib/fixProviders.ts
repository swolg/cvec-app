import type { ProviderConnectionSnapshot } from "./types";

/** Configuration eligibility complements runtime negotiation; health alone
 * cannot promise that a provider supports CVEC's fix permission boundary. */
export function fixProviderUnavailable(snapshot: ProviderConnectionSnapshot | undefined): string | null {
  if (!snapshot) return "Choose an AI connection and model before starting a fix.";
  if (!snapshot.connection.enabled) return "Enable this AI connection before starting a fix.";
  const { connection } = snapshot;
  switch (connection.driver) {
    case "codex":
      return connection.id === "codex_default" ? null : "Additional Codex runtimes are not initialized. Select the built-in Codex connection.";
    case "grok": return null;
    case "open_code": {
      if (connection.config.kind !== "open_code" || (connection.config.mode ?? "managed_local") !== "managed_local") return "Fixes require managed local OpenCode. Existing servers cannot enforce the isolated tool policy.";
      const major = snapshot.installedVersion?.match(/(\d+)\.\d+\.\d+/)?.[1];
      return major && Number(major) < 2 ? "OpenCode fixes require V2. Update the runtime first." : null;
    }
    default: return "This provider does not support fix workspaces yet.";
  }
}
