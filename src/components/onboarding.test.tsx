import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  WelcomeOnboarding,
  type WelcomeOnboardingContext,
} from "@/components/WelcomeOnboarding";
import { EMPTY_SNAPSHOT, READY_SNAPSHOT } from "@/test/fixtures";

function createContext(
  overrides: Partial<WelcomeOnboardingContext> = {},
): WelcomeOnboardingContext {
  return {
    snapshot: EMPTY_SNAPSHOT,
    githubDisabledReason: "Scanner setup required",
    isScanning: false,
    isUpdatingVulnerabilityDb: false,
    onAddGithubRepository: vi.fn(async () => undefined),
    onChooseFolder: vi.fn(async () => null),
    onClose: vi.fn(),
    onComplete: vi.fn(async () => undefined),
    onGithubAuthChange: vi.fn(),
    onRefresh: vi.fn(async () => null),
    onScheduleChange: vi.fn(async () => null),
    onScanAll: vi.fn(async () => null),
    onSkip: vi.fn(async () => undefined),
    onUpdateVulnerabilityDb: vi.fn(async () => null),
    ...overrides,
  };
}

describe("onboarding flow", () => {
  it("communicates blocked scanner and source progression without hiding actions", async () => {
    const user = userEvent.setup();
    const context = createContext();
    render(<WelcomeOnboarding context={context} />);

    expect(
      screen.getByRole("heading", { name: "Scan your repositories for vulnerabilities" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Set up cvec/ }));
    expect(screen.getByRole("heading", { name: "Check Syft and Grype" })).toHaveFocus();
    expect(screen.getByText("Scanner setup required")).toBeInTheDocument();
    expect(screen.getByText("brew install syft grype")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Refresh checks/ }));
    expect(context.onRefresh).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: /Sources/ }));
    expect(screen.getByRole("button", { name: /Database/ })).not.toHaveTextContent("ready");
    expect(screen.getByRole("button", { name: /Sources/ })).toHaveAttribute("aria-current", "step");
    expect(screen.getByText("No repositories selected yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: /First scan/ }));
    expect(screen.getByRole("button", { name: "Set up scanner" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: /Run first scan/ })).not.toBeInTheDocument();
  });

  it("supports skip, ready-state progress, first scan success, and both completion routes", async () => {
    const user = userEvent.setup();
    const context = createContext({
      snapshot: READY_SNAPSHOT,
      githubDisabledReason: null,
      onScanAll: vi.fn(async () => ({
        startedAt: "2026-08-12T08:00:00Z",
        finishedAt: "2026-08-12T08:00:01Z",
        reposAttempted: 1,
        reposScanned: 1,
        reposFailed: 0,
        alertsCreated: 1,
      })),
    });
    render(<WelcomeOnboarding context={context} />);

    await user.click(screen.getByRole("button", { name: "Skip for now" }));
    expect(context.onSkip).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: /Sources/ }));
    expect(screen.queryByText("No repositories selected yet")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    const scanButton = screen.getByRole("button", { name: /Run first scan/ });
    expect(scanButton).toBeEnabled();
    await user.click(scanButton);
    expect(await screen.findByText("First scan finished")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open Review" }));
    expect(context.onComplete).toHaveBeenCalledWith("review");
    await user.click(screen.getByRole("button", { name: "Open dashboard" }));
    expect(context.onComplete).toHaveBeenCalledWith("overview");
  });

  it("takes the first scan back to the missing database with the setup escape still available", async () => {
    const user = userEvent.setup();
    const context = createContext({ snapshot: {
      ...READY_SNAPSHOT,
      versionInfo: { ...READY_SNAPSHOT.versionInfo, grypeDb: null },
    } });
    render(<WelcomeOnboarding context={context} />);
    await user.click(screen.getByRole("button", { name: /First scan/ }));
    expect(screen.getByRole("button", { name: "Finish setup without scanning" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Prepare database" }));
    expect(screen.getByRole("heading", { name: "Prepare the vulnerability database" })).toHaveFocus();
    expect(context.onComplete).not.toHaveBeenCalled();
    expect(context.onScanAll).not.toHaveBeenCalled();
  });

  it("offers recovery when every repository fails its first scan", async () => {
    const user = userEvent.setup();
    const onScanAll = vi.fn(async () => ({
      startedAt: "2026-08-12T08:00:00Z",
      finishedAt: "2026-08-12T08:00:01Z",
      reposAttempted: 1,
      reposScanned: 0,
      reposFailed: 1,
      alertsCreated: 0,
    }));
    render(<WelcomeOnboarding context={createContext({ snapshot: READY_SNAPSHOT, onScanAll })} />);

    await user.click(screen.getByRole("button", { name: /First scan/ }));
    await user.click(screen.getByRole("button", { name: "Run first scan" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Scan failed");
    expect(screen.queryByRole("button", { name: "Open Review" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry scan" }));
    expect(onScanAll).toHaveBeenCalledTimes(2);
  });
});
