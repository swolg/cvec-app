import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  parseCompanionAccountStatus,
  parseCompanionCachedThreadSnapshot,
  parseCompanionConversationIndex,
  parseCompanionEvent,
  parseCompanionLoginStart,
  parseCompanionMentions,
  parseCompanionModels,
  parseCompanionRuntimeStatus,
  parseCompanionSourceDetail,
  parseCompanionThread,
  parseCompanionThreads,
  parseCompanionThreadSnapshot,
  parseCompanionTurnStart,
  parseCompanionUsage,
  parsePrivacyConsent,
  parseRepositoryChatContext,
  parseRepositoryChatScanStatus,
} from "./contracts";
import type {
  CompanionAccountStatus,
  CompanionCachedThreadSnapshot,
  CompanionConversationIndex,
  CompanionEventEnvelope,
  CompanionLoginStart,
  CompanionMention,
  CompanionModel,
  CompanionRuntimeStatus,
  CompanionSourceDetail,
  CompanionThread,
  CompanionThreadSnapshot,
  CompanionTurnRequest,
  CompanionTurnStart,
  CompanionUsageStatus,
  ContextAttachmentKind,
  ContextPreviewRequest,
  PrivacyConsentStatus,
  RepositoryChatContext,
  RepositoryChatScanStatus,
} from "./types";
import {
  listProviderConnections,
  providerConnectionConsent,
  refreshProviderConnection,
  setProviderConnectionConsent,
} from "./providerConnectionsApi";

export const COMPANION_EVENT_NAME = "cvec://companion";

export async function companionReadDraft(repositoryId: number, scope: string): Promise<string | null> {
  const text = await invoke<unknown>("companion_read_draft", { repositoryId, scope });
  if (text !== null && (typeof text !== "string" || text.length > 65_536)) throw new Error("Invalid local draft response");
  return text;
}

export async function companionWriteDraft(repositoryId: number, scope: string, text: string): Promise<void> {
  await invoke<void>("companion_write_draft", { repositoryId, scope, text });
}

export async function companionRuntimeStatus(): Promise<CompanionRuntimeStatus> {
  return parseCompanionRuntimeStatus(await invoke<unknown>("companion_runtime_status"));
}

export async function companionStartRuntime(): Promise<CompanionRuntimeStatus> {
  return parseCompanionRuntimeStatus(await invoke<unknown>("companion_start_runtime"));
}

export async function companionStopRuntime(): Promise<void> {
  return invoke<void>("companion_stop_runtime");
}

export async function companionAccountStatus(): Promise<CompanionAccountStatus> {
  return parseCompanionAccountStatus(await invoke<unknown>("companion_account_status"));
}

export async function companionLoginChatgpt(deviceCode = false): Promise<CompanionLoginStart> {
  return parseCompanionLoginStart(await invoke<unknown>("companion_login_chatgpt", { deviceCode }));
}

export async function companionLoginApiKey(apiKey: string): Promise<CompanionLoginStart> {
  return parseCompanionLoginStart(await invoke<unknown>("companion_login_api_key", { apiKey }));
}

export async function companionCancelLogin(loginId: string): Promise<void> {
  return invoke<void>("companion_cancel_login", { loginId });
}

export async function companionLogout(confirmSharedAccount: boolean): Promise<void> {
  return invoke<void>("companion_logout", { confirmSharedAccount });
}

export async function companionListModels(): Promise<CompanionModel[]> {
  return parseCompanionModels(await invoke<unknown>("companion_list_models"));
}

export async function companionUsage(forceRefresh = false): Promise<CompanionUsageStatus> {
  return parseCompanionUsage(await invoke<unknown>("companion_usage", { forceRefresh }));
}

export async function companionListThreads(repositoryId: number, includeArchived = false): Promise<CompanionThread[]> {
  return parseCompanionThreads(await invoke<unknown>("companion_list_threads", { repositoryId, includeArchived }));
}

export async function companionConversationIndex(): Promise<CompanionConversationIndex> {
  return parseCompanionConversationIndex(await invoke<unknown>("companion_conversation_index"));
}

export async function companionHistoryPage(repositoryId:number,threadId:number,cursor:string|null):Promise<CompanionCachedThreadSnapshot>{
  return parseCompanionCachedThreadSnapshot(await invoke<unknown>("companion_history_page",{repositoryId,threadId,cursor}));
}

export async function companionCachedThread(repositoryId: number, threadId: number): Promise<CompanionCachedThreadSnapshot> {
  return parseCompanionCachedThreadSnapshot(await invoke<unknown>("companion_cached_thread", { repositoryId, threadId }));
}

export async function companionCreateThread(repositoryId: number, connectionId: string, model: string | null, initialTitle: string | null): Promise<CompanionThread> {
  return parseCompanionThread(await invoke<unknown>("companion_create_thread", { repositoryId, connectionId, model, initialTitle }));
}

export async function companionResumeThread(repositoryId: number, threadId: number): Promise<CompanionThreadSnapshot> {
  return parseCompanionThreadSnapshot(await invoke<unknown>("companion_resume_thread", { repositoryId, threadId }));
}

export async function companionRenameThread(repositoryId: number, threadId: number, title: string): Promise<CompanionThread> {
  return parseCompanionThread(await invoke<unknown>("companion_rename_thread", { repositoryId, threadId, title }));
}

export async function companionSetThreadArchived(repositoryId: number, threadId: number, archived: boolean): Promise<CompanionThread> {
  return parseCompanionThread(await invoke<unknown>("companion_set_thread_archived", { repositoryId, threadId, archived }));
}

export async function companionPrivacyConsent(): Promise<PrivacyConsentStatus> {
  return parsePrivacyConsent(await invoke<unknown>("companion_privacy_consent"));
}

export async function companionSetPrivacyConsent(accepted: boolean): Promise<PrivacyConsentStatus> {
  return parsePrivacyConsent(await invoke<unknown>("companion_set_privacy_consent", { accepted }));
}

export async function companionPreviewContext(request: ContextPreviewRequest): Promise<RepositoryChatContext> {
  return parseRepositoryChatContext(await invoke<unknown>("companion_preview_context", { request }));
}

export async function companionScanStatus(repositoryId: number): Promise<RepositoryChatScanStatus> {
  return parseRepositoryChatScanStatus(await invoke<unknown>("companion_scan_status", { repositoryId }));
}

export async function companionSearchMentions(repositoryId: number, kind: ContextAttachmentKind, query: string, limit = 30): Promise<CompanionMention[]> {
  return parseCompanionMentions(await invoke<unknown>("companion_search_mentions", { repositoryId, kind, query, limit }));
}

export async function companionSourceDetail(repositoryId: number, sourceId: string): Promise<CompanionSourceDetail> {
  return parseCompanionSourceDetail(await invoke<unknown>("companion_source_detail", { repositoryId, sourceId }));
}

export async function companionStartTurn(request: CompanionTurnRequest): Promise<CompanionTurnStart> {
  return parseCompanionTurnStart(await invoke<unknown>("companion_start_turn", { request }));
}

export async function companionInterruptTurn(repositoryId: number, threadId: number, turnId: string): Promise<void> {
  return invoke<void>("companion_interrupt_turn", { repositoryId, threadId, turnId });
}

export async function listenCompanionEvents(listener: (event: CompanionEventEnvelope) => void): Promise<UnlistenFn> {
  return listen<unknown>(COMPANION_EVENT_NAME, (event) => listener(parseCompanionEvent(event.payload)));
}

export const companionClient = {
  readDraft: companionReadDraft,
  writeDraft: companionWriteDraft,
  runtimeStatus: companionRuntimeStatus,
  startRuntime: companionStartRuntime,
  stopRuntime: companionStopRuntime,
  accountStatus: companionAccountStatus,
  loginChatgpt: companionLoginChatgpt,
  loginApiKey: companionLoginApiKey,
  cancelLogin: companionCancelLogin,
  logout: companionLogout,
  listModels: companionListModels,
  usage: companionUsage,
  listConnections: listProviderConnections,
  refreshConnection: refreshProviderConnection,
  connectionConsent: providerConnectionConsent,
  setConnectionConsent: setProviderConnectionConsent,
  listThreads: companionListThreads,
  conversationIndex: companionConversationIndex,
  cachedThread: companionCachedThread,
  historyPage: companionHistoryPage,
  createThread: companionCreateThread,
  resumeThread: companionResumeThread,
  renameThread: companionRenameThread,
  setThreadArchived: companionSetThreadArchived,
  privacyConsent: companionPrivacyConsent,
  setPrivacyConsent: companionSetPrivacyConsent,
  previewContext: companionPreviewContext,
  searchMentions: companionSearchMentions,
  sourceDetail: companionSourceDetail,
  startTurn: companionStartTurn,
  interruptTurn: companionInterruptTurn,
  listen: listenCompanionEvents,
};

export type CompanionClient = Omit<typeof companionClient,"historyPage"> & Partial<Pick<typeof companionClient,"historyPage">>;
