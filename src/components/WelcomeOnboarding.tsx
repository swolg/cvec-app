import { ArrowRight, ArrowsClockwise, Check, FolderOpen, GithubLogo, X } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import onboardingHero from "@/assets/onboarding-hero.webp";
import type { DashboardSection } from "@/components/DashboardSidebar";
import { GithubRepositoryForm } from "@/components/GithubRepositoryForm";
import { GithubSavedSignIn } from "@/components/GithubSavedSignIn";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { AppIcon } from "@/components/ui/app-icon";
import { StatusBadge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useGithubLoginPolling } from "@/hooks/useGithubLoginPolling";
import { listGithubRepositories, openExternalUrl, startGithubLogin } from "@/lib/api";
import { describeScanResult, formatScheduleLabel } from "@/lib/format";
import { describeGrypeDb, describeGrypeDbUpdateStatus } from "@/lib/grypeStatus";
import type { AppSnapshot, GithubLoginStart, GithubRepositoryOption, ScanAllSummary } from "@/lib/types";

type OnboardingStep = "welcome" | "scanner" | "database" | "sources" | "scan";

export interface WelcomeOnboardingContext {
  snapshot: AppSnapshot;
  githubDisabledReason: string | null;
  error?: string | null;
  isScanning: boolean;
  isUpdatingVulnerabilityDb: boolean;
  onAddGithubRepository: (url: string) => Promise<void>;
  onChooseFolder: () => Promise<AppSnapshot | null>;
  onClose: () => void;
  onComplete: (section?: DashboardSection) => Promise<void>;
  onGithubAuthChange: (auth: AppSnapshot["githubAuth"]) => void;
  onRefresh: () => Promise<AppSnapshot | null>;
  onScheduleChange: (nextInterval: "off" | "12h") => Promise<AppSnapshot | null>;
  onScanAll: () => Promise<ScanAllSummary | null>;
  onSkip: () => Promise<void>;
  onUpdateVulnerabilityDb: () => Promise<AppSnapshot | null>;
}

interface WelcomeOnboardingProps {
  context: WelcomeOnboardingContext;
}

const ONBOARDING_STEPS: Array<{
  id: OnboardingStep;
  label: string;
  description: string;
}> = [
  {
    id: "welcome",
    label: "Welcome",
    description: "Get started with CVEC",
  },
  {
    id: "scanner",
    label: "Scanner",
    description: "syft and grype checks",
  },
  {
    id: "database",
    label: "Database",
    description: "Local advisory cache",
  },
  {
    id: "sources",
    label: "Sources",
    description: "Choose repositories",
  },
  {
    id: "scan",
    label: "First scan",
    description: "Scan your repositories",
  },
];

export function WelcomeOnboarding({ context }: WelcomeOnboardingProps) {
  const {
    snapshot,
    githubDisabledReason,
    error,
    isScanning,
    isUpdatingVulnerabilityDb,
    onAddGithubRepository,
    onChooseFolder,
    onClose,
    onComplete,
    onGithubAuthChange,
    onRefresh,
    onScheduleChange,
    onScanAll,
    onSkip,
    onUpdateVulnerabilityDb,
  } = context;
  const [step, setStep] = useState<OnboardingStep>("welcome");
  const stepContent = useRef<HTMLElement>(null);
  const focusStep = useRef(false);
  const [githubLoginStart, setGithubLoginStart] =
    useState<GithubLoginStart | null>(null);
  const [githubAuthError, setGithubAuthError] = useState<string | null>(null);
  const [githubRepositories, setGithubRepositories] = useState<
    GithubRepositoryOption[]
  >([]);
  const [isStartingGithubLogin, setIsStartingGithubLogin] = useState(false);
  const [isLoadingGithubRepositories, setIsLoadingGithubRepositories] =
    useState(false);
  const [addingGithubRepoUrl, setAddingGithubRepoUrl] = useState<string | null>(null);
  const [isRefreshingChecks, setIsRefreshingChecks] = useState(false);
  const [isRunningFirstScan, setIsRunningFirstScan] = useState(false);
  const [firstScanSummary, setFirstScanSummary] =
    useState<ScanAllSummary | null>(null);

  const status = snapshot.workspaceStatus;
  const versionInfo = snapshot.versionInfo;
  const githubAuth = snapshot.githubAuth;
  const localRepos = snapshot.repos.filter((repo) => repo.sourceType === "local");
  const githubRepos = snapshot.repos.filter((repo) => repo.sourceType === "github");
  const hasRepos = snapshot.repos.length > 0;
  const databaseReady = Boolean(versionInfo.grypeDb?.present && versionInfo.grypeDb.valid);
  const databaseMissing = Boolean(versionInfo.grypeDb && !versionInfo.grypeDb.present);
  const isBusy = isScanning || status.scanRunning;
  const canRunFirstScan = status.scannerReady && databaseReady && hasRepos && !isBusy;
  const grypeDbState = describeGrypeDb(versionInfo.grypeDb);
  const grypeDbUpdateStatus = describeGrypeDbUpdateStatus(versionInfo.grypeDb);
  const disableDbUpdate =
    !versionInfo.grype || isBusy || isUpdatingVulnerabilityDb;
  const currentIndex = ONBOARDING_STEPS.findIndex((item) => item.id === step);
  const scanResult = firstScanSummary ? describeScanResult(firstScanSummary) : null;
  const firstScanSucceeded = Boolean(firstScanSummary?.reposScanned);
  const stepComplete: Record<OnboardingStep, boolean> = {
    welcome: step !== "welcome",
    scanner: status.scannerReady,
    database: databaseReady,
    sources: hasRepos,
    scan: scanResult?.kind === "success",
  };

  useEffect(() => {
    if (!focusStep.current) return;
    focusStep.current = false;
    const content = stepContent.current;
    if (!content) return;
    content.scrollTop = 0;
    content.querySelector<HTMLElement>("h1")?.focus({ preventScroll: true });
  }, [step]);

  useGithubLoginPolling(githubLoginStart, setGithubLoginStart, onGithubAuthChange, setGithubAuthError);

  async function handleRefreshChecks() {
    setIsRefreshingChecks(true);
    try {
      await onRefresh();
    } finally {
      setIsRefreshingChecks(false);
    }
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

  async function handleLoadGithubRepositories() {
    setIsLoadingGithubRepositories(true);
    setGithubAuthError(null);
    try {
      setGithubRepositories(await listGithubRepositories());
    } catch (cause) {
      setGithubAuthError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsLoadingGithubRepositories(false);
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

  async function handleRunFirstScan() {
    setIsRunningFirstScan(true);
    setFirstScanSummary(null);
    try {
      const summary = await onScanAll();
      if (summary) {
        setFirstScanSummary(summary);
      }
    } finally {
      setIsRunningFirstScan(false);
    }
  }

  function openStep(next: OnboardingStep) {
    if (next === step) return;
    focusStep.current = true;
    setStep(next);
  }

  function goNext() {
    const next = ONBOARDING_STEPS[currentIndex + 1];
    if (next) {
      openStep(next.id);
    }
  }

  function goBack() {
    const previous = ONBOARDING_STEPS[currentIndex - 1];
    if (previous) {
      openStep(previous.id);
    }
  }

  function renderStepContent() {
    if (step === "welcome") {
      return (
        <div className="grid min-h-0 gap-5 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] lg:items-center">
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase tracking-[0.16em] text-[color:var(--primary)]">
              cvec setup
            </p>
            <h1 tabIndex={-1} className="mt-3 max-w-[620px] text-4xl font-semibold leading-[1.05] tracking-tight text-ink lg:text-5xl">
              Scan your repositories for vulnerabilities
            </h1>
            <p className="mt-4 max-w-[560px] text-sm leading-6 text-ink-subtle">
              Check your dependencies with Syft and Grype. CVEC stores scan results on this Mac.
              You can start with a local Git folder and link GitHub repositories later.
            </p>
            <div className="mt-6 flex flex-wrap gap-2.5">
              <Button className="rounded-lg" onClick={goNext} type="button">
                Set up cvec
                <AppIcon className="size-4" icon={ArrowRight} />
              </Button>
              <Button
                className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                onClick={() => void onSkip()}
                size="sm"
                type="button"
                variant="secondary"
              >
                Skip for now
              </Button>
            </div>
          </div>

          <div className="relative hidden min-h-[300px] overflow-hidden rounded-lg border bg-surface-1 [border-color:var(--hairline)] lg:block">
            <img
              alt=""
              className="absolute inset-0 h-full w-full object-cover"
              src={onboardingHero}
            />
          </div>
        </div>
      );
    }

    if (step === "scanner") {
      return (
        <OnboardingStepFrame
          kicker="Scanner check"
          title="Check Syft and Grype"
          description="Syft lists the packages in your repositories. Grype checks them for known vulnerabilities."
        >
          <div className="grid gap-3 md:grid-cols-2">
            <OnboardingCheckCard
              label="syft"
              detail={versionInfo.syft ?? "Not available"}
              ok={Boolean(versionInfo.syft)}
            />
            <OnboardingCheckCard
              label="grype"
              detail={versionInfo.grype ?? "Not available"}
              ok={Boolean(versionInfo.grype)}
            />
          </div>

          {!status.scannerReady ? (
            <div className="rounded-md border px-3 py-3 [border-color:var(--alert-warning-border)] [background:var(--alert-warning-bg)]">
              <p className="text-sm font-semibold [color:var(--alert-warning-fg)]">
                Scanner setup required
              </p>
              <p className="mt-1 text-xs leading-5 [color:var(--alert-warning-fg)]">
                {status.scannerMessage ??
                  "Install Syft and Grype with Homebrew, then select Refresh checks."}
              </p>
              <code className="mt-3 block overflow-x-auto rounded-md bg-surface-4 px-3 py-2 text-xs text-ink">
                brew install syft grype
              </code>
            </div>
          ) : (
            <div className="app-surface rounded-md border px-3 py-3">
              <p className="text-sm font-semibold text-status-success">
                Scanner tools installed
              </p>
              <p className="mt-1 text-xs text-ink-subtle">
                cvec can scan local folders and linked GitHub repositories.
              </p>
            </div>
          )}

          <OnboardingFooter
            backLabel="Back"
            nextLabel="Continue"
            onBack={goBack}
            onNext={goNext}
            secondaryAction={
              <Button
                className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                disabled={isRefreshingChecks}
                onClick={() => void handleRefreshChecks()}
                size="sm"
                type="button"
                variant="secondary"
              >
                <AppIcon
                  className={isRefreshingChecks ? "size-4 animate-spin" : "size-4"}
                  icon={ArrowsClockwise}
                />
                {isRefreshingChecks ? "Refreshing" : "Refresh checks"}
              </Button>
            }
          />
        </OnboardingStepFrame>
      );
    }

    if (step === "database") {
      return (
        <OnboardingStepFrame
          kicker="Vulnerability database"
          title="Prepare the vulnerability database"
          description="Grype uses a local copy of its advisory database. Download it here to prepare for your first scan."
        >
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_280px]">
            <div className="app-surface rounded-md border px-3 py-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <p className="text-xs font-semibold text-ink">
                    Grype DB
                  </p>
                  <p className={`mt-2 text-lg font-semibold ${grypeDbState.tone}`}>
                    {grypeDbState.label}
                  </p>
                  <p className="mt-1 text-xs text-ink-subtle">
                    {grypeDbState.detail}
                  </p>
                </div>
                <span className={`rounded-full px-2.5 py-1 text-xs font-medium ${grypeDbState.badgeClass}`}>
                  {grypeDbState.badge}
                </span>
              </div>
            </div>

            <div className="app-surface rounded-md border px-3 py-3">
              <p className="text-xs font-semibold text-ink">
                Update check
              </p>
              <p className={`mt-2 text-sm font-semibold ${grypeDbUpdateStatus.tone}`}>
                {grypeDbUpdateStatus.title}
              </p>
              <p className="mt-1 text-xs text-ink-subtle">
                {grypeDbUpdateStatus.description}
              </p>
            </div>
          </div>

          <OnboardingFooter
            backLabel="Back"
            nextLabel={databaseReady ? "Continue" : "Continue without downloading"}
          nextVariant={databaseReady || !versionInfo.grype ? "default" : "secondary"}
            onBack={goBack}
            onNext={goNext}
            secondaryAction={
              <Button
                className="rounded-lg"
                disabled={disableDbUpdate}
                onClick={() => void onUpdateVulnerabilityDb()}
                size="sm"
                type="button"
                variant={databaseReady ? "secondary" : "default"}
              >
                <AppIcon
                  className={
                    isUpdatingVulnerabilityDb ? "size-4 animate-spin" : "size-4"
                  }
                  icon={ArrowsClockwise}
                />
                {isUpdatingVulnerabilityDb
                    ? "Updating database…"
                    : versionInfo.grypeDb?.present ? "Update database" : "Download database"}
              </Button>
            }
          />
        </OnboardingStepFrame>
      );
    }

    if (step === "sources") {
      return (
        <OnboardingStepFrame
          kicker="Repository sources"
          title="Choose what cvec should watch"
          description="Choose a local Git folder or add a GitHub repository by URL."
        >
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(320px,0.9fr)]">
            <div className="app-surface rounded-md border px-3 py-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <AppIcon className="size-4 text-ink-subtle" icon={FolderOpen} />
                    <p className="text-xs font-semibold text-ink">
                      Local folder
                    </p>
                  </div>
                  <p className="mt-2 break-all text-xs font-medium text-ink">
                    {snapshot.discoveryRoot}
                  </p>
                  <p className="mt-2 text-xs text-ink-subtle">
                    {localRepos.length === 0
                      ? "No local repositories found under this root yet."
                      : `${localRepos.length} local repos discovered under this root.`}
                  </p>
                </div>
                <Button
                  className="rounded-lg"
                  onClick={() => void onChooseFolder()}
                  size="sm"
                  type="button"
                  variant="default"
                >
                  <AppIcon className="size-4" icon={FolderOpen} />
                  Choose folder
                </Button>
              </div>
            </div>

            <div className="app-surface rounded-md border px-3 py-3">
              <div className="flex items-center gap-2">
                <AppIcon className="size-4 text-ink-subtle" icon={GithubLogo} />
                <p className="text-xs font-semibold text-ink">
                  GitHub repositories (optional)
                </p>
              </div>
              <p className="mt-2 text-xs text-ink-subtle">
                CVEC fetches and scans a linked repository when you add it.
                Public repositories can be added by URL.
              </p>
              <GithubRepositoryForm
                className="mt-3"
                disabled={Boolean(githubDisabledReason)}
                disabledReason={githubDisabledReason}
                submitVariant="secondary"
                onAdd={onAddGithubRepository}
                submitLabel="Add repo"
              />
            </div>
          </div>

          {githubAuth.configured ? (
            <div className="app-surface rounded-md border px-3 py-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-xs font-semibold text-ink">
                    Optional GitHub account
                  </p>
                  <p className="mt-1 text-xs text-ink-subtle">
                    {githubAuth.checking ? "Checking GitHub sign-in status…" : githubAuth.signedIn && githubAuth.login
                      ? `Signed in as ${githubAuth.login}. Installed private repos can be added from this setup.`
                      : githubAuth.deferred ? "Connect your saved GitHub sign-in when needed, or sign in with another account." : "Sign in to raise rate limits and add private repositories where your GitHub App is installed."}
                  </p>
                </div>
                <div className="flex shrink-0 flex-wrap gap-2">
                  <GithubSavedSignIn auth={githubAuth} onChange={onGithubAuthChange} onError={setGithubAuthError} />
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
                          Manage app
                        </Button>
                      ) : null}
                      <Button
                        className="rounded-lg"
                        disabled={isLoadingGithubRepositories}
                        onClick={() => void handleLoadGithubRepositories()}
                        size="sm"
                        type="button"
                        variant="secondary"
                      >
                        {isLoadingGithubRepositories ? "Loading" : "Load repos"}
                      </Button>
                    </>
                  ) : (
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
                  )}
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

              {githubAuthError ? (
                <p className="mt-2 text-xs leading-4 [color:var(--alert-danger-fg)]">
                  {githubAuthError}
                </p>
              ) : null}

              {githubRepositories.length > 0 ? (
                <div className="mt-3 grid max-h-44 gap-1.5 overflow-auto pr-1 sm:grid-cols-2">
                  {githubRepositories.map((repo) => (
                    <Button
                      key={repo.fullName}
                      className="h-auto min-w-0 justify-between px-2.5 py-2 text-left"
                      disabled={Boolean(githubDisabledReason) || addingGithubRepoUrl !== null}
                      onClick={() => void handleAddInstalledGithubRepository(repo)}
                      type="button"
                      variant="outline"
                    >
                      <span className="min-w-0">
                        <span className="block truncate text-xs font-medium text-ink">
                          {repo.fullName}
                        </span>
                        <span className="block text-xs text-ink-subtle">
                          {repo.private ? "Private" : "Public"} - {repo.defaultBranch}
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

          {!hasRepos ? (
            <Alert variant="warning">
              <AlertTitle>No repositories selected yet</AlertTitle>
              <AlertDescription>
                Choose a folder with Git repositories or add a GitHub repository
                before running the first scan.
              </AlertDescription>
            </Alert>
          ) : null}

          <OnboardingFooter
            backLabel="Back"
            nextDisabled={!hasRepos}
            nextLabel="Continue"
            onBack={goBack}
            onNext={goNext}
          />
        </OnboardingStepFrame>
      );
    }

    return (
      <OnboardingStepFrame
        kicker="First scan"
        title="Run your first scan"
        description="Scan the selected repositories to see which dependencies need your attention."
      >
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <OnboardingStatusTile
            label="Scanner"
            value={!status.scannerReady ? "Tools needed" : databaseReady ? "Ready to scan" : "Database needed"}
            ok={status.scannerReady && databaseReady}
          />
          <OnboardingStatusTile
            label="Repositories"
            value={`${snapshot.repos.length} watched`}
            ok={hasRepos}
          />
          <OnboardingStatusTile
            label="GitHub"
            value={`${githubRepos.length} linked`}
            ok={githubRepos.length > 0}
            neutral={githubRepos.length === 0}
          />
          <OnboardingStatusTile
            label="Schedule"
            value={formatScheduleLabel(status.schedule)}
            ok={status.schedule !== "off"}
            neutral={status.schedule === "off"}
          />
        </div>

        <div className="app-surface rounded-md border px-3 py-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="text-xs font-semibold text-ink">
                Scan schedule
              </p>
              <p className="mt-1 text-xs text-ink-subtle">
                Keep it manual for now or let cvec check every 12 hours.
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                className={
                  status.schedule === "off"
                    ? "rounded-lg"
                    : "app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                }
                onClick={() => void onScheduleChange("off")}
                size="sm"
                type="button"
                variant={status.schedule === "off" ? "default" : "secondary"}
              >
                Off
              </Button>
              <Button
                className={
                  status.schedule === "12h"
                    ? "rounded-lg"
                    : "app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                }
                onClick={() => void onScheduleChange("12h")}
                size="sm"
                type="button"
                variant={status.schedule === "12h" ? "default" : "secondary"}
              >
                Every 12 hours
              </Button>
            </div>
          </div>
        </div>

        {scanResult && firstScanSummary ? (
          <Alert variant={scanResult.kind === "error" ? "destructive" : scanResult.kind === "info" ? "neutral" : scanResult.kind}>
            <AlertTitle>{scanResult.kind === "success" ? "First scan finished" : scanResult.title}</AlertTitle>
            <AlertDescription>
              {scanResult.kind === "error"
                ? databaseMissing
                  ? "Grype needs a vulnerability database. Download it before trying the scan again."
                  : "CVEC could not scan any repository. You can retry or open the dashboard to inspect the scan activity."
                : scanResult.kind === "warning"
                  ? `Scanned ${firstScanSummary.reposScanned} of ${firstScanSummary.reposAttempted} repositories. ${firstScanSummary.reposFailed} failed.`
                  : scanResult.description}
            </AlertDescription>
          </Alert>
        ) : null}

        <OnboardingFooter
          backLabel="Back"
          nextLabel={scanResult && scanResult.kind !== "info" ? "Open dashboard" : "Finish setup without scanning"}
          nextVariant={firstScanSucceeded && status.scannerReady && databaseReady && hasRepos ? "default" : "secondary"}
          onBack={goBack}
          onNext={() => void onComplete("overview")}
          secondaryAction={
            <div className="flex flex-wrap gap-2">
              {!status.scannerReady ? (
                <Button onClick={() => openStep("scanner")}>Set up scanner</Button>
              ) : !databaseReady ? (
                <Button onClick={() => openStep("database")}>Prepare database</Button>
              ) : !hasRepos ? (
                <Button onClick={() => openStep("sources")}>Choose repositories</Button>
              ) : (
                <Button
                  className="rounded-lg"
                  disabled={!canRunFirstScan || isRunningFirstScan}
                  onClick={() => void handleRunFirstScan()}
                  type="button"
                  variant={firstScanSucceeded ? "secondary" : "default"}
                >
                  <AppIcon className={isRunningFirstScan || isBusy ? "size-4 motion-safe:animate-spin" : "size-4"} icon={ArrowsClockwise} />
                  {isRunningFirstScan || isBusy ? "Scanning…" : scanResult?.kind === "error" ? "Retry scan" : firstScanSucceeded ? "Scan again" : "Run first scan"}
                </Button>
              )}
              {firstScanSucceeded ? (
                <Button
                  className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
                  onClick={() => void onComplete("review")}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  Open Review
                </Button>
              ) : null}
            </div>
          }
        />
      </OnboardingStepFrame>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden bg-[var(--canvas)]">
      <div
        className="flex h-[var(--shell-topbar-height)] shrink-0 items-center justify-between border-b px-3 pl-[var(--window-controls-inset)] [border-color:var(--hairline)]"
        data-tauri-drag-region="deep"
      >
        <span className="pointer-events-none text-xs font-semibold uppercase tracking-[0.16em] text-ink-subtle">
          CVEC setup
        </span>
        <span className="ml-auto mr-3 text-xs text-ink-subtle lg:hidden">
          Step {currentIndex + 1} of {ONBOARDING_STEPS.length}
        </span>
        <IconButton
          aria-label="Close onboarding"
          data-no-window-drag
          onClick={onClose}
          type="button"
          variant="ghost"
        >
          <AppIcon className="size-4" icon={X} />
        </IconButton>
      </div>

      <div className="grid min-h-0 flex-1 lg:grid-cols-[260px_minmax(0,1fr)]">
        <nav aria-label="Setup steps" className="hidden border-r p-4 [border-color:var(--hairline)] lg:block">
          <div className="rounded-lg border bg-[color:var(--surface-2)] p-3 [border-color:var(--hairline)]">
            <p className="px-2 text-xs font-semibold uppercase tracking-[0.14em] text-ink-subtle">
              Setup path
            </p>
            <div className="mt-3 space-y-1.5">
              {ONBOARDING_STEPS.map((item, index) => {
                const active = item.id === step;
                const complete = stepComplete[item.id];
                return (
                  <Button
                    key={item.id}
                    aria-current={active ? "step" : undefined}
                    className={`h-auto w-full items-start justify-start gap-2 px-2.5 py-2.5 text-left ${
                      active
                        ? "border-[color:var(--primary)]/45 bg-[color:var(--primary-soft)]"
                        : "border-transparent hover:bg-[color:var(--surface-4)]"
                    }`}
                    onClick={() => openStep(item.id)}
                    type="button"
                    variant="ghost"
                  >
                    <span aria-hidden="true"
                      className={`mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full text-xs font-semibold ${
                        active
                            ? "bg-primary text-primary-foreground"
                            : complete
                              ? "bg-[color:var(--status-success)] text-[color:var(--on-success)]"
                            : "bg-[color:var(--surface-3)] text-ink-subtle"
                      }`}
                    >
                      {complete && !active ? <AppIcon className="size-3.5" icon={Check} /> : index + 1}
                    </span>
                    <span className="min-w-0">
                      <span className="block text-xs font-semibold text-ink">
                        {item.label}
                        {complete && !active ? <span className="sr-only">, ready</span> : null}
                      </span>
                      <span className="mt-0.5 block text-xs leading-4 text-ink-subtle">
                        {item.description}
                      </span>
                    </span>
                  </Button>
                );
              })}
            </div>
          </div>
        </nav>

        <section ref={stepContent} className="min-h-0 overflow-auto p-4 lg:p-6">
          <div className="mx-auto flex min-h-full max-w-6xl flex-col justify-center">
            {error ? <Alert className="mb-3" variant="destructive"><AlertTitle>Could not complete the last action</AlertTitle><AlertDescription className="[overflow-wrap:anywhere]">{error}</AlertDescription></Alert> : null}
            {renderStepContent()}
          </div>
        </section>
      </div>
    </div>
  );
}

interface OnboardingStepFrameProps {
  kicker: string;
  title: string;
  description: string;
  children: ReactNode;
}

function OnboardingStepFrame({
  kicker,
  title,
  description,
  children,
}: OnboardingStepFrameProps) {
  return (
    <div className="mx-auto w-full max-w-4xl">
      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-[color:var(--primary)]">
        {kicker}
      </p>
      <h1 tabIndex={-1} className="mt-2 text-3xl font-semibold leading-tight tracking-tight text-ink">
        {title}
      </h1>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-ink-subtle">
        {description}
      </p>
      <div className="mt-5 space-y-3">{children}</div>
    </div>
  );
}

interface OnboardingFooterProps {
  backLabel: string;
  nextLabel: string;
  nextDisabled?: boolean;
  nextVariant?: "default" | "secondary";
  secondaryAction?: ReactNode;
  onBack: () => void;
  onNext: () => void;
}

function OnboardingFooter({
  backLabel,
  nextLabel,
  nextDisabled = false,
  nextVariant = "default",
  secondaryAction,
  onBack,
  onNext,
}: OnboardingFooterProps) {
  return (
    <div className="flex flex-col gap-3 border-t pt-4 [border-color:var(--hairline)] sm:flex-row sm:items-center sm:justify-between">
      <Button
        className="app-surface rounded-lg border text-[color:var(--ink-muted)] shadow-none hover:bg-[color:var(--surface-4)]"
        onClick={onBack}
        size="sm"
        type="button"
        variant="secondary"
      >
        {backLabel}
      </Button>
      <div className="flex flex-wrap gap-2 sm:justify-end">
        {secondaryAction}
        <Button
          className="rounded-lg"
          disabled={nextDisabled}
          onClick={onNext}
          type="button"
          variant={nextVariant}
        >
          {nextLabel}
          <AppIcon className="size-4" icon={ArrowRight} />
        </Button>
      </div>
    </div>
  );
}

interface OnboardingCheckCardProps {
  label: string;
  detail: string;
  ok: boolean;
}

function OnboardingCheckCard({ label, detail, ok }: OnboardingCheckCardProps) {
  return (
    <Card className="gap-2" size="sm">
      <CardContent>
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs font-semibold text-ink">{label}</p>
        <StatusBadge status={ok ? "success" : "warning"}>
          {ok ? "Found" : "Missing"}
        </StatusBadge>
      </div>
      <p className="mt-2 text-sm font-medium text-ink">{detail}</p>
      </CardContent>
    </Card>
  );
}

interface OnboardingStatusTileProps {
  label: string;
  value: string;
  ok: boolean;
  neutral?: boolean;
}

function OnboardingStatusTile({
  label,
  value,
  ok,
  neutral = false,
}: OnboardingStatusTileProps) {
  const dotClass = neutral
    ? "bg-[color:var(--ink-subtle)]"
    : ok
      ? "bg-[color:var(--status-success)]"
      : "bg-[color:var(--status-high)]";

  return (
    <Card className="gap-2" size="sm">
      <CardContent>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium uppercase tracking-[0.12em] text-ink-subtle">
          {label}
        </p>
        <span className={`size-2.5 rounded-full ${dotClass}`} />
      </div>
      <p className="mt-2 text-sm font-semibold text-ink">{value}</p>
      </CardContent>
    </Card>
  );
}
