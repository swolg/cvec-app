import { ArrowsClockwise, CaretDown, Cpu, Plus, Trash, Wrench } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { AppIcon } from "@/components/ui/app-icon";
import { StatusBadge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/form-controls";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/overlays";
import { formatRelativeTimestamp } from "@/lib/format";
import { companionStore } from "@/lib/companionState";
import {
  listProviderConnections,
  refreshProviderConnection,
  removeProviderConnection,
  resetProviderConnection,
  saveProviderConnection,
  setProviderConnectionEnabled,
  validateProviderConnection,
} from "@/lib/providerConnectionsApi";
import type {
  OpenCodeConnectionMode,
  ProviderConnectionDraft,
  ProviderConnectionSnapshot,
  ProviderDriverKind,
} from "@/lib/types";

type WizardStep = 1 | 2 | 3;

function emptyDraft(driver: ProviderDriverKind): ProviderConnectionDraft {
  const suffix = Math.random().toString(36).slice(2, 8);
  const prefix = driver === "codex" ? "codex" : driver === "grok" ? "grok" : "opencode";
  return {
    id: `${prefix}_${suffix}`,
    driver,
    displayName: driver === "codex" ? "Codex" : driver === "grok" ? "Grok" : "OpenCode",
    accentColor: null,
    enabled: true,
    config: driver === "codex"
      ? { kind: "codex", binaryPath: null, homePath: null, profile: null, launchArgs: null }
      : driver === "grok"
        ? { kind: "grok", binaryPath: null, customModels: [] }
        : { kind: "open_code", binaryPath: null, mode: "managed_local", serverUrl: null },
    serverPassword: null,
  };
}

export function AiConnectionsSettings() {
  const [connections, setConnections] = useState<ProviderConnectionSnapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [editingExisting, setEditingExisting] = useState(false);
  const [validating, setValidating] = useState(false);
  const [wizardError, setWizardError] = useState<string | null>(null);
  const validationRequest = useRef(0);
  const wizardHeading = useRef<HTMLHeadingElement>(null);
  const [step, setStep] = useState<WizardStep>(1);
  const [draft, setDraft] = useState<ProviderConnectionDraft>(() => emptyDraft("open_code"));
  const [validated, setValidated] = useState<ProviderConnectionSnapshot | null>(null);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await listProviderConnections();
      setConnections(next);
      companionStore.synchronizeConnections(next);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  async function mutate(id: string, operation: () => Promise<ProviderConnectionSnapshot>) {
    setBusyId(id);
    setError(null);
    try {
      const next = await operation();
      const updated = connections.map((item) => item.connection.id === id ? next : item);
      setConnections(updated);
      companionStore.synchronizeConnections(updated);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusyId(null);
    }
  }

  async function refreshAll() {
    if (!connections.length) return;
    setBusyId("__all__");
    setError(null);
    try {
      const next = await Promise.all(connections.map((snapshot) => refreshProviderConnection(snapshot.connection.id)));
      setConnections(next);
      companionStore.synchronizeConnections(next);
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusyId(null);
    }
  }

  useEffect(() => {
    if (wizardOpen) wizardHeading.current?.focus();
  }, [step, wizardOpen]);

  function changeWizardOpen(open: boolean) {
    if (!open) {
      validationRequest.current += 1;
      setValidating(false);
      setWizardError(null);
    }
    setWizardOpen(open);
  }

  function beginAdd(driver: ProviderDriverKind = "open_code") {
    setEditingExisting(false);
    setWizardError(null);
    setDraft(emptyDraft(driver));
    setValidated(null);
    setStep(1);
    setWizardOpen(true);
  }

  function edit(snapshot: ProviderConnectionSnapshot) {
    setEditingExisting(true);
    setWizardError(null);
    setDraft({
      id: snapshot.connection.id,
      driver: snapshot.connection.driver,
      displayName: snapshot.connection.displayName,
      accentColor: snapshot.connection.accentColor,
      enabled: snapshot.connection.enabled,
      config: snapshot.connection.config,
      serverPassword: null,
    });
    setValidated(snapshot);
    setStep(2);
    setWizardOpen(true);
  }

  async function validate() {
    if (validating) return;
    const request = ++validationRequest.current;
    setWizardError(null);
    setValidating(true);
    try {
      const result = await validateProviderConnection(draft);
      if (validationRequest.current === request) {
        setValidated(result);
        setStep(3);
      }
    } catch (cause) {
      if (validationRequest.current === request) setWizardError(message(cause));
    } finally {
      if (validationRequest.current === request) setValidating(false);
    }
  }

  async function save() {
    if (validating || busyId === draft.id) return;
    setBusyId(draft.id);
    setWizardError(null);
    try {
      await saveProviderConnection(draft);
      changeWizardOpen(false);
      await load();
    } catch (cause) {
      setWizardError(message(cause));
    } finally {
      setBusyId(null);
    }
  }

  async function remove(snapshot: ProviderConnectionSnapshot) {
    if (!window.confirm(`Remove “${snapshot.connection.displayName}”? Existing chats stay visible but cannot send until this connection id is restored.`)) return;
    setBusyId(snapshot.connection.id);
    try {
      try {
        await removeProviderConnection(snapshot.connection.id, false);
      } catch (cause) {
        if (!/used by .*chat thread/i.test(message(cause)) || !window.confirm(`${message(cause)}\n\nRemove it anyway?`)) throw cause;
        await removeProviderConnection(snapshot.connection.id, true);
      }
      await load();
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusyId(null);
    }
  }


  const needsModel = validated ? missingModel(validated) : false;
  const latestCheckedAt = connections.reduce<string | null>((latest, snapshot) => {
    if (!snapshot.checkedAt) return latest;
    if (!latest) return snapshot.checkedAt;
    return new Date(snapshot.checkedAt).getTime() > new Date(latest).getTime() ? snapshot.checkedAt : latest;
  }, null);
  return (
    <>
      <section id="ai-connections" className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-hairline pb-3">
          <div>
            <p className="text-sm font-medium text-ink">{connections.length} provider {connections.length === 1 ? "connection" : "connections"}</p>
            <p className="mt-1 text-xs text-ink-subtle">Checked {formatRelativeTimestamp(latestCheckedAt).toLowerCase()}</p>
          </div>
          <div className="flex items-center gap-1.5">
            <Button size="sm" variant="secondary" onClick={() => beginAdd()}>
              <AppIcon className="size-4" icon={Plus} /> Add another
            </Button>
            <IconButton aria-label="Refresh all provider statuses" disabled={Boolean(busyId) || !connections.length} size="icon-sm" variant="ghost" onClick={() => void refreshAll()}>
              <AppIcon className={busyId === "__all__" ? "size-4 animate-spin" : "size-4"} icon={ArrowsClockwise} />
            </IconButton>
          </div>
        </div>

        {error ? <Alert variant="destructive" role="alert"><AlertTitle>AI provider error</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
        {loading ? <p className="py-6 text-sm text-ink-subtle">Loading providers…</p> : null}
        {!loading && connections.length === 0 ? (
          <div className="app-surface rounded-lg border border-dashed p-8 text-center">
            <p className="text-sm font-semibold text-ink">No AI provider connection yet</p>
            <p className="mt-1 text-xs text-ink-subtle">Connect Codex or Grok directly, or add OpenCode for local models and BYOK providers.</p>
            <Button className="mt-4" size="sm" onClick={() => beginAdd()}><AppIcon className="size-4" icon={Plus} />Add provider</Button>
          </div>
        ) : null}

        {!loading && connections.length ? (
          <div className="overflow-hidden rounded-lg border border-hairline bg-surface-1 divide-y divide-hairline">
            {connections.map((snapshot) => {
              const expanded = expandedIds.has(snapshot.connection.id);
              return (
                <ConnectionRow
                  key={snapshot.connection.id}
                  snapshot={snapshot}
                  busy={busyId === snapshot.connection.id || busyId === "__all__"}
                  expanded={expanded}
                  onExpandedChange={() => setExpandedIds((current) => {
                    const next = new Set(current);
                    if (expanded) next.delete(snapshot.connection.id); else next.add(snapshot.connection.id);
                    return next;
                  })}
                  onEdit={() => edit(snapshot)}
                  onRefresh={() => void mutate(snapshot.connection.id, () => refreshProviderConnection(snapshot.connection.id))}
                  onToggle={() => void mutate(snapshot.connection.id, () => setProviderConnectionEnabled(snapshot.connection.id, !snapshot.connection.enabled))}
                  onReset={() => void mutate(snapshot.connection.id, () => resetProviderConnection(snapshot.connection.id))}
                  onRemove={() => void remove(snapshot)}
                />
              );
            })}
          </div>
        ) : null}

        <p className="max-w-3xl text-xs leading-5 text-ink-subtle">
          Local does not automatically mean private. cvec labels inference “verified local” only when both runtime and upstream can be proven local; unknown boundaries require separate consent.
        </p>
        <details className="app-surface rounded-lg border p-3">
          <summary className="cursor-pointer text-sm font-semibold text-ink">Provider setup help</summary>
          <div className="mt-3 grid gap-3 text-xs leading-5 text-ink-subtle sm:grid-cols-2">
            <div><p className="font-semibold text-ink">Codex</p><p>Install the Codex CLI, then use the existing <code>codex login</code> flow or sign in from Chat. Refresh after changing account or profile.</p></div>
            <div><p className="font-semibold text-ink">Grok</p><p>Install and sign in with the Grok CLI. cvec starts <code>grok agent stdio</code> directly and never routes Grok through OpenCode.</p></div>
            <div><p className="font-semibold text-ink">OpenCode authentication</p><p>Configure cloud-provider credentials with OpenCode itself. cvec intentionally has no vendor API-key form.</p></div>
            <div><p className="font-semibold text-ink">Ollama / llama.cpp</p><p>Start the local model server, configure its loopback endpoint in OpenCode, then refresh this connection.</p></div>
            <div><p className="font-semibold text-ink">LM Studio</p><p>Enable LM Studio's local server, connect its OpenAI-compatible loopback URL in OpenCode, then refresh.</p></div>
          </div>
        </details>
      </section>

      <Dialog open={wizardOpen} onOpenChange={(open) => { if (busyId !== draft.id) changeWizardOpen(open); }}>
        <DialogContent className="max-w-xl">
          <DialogHeader>
            <p className="text-xs text-ink-subtle">Step {step} of 3</p>
            <DialogTitle ref={wizardHeading} tabIndex={-1}>
              {step === 1 ? "Choose a runtime" : step === 2 ? "Configure connection" : "Review and save"}
            </DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-3">
            {step === 1 ? <DriverStep driver={draft.driver} onChange={(driver) => setDraft(emptyDraft(driver))} /> : null}
            {step === 2 ? <fieldset disabled={validating} className="min-w-0">
              <ConfigurationStep draft={draft} onChange={setDraft} editingExisting={editingExisting} />
            </fieldset> : null}
            {step === 3 && validated ? <ReviewStep snapshot={validated} /> : null}
            {wizardError ? <p role="alert" className="text-sm text-severity-critical">{wizardError}</p> : null}
          </DialogBody>
          <DialogFooter className="justify-between">
            <Button disabled={validating || busyId === draft.id} variant="secondary" onClick={() => {
              setWizardError(null);
              if (step === 1) changeWizardOpen(false);
              else setStep((step - 1) as WizardStep);
            }}>{step === 1 ? "Cancel" : "Back"}</Button>
            {step === 1 ? <Button onClick={() => setStep(2)}>Continue</Button> : null}
            {step === 2 ? <Button loading={validating} onClick={() => void validate()}>{validating ? "Validating…" : "Validate"}</Button> : null}
            {step === 3 ? <div className="flex flex-wrap justify-end gap-2">
              <Button disabled={validating} loading={busyId === draft.id} variant={needsModel ? "secondary" : "default"} onClick={() => void save()}>
                {busyId === draft.id ? "Saving…" : needsModel ? "Save setup" : "Save connection"}
              </Button>
              {needsModel ? <Button disabled={busyId === draft.id} loading={validating} onClick={() => void validate()}>
                {validating ? "Checking…" : "Check again"}
              </Button> : null}
            </div> : null}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ConnectionRow({ snapshot, busy, expanded, onExpandedChange, onEdit, onRefresh, onToggle, onReset, onRemove }: { snapshot: ProviderConnectionSnapshot; busy: boolean; expanded: boolean; onExpandedChange: () => void; onEdit: () => void; onRefresh: () => void; onToggle: () => void; onReset: () => void; onRemove: () => void }) {
  const providers = snapshot.upstreamProviders.filter((provider) => provider.connected);
  const boundary = summarizeBoundary(snapshot);
  const detailsId = `provider-details-${snapshot.connection.id}`;
  const runtime = snapshot.connection.driver === "codex" ? "Codex" : snapshot.connection.driver === "grok" ? "Grok" : "OpenCode";
  const summary = providerSummary(snapshot, providers.length, boundary);
  return (
    <article aria-label={`${snapshot.connection.displayName} AI connection`}>
      <div className="flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-start gap-3">
          <span className="relative flex size-9 shrink-0 items-center justify-center rounded-md bg-surface-2 text-ink-subtle">
            <AppIcon className="size-4" icon={Cpu} />
            <span aria-hidden className={`absolute -left-0.5 -top-0.5 size-2.5 rounded-full ring-2 ring-surface-1 ${healthDot(snapshot.health)}`} />
          </span>
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <h3 className="truncate text-sm font-semibold text-ink">{snapshot.connection.displayName}</h3>
              {snapshot.installedVersion ? <code className="text-xs text-ink-tertiary">{/^\d/.test(snapshot.installedVersion) ? `v${snapshot.installedVersion}` : snapshot.installedVersion}</code> : null}
              <StatusBadge status={missingModel(snapshot) ? "warning" : healthStatus(snapshot.health)}>
                {missingModel(snapshot) ? "No usable model" : healthLabel(snapshot.health)}
              </StatusBadge>
            </div>
            <p className="mt-1 text-xs leading-5 text-ink-subtle">{summary}</p>
            {!expanded && snapshot.health !== "ready" && (snapshot.error?.message ?? snapshot.message) ? <p className="mt-1 text-xs leading-5 text-ink-subtle">{snapshot.error?.message ?? snapshot.message}</p> : null}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2 self-end sm:self-auto">
          <IconButton aria-controls={detailsId} aria-expanded={expanded} aria-label={`${expanded ? "Hide" : "Show"} ${snapshot.connection.displayName} details`} size="icon-sm" variant="ghost" onClick={onExpandedChange}>
            <AppIcon className={`size-4 transition-transform ${expanded ? "rotate-180" : ""}`} icon={CaretDown} />
          </IconButton>
          <Button
            aria-checked={snapshot.connection.enabled}
            aria-label={`${snapshot.connection.enabled ? "Disable" : "Enable"} ${snapshot.connection.displayName}`}
            className={`h-6 w-11 rounded-full p-0 ${snapshot.connection.enabled ? "justify-end bg-primary hover:bg-primary-hover" : "justify-start bg-surface-4 hover:bg-surface-4"}`}
            disabled={busy}
            onClick={onToggle}
            role="switch"
            variant="ghost"
          >
            <span aria-hidden className="mx-0.5 size-5 rounded-full bg-white shadow-sm" />
          </Button>
        </div>
      </div>
      {expanded ? (
        <div id={detailsId} className="border-t border-hairline bg-surface-2/55 px-4 py-4">
          <dl className="grid gap-3 text-xs sm:grid-cols-4">
            <div><dt className="text-ink-subtle">Runtime</dt><dd className="mt-1 font-medium text-ink">{runtime}</dd></div>
            <div><dt className="text-ink-subtle">Models</dt><dd className="mt-1 font-medium text-ink">{snapshot.models.length}</dd></div>
            <div><dt className="text-ink-subtle">Upstream providers</dt><dd className="mt-1 font-medium text-ink">{providers.length}</dd></div>
            <div><dt className="text-ink-subtle">Last checked</dt><dd className="mt-1 font-medium text-ink">{formatRelativeTimestamp(snapshot.checkedAt)}</dd></div>
          </dl>
          {snapshot.message || snapshot.error ? <p className="mt-3 text-xs leading-5 text-ink-subtle">{snapshot.error?.message ?? snapshot.message}</p> : null}
          <div className="mt-4 flex flex-wrap gap-2">
            <Button size="sm" variant="secondary" onClick={onEdit}><AppIcon className="size-4" icon={Wrench} /> Configure</Button>
            <Button size="sm" variant="secondary" disabled={busy} onClick={onRefresh}><AppIcon className={busy ? "size-4 animate-spin" : "size-4"} icon={ArrowsClockwise} />Refresh</Button>
            {snapshot.connection.builtIn ? <Button size="sm" variant="secondary" disabled={busy} onClick={onReset}>Reset</Button> : <Button size="sm" variant="secondary" disabled={busy} onClick={onRemove}><AppIcon className="size-4" icon={Trash} /> Remove</Button>}
          </div>
        </div>
      ) : null}
    </article>
  );
}

function providerSummary(snapshot: ProviderConnectionSnapshot, providerCount: number, boundary: string): string {
  const state = missingModel(snapshot) ? "Runtime detected"
    : snapshot.health === "ready" ? snapshot.authKind !== "none" ? "Authenticated" : "Available" : healthLabel(snapshot.health);
  const providerLabel = `${providerCount} upstream ${providerCount === 1 ? "provider" : "providers"}`;
  const modelLabel = `${snapshot.models.length} ${snapshot.models.length === 1 ? "model" : "models"}`;
  return `${state} · ${providerLabel} · ${modelLabel} · ${boundary}`;
}

function healthDot(value: ProviderConnectionSnapshot["health"]): string {
  if (value === "ready") return "bg-status-success-text";
  if (value === "unavailable" || value === "incompatible") return "bg-severity-critical";
  if (value === "setup_required" || value === "stale") return "bg-status-warning-text";
  return "bg-ink-tertiary";
}

function DriverStep({ driver, onChange }: { driver: ProviderDriverKind; onChange: (driver: ProviderDriverKind) => void }) {
  return <div className="grid gap-3 sm:grid-cols-3">
    {(["codex", "grok", "open_code"] as const).map((value) => <Button key={value} type="button" variant="secondary" aria-pressed={driver === value} onClick={() => onChange(value)} className={`h-auto items-start rounded-lg border p-4 text-left whitespace-normal transition-colors ${driver === value ? "border-[color:var(--accent)] bg-[color:var(--surface-3)]" : "app-surface"}`}><span><span className="block font-semibold text-ink">{value === "codex" ? "Codex" : value === "grok" ? "Grok" : "OpenCode"}</span><span className="mt-2 block text-xs leading-5 text-ink-subtle">{value === "codex" ? "Use your existing Codex ChatGPT or API-key login." : value === "grok" ? "Use the local Grok CLI as a direct ACP coding-agent runtime." : "Use local models and BYOK providers configured in OpenCode."}</span></span></Button>)}
  </div>;
}

function ConfigurationStep({ draft, onChange, editingExisting }: { draft: ProviderConnectionDraft; onChange: (draft: ProviderConnectionDraft) => void; editingExisting: boolean }) {
  const opencode = draft.config.kind === "open_code" ? draft.config : null;
  const codex = draft.config.kind === "codex" ? draft.config : null;
  const grok = draft.config.kind === "grok" ? draft.config : null;
  return <div className="space-y-4">
    <label className="block text-xs font-medium text-ink">Display name<Input className="mt-2" value={draft.displayName} onChange={(event) => onChange({ ...draft, displayName: event.target.value })} /></label>
    {opencode ? <>
      <label className="block text-xs font-medium text-ink">Runtime mode<Select className="mt-2 w-full" value={opencode.mode ?? "managed_local"} onValueChange={(mode) => onChange({ ...draft, config: { ...opencode, mode: mode as OpenCodeConnectionMode } })}><option value="managed_local">Managed local process</option><option value="existing_loopback">Existing loopback server</option></Select></label>
      <label className="block text-xs font-medium text-ink">OpenCode executable<Input className="mt-2" placeholder="opencode (uses PATH)" value={opencode.binaryPath ?? ""} onChange={(event) => onChange({ ...draft, config: { ...opencode, binaryPath: event.target.value || null } })} /></label>
      {opencode.mode === "existing_loopback" ? <><label className="block text-xs font-medium text-ink">Loopback server URL<Input className="mt-2" placeholder="http://127.0.0.1:4096" value={opencode.serverUrl ?? ""} onChange={(event) => onChange({ ...draft, config: { ...opencode, serverUrl: event.target.value || null } })} /></label><label className="block text-xs font-medium text-ink">Server password (write only)<Input className="mt-2" type="password" autoComplete="new-password" placeholder="Leave blank to keep existing password" value={draft.serverPassword ?? ""} onChange={(event) => onChange({ ...draft, serverPassword: event.target.value || null })} /></label></> : null}
      <Alert><AlertTitle>Provider credentials stay in OpenCode</AlertTitle><AlertDescription>Configure Ollama, LM Studio, llama.cpp, or cloud provider keys with OpenCode. cvec only stores an optional loopback-server password in macOS Keychain.</AlertDescription></Alert>
    </> : null}
    {codex ? <><label className="block text-xs font-medium text-ink">Codex executable<Input className="mt-2" placeholder="codex (uses PATH)" value={codex.binaryPath ?? ""} onChange={(event) => onChange({ ...draft, config: { ...codex, binaryPath: event.target.value || null } })} /></label><details className="rounded-lg border p-3"><summary className="cursor-pointer text-xs font-semibold text-ink">Advanced Codex isolation</summary><div className="mt-3 space-y-3"><Input placeholder="CODEX_HOME path" value={codex.homePath ?? ""} onChange={(event) => onChange({ ...draft, config: { ...codex, homePath: event.target.value || null } })} /><Input placeholder="Profile" value={codex.profile ?? ""} onChange={(event) => onChange({ ...draft, config: { ...codex, profile: event.target.value || null } })} /><Input placeholder="Allowed app-server launch args" value={codex.launchArgs ?? ""} onChange={(event) => onChange({ ...draft, config: { ...codex, launchArgs: event.target.value || null } })} /></div></details></> : null}
    {grok ? <>
      <label className="block text-xs font-medium text-ink">Grok executable<Input className="mt-2" placeholder="grok (uses PATH)" value={grok.binaryPath ?? ""} onChange={(event) => onChange({ ...draft, config: { ...grok, binaryPath: event.target.value || null } })} /></label>
      <label className="block text-xs font-medium text-ink">Additional Grok models<Input className="mt-2" placeholder="grok-model-a, grok-model-b" value={(grok.customModels ?? []).join(", ")} onChange={(event) => onChange({ ...draft, config: { ...grok, customModels: event.target.value.split(",").map((value) => value.trim()).filter(Boolean) } })} /></label>
      <Alert><AlertTitle>Authentication stays with Grok</AlertTitle><AlertDescription>Sign in with the Grok CLI. cvec does not store xAI API keys and denies ACP filesystem, terminal, write, and network-tool requests.</AlertDescription></Alert>
    </> : null}
    <details className="rounded-lg border p-3">
      <summary className="cursor-pointer text-xs font-semibold text-ink">Connection ID</summary>
    <label className="block text-xs font-medium text-ink">Stable connection ID<Input className="mt-2" readOnly={editingExisting} value={draft.id} onChange={(event) => onChange({ ...draft, id: event.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, "_") })} /></label>
    </details>
  </div>;
}

function ReviewStep({ snapshot }: { snapshot: ProviderConnectionSnapshot }) {
  const models = snapshot.models.filter((model) => !model.hidden);
  const usable = snapshot.connection.enabled && snapshot.health === "ready" && models.length > 0;
  const state = !snapshot.connection.enabled ? "Disabled" : snapshot.health !== "ready"
    ? healthLabel(snapshot.health) : usable ? "Models available" : "No usable model";
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="font-semibold text-ink">{snapshot.connection.displayName}</p>
        <StatusBadge status={usable ? "success" : "warning"}>{state}</StatusBadge>
      </div>
      {missingModel(snapshot) ? <p className="text-sm text-ink-subtle">
        {snapshot.connection.driver === "open_code"
          ? "Configure a model in OpenCode, then choose Check again. You can save the setup and finish it later."
          : "Make a model available in this provider, then choose Check again. You can save the setup and finish it later."}
      </p> : <p className="text-sm text-ink-subtle">{snapshot.error?.message ?? snapshot.message ?? "Configuration validated."}</p>}
      {usable ? <p className="text-sm text-ink-subtle">{models.map((model) => model.displayName).join(", ")}</p> : null}
      {usable ? <p className="text-xs text-ink-subtle">Inference {summarizeBoundary({ ...snapshot, models })}. Unknown boundaries require consent in Chat.</p> : null}
    </div>
  );
}

function missingModel(snapshot: ProviderConnectionSnapshot): boolean {
  return snapshot.connection.enabled && snapshot.health === "ready" && !snapshot.models.some((model) => !model.hidden);
}

function summarizeBoundary(snapshot: ProviderConnectionSnapshot): string {
  const values = new Set(snapshot.models.map((model) => model.inferenceBoundary));
  if (values.size !== 1) return values.size ? "mixed boundaries" : "boundary unknown";
  const value = [...values][0];
  return value === "verified_local" ? "verified local" : value === "external" ? "external inference" : "boundary unknown";
}

function healthLabel(value: ProviderConnectionSnapshot["health"]): string { return value.replaceAll("_", " "); }
function healthStatus(value: ProviderConnectionSnapshot["health"]): "success" | "warning" | "neutral" | "critical" { return value === "ready" ? "success" : value === "setup_required" || value === "stale" ? "warning" : value === "unavailable" || value === "incompatible" ? "critical" : "neutral"; }
function message(cause: unknown): string { return cause instanceof Error ? cause.message : String(cause); }
