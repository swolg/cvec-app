import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { RepositoryChatContext } from "@/lib/types";
import { fixSnapshot } from "@/test/fixWorkspaceFixture";
import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters } from "@/lib/performanceCounters";
import { ChatContextPreview } from "./ChatContextPreview";
import ChatMarkdown from "./ChatMarkdown";
import { CopyButton } from "./CopyButton";
import { FixDiffPreview } from "./FixDiffPreview";
import { fixEvidenceSummary, fixWorkspaceLabel } from "./fixWorkspacePresentation";

describe("Chat refinements", () => {
  it("shows actual prepared source paths, line ranges, stale state and omissions without fetching content", async () => {
    const inspect = vi.fn();
    const source = { id: "file:1", kind: "file" as const, label: "Dependency manifest", repositoryRelativePath: "package.json", lineStart: 3, lineEnd: 12, scanId: null, findingFingerprint: null, packageIdentity: null, revision: "abc", stale: true };
    const context: RepositoryChatContext = {
      schemaVersion: 1, repository: { id: 1, name: "Fixture", sourceType: "local", sourceMode: "local_source", approvedRoot: null, branch: "main", revision: "abc" },
      scan: { id: 10, status: "completed", scannedAt: "now", revision: "abc", evidenceComplete: true },
      freshness: { state: "matched", sourceRevision: "abc", scanRevision: "abc", sourceDirty: false, message: "Aligned" },
      attachments: [], sources: [source], omissions: [{ category: "files", count: 2, reason: "Excluded by source policy" }],
      untrustedEvidence: [], instructions: [], estimatedBytes: 512, bounded: true,
    };
    render(<ChatContextPreview context={context} onInspectSource={inspect} />);
    expect(screen.getByText("package.json:3–12")).toBeVisible();
    expect(screen.getByText("Stale or unavailable")).toBeVisible();
    expect(screen.getByLabelText("Context omissions")).toHaveTextContent("2 files: Excluded by source policy");
    expect(inspect).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Inspect source: Dependency manifest" }));
    expect(inspect).toHaveBeenCalledWith(source, screen.getByRole("button", { name: "Inspect source: Dependency manifest" }));
  });

  it("does not mistake successful dependency preparation for passed checks", () => {
    const snapshot = fixSnapshot();
    const check = { action: "npm:prepare", status: "passed", exitCode: 0, output: "Installed", digest: snapshot.diff!.digest, stale: false, finishedAt: "now" };
    snapshot.workspace.checks = [check];
    expect(fixEvidenceSummary(snapshot)).toMatchObject({ checkLabel: "Checks: not run", checkPassed: false });
    snapshot.workspace.checks.push({ ...check, action: "npm:test:run", status: "failed", exitCode: 1 });
    expect(fixEvidenceSummary(snapshot)).toMatchObject({ checkLabel: "Checks: 1 failed", checkWarning: true, checkPassed: false });
    snapshot.workspace.checks.push({ ...check, action: "npm:test:run" });
    expect(fixEvidenceSummary(snapshot)).toMatchObject({ checkLabel: "Checks: passed", checkPassed: true });
    snapshot.diff = null;
    expect(fixEvidenceSummary(snapshot)).toMatchObject({ checkLabel: "Checks: passed · freshness unavailable", checkWarning: true, checkPassed: false });
  });

  it("retains stale failure and rescan evidence independently of workspace readiness", () => {
    const snapshot = fixSnapshot();
    snapshot.workspace.checks = [{ action: "cargo:test", status: "failed", exitCode: 1, output: "Failed", digest: "old", stale: true, finishedAt: "now" }];
    snapshot.workspace.rescan = { findingCount: 0, findings: [], notDetected: 2, digest: "old", stale: true, scannedAt: "now", scannerVersions: "Synthetic" };
    expect(fixEvidenceSummary(snapshot)).toMatchObject({ checkLabel: "Checks: 1 failed · stale", scanLabel: "Rescan: 0 findings · stale", scanWarning: true });
    snapshot.workspace.messages = [{ role: "user", text: "Update the dependency\nPreserve all tests" }];
    expect(fixWorkspaceLabel(snapshot.workspace)).toContain("Update the dependency");
    expect(fixWorkspaceLabel(snapshot.workspace)).toContain(snapshot.workspace.id.slice(0, 8));
  });

  it("counts payload lines that resemble diff headers and copies the exact patch", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    const patch = "diff --git a/demo b/demo\n--- a/demo\n+++ b/demo\n@@ -2,1 +2,1 @@\n--- payload\n+++ payload\n";
    render(<FixDiffPreview file={{ path: "demo", patch, truncated: false }} />);
    expect(screen.getByText("+1 / −1 lines")).toBeVisible();
    expect(screen.getByText("--- payload").parentElement).toHaveAttribute("data-diff-kind", "deletion");
    expect(screen.getByText("+++ payload").parentElement).toHaveAttribute("data-diff-kind", "addition");
    await user.click(screen.getByRole("button", { name: "Copy patch preview: demo" }));
    expect(write).toHaveBeenCalledWith(patch);
    expect(await screen.findByRole("status")).toHaveTextContent("Copied to clipboard.");
  });

  it("bounds highlighted diff rows while preserving access to the full bounded patch", async () => {
    const patch = Array.from({ length: 2005 }, (_, i) => `+line ${i}`).join("\n");
    const { container } = render(<FixDiffPreview file={{ path: "large", patch, truncated: false }} />);
    expect(container.querySelectorAll("[data-diff-kind]")).toHaveLength(2000);
    expect(screen.queryByText("+line 2004")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show full plain patch" }));
    expect(container.querySelector("pre")?.textContent).toBe(patch);
  });

  it("recognizes header-like content in an untracked preview without a hunk marker", () => {
    render(<FixDiffPreview file={{ path: "new", patch: "--- /dev/null\n+++ b/new\n+++ literal\n+another\n", truncated: false }} />);
    expect(screen.getByText("+2 / −0 lines")).toBeVisible();
    expect(screen.getByText("+++ literal").parentElement).toHaveAttribute("data-diff-kind", "addition");
  });

  it("copies code without toolbar text or Markdown fences and does not reparse the answer", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    render(<ChatMarkdown text={'```ts\nconst value = 1;\n```'} />);
    enablePerformanceCounters();
    try {
      takePerformanceCounters();
      await user.click(screen.getByRole("button", { name: "Copy code" }));
      expect(write).toHaveBeenCalledWith("const value = 1;\n");
      expect(takePerformanceCounters().markdownParses).toBe(0);
    } finally { disablePerformanceCounters(); }
  });

  it("lets the keyboard enter a code scroll region between its copy control and the following link", async () => {
    const user = userEvent.setup();
    const code = `const longLine = "${"code-content-".repeat(40)}END";\n`;
    render(<ChatMarkdown text={`\`\`\`ts\n${code}\`\`\`\n\n[Next section](#next)`} />);
    await user.tab();
    expect(screen.getByRole("button", { name: "Copy code" })).toHaveFocus();
    await user.tab();
    const region = screen.getByRole("region", { name: "Code block" });
    expect(region).toHaveFocus();
    expect(region.textContent).toBe(code);
    await user.tab();
    expect(screen.getByRole("link", { name: "Next section" })).toHaveFocus();
  });

  it.each(["Code block", "Response table"])("scrolls an overflowing %s with unmodified arrows and leaves other keys available", async (name) => {
    const user = userEvent.setup();
    render(<ChatMarkdown text={name === "Code block" ? '```ts\nconst longLine = "content";\n```' : "| Package | Fix |\n| --- | --- |\n| example | 2.0 |"} />);
    const region = screen.getByRole("region", { name });
    Object.defineProperties(region, {
      scrollWidth: { configurable: true, value: 2_000 },
      clientWidth: { configurable: true, value: 300 },
    });
    region.focus();
    await user.keyboard("{ArrowRight}");
    expect(region.scrollLeft).toBeGreaterThan(0);
    await user.keyboard("{ArrowLeft}");
    expect(region.scrollLeft).toBe(0);

    for (const input of [{ key: "ArrowDown" }, { key: "ArrowRight", altKey: true }, { key: "ArrowRight", metaKey: true }, { key: "ArrowRight", ctrlKey: true }, { key: "ArrowRight", shiftKey: true }]) {
      const event = new KeyboardEvent("keydown", { ...input, bubbles: true, cancelable: true });
      act(() => { region.dispatchEvent(event); });
      expect(event.defaultPrevented).toBe(false);
      expect(region.scrollLeft).toBe(0);
    }

    Object.defineProperty(region, "scrollWidth", { value: 300 });
    const fitting = new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true });
    act(() => { region.dispatchEvent(fitting); });
    expect(fitting.defaultPrevented).toBe(false);
  });

  it("keeps table links in the tab order without hijacking keys from a focused link", async () => {
    const user = userEvent.setup();
    render(<ChatMarkdown text={"| Package | Reference |\n| --- | --- |\n| example | [Details](#details) |\n\n[After table](#after)"} />);
    const region = screen.getByRole("region", { name: "Response table" });
    Object.defineProperties(region, {
      scrollWidth: { value: 2_000 }, clientWidth: { value: 300 },
    });
    await user.tab();
    expect(region).toHaveFocus();
    await user.tab();
    const details = screen.getByRole("link", { name: "Details" });
    expect(details).toHaveFocus();
    const arrow = new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true });
    act(() => { details.dispatchEvent(arrow); });
    expect(arrow.defaultPrevented).toBe(false);
    expect(region.scrollLeft).toBe(0);
    await user.tab();
    expect(screen.getByRole("link", { name: "After table" })).toHaveFocus();
  });

  it("reports clipboard failure, supports retry and clears success for changed content", async () => {
    const user = userEvent.setup();
    const write = vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("Denied")).mockResolvedValue();
    const view = render(<CopyButton label="Copy fixture" text="first" />);
    await user.click(screen.getByRole("button", { name: "Copy fixture" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Copy failed.");
    expect(screen.getByRole("button", { name: "Copy fixture" })).toHaveTextContent("Retry copy");
    await user.click(screen.getByRole("button", { name: "Copy fixture" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Copied to clipboard."));
    view.rerender(<CopyButton label="Copy fixture" text="second" />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(write).toHaveBeenCalledTimes(2);
  });

  it("ignores an old clipboard completion after the source changes", async () => {
    const user = userEvent.setup();
    let finish!: () => void;
    vi.spyOn(navigator.clipboard, "writeText").mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    const view = render(<CopyButton label="Copy fixture" text="old" />);
    await user.click(screen.getByRole("button", { name: "Copy fixture" }));
    view.rerender(<CopyButton label="Copy fixture" text="new" />);
    await act(async () => finish());
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy fixture" })).toBeEnabled();
  });
});
