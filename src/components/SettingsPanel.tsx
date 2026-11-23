import { ArrowsClockwise, FolderOpen, GithubLogo, SignOut } from "@phosphor-icons/react";
import { useLayoutEffect, useRef, useState } from "react";

import { useAppearance } from "@/components/AppearanceProvider";
import { GithubSavedSignIn } from "@/components/GithubSavedSignIn";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input, SegmentedControl } from "@/components/ui/form-controls";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from "@/components/ui/overlays";
import { useGithubLoginPolling } from "@/hooks/useGithubLoginPolling";
import { configureGithubApp, listGithubRepositories, logoutGithub, openExternalUrl, startGithubLogin } from "@/lib/api";
import { formatRelativeTimestamp, formatTimestamp } from "@/lib/format";
import { describeGrypeDb, describeGrypeDbUpdateStatus, formatDbBuildSummary, scannerIssue } from "@/lib/grypeStatus";
import type { AppSnapshot, GithubAuthSnapshot, GithubLoginStart, GithubRepositoryOption, GrypeDbUpdateSummary } from "@/lib/types";

interface SettingsPanelProps {
  discoveryRoot: string;
  githubAuth: AppSnapshot["githubAuth"];
  githubDisabledReason: string | null;
  status: AppSnapshot["workspaceStatus"];
  versionInfo: AppSnapshot["versionInfo"];
  onAddGithubRepository: (url: string) => Promise<void>;
  onGithubAuthChange: (auth: AppSnapshot["githubAuth"]) => void;
  onGithubSnapshot: (snapshot: AppSnapshot) => void;
  onChooseFolder: () => Promise<AppSnapshot | null>;
  onScheduleChange: (nextInterval: "off" | "12h") => Promise<AppSnapshot | null>;
  onUpdateVulnerabilityDb: () => Promise<AppSnapshot | null>;
  focusRequest?: { section: "github" | "scanner"; fromImport?: boolean };
  onOpenGithubImport: () => void;
  onRefreshRepositories: () => Promise<AppSnapshot | null>;
  onCheckVulnerabilityDb: (checkUpdates: boolean) => Promise<AppSnapshot | null>;
  onReplayOnboarding: () => void;
  isScanning: boolean;
  isUpdatingVulnerabilityDb: boolean;
}

export function SettingsPanel({
  discoveryRoot,
  githubAuth,
  githubDisabledReason,
  status,
  versionInfo,
  onAddGithubRepository,
  onGithubAuthChange,
  onGithubSnapshot,
  onChooseFolder,
  onScheduleChange,
  onUpdateVulnerabilityDb,
  focusRequest,
  onOpenGithubImport,
  onRefreshRepositories,
  onCheckVulnerabilityDb,
  onReplayOnboarding,
  isScanning,
  isUpdatingVulnerabilityDb,
}: SettingsPanelProps) {
  const { appearance, resolvedTheme, setAppearance } = useAppearance();
  const githubHeading = useRef<HTMLHeadingElement>(null);
  const scannerHeading = useRef<HTMLHeadingElement>(null);
  const [refreshingRepositories, setRefreshingRepositories] = useState(false);
  const [githubLoginStart, setGithubLoginStart] =
    useState<GithubLoginStart | null>(null);
  const [isStartingGithubLogin, setIsStartingGithubLogin] = useState(false);
  const [isLoggingOutGithub, setIsLoggingOutGithub] = useState(false);
  const [githubAuthError, setGithubAuthError] = useState<string | null>(null);
  const [githubRepositories, setGithubRepositories] = useState<
    GithubRepositoryOption[]
  >([]);
  const [isLoadingGithubRepositories, setIsLoadingGithubRepositories] =
    useState(false);
  const githubRepositoriesRequest = useRef(0);
  const [isCheckingVulnerabilityDb, setIsCheckingVulnerabilityDb] =
    useState(false);
  const [addingGithubRepoUrl, setAddingGithubRepoUrl] = useState<string | null>(null);
  const grypeDb = versionInfo.grypeDb;
  const grypeDbState = describeGrypeDb(grypeDb);
  const grypeDbUpdateStatus = describeGrypeDbUpdateStatus(grypeDb);
  const lastDbUpdateState = describeGrypeDbUpdate(versionInfo.lastGrypeDbUpdate);
  const issue = scannerIssue(status, grypeDb);
  const disableDbCheck = isScanning || isUpdatingVulnerabilityDb || isCheckingVulnerabilityDb;
  const disableDbUpdate = disableDbCheck || !versionInfo.grype;
  const needsDatabase = Boolean(grypeDb && (!grypeDb.present || !grypeDb.valid));
  const canCheck = Boolean(versionInfo.grype && status.scannerReady);

  useLayoutEffect(() => {
    if (!focusRequest) return;
    const heading = focusRequest.section === "github" ? githubHeading.current : scannerHeading.current;
    heading?.focus();
  }, [focusRequest]);

  useGithubLoginPolling(githubLoginStart, setGithubLoginStart, onGithubAuthChange, setGithubAuthError);

  useLayoutEffect(() => {
    setGithubRepositories([]);
    setIsLoadingGithubRepositories(false);
    return () => { githubRepositoriesRequest.current += 1; };
  }, [githubAuth.configRevision, githubAuth.signedIn, githubAuth.login]);

  function handleGithubAppSaved(auth: GithubAuthSnapshot) {
    setGithubLoginStart(null);
    setGithubAuthError(null);
    onGithubAuthChange(auth);
  }

  async function handleStartGithubLogin() {
    setIsStartingGithubLogin(true);
    setGithubAuthError(null);
    try {
      const login = await startGithubLogin();
      setGithubLoginStart(login);
      await openExternalUrl(login.verificationUri);
    } catch (cause) {
      setGithubAuthError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsStartingGithubLogin(false);
    }
  }

  async function handleLogoutGithub() {
    githubRepositoriesRequest.current += 1;
    setIsLoadingGithubRepositories(false);
    setIsLoggingOutGithub(true);
    setGithubAuthError(null);
    try {
      const nextSnapshot = await logoutGithub();
      onGithubAuthChange(nextSnapshot.githubAuth);
      onGithubSnapshot(nextSnapshot);
      setGithubLoginStart(null);
      setGithubRepositories([]);
    } catch (cause) {
      setGithubAuthError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsLoggingOutGithub(false);
    }
  }

  async function handleLoadGithubRepositories() {
    const request = ++githubRepositoriesRequest.current;
    setIsLoadingGithubRepositories(true);
    setGithubAuthError(null);
    try {
      const repositories = await listGithubRepositories();
      if (request === githubRepositoriesRequest.current) setGithubRepositories(repositories);
    } catch (cause) {
      if (request === githubRepositoriesRequest.current) {
        setGithubAuthError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (request === githubRepositoriesRequest.current) setIsLoadingGithubRepositories(false);
    }
  }

  async function handleAddInstalledGithubRepository(repo: GithubRepositoryOption) {
    setAddingGithubRepoUrl(repo.htmlUrl);
    setGithubAuthError(null);
    try {
      await onAddGithubRepository(repo.htmlUrl);
    } catch (cause) {
      setGithubAuthError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setAddingGithubRepoUrl(null);
    }
  }

  async function handleCheckVulnerabilityDb() {
    setIsCheckingVulnerabilityDb(true);
    try {
      await onCheckVulnerabilityDb(canCheck);
    } finally {
      setIsCheckingVulnerabilityDb(false);
    }
  }

  async function handleRefreshRepositories() {
    setRefreshingRepositories(true);
    try {
      await onRefreshRepositories();
    } finally {
      setRefreshingRepositories(false);
    }
  }

  return (
    <div className="space-y-3.5">
      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
          <CardDescription>
            Choose a theme for this device. System follows the macOS appearance automatically.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center justify-between gap-3">
          <SegmentedControl
            label="Application appearance"
            onValueChange={setAppearance}
            options={[
              { value: "system", label: "System" },
              { value: "dark", label: "Dark" },
              { value: "light", label: "Light" },
            ]}
            value={appearance}
          />
          <p className="text-xs text-ink-subtle" aria-live="polite">
            Currently using {resolvedTheme} mode
          </p>
        </CardContent>
      </Card>
      <Card className="app-panel rounded-lg border shadow-none">
        <CardContent className="flex flex-col gap-3 pt-0 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-semibold text-ink">
              Welcome & setup
            </p>
            <p className="mt-1 text-xs text-ink-subtle">
              Replay the first-run guide for scanner checks, repository sources,
              GitHub setup, and the first scan.
            </p>
          </div>
          <Button
            className="app-surface shrink-0 rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
            onClick={onReplayOnboarding}
            size="sm"
            type="button"
            variant="secondary"
          >
            Replay onboarding
          </Button>
        </CardContent>
      </Card>

      <div className="grid items-start gap-3.5 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Local repositories</CardTitle>
            <CardDescription>CVEC discovers Git repositories under the selected folder.</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-3">
            <p className="break-all text-sm text-ink">{discoveryRoot}</p>
            <div className="flex flex-wrap gap-2">
              <Button disabled={refreshingRepositories} size="sm" variant="secondary" onClick={onChooseFolder}>
                <AppIcon className="size-4" icon={FolderOpen} /> Choose folder
              </Button>
              <Button disabled={isScanning || isUpdatingVulnerabilityDb} size="sm" variant="secondary" loading={refreshingRepositories} onClick={() => void handleRefreshRepositories()}>
                {refreshingRepositories ? "Finding repositories…" : "Refresh local repositories"}
              </Button>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle ref={githubHeading} tabIndex={-1}>GitHub account</CardTitle>
            <CardDescription>A GitHub connection is optional. Public repositories can be added by URL.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm text-ink-subtle">
                {!githubAuth.configured ? (githubAuth.canConfigure === false ? "GitHub sign-in is unavailable in this test profile." : "Configure your own GitHub App to access private repositories.")
                  : githubAuth.checking ? "Checking GitHub sign-in status…" : githubAuth.signedIn && githubAuth.login
                  ? `Signed in as ${githubAuth.login}.`
                  : "Sign in to access repositories from your GitHub App."}
              </p>
              {githubAuth.rateLimitRemaining !== null &&
              githubAuth.rateLimitLimit !== null ? (
                <p className="mt-2 text-xs text-ink-subtle">
                  GitHub API: {githubAuth.rateLimitRemaining} /{" "}
                  {githubAuth.rateLimitLimit} requests remaining
                </p>
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              {githubAuth.canConfigure !== false ? (
                <GithubAppSettings
                  auth={githubAuth}
                  disabled={isStartingGithubLogin || isLoggingOutGithub || addingGithubRepoUrl !== null}
                  onSaved={handleGithubAppSaved}
                />
              ) : null}
              <GithubSavedSignIn key={githubAuth.configRevision} auth={githubAuth} onChange={onGithubAuthChange} onError={setGithubAuthError} />
              {githubAuth.signedIn ? (
                <>
                  {githubAuth.installUrl ? (
                    <Button
                      className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                      onClick={() => void openExternalUrl(githubAuth.installUrl!)}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <AppIcon className="size-4" icon={GithubLogo} />
                      Manage app
                    </Button>
                  ) : null}
                  <Button
                    className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                    disabled={isLoggingOutGithub}
                    onClick={() => void handleLogoutGithub()}
                    size="sm"
                    type="button"
                    variant="secondary"
                  >
                    <AppIcon className="size-4" icon={SignOut} />
                    {isLoggingOutGithub ? "Signing out" : "Sign out"}
                  </Button>
                </>
              ) : githubAuth.configured ? (
                <Button
                  className="rounded-lg"
                  disabled={isStartingGithubLogin || githubAuth.checking}
                  onClick={() => void handleStartGithubLogin()}
                  size="sm"
                  type="button"
                >
                  <AppIcon
                    className={
                      isStartingGithubLogin ? "size-4 animate-spin" : "size-4"
                    }
                    icon={GithubLogo}
                  />
                  {isStartingGithubLogin ? "Starting" : "Sign in"}
                </Button>
              ) : null}
            </div>
          </div>

          {githubLoginStart ? (
            <div className="mt-3 rounded-md border bg-[color:var(--surface-2)] px-3 py-2 [border-color:var(--hairline)]">
              <p className="text-xs text-ink-subtle">Enter this code on GitHub</p>
              <p className="mt-1 font-mono text-lg font-semibold tracking-normal text-ink">
                {githubLoginStart.userCode}
              </p>
            </div>
          ) : null}

          {githubAuth.message && githubAuth.canConfigure !== false && githubAuth.message !== githubAuthError ? (
            <p className="mt-2 text-xs leading-4 text-ink-subtle">
              {githubAuth.message}
            </p>
          ) : null}
          {githubAuthError ? (
            <p role="alert" className="mt-2 text-xs leading-4 [color:var(--alert-danger-fg)]">
              {githubAuthError}
            </p>
          ) : null}

          {githubAuth.signedIn ? (
            <div className="mt-3">
              <div className="flex items-center justify-between gap-2">
                <p className="text-xs font-semibold text-ink">
                  Installed repositories
                </p>
                <Button
                  className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                  disabled={isLoadingGithubRepositories || isLoggingOutGithub}
                  onClick={() => void handleLoadGithubRepositories()}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {isLoadingGithubRepositories ? "Loading" : "Load repos"}
                </Button>
              </div>
              {githubDisabledReason ? <p className="mt-2 text-xs text-ink-subtle">{githubDisabledReason}</p> : null}
              {githubRepositories.length > 0 ? (
                <div className="mt-2 max-h-48 space-y-1.5 overflow-auto pr-1">
                  {githubRepositories.map((repo) => (
                    <Button
                      key={repo.fullName}
                      className="h-auto w-full justify-between px-2.5 py-2 text-left"
                      disabled={Boolean(githubDisabledReason) || addingGithubRepoUrl !== null || isLoggingOutGithub}
                      onClick={() => void handleAddInstalledGithubRepository(repo)}
                      type="button"
                      variant="outline"
                    >
                      <span className="min-w-0">
                        <span className="block truncate text-xs font-medium text-ink">
                          {repo.fullName}
                        </span>
                        <span className="block text-xs text-ink-subtle">
                          {repo.private ? "Private" : "Public"} • {repo.defaultBranch}
                        </span>
                      </span>
                      <span className="shrink-0 text-xs font-medium text-ink-subtle">
                        {addingGithubRepoUrl === repo.htmlUrl ? "Adding" : "Add"}
                      </span>
                    </Button>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}
            <Button size="sm" variant="secondary" disabled={Boolean(githubLoginStart) || isStartingGithubLogin} onClick={onOpenGithubImport}>
              {focusRequest?.fromImport ? "Return to GitHub import" : "Import GitHub repository"}
            </Button>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle ref={scannerHeading} tabIndex={-1}>Scanner</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm font-semibold text-ink">Vulnerability database</p>
              <p role="status" className="mt-1 text-xs text-ink-subtle">
                {isUpdatingVulnerabilityDb ? "Installing the database…" : isCheckingVulnerabilityDb ? "Checking scanner and database…" : issue ?? "Ready to scan"}
              </p>
              {grypeDb?.present ? <p className="mt-1 text-xs text-ink-subtle">
                Installed {formatDbBuildSummary(grypeDb.schemaVersion, grypeDb.builtAt)}
                {!needsDatabase && grypeDb.updateAvailable != null ? " · " + grypeDbState.label : ""}
              </p> : null}
              {grypeDb?.updateCheckError && !isCheckingVulnerabilityDb ? (
                <p className="mt-1 text-xs text-ink-subtle">The update check failed. Details are in Scanner diagnostics.</p>
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button disabled={disableDbCheck} onClick={() => void handleCheckVulnerabilityDb()} size="sm" variant={needsDatabase || grypeDb?.updateAvailable ? "secondary" : "default"}>
                <AppIcon className="size-4" icon={ArrowsClockwise} />
                {isCheckingVulnerabilityDb ? "Checking…" : canCheck ? "Check DB updates" : "Recheck scanner"}
              </Button>
              {versionInfo.grype ? <Button disabled={disableDbUpdate} onClick={onUpdateVulnerabilityDb} size="sm" variant={needsDatabase || grypeDb?.updateAvailable ? "default" : "secondary"}>
                {isUpdatingVulnerabilityDb ? "Installing…" : grypeDb?.present === false ? "Download vulnerability DB" : "Update vulnerability DB"}
              </Button> : null}
            </div>
          </div>

          <section className="space-y-2 border-t border-hairline pt-4" aria-label="Scan schedule">
            <h3 className="text-sm font-semibold text-ink">Scan schedule</h3>
            <SegmentedControl
              label="Scan schedule"
              onValueChange={(interval) => { if (interval === "off" || interval === "12h") void onScheduleChange(interval); }}
              options={[{ value: "off", label: "Manual only" }, { value: "12h", label: "Every 12 hours" }]}
              value={status.schedule}
            />
            {status.schedule !== "off" ? <p className="text-xs text-ink-subtle">
              {issue ? "Scheduled scans need a ready scanner." : status.nextScheduledScanAt ? "Next scan " + formatRelativeTimestamp(status.nextScheduledScanAt) : "Waiting for the first scheduled scan."}
            </p> : null}
          </section>

          <details className="rounded-md border border-hairline bg-surface-2 px-3 py-2.5">
            <summary className="cursor-pointer text-sm font-medium text-ink">Scanner diagnostics</summary>
            <dl className="mt-3 grid gap-3 text-xs sm:grid-cols-3">
              {[["CVEC", versionInfo.app], ["Syft", versionInfo.syft], ["Grype", versionInfo.grype]].map(([label, value]) => (
                <div key={label}><dt className="text-ink-subtle">{label}</dt><dd>{value ?? "Unavailable"}</dd></div>
              ))}
              <div className="sm:col-span-3"><dt className="text-ink-subtle">Database cache</dt><dd className="[overflow-wrap:anywhere]">{grypeDb?.path ?? "Unavailable"}</dd></div>
            </dl>
            {grypeDb?.error ? <p className="mt-3 text-xs text-severity-high">{grypeDb.error}</p> : null}
            <p className="mt-3 text-xs text-ink-subtle">{grypeDbUpdateStatus.description}</p>
            {grypeDb?.latestBuiltAt ? <p className="mt-1 text-xs text-ink-subtle">Latest build {formatTimestamp(grypeDb.latestBuiltAt)}</p> : null}
            <div className="mt-3 border-t border-hairline pt-3">
              <p className="text-xs font-medium text-ink">Last database update</p>
              <p className="mt-1 text-xs text-ink-subtle">{lastDbUpdateState.description}</p>
              {versionInfo.lastGrypeDbUpdate ? <LastDbActionDetails summary={versionInfo.lastGrypeDbUpdate} /> : null}
            </div>
          </details>
        </CardContent>
      </Card>
    </div>
  );
}

function GithubAppSettings({ auth, disabled, onSaved }: {
  auth: GithubAuthSnapshot;
  disabled: boolean;
  onSaved: (auth: GithubAuthSnapshot) => void;
}) {
  const [open, setOpen] = useState(false);
  const [clientId, setClientId] = useState("");
  const [installUrl, setInstallUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  function changeOpen(next: boolean) {
    if (saving) return;
    if (next) {
      setClientId(auth.clientId ?? "");
      setInstallUrl(auth.installUrl ?? "");
      setError(null);
    }
    setOpen(next);
  }

  async function save(nextClientId: string | null, nextInstallUrl: string | null) {
    setSaving(true);
    setError(null);
    try {
      onSaved(await configureGithubApp(nextClientId, nextInstallUrl));
      setOpen(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger render={<Button disabled={disabled} size="sm" variant={auth.configured ? "ghost" : "default"} />}>
        {auth.configured ? "Edit GitHub App" : "Set up GitHub App"}
      </DialogTrigger>
      <DialogContent className="max-w-md p-0" showClose={false}>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void save(clientId.trim(), installUrl.trim() || null);
          }}
        >
          <header className="space-y-1 px-5 pt-5">
            <DialogTitle className="text-base font-semibold">GitHub App</DialogTitle>
            <DialogDescription className="text-xs text-ink-muted">
              Save the Client ID for your own GitHub App, then sign in from GitHub account.
            </DialogDescription>
          </header>
          <div className="space-y-4 px-5 py-4">
            <div className="space-y-1.5 text-xs text-ink-subtle">
              <p>Enable Device Flow, grant read-only Contents, and install the app on the repositories you want to import.</p>
              <Button
                className="h-auto px-0"
                size="sm"
                type="button"
                variant="ghost"
                onClick={() => void openExternalUrl("https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app").catch((cause) => setError(String(cause)))}
              >GitHub App registration guide</Button>
            </div>
            <Input
              autoCapitalize="none"
              autoComplete="off"
              disabled={saving}
              label="Client ID"
              maxLength={128}
              onChange={(event) => setClientId(event.target.value)}
              required
              spellCheck={false}
              value={clientId}
            />
            <Input
              disabled={saving}
              label="Installation link (optional)"
              maxLength={2048}
              onChange={(event) => setInstallUrl(event.target.value)}
              placeholder="https://github.com/apps/…"
              type="url"
              value={installUrl}
            />
            {error ? <p role="alert" className="text-xs text-severity-critical">{error}</p> : null}
          </div>
          <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-hairline px-5 py-4">
            {auth.configured ? (
              <Button className="mr-auto" disabled={saving} onClick={() => void save(null, null)} size="sm" type="button" variant="ghost">
                Remove app
              </Button>
            ) : null}
            <DialogClose render={<Button disabled={saving} size="sm" type="button" variant="secondary" />}>Cancel</DialogClose>
            <Button disabled={!clientId.trim()} loading={saving} size="sm" type="submit">Save app</Button>
          </footer>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function describeGrypeDbUpdate(summary: GrypeDbUpdateSummary | null) {
  if (!summary) {
    return {
      title: "No recent DB action",
      description:
        "Run a database update to capture exactly what changed in the local Grype cache.",
      tone: "text-ink-subtle",
      badge: "No history",
      badgeClass:
        "border border-[color:var(--hairline)] bg-[color:var(--surface-1)] text-ink-subtle",
    };
  }

  if (summary.outcome === "already_current") {
    return {
      title: "Already current",
      description:
        "cvec checked for a newer Grype DB build and found that the local database was already current.",
      tone: "text-[color:var(--status-success)]",
      badge: "No change",
      badgeClass:
        "border border-[color:color-mix(in_srgb,var(--status-success)_35%,var(--hairline))] bg-[color:var(--status-success-soft)] text-[color:var(--status-success)]",
    };
  }

  return {
    title: summary.outcome === "downloaded" ? "Initial DB downloaded" : "DB updated",
    description:
      summary.outcome === "downloaded"
        ? "Downloaded the first local Grype database into cvec's managed cache."
        : "Installed a newer Grype DB build in the local cache.",
    tone: "text-[color:var(--status-success)]",
    badge: summary.outcome === "downloaded" ? "Downloaded" : "Updated",
    badgeClass:
      "border border-[color:color-mix(in_srgb,var(--status-success)_35%,var(--hairline))] bg-[color:var(--status-success-soft)] text-[color:var(--status-success)]",
  };
}

function LastDbActionDetails({
  summary,
}: {
  summary: GrypeDbUpdateSummary;
}) {
  const schemaChanged =
    summary.previousSchemaVersion !== summary.currentSchemaVersion;
  const buildChanged = summary.previousBuiltAt !== summary.currentBuiltAt;

  return (
    <div className="mt-3 space-y-2 text-xs text-ink-subtle">
      {summary.outcome !== "already_current" && (
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
          <DbActionVersionCard
            builtAt={summary.previousBuiltAt}
            label="Before"
            schemaVersion={summary.previousSchemaVersion}
          />
          <DbActionVersionCard
            builtAt={summary.currentBuiltAt}
            isCurrent
            label="After"
            schemaVersion={summary.currentSchemaVersion}
          />
        </div>
      )}

      {summary.outcome === "already_current" && (
        <DbActionVersionCard
          builtAt={summary.currentBuiltAt}
          isCurrent
          label="Current"
          schemaVersion={summary.currentSchemaVersion}
        />
      )}

      {summary.outcome === "updated" && (
        <p className="rounded-md border border-[color:var(--hairline)]/70 bg-[color:var(--surface-2)]/45 px-3 py-2">
          {schemaChanged
            ? "Schema changed together with the advisory build."
            : buildChanged
              ? "Schema stayed the same; the advisory build changed."
              : "No schema or build timestamp changed."}
        </p>
      )}

      <p className="text-xs">
        Checked {formatRelativeTimestamp(summary.checkedAt)} -{" "}
        {formatTimestamp(summary.checkedAt)}
      </p>
    </div>
  );
}

function DbActionVersionCard({
  builtAt,
  isCurrent = false,
  label,
  schemaVersion,
}: {
  builtAt: string | null;
  isCurrent?: boolean;
  label: string;
  schemaVersion: string | null;
}) {
  return (
    <div className="rounded-md border border-[color:var(--hairline)]/70 bg-[color:var(--surface-2)]/55 px-3 py-2">
      <p className="text-xs font-medium uppercase tracking-[0.12em] text-ink-subtle">
        {label}
      </p>
      <p
        className={`mt-1 text-xs font-semibold ${
          isCurrent ? "text-ink" : "text-ink-subtle"
        }`}
      >
        {formatDbSchemaLabel(schemaVersion)}
      </p>
      <p className="mt-0.5 text-xs">
        {builtAt ? `Build ${formatRelativeTimestamp(builtAt)}` : "No build timestamp"}
      </p>
      {builtAt && (
        <p className="mt-0.5 text-xs text-ink-subtle">
          {formatTimestamp(builtAt)}
        </p>
      )}
    </div>
  );
}

function formatDbSchemaLabel(schemaVersion: string | null): string {
  return schemaVersion ? `schema ${schemaVersion}` : "no local DB";
}
