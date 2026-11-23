import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ArrowClockwise, GitBranch, Stop, Wrench } from "@phosphor-icons/react";
import { AppIcon } from "@/components/ui/app-icon";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/feedback";
import { Select, Tabs } from "@/components/ui/form-controls";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/overlays";
import { checkFixWorkspace, createFixWorkspace, fixWorkspaceBusy, getFixWorkspace, listFixWorkspaces, listenFixWorkspaces, openFixCheckout, openFixWorkspaceDirectory, rescanFixWorkspace, setFixWorkspaceArchived, startFixWorkspace, stopFixWorkspace } from "@/lib/fixWorkspaceApi";
import { fixProviderUnavailable } from "@/lib/fixProviders";
import { incompleteInventoryReason } from "@/lib/format";
import { ChatComposer } from "./ChatComposer";
import { ChatTimeline } from "./ChatTimeline";
import { FixDiffPreview } from "./FixDiffPreview";
import { FixWorkspaceArchive } from "./FixWorkspaceArchive";
import { fixEvidenceFreshness, fixEvidenceSummary, fixStatusLabel, fixWorkspaceLabel, isDependencyPreparation } from "./fixWorkspacePresentation";
import { fixDraftKey, readFixDraft, loadFixDraft, saveFixDraft, retryFixDraft, useFixDraftState, readFixSelection, saveFixSelection, readFixWorkspaceView, saveFixWorkspaceView, type FixWorkspaceView } from "./fixWorkspaceSession";
import { CopyButton } from "./CopyButton";
import { DraftStorageNotice } from "./DraftStorageNotice";
import type { CompanionTimelineItem, FindingChatRequest, FixSnapshot, FixWorkspace, ProviderConnectionSnapshot, RepoSummary } from "@/lib/types";

interface Props {
  repository: RepoSummary;
  findingFingerprint?: string;
  findingRequest?: FindingChatRequest;
  onOpenFinding?: (fingerprint: string) => void;
  onOpenRepository?: () => void;
  onChooseLocalCheckout?: () => void;
  onOpenScannerSettings?: () => void;
  scannerUnavailable?: string | null;
  model: string | null;
  modelLabel: string;
  effort: string | null;
  providerReady: boolean;
  connectionId: string;
  connections?: ProviderConnectionSnapshot[];
  selectionLoading?: boolean;
  onSelectModel?: (connectionId: string, modelId: string) => void;
  onEffortChange?: (effort: string) => void;
  onOpenSettings?: () => void;
}

export function FixWorkspacePanel({ repository, findingFingerprint: standaloneFinding, findingRequest, onOpenFinding, onOpenRepository, onChooseLocalCheckout, onOpenScannerSettings, scannerUnavailable, model, modelLabel, effort, providerReady, connectionId, connections = [], selectionLoading = false, onSelectModel = () => {}, onEffortChange = () => {}, onOpenSettings }: Props) {
  const findingFingerprint = findingRequest?.fingerprint ?? standaloneFinding;
  const evidenceScanId = findingRequest?.scanRunId ?? undefined;
  const [workspaces, setWorkspaces] = useState<FixWorkspace[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<FixSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [eventsReady, setEventsReady] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<{ step: "checks" | "rescan"; details: string }>();
  const [createOpen, setCreateOpen] = useState(false);
  const [sendOpen, setSendOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [archiveConfirm, setArchiveConfirm] = useState(false);
  const [draft, setDraft] = useState(findingFingerprint ? "Fix the selected finding with focused dependency changes. Run relevant tests and rescan the worktree." : "");
  const [action, setAction] = useState("");
  const [panel, setPanel] = useState<FixWorkspaceView>("conversation");
  const [stopPending, setStopPending] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [draftReload, setDraftReload] = useState(0);
  const draftKey = fixDraftKey(repository.id, activeId, findingFingerprint);
  const draftStorage = useFixDraftState(draftKey);
  const panelId = useId();
  const active = useRef<string | null>(null);
  const generation = useRef(0);
  const scope = useRef(0);
  const mounted = useRef(true);
  const latest = useRef(new Map<string, FixWorkspace>());
  const workspace = snapshot?.workspace ?? null;
  const archived = Boolean(workspace?.archivedAt);
  const busy = fixWorkspaceBusy(workspace) || Boolean(pending) || stopPending;
  const diff = snapshot?.diff ?? null;
  const sourceAvailable = Boolean(diff);
  const defaultDraft = findingFingerprint && (!workspace || workspace.findingFingerprint === findingFingerprint) ? "Fix the selected finding with focused dependency changes. Run relevant tests and rescan the worktree." : "";
  const timelineItems = useMemo<CompanionTimelineItem[]>(() => (workspace?.messages ?? []).map((item, index) => ({
    id: `fix:${workspace?.id}:${index}`, kind: item.role === "user" ? "user_message" : "assistant_message",
    status: "completed", text: item.text, activity: null, citations: [], error: null, createdAt: workspace?.createdAt ?? "",
  })), [workspace?.id, workspace?.messages, workspace?.createdAt]);
  const selectedConnection = connections.find((item) => item.connection.id === connectionId);
  const effectiveEffort = selectedConnection?.connection.driver === "codex" ? effort : null;
  const fixModels = selectedConnection?.models.map((model) => selectedConnection.connection.driver === "codex" ? model : { ...model, reasoningEfforts: [] }) ?? [];
  const providerLabel = selectedConnection?.connection.displayName ?? "Selected provider";
  const providerUnavailable = fixProviderUnavailable(selectedConnection);
  const providerLabels = useMemo(() => Object.fromEntries((workspace?.messages ?? []).map((item, index) => [`fix:${workspace?.id}:${index}`, item.provider ? `${item.provider.displayName}${item.provider.model ? ` · ${item.provider.model}` : ""}` : "Codex · historical session"])), [workspace?.id, workspace?.messages]);
  // Approval is scoped to the visible connection/model/configuration, never a
  // later selection or a replaced connection with the same identifier.
  const disclosureScope = JSON.stringify([repository.id, activeId, connectionId, model, effort, selectedConnection?.connection.config, selectedConnection?.connection.updatedAt]);
  useEffect(() => { setSendOpen(false); }, [disclosureScope]);
  const promptBytes = new TextEncoder().encode(draft).length;
  const modelSelected = fixModels.some((option) => option.id === model);
  const canSend = Boolean(draftStorage.loaded && workspace && !archived && draft.trim() && promptBytes <= 32_000 && !busy && !loading && providerReady && modelSelected && !providerUnavailable && !selectionLoading && eventsReady && sourceAvailable && repository.sourceType === "local");

  useEffect(() => {
    let disposed = false;
    void loadFixDraft(draftKey).then((text) => {
      if (!disposed) setDraft(text ?? defaultDraft);
    }).catch(() => undefined);
    return () => { disposed = true; };
  }, [draftKey, defaultDraft, draftReload]);

  function updateDraft(value: string) {
    saveFixDraft(fixDraftKey(repository.id, active.current, findingFingerprint), value);
    setDraft(value);
  }

  const activateWorkspace = useCallback((id: string) => {
    active.current = id;
    for (const [key, value] of latest.current) if (value.archivedAt && key !== id) latest.current.delete(key);
    setActiveId(id);
    setPanel(readFixWorkspaceView(repository.id, id));
  }, [repository.id]);

  const remember = useCallback((value: FixWorkspace) => {
    if (value.repositoryId !== repository.id) throw new Error("The workspace response belongs to another repository.");
    const previous = latest.current.get(value.id);
    if (previous && previous.revision > value.revision) return false;
    if (value.archivedAt && active.current !== value.id) latest.current.delete(value.id);
    else latest.current.set(value.id, value);
    setWorkspaces([...latest.current.values()].filter((item) => !item.archivedAt).sort((a, b) => b.createdAt.localeCompare(a.createdAt)));
    return true;
  }, [repository.id]);

  const acceptSnapshot = useCallback((value: FixSnapshot) => {
    if (active.current !== value.workspace.id) return;
    if (remember(value.workspace)) {
      setSnapshot(value);
      return;
    }
    // Events can overtake the first source read. Retain their newer record
    // while installing the initial source view, so messages and Stop appear.
    // Once a view exists, an older response must not replace its evidence.
    const current = latest.current.get(value.workspace.id)!;
    setSnapshot((previous) => previous?.workspace.id === value.workspace.id ? previous : { ...value, workspace: current });
  }, [remember]);

  const refresh = useCallback(async (id: string) => {
    const request = ++generation.current;
    setLoading(true);
    try {
      const value = await getFixWorkspace(repository.id, id);
      if (mounted.current && request === generation.current && active.current === id) { acceptSnapshot(value); setError(null); }
    } catch (cause) {
      if (mounted.current && request === generation.current) setError(message(cause));
    } finally {
      if (mounted.current && request === generation.current) setLoading(false);
    }
  }, [repository.id, acceptSnapshot]);

  useEffect(() => {
    mounted.current = true;
    scope.current += 1;
    latest.current.clear();
    active.current = null;
    setActiveId(null); setSnapshot(null); setLoading(true); setEventsReady(false); setEventsError(null); setPending(null); setStopPending(false); setError(null); setCreateError(null); setOperationError(undefined); setCreateOpen(false); setSendOpen(false);
    setArchiveOpen(false); setArchiveConfirm(false);
    setDraft(readFixDraft(fixDraftKey(repository.id, null, findingFingerprint)) ?? defaultDraft); setAction(""); setPanel("conversation");
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listFixWorkspaces(repository.id).then((values) => {
      if (disposed) return;
      for (const value of values) remember(value);
      const eligible = [...latest.current.values()].filter((value) => !findingFingerprint || value.findingFingerprint === findingFingerprint).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
      const selected = readFixSelection(repository.id, findingFingerprint);
      const first = eligible.find((value) => value.id === selected)
        ?? eligible.find((value) => evidenceScanId !== undefined && value.baselineScanId === evidenceScanId)
        ?? eligible[0];
      const firstId = first?.id ?? (findingFingerprint ? null : selected);
      if (!active.current && firstId) {
        activateWorkspace(firstId);
        saveFixSelection(repository.id, findingFingerprint, firstId);
        setDraft(readFixDraft(fixDraftKey(repository.id, firstId)) ?? defaultDraft);
        void refresh(firstId);
      }
      else if (!active.current) setLoading(false);
    }).catch((cause) => { if (!disposed) { setError(message(cause)); setLoading(false); } });
    listenFixWorkspaces((value) => {
      if (disposed || value.repositoryId !== repository.id) return;
      // Events update records owned by the inventory or selected snapshot.
      // An evicted archive must not be reintroduced by an older active event;
      // new records are admitted by list/create/restore responses instead.
      if (!latest.current.has(value.id) && active.current !== value.id) return;
      if (!remember(value)) return;
      if (active.current !== value.id) return;
      setSnapshot((current) => current?.workspace.id === value.id ? { ...current, workspace: value } : current);
      if (!fixWorkspaceBusy(value)) void refresh(value.id);
    }).then((stop) => { if (disposed) stop(); else { unlisten = stop; setEventsReady(true); } }).catch((cause) => { if (!disposed) { setEventsReady(false); setEventsError(message(cause)); } });
    return () => { disposed = true; mounted.current = false; generation.current += 1; scope.current += 1; unlisten?.(); };
  }, [repository.id, findingFingerprint, evidenceScanId, refresh, remember, reloadKey, activateWorkspace]);

  function selectPanel(next: FixWorkspaceView) {
    setPanel(next);
    if (active.current) saveFixWorkspaceView(repository.id, active.current, next);
  }

  function select(id: string) {
    activateWorkspace(id);
    setSnapshot(null);
    setDraft(readFixDraft(fixDraftKey(repository.id, id)) ?? "");
    const selected = latest.current.get(id);
    if (!findingFingerprint || selected?.findingFingerprint === findingFingerprint) saveFixSelection(repository.id, findingFingerprint, id);
    setOperationError(undefined);
    setAction("");
    void refresh(id);
  }

  async function perform(label: string, operation: () => Promise<void | FixSnapshot>, step?: "checks" | "rescan") {
    setPending(label);
    setError(null);
    if (step) setOperationError(undefined);
    const id = active.current;
    const requestScope = scope.current;
    try {
      const value = await operation();
      if (!mounted.current || scope.current !== requestScope || active.current !== id) return;
      if (value) acceptSnapshot(value);
      else if (id) await refresh(id);
    } catch (cause) {
      if (mounted.current && scope.current === requestScope && active.current === id) {
        if (step) setOperationError({ step, details: message(cause) });
        else setError(message(cause));
      }
    } finally { if (mounted.current && scope.current === requestScope) setPending(null); }
  }

  async function create() {
    const creationDraft = active.current === null
      ? readFixDraft(fixDraftKey(repository.id, null, findingFingerprint)) ?? defaultDraft
      : defaultDraft;
    setCreateOpen(false);
    setPending("Creating worktree");
    setCreateError(null);
    generation.current += 1;
    const requestScope = scope.current;
    try {
      const value = await createFixWorkspace(repository.id, findingFingerprint ?? null, evidenceScanId);
      if (!mounted.current || scope.current !== requestScope) return;
      activateWorkspace(value.workspace.id);
      saveFixSelection(repository.id, findingFingerprint, value.workspace.id);
      saveFixDraft(fixDraftKey(repository.id, value.workspace.id), creationDraft);
      setDraft(creationDraft);
      setPanel("conversation");
      acceptSnapshot(value);
      setLoading(false);
    } catch (cause) { if (mounted.current && scope.current === requestScope) setCreateError(message(cause)); }
    finally { if (mounted.current && scope.current === requestScope) setPending(null); }
  }

  async function stop(id: string) {
    const requestScope = scope.current;
    if (stopPending) return;
    setStopPending(true);
    try { await stopFixWorkspace(repository.id, id); }
    catch (cause) { if (mounted.current && scope.current === requestScope && active.current === id) setError(message(cause)); }
    finally { if (mounted.current && scope.current === requestScope) setStopPending(false); }
  }

  const chosenAction = snapshot?.actions.includes(action) ? action : snapshot?.actions[0] ?? "";
  const preparesDependencies = isDependencyPreparation(chosenAction);
  const checkoutProblem = !!createError && /checkout|\bHEAD\b/i.test(createError);
  const unavailable = repository.sourceType !== "local" ? "Fix mode requires a durable local Git checkout."
    : providerUnavailable ?? (!providerReady ? `${providerLabel} is not ready. Choose an available model or open provider settings. Saved results and local checks remain available.` : !modelSelected ? "Choose an available model before starting a fix." : null);
  const rescan = workspace?.rescan;
  const verification = snapshot ? fixEvidenceSummary(snapshot) : null;
  const statusLabel = workspace ? fixStatusLabel(workspace.status) : "";
  const activity = stopPending ? "Stopping…" : pending ?? (workspace?.status === "failed" ? "" : workspace?.activity ?? "");
  const showActivity = activity && activity !== statusLabel
    && !/^(?:workspace ready\.?$|workspace created from the recorded checkout\.|workspace check complete\.|worktree rescan complete\.)/i.test(activity);
  const scanFreshness = rescan ? fixEvidenceFreshness(rescan, diff) : null;
  const rescanFailed = operationError?.step === "rescan";
  const incompleteRescan = incompleteInventoryReason(rescanFailed ? operationError.details : workspace?.status === "failed" ? workspace.activity : null);
  const scannerFailed = rescanFailed && !incompleteRescan && /database|\bgrype db\b|invalid db/i.test(operationError.details);
  const canUseChanges = Boolean(rescan && !scanFreshness && !rescanFailed && !busy && sourceAvailable
    && !snapshot?.verificationWarning && !archived && workspace?.status === "ready" && rescan.notDetected !== null
    && (!findingRequest || workspace.findingFingerprint === findingFingerprint)
    && (!workspace.findingFingerprint || rescan.targetDetected === false));
  const noChecksLabel = archived ? "Restore the workspace to run checks."
    : !diff || snapshot?.verificationWarning ? "Checks are unavailable for this worktree."
    : "No supported npm or Cargo checks detected.";

  return <section aria-label="Fix workspace" className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface-1">
    <div role="region" aria-label="Fix workspace details" tabIndex={0} className="fix-workspace-header max-h-[45%] shrink-0 overflow-y-auto border-b border-hairline bg-surface-2 p-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus sm:px-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex min-w-0 items-center gap-2 text-sm font-semibold text-ink"><AppIcon icon={Wrench} className="size-4 shrink-0" /><span className="truncate">{repository.name} · Fix workspace</span></h2>
        <div className="flex flex-wrap gap-1"><Button disabled={Boolean(pending) || stopPending} size="sm" variant="ghost" onClick={() => setArchiveOpen(true)}>Browse archived fixes</Button><Button disabled={busy || loading || Boolean(createError) || repository.sourceType !== "local"} size="sm" variant={workspace ? "ghost" : "default"} onClick={() => setCreateOpen(true)}><AppIcon icon={GitBranch} className="size-4" />{workspace?.findingFingerprint && workspace.findingFingerprint === findingFingerprint ? "Create another fix" : workspace ? "New fix workspace" : "Create isolated fix"}</Button></div>
      </div>
      {findingRequest ? (
        <div className="mt-2 text-xs">
          <p className="break-words font-medium text-ink">{findingRequest.advisoryId} · {findingRequest.packageName}{findingRequest.installedVersion ? " " + findingRequest.installedVersion : ""}{findingRequest.targetVersion ? " → " + findingRequest.targetVersion : ""}</p>
          {!workspace ? <p className="text-ink-subtle">{evidenceScanId ? "Finding from scan #" + evidenceScanId : "Scan unavailable"}</p> : null}
          {workspace && (workspace.findingFingerprint !== findingFingerprint || (evidenceScanId && evidenceScanId !== workspace.baselineScanId)) ? (
            <p role="status" className="mt-1 text-ink-muted">{workspace.findingFingerprint !== findingFingerprint
              ? "This workspace addresses a different finding. Choose the saved fix for the selected CVE or start a separate workspace."
              : "This workspace started from an earlier scan. Review its base revision and evidence before continuing."}</p>
          ) : null}
        </div>
      ) : null}
      {workspaces.length > 1 || (workspaces.length === 1 && (!activeId || archived)) ? <Select containerClassName="mt-2" aria-label="Fix workspace" disabled={Boolean(pending) || stopPending} value={activeId ?? ""} onValueChange={select}>{!activeId ? <option value="" disabled>Choose a saved fix workspace</option> : null}{archived && workspace ? <option value={workspace.id}>{fixWorkspaceLabel(workspace)} · Archived</option> : null}{workspaces.map((item) => <option value={item.id} key={item.id}>{fixWorkspaceLabel(item)}</option>)}</Select> : workspace && !findingRequest ? <p className="mt-2 break-words text-xs text-ink-subtle">{fixWorkspaceLabel(workspace)}</p> : null}
      {workspace ? <>
        <div className="mt-2 flex items-start justify-between gap-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2"><Badge variant={busy ? "warning" : "outline"}>{statusLabel}</Badge>{showActivity ? <p aria-live="polite" className="min-w-0 break-words text-xs text-ink">{activity}</p> : null}</div>
          {fixWorkspaceBusy(workspace) ? <Button disabled={stopPending} size="sm" variant="secondary" onClick={() => void stop(workspace.id)}><AppIcon icon={Stop} className="size-4" />{stopPending ? "Stopping…" : "Stop"}</Button> : null}
        </div>
        {workspace.status === "failed" && !operationError ? (
          <details className="mt-2 text-xs text-ink-subtle">
            <summary className="cursor-pointer">Last action error</summary>
            <p className="mt-1 whitespace-pre-wrap [overflow-wrap:anywhere]">{workspace.activity}</p>
          </details>
        ) : null}
        <div className="mt-2 flex flex-wrap items-start gap-x-3 gap-y-1">
          <details className="min-w-0 flex-1 text-xs text-ink-subtle">
            <summary className="w-fit cursor-pointer py-1.5">Worktree details{workspace.baselineScanId ? ` · Scan #${workspace.baselineScanId}` : ""}</summary>
            <div className="mt-1 grid gap-1">
              <p className="break-all font-mono">{workspace.branch}</p>
              <p className="break-all">{workspace.worktreePath}</p>
              <p>Base {workspace.baseRevision.slice(0, 12)} · {workspace.baselineMatched ? "Matching baseline" : "Baseline comparison unavailable"}</p>
              <p>Workspace {workspace.id.slice(0, 8)}</p>
              <p>{workspaces.length}/20 active workspaces</p>
              <p>Last task: {workspace.provider ? `${workspace.provider.displayName} · ${workspace.provider.model ?? "default model"}` : workspace.messages.length ? "Codex · historical session" : "No provider task yet"}</p>
            </div>
          </details>
          {archived ? <div className="flex basis-full flex-wrap items-center gap-2">
            <p className="text-xs text-ink-subtle">Archived · saved evidence is read-only. Your draft is retained.</p>
            <Button size="sm" variant="secondary" disabled={busy || loading} onClick={() => void perform("Restoring workspace", () => setFixWorkspaceArchived(repository.id, workspace.id, false))}>Restore workspace</Button>
          </div> : <Button size="xs" variant="ghost" disabled={busy || loading} onClick={() => setArchiveConfirm(true)}>Archive workspace</Button>}
        </div>
      </> : null}
      {createError ? <Alert className="mt-2" variant="warning">
        <AlertTitle>Could not create the worktree</AlertTitle>
        <AlertDescription>{createError}</AlertDescription>
        <div className="mt-2 flex flex-wrap gap-2">
          {findingFingerprint && onOpenFinding ? <Button size="sm" variant="secondary" onClick={() => onOpenFinding(findingFingerprint)}>Open finding in Review</Button> : null}
          {checkoutProblem && repository.sourceType === "local" ? <Button size="sm" variant="ghost" onClick={() => {
            void openFixCheckout(repository.id).catch((cause) => setError(message(cause)));
          }}>Open checkout</Button> : null}
          {checkoutProblem && onChooseLocalCheckout ? <Button size="sm" variant="ghost" onClick={onChooseLocalCheckout}>Choose local folder</Button> : null}
          <Button size="sm" variant="ghost" onClick={() => { setCreateError(null); setReloadKey((value) => value + 1); }}>Check saved workspaces</Button>
        </div>
      </Alert> : null}
      {error ? <Alert className="mt-2" variant="warning"><AlertTitle>Workspace action could not complete</AlertTitle><AlertDescription>{error}</AlertDescription>{!snapshot ? <Button className="mt-2" size="sm" variant="secondary" disabled={loading || Boolean(pending)} onClick={() => setReloadKey((value) => value + 1)}>Retry loading workspaces</Button> : null}</Alert> : null}
      {eventsError ? <Alert className="mt-2" variant="warning"><AlertTitle>Workspace updates unavailable</AlertTitle><AlertDescription>{eventsError} Reopen this panel before starting a fix task.</AlertDescription></Alert> : null}
      {snapshot?.sourceError ? <Alert className="mt-2" variant="warning"><AlertTitle>Worktree unavailable</AlertTitle><AlertDescription>{snapshot.sourceError} Saved results remain readable. Their current freshness is unavailable.</AlertDescription></Alert> : null}
      {snapshot?.verificationWarning ? <Alert className="mt-2" variant="warning"><AlertTitle>Verification unavailable</AlertTitle><AlertDescription>{snapshot.verificationWarning}</AlertDescription></Alert> : null}
    </div>
    {!workspace ? <div className="min-h-0 flex-1 overflow-auto p-4">
      {loading ? <Spinner label="Loading fix workspace" /> : <div className="mx-auto max-w-lg py-8">
        <h3 className="text-base font-semibold text-ink">{repository.sourceType === "local" ? "Fix in an isolated checkout" : "Choose a local checkout to make changes"}</h3>
        <p className="mt-2 text-sm text-ink-subtle">{repository.sourceType === "local"
          ? "Use a separate Git worktree so changes and checks stay isolated from your everyday checkout. Reopen a saved fix or create a new one."
          : "Use Analysis with the saved scan. To edit and verify a fix, add the local checkout and open it in Chat."}</p>
        {repository.sourceType !== "local" && onChooseLocalCheckout ? <Button className="mt-3" size="sm" onClick={onChooseLocalCheckout}>Choose local folder</Button> : null}
      </div>}
    </div> : snapshot ? <>
      <div className="shrink-0 px-3 pt-1"><Tabs idPrefix={panelId} label="Fix workspace sections" value={panel} onValueChange={selectPanel} options={[
        { value: "conversation", label: "Conversation" },
        { value: "diff", label: diff ? `Diff · ${diff.files.length}` : "Diff unavailable" },
        { value: "checks", label: <span className={verification?.checkWarning ? "text-status-warning-foreground" : verification?.checkPassed ? "text-status-success-foreground" : undefined}>{workspace.checks.length ? verification?.checkLabel : "Checks"}</span> },
        { value: "rescan", label: <span className={verification?.scanWarning ? "text-status-warning-foreground" : undefined}>{rescan ? verification?.scanLabel : "Rescan"}</span> },
      ]} /></div>
      <div id={`${panelId}-panel-conversation`} role="tabpanel" aria-labelledby={`${panelId}-tab-conversation`} hidden={panel !== "conversation"} className={panel === "conversation" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
        {panel === "conversation" ? <ChatTimeline ariaLabel="Fix conversation" conversationKey={`fix:${workspace.id}`} repositoryName={repository.name} providerLabel={providerLabel} providerLabelsByItem={providerLabels} segmentProviderLabels={[]} items={timelineItems} starters={[]} onStarter={updateDraft} onCitation={() => {}} emptyState={<div className="chat-empty-state mx-auto grid min-h-full max-w-xl content-center gap-2 px-5"><h3 className="text-base font-semibold text-ink">Describe the fix you need</h3><p className="text-sm leading-6 text-ink-subtle">Your selected provider can inspect and edit this worktree, run checks and rescan. Review the source-sharing details before the task starts.</p></div>} /> : null}
        <ChatComposer inputLabel="Fix task" sendLabel="Send fix task" maxLength={32_000} draft={draft}
          draftNotice={<DraftStorageNotice error={draftStorage.error} onRetry={() => { if (draftStorage.error === "restore") setDraftReload((value) => value + 1); else void retryFixDraft(draftKey); }} />}
          selection={{ repositoryId: repository.id, connectionId, modelId: model, effort: effectiveEffort }}
          connections={connections.filter((item) => !fixProviderUnavailable(item))} models={fixModels}
          selectionLoading={selectionLoading} threadUsage={null} providerUsage={null} contextControls={null} contextChips={null}
          statusMessage={!draftStorage.loaded && !draftStorage.error ? "Restoring local draft…" : promptBytes > 32_000 ? "Task is too long. Shorten it to 32,000 UTF-8 bytes before sending." : unavailable ?? ""}
          placeholder="Describe the focused change and the checks you need…" disabled={busy || archived} inputDisabled={busy || archived || !draftStorage.loaded} sending={false} canSend={canSend} historyCachedOnly={false} retryableError={false}
          onDraftChange={updateDraft} onSelectModel={onSelectModel} onEffortChange={onEffortChange} onOpenSettings={onOpenSettings}
          onSend={() => { if (canSend) setSendOpen(true); }} onRetry={() => {}} onStop={() => void stop(workspace.id)} />
      </div>
      {(["diff", "checks", "rescan"] as const).map((name) => <div key={name} id={`${panelId}-panel-${name}`} role="tabpanel" aria-labelledby={`${panelId}-tab-${name}`} hidden={panel !== name} tabIndex={0} className={panel === name ? "min-h-0 min-w-0 flex-1 overflow-auto p-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary-focus sm:p-4" : "hidden"}>
        {name === "diff" ? <>
        <section aria-label="Workspace diff" className="min-w-0 rounded-lg border border-hairline">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-hairline p-3">
            <h3 className="text-sm font-semibold text-ink">{diff ? `Diff · ${diff.files.length} ${diff.files.length === 1 ? "file" : "files"}` : "Diff unavailable"}</h3>
            <Button size="sm" variant="ghost" disabled={busy || loading} onClick={() => void refresh(workspace.id)}><AppIcon icon={ArrowClockwise} className="size-4" />Refresh diff and check freshness</Button>
          </div>
          <div className="grid gap-2 p-3">
            {!diff ? <p className="text-sm text-ink-subtle">{snapshot.sourceError ?? "Refresh the diff to try again."}</p>
              : diff.files.length === 0 ? <p className="text-sm text-ink-subtle">No visible source changes.</p>
              : diff.files.map((file) => <details key={file.path} open={diff.files.length === 1} className="min-w-0 rounded-md border border-hairline"><summary className="cursor-pointer break-all p-2 font-mono text-xs text-ink">{file.path}{file.truncated ? " · truncated" : ""}</summary><FixDiffPreview file={file} /></details>)}
            {diff && (diff.truncated || diff.omittedFiles > 0) ? <p className="text-xs text-ink-subtle">Bounded preview. {diff.omittedFiles} files omitted. Sensitive, binary, symlink and oversized files are excluded.</p> : null}
          </div>
        </section>
        </> : name === "checks" ? <>
        <section aria-label="Workspace tests" className="grid gap-3 rounded-lg border border-hairline p-3">
          <h3 className="text-sm font-semibold text-ink">Tests and dependency preparation</h3>
          {snapshot.actions.length ? (
            <div className="flex flex-wrap items-end gap-2">
              <Select containerClassName="min-w-48 flex-1" aria-label="Workspace check" disabled={busy} value={chosenAction} onValueChange={setAction}>
                {snapshot.actions.some((item) => !isDependencyPreparation(item)) ? <optgroup label="Offline checks">
                  {snapshot.actions.filter((item) => !isDependencyPreparation(item)).map((item) => <option key={item} value={item}>{actionLabel(item)}</option>)}
                </optgroup> : null}
                {snapshot.actions.some(isDependencyPreparation) ? <optgroup label="Dependency preparation (network allowed)">
                  {snapshot.actions.filter(isDependencyPreparation).map((item) => <option key={item} value={item}>{actionLabel(item)}</option>)}
                </optgroup> : null}
              </Select>
              <Button disabled={busy || archived || !chosenAction} size="sm" variant="secondary" onClick={() => void perform(
                preparesDependencies ? "Preparing dependencies" : "Running check",
                () => checkFixWorkspace(repository.id, workspace.id, chosenAction),
                "checks",
              )}>{preparesDependencies ? "Prepare dependencies" : "Run check"}</Button>
            </div>
          ) : <p className="text-sm text-ink-subtle">{noChecksLabel}</p>}
          {chosenAction ? <p className="text-xs text-ink-subtle">{preparesDependencies
            ? "This step can download dependencies and may change the worktree lockfile. npm lifecycle scripts are disabled."
            : "Runs in the worktree sandbox with network disabled."}</p> : null}
          {operationError?.step === "checks" ? <OperationError title="Check could not run" details={operationError.details} /> : null}
          {[...workspace.checks].reverse().map((check, index) => {
            const freshness = fixEvidenceFreshness(check, diff);
            return <details key={`${check.finishedAt}:${index}`} open={index === 0 && check.status !== "passed"} className="rounded-md border border-hairline p-2"><summary className="cursor-pointer text-sm text-ink">{index === 0 ? "Latest · " : ""}{actionLabel(check.action)} · {check.status}{check.exitCode !== null ? ` (exit ${check.exitCode})` : ""}{freshness ? ` · ${freshness}` : ""}</summary><p className="mt-2 text-xs text-ink-subtle">{formatTime(check.finishedAt)}</p><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-ink-subtle">{check.output || "No command output."}</pre></details>;
          })}
        </section>
        </> : <>
        <section aria-label="Worktree rescan" className="grid gap-3 rounded-lg border border-hairline p-3">
          <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-sm font-semibold text-ink">Worktree rescan</h3><Button disabled={busy || archived || !sourceAvailable || Boolean(snapshot.verificationWarning) || Boolean(scannerUnavailable)} size="sm" variant={!rescan || scanFreshness || rescanFailed ? "default" : "secondary"} onClick={() => void perform("Rescanning worktree", () => rescanFixWorkspace(repository.id, workspace.id), "rescan")}>Rescan worktree</Button></div>
          {incompleteRescan ? <Alert variant="warning">
            <AlertTitle>Incomplete worktree inventory</AlertTitle>
            <AlertDescription>{incompleteRescan}</AlertDescription>
          </Alert> : rescanFailed ? <OperationError title={scannerFailed ? "Worktree scanner needs attention" : "Worktree rescan failed"} details={operationError.details} /> : null}
          {scannerUnavailable || scannerFailed ? <Alert variant="warning">
            <AlertTitle>Scanner setup required</AlertTitle><AlertDescription>{scannerUnavailable ?? "Check the vulnerability database, then retry the worktree scan."}</AlertDescription>
            {onOpenScannerSettings ? <Button className="mt-2" size="sm" variant="secondary" onClick={onOpenScannerSettings}>Open scanner settings</Button> : null}
          </Alert> : null}
          <p className="text-xs text-ink-subtle">Scans this worktree with local Syft and Grype. Results are kept here and do not resolve findings in the original checkout.</p>
          {rescan ? <>
            {scanFreshness === "stale" ? <p role="status" className="text-sm text-status-warning-foreground">Files changed after this scan. Rescan the worktree to verify the current files.</p> : null}
            <p className="text-sm text-ink">
              {scanFreshness || rescanFailed ? "Last recorded: " : ""}
              {rescan.findingCount} {rescan.findingCount === 1 ? "finding" : "findings"} detected
              {rescan.notDetected !== null ? ` · ${rescan.notDetected} baseline findings not detected` : ""}
              {scanFreshness && scanFreshness !== "stale" ? ` · ${scanFreshness}` : ""}
            </p>
            {rescan.notDetected === null ? <p className="text-sm text-ink-subtle">The recorded repository scan could not be compared with this worktree.</p> : null}
            {!scanFreshness && !rescanFailed && rescan.targetDetected != null ? <p className="text-sm text-ink">
              {rescan.targetDetected ? "The selected advisory is still detected for this package." : "The selected advisory was not detected in this worktree scan."}
            </p> : null}
            <p className="text-xs text-ink-subtle">{rescan.scannerVersions} · {formatTime(rescan.scannedAt)}</p>
            {rescan.findings.length ? <ul className="max-h-64 overflow-auto text-xs text-ink-subtle">{rescan.findings.map((finding, index) => <li className="py-1" key={`${finding.advisoryId}:${finding.packageName}:${index}`}>{finding.severity} · {finding.advisoryId} · {finding.packageName} {finding.version}</li>)}</ul> : null}
            {rescan.findingCount > rescan.findings.length ? <p className="text-xs text-ink-subtle">Showing the first {rescan.findings.length} findings.</p> : null}
          </> : <p className="text-sm text-ink-subtle">No worktree rescan recorded.</p>}
        </section>
        {rescan ? <section aria-label={canUseChanges ? "Use the changes" : "Worktree files"} className="mt-3 grid gap-2 rounded-lg border border-hairline p-3">
          <h3 className="text-sm font-semibold text-ink">{canUseChanges ? "Use the changes" : "Worktree files"}</h3>
          {canUseChanges ? <p className="text-xs text-ink-subtle">Review the diff, apply the changes in your Git workflow, then scan the original repository.</p> : null}
          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" variant="secondary" onClick={() => selectPanel("diff")}>Review diff</Button>
            <Button size="sm" variant="secondary" disabled={!sourceAvailable} onClick={() => {
              void openFixWorkspaceDirectory(repository.id, workspace.id).catch((cause) => setError(message(cause)));
            }}>Open worktree</Button>
            <CopyButton label="Copy branch" showLabel text={workspace.branch} />
            <CopyButton label="Copy worktree path" showLabel text={workspace.worktreePath} />
            {onOpenRepository ? <Button size="sm" variant="ghost" onClick={onOpenRepository}>Open original repository</Button> : null}
          </div>
        </section> : null}
        </>}
      </div>)}
    </> : null}
    <FixWorkspaceArchive repositoryId={repository.id} findingFingerprint={findingFingerprint} open={archiveOpen} onOpenChange={setArchiveOpen} onSelect={select} />
    <Dialog open={archiveConfirm} onOpenChange={setArchiveConfirm}><DialogContent><DialogHeader><DialogTitle data-slot="dialog-title">Archive this fix workspace?</DialogTitle><DialogDescription data-slot="dialog-description">Free an active slot and keep all local work.</DialogDescription></DialogHeader><DialogBody className="grid gap-3 text-sm text-ink-subtle"><p>The branch, worktree files, messages, unsent draft, checks and rescan evidence are retained. Archived workspaces remain readable and can be restored from Browse archived.</p><p>Archiving does not delete files or reclaim disk space.</p></DialogBody><DialogFooter><Button variant="ghost" onClick={() => setArchiveConfirm(false)}>Keep active</Button><Button disabled={busy || !workspace || archived} onClick={() => { setArchiveConfirm(false); if (workspace) void perform("Archiving workspace", () => setFixWorkspaceArchived(repository.id, workspace.id, true)); }}>Archive and keep files</Button></DialogFooter></DialogContent></Dialog>
    <Dialog open={createOpen} onOpenChange={setCreateOpen}><DialogContent><DialogHeader><DialogTitle data-slot="dialog-title">Create a dedicated fix worktree?</DialogTitle><DialogDescription data-slot="dialog-description">CVEC will create a local fix branch from {repository.name}’s clean HEAD.</DialogDescription></DialogHeader><DialogBody className="grid gap-3 text-sm text-ink-subtle"><p>The original checkout and uncommitted work are preserved. A dirty checkout must be cleaned by you before starting. The worktree remains available after stopping or restarting CVEC.</p><p>Creation sends no repository content to an AI provider. You review source sharing when submitting a fix task.</p><p>No commits, pushes, merges or pull requests are created.</p></DialogBody><DialogFooter><Button variant="ghost" onClick={() => setCreateOpen(false)}>Cancel</Button><Button onClick={() => void create()}>Create branch and worktree</Button></DialogFooter></DialogContent></Dialog>
    <Dialog open={sendOpen} onOpenChange={setSendOpen}><DialogContent><DialogHeader><DialogTitle data-slot="dialog-title">Allow {providerLabel} to work on this fix?</DialogTitle><DialogDescription data-slot="dialog-description">{modelLabel} through {providerLabel}.</DialogDescription></DialogHeader><DialogBody className="grid gap-3 text-sm text-ink-subtle"><p>Your task, scan evidence, eligible worktree source and check output may be sent to the model provider configured for {providerLabel}. Running its client locally does not guarantee local inference. This includes additional source selected by the agent, beyond the analysis chat’s context pack.</p><p>The selected provider can edit this worktree, prepare dependencies, run supported checks and rescan. Repository content cannot grant additional permissions. Tests and preparation may execute repository code inside the local sandbox.</p><p>A new provider session receives a bounded selection of this workspace’s previous messages, including messages from other providers. Files and visible results are retained locally. Publication and changes to the original checkout are unavailable.</p></DialogBody><DialogFooter><Button variant="ghost" onClick={() => setSendOpen(false)}>Cancel and keep draft</Button><Button disabled={!canSend} onClick={() => { setSendOpen(false); const text = draft; const id = workspace!.id; const draftKey = fixDraftKey(repository.id, id); const requestScope = scope.current; void perform(`Starting ${providerLabel}`, async () => {
        await startFixWorkspace(repository.id, id, { prompt: text, model, effort: effectiveEffort, connectionId, connectionUpdatedAt: selectedConnection!.connection.updatedAt });
        if (readFixDraft(draftKey) === undefined || readFixDraft(draftKey) === text) {
          saveFixDraft(draftKey, "");
          if (mounted.current && scope.current === requestScope && active.current === id) setDraft("");
        }
      }); }}>Allow and start fix</Button></DialogFooter></DialogContent></Dialog>
  </section>;
}

function OperationError({ title, details }: { title: string; details: string }) {
  return <Alert variant="warning">
    <AlertTitle>{title}</AlertTitle>
    <details className="text-xs text-ink-subtle">
      <summary className="cursor-pointer">Error details</summary>
      <p className="mt-1 whitespace-pre-wrap [overflow-wrap:anywhere]">{details}</p>
    </details>
  </Alert>;
}

function message(cause: unknown) { return cause instanceof Error ? cause.message : String(cause); }
function actionLabel(action: string) {
  if (action === "npm:prepare") return "npm · install dependencies (no scripts)";
  if (action === "npm:lockfile") return "npm · update lockfile (no scripts)";
  return action.replace(":", " · ");
}

function formatTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
