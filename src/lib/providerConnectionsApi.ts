import { invoke } from "@tauri-apps/api/core";

import {
  parseProviderConnectionSnapshot,
  parseProviderConnectionSnapshots,
} from "@/lib/contracts";
import type {
  PrivacyConsentStatus,
  ProviderConnectionDraft,
  ProviderConnectionSnapshot,
} from "@/lib/types";

export async function listProviderConnections(): Promise<ProviderConnectionSnapshot[]> {
  return parseProviderConnectionSnapshots(await invoke("provider_connection_list"));
}

export async function validateProviderConnection(draft: ProviderConnectionDraft): Promise<ProviderConnectionSnapshot> {
  return parseProviderConnectionSnapshot(await invoke("provider_connection_validate", { draft }));
}

export async function saveProviderConnection(draft: ProviderConnectionDraft): Promise<ProviderConnectionSnapshot> {
  return parseProviderConnectionSnapshot(await invoke("provider_connection_save", { draft }));
}

export async function refreshProviderConnection(connectionId: string): Promise<ProviderConnectionSnapshot> {
  return parseProviderConnectionSnapshot(await invoke("provider_connection_refresh", { connectionId }));
}

export async function setProviderConnectionEnabled(connectionId: string, enabled: boolean): Promise<ProviderConnectionSnapshot> {
  return parseProviderConnectionSnapshot(await invoke("provider_connection_set_enabled", { connectionId, enabled }));
}

export async function resetProviderConnection(connectionId: string): Promise<ProviderConnectionSnapshot> {
  return parseProviderConnectionSnapshot(await invoke("provider_connection_reset", { connectionId }));
}

export async function removeProviderConnection(connectionId: string, confirmBoundThreads = false): Promise<number> {
  return invoke<number>("provider_connection_remove", { connectionId, confirmBoundThreads });
}

export async function providerConnectionConsent(connectionId: string, modelId: string): Promise<PrivacyConsentStatus> {
  return invoke<PrivacyConsentStatus>("provider_connection_consent", { connectionId, modelId });
}

export async function setProviderConnectionConsent(connectionId: string, modelId: string, accepted: boolean): Promise<PrivacyConsentStatus> {
  return invoke<PrivacyConsentStatus>("provider_connection_set_consent", { connectionId, modelId, accepted });
}
