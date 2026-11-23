import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { FindingRemediationProjection, RepoFinding } from "@/lib/types";
import { FindingInspectorBody } from "./FindingInspectorBody";

const finding: RepoFinding = {
  fingerprint: "fp-markdown",
  advisoryId: "GHSA-markdown",
  packageName: "litellm",
  packageVersion: "1.83.14",
  packageType: "python",
  packageUrl: null,
  severity: "CRITICAL",
  fixVersion: "1.84.0",
  title: "### Impact\n\n**Most deployments are not affected.** Use `Host` validation.\n\n- Add an allowlist",
  sourceUrl: null,
  state: "open",
  intel: null,
  reviewState: "unreviewed",
  reviewedAt: null,
  snoozeUntil: null,
  reviewNote: null,
};

const remediation: FindingRemediationProjection = {
  schemaVersion: 1, repoId: 1, repoName: "cvec", fingerprint: finding.fingerprint,
  advisoryId: finding.advisoryId, packageName: finding.packageName, installedVersion: finding.packageVersion,
  packageType: finding.packageType, packageUrl: finding.packageUrl, scannerReportedTarget: finding.fixVersion,
  evidenceScanRunId: 12, evidenceScannedAt: "2026-08-17T12:00:00Z", sourceSnapshot: { revision: "abcdef1234567890", dirty: true },
  relativePath: "pyproject.toml", locationKind: "manifest", relationship: "unknown", parentPackage: null,
  evidenceQuality: "partial", evidenceSource: "syft_location", locationCount: 1, verificationState: "still_detected",
  verificationScanRunId: 12, verificationScannedAt: "2026-08-17T12:00:00Z", canOpenManifest: true,
  priorityReasons: ["CRITICAL severity", "Fix available"], briefMarkdown: "## CVEC fix brief\n\n- Repository: cvec",
};

describe("FindingInspectorBody", () => {
  it("renders advisory markdown as structured content instead of raw markers", async () => {
    render(<FindingInspectorBody finding={finding} />);
    expect(await screen.findByRole("heading", { name: "Impact" })).toBeInTheDocument();
    expect(screen.getByText("Most deployments are not affected.")).toHaveClass("font-semibold");
    expect(screen.getByText("Host")).toHaveClass("bg-surface-3");
    expect(screen.queryByText(/### Impact/)).not.toBeInTheDocument();
  });

  it("preserves markdown link semantics and only intercepts links when requested", async () => {
    const linkedFinding = { ...finding, title: "Read the [upstream advisory](https://example.com/advisory)." };
    const { rerender } = render(<FindingInspectorBody finding={linkedFinding} />);
    const link = await screen.findByRole("link", { name: "upstream advisory" });

    expect(link).toHaveAttribute("href", "https://example.com/advisory");

    const onOpenUrl = vi.fn();
    rerender(<FindingInspectorBody finding={linkedFinding} onOpenUrl={onOpenUrl} />);
    fireEvent.click(await screen.findByRole("link", { name: "upstream advisory" }));
    expect(onOpenUrl).toHaveBeenCalledWith("https://example.com/advisory");
  });

  it("keeps provenance collapsed and copies the bounded backend brief", async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<FindingInspectorBody finding={finding} remediation={remediation} />);

    expect(screen.getByRole("heading", { name: "Fix plan" })).toBeInTheDocument();
    expect(screen.getByText("Fix plan details").closest("details")).not.toHaveAttribute("open");
    fireEvent.click(screen.getByText("Fix plan details"));
    fireEvent.click(screen.getByRole("button", { name: "Copy fix brief" }));
    expect(writeText).toHaveBeenCalledWith(remediation.briefMarkdown);
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(screen.getByText("pyproject.toml")).toBeInTheDocument();
    expect(screen.getByText("Not proven")).toBeInTheDocument();
    expect(screen.getByText("Evidence scan")).toBeInTheDocument();
    expect(screen.queryByText("Verification scan")).not.toBeInTheDocument();
  });

  it("shows a separate verification scan only when a newer scan supplied it", () => {
    render(<FindingInspectorBody finding={finding} remediation={{ ...remediation, verificationScanRunId: 13, verificationScannedAt: "2026-08-18T12:00:00Z" }} />);
    fireEvent.click(screen.getByText("Fix plan details"));
    expect(screen.getByText("Evidence scan")).toBeInTheDocument();
    expect(screen.getByText("Verification scan")).toBeInTheDocument();
    expect(screen.getByText(/#13/)).toBeInTheDocument();
  });

  it("reports clipboard failure and leaves Review controls outside the plan", async () => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn(async () => { throw new Error("denied"); }) } });
    render(<FindingInspectorBody finding={finding} remediation={remediation} reviewState="unreviewed" />);
    fireEvent.click(screen.getByText("Fix plan details"));
    fireEvent.click(screen.getByRole("button", { name: "Copy fix brief" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not copy");
    expect(screen.queryByRole("button", { name: /review/i })).not.toBeInTheDocument();
  });

  it("retains the one-release rollback rendering path", () => {
    render(<FindingInspectorBody finding={finding} remediation={remediation} fixPlanEnabled={false} />);
    expect(screen.getByRole("heading", { name: "Recommended fix" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy fix brief" })).not.toBeInTheDocument();
  });

  it("opens only the backend-verified manifest action", async () => {
    const onOpenManifest = vi.fn(async () => undefined);
    render(<FindingInspectorBody finding={finding} remediation={remediation} onOpenManifest={onOpenManifest} />);
    fireEvent.click(screen.getByRole("button", { name: "Open manifest" }));
    expect(onOpenManifest).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /review/i })).not.toBeInTheDocument();
  });
});
