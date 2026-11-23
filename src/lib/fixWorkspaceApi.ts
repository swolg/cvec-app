import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";
import type { FixArchivePage, FixSnapshot, FixWorkspace, FixStartRequest } from "./types";

const short = z.string().max(4096);
const digest = z.string().regex(/^[a-f0-9]{64}$/);
const finding = z.strictObject({ fingerprint: short, advisoryId: short, packageName: short, version: short.nullable(), packageType: short.nullable(), packageUrl: short.nullable(), fixVersion: short.nullable(), severity: z.enum(["MEDIUM", "HIGH", "CRITICAL"]) });
const check = z.strictObject({ action: short, status: z.enum(["passed", "failed", "unavailable", "interrupted"]), exitCode: z.number().int().nullable(), output: z.string().max(128 * 1024), digest, stale: z.boolean(), finishedAt: short });
const rescan = z.strictObject({ findings: z.array(finding).max(500), findingCount: z.number().int().nonnegative(), notDetected: z.number().int().nonnegative().nullable(), targetDetected: z.boolean().nullish(), digest, stale: z.boolean(), scannedAt: short, scannerVersions: short });
const provider = z.strictObject({ connectionId: z.string().regex(/^[A-Za-z][A-Za-z0-9_-]{0,63}$/), driver: z.enum(["codex", "grok", "open_code"]), displayName: short, model: z.string().max(256).nullable() });
const workspace = z.strictObject({
  schemaVersion: z.union([z.literal(1), z.literal(2), z.literal(3)]), archivedAt: z.string().min(1).max(64).nullable().optional(), provider: provider.nullable().optional(), revision: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER).default(0), id: z.string().regex(/^[a-f0-9]{24}$/), repositoryId: z.number().int().positive(), sourcePath: short, worktreePath: short, branch: short,
  baseRevision: z.string().regex(/^[a-f0-9]{40}$/), baselineScanId: z.number().int().positive().nullable(), baselineMatched: z.boolean(), baselineFindings: z.array(finding),
  findingFingerprint: short.nullable(), providerThreadId: short.nullable(), turnId: short.nullable(), status: z.enum(["ready", "working", "testing", "scanning", "interrupted", "failed"]),
  activity: short, messages: z.array(z.strictObject({ role: z.enum(["user", "assistant"]), provider: provider.nullable().optional(), text: z.string().max(32_000) })).max(100), checks: z.array(check).max(20), rescan: rescan.nullable(), createdAt: short, updatedAt: short,
}).refine((value) => !value.archivedAt || (value.schemaVersion === 3 && !["working", "testing", "scanning"].includes(value.status)), { message: "Archived workspaces require lifecycle-aware records and must be idle" });
const snapshot = z.strictObject({ workspace, diff: z.strictObject({ files: z.array(z.strictObject({ path: short, patch: z.string().max(256 * 1024), truncated: z.boolean() })).max(100), omittedFiles: z.number().int().nonnegative(), truncated: z.boolean(), digest }).nullable(), sourceError: short.nullable(), verificationWarning: short.nullable().optional(), actions: z.array(short).max(24) });

export function parseFixWorkspace(value: unknown): FixWorkspace { return workspace.parse(value); }
export function parseFixSnapshot(value: unknown): FixSnapshot { return snapshot.parse(value); }
export const fixWorkspaceBusy = (value: FixWorkspace | null) => Boolean(value && ["working", "testing", "scanning"].includes(value.status));

export async function listFixWorkspaces(repositoryId: number): Promise<FixWorkspace[]> {
  return z.array(workspace).max(20).parse(await invoke<unknown>("fix_workspace_list", { repositoryId }));
}
const archivePage = z.strictObject({ entries: z.array(z.strictObject({ id: z.string().regex(/^[a-f0-9]{24}$/), repositoryId: z.number().int().positive(), branch: short, title: z.string().max(480), findingFingerprint: short.optional(), status: z.string().max(64).optional(), archivedAt: z.string().min(1).max(64) })).max(20), nextCursor: z.string().max(90).nullable() });
export async function listArchivedFixWorkspaces(repositoryId: number, cursor: string | null = null, findingFingerprint?: string): Promise<FixArchivePage> {
  return archivePage.parse(await invoke<unknown>("fix_workspace_archived", { repositoryId, cursor, findingFingerprint }));
}
export async function setFixWorkspaceArchived(repositoryId: number, workspaceId: string, archived: boolean): Promise<FixSnapshot> {
  return parseFixSnapshot(await invoke<unknown>("fix_workspace_set_archived", { repositoryId, workspaceId, archived }));
}
export async function createFixWorkspace(repositoryId: number, findingFingerprint: string | null, expectedScanRunId?: number): Promise<FixSnapshot> {
  return parseFixSnapshot(await invoke<unknown>("fix_workspace_create", { repositoryId, findingFingerprint, confirmed: true, expectedScanRunId }));
}
export async function getFixWorkspace(repositoryId: number, workspaceId: string): Promise<FixSnapshot> {
  return parseFixSnapshot(await invoke<unknown>("fix_workspace_snapshot", { repositoryId, workspaceId }));
}
export async function openFixCheckout(repositoryId: number): Promise<void> {
  await invoke<void>("fix_workspace_open_checkout", { repositoryId });
}
export async function openFixWorkspaceDirectory(repositoryId: number, workspaceId: string): Promise<void> {
  await invoke<void>("fix_workspace_open_directory", { repositoryId, workspaceId });
}
export async function startFixWorkspace(repositoryId: number, workspaceId: string, request: Omit<FixStartRequest, "consent">): Promise<void> {
  await invoke("fix_workspace_start", { repositoryId, workspaceId, request: { ...request, consent: true } });
}

export async function stopFixWorkspace(repositoryId: number, workspaceId: string): Promise<void> {
  await invoke("fix_workspace_stop", { repositoryId, workspaceId });
}
export async function checkFixWorkspace(repositoryId: number, workspaceId: string, action: string): Promise<FixSnapshot> {
  return parseFixSnapshot(await invoke<unknown>("fix_workspace_check", { repositoryId, workspaceId, action }));
}
export async function rescanFixWorkspace(repositoryId: number, workspaceId: string): Promise<FixSnapshot> {
  return parseFixSnapshot(await invoke<unknown>("fix_workspace_rescan", { repositoryId, workspaceId }));
}
export async function listenFixWorkspaces(callback: (value: FixWorkspace) => void): Promise<UnlistenFn> {
  return listen<unknown>("cvec://fix-workspace", (event) => callback(parseFixWorkspace(event.payload)));
}
