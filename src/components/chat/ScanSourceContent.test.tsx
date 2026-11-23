import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ScanSourceContent } from "./ScanSourceContent";

describe("scan source details", () => {
  it("shows recorded scan facts and keeps formatted raw data available on demand", async () => {
    const scan = { status: "completed", revision: "recorded-revision", packageCount: 4, new: 2, freshness: "mismatched", evidenceComplete: false, finishedAt: "2026-11-06T14:00:00Z", findingSummary: { totalOpen: 3, resolvedSincePreviousScan: 1, openBySeverity: { CRITICAL: 1, HIGH: 2 } } };
    render(<ScanSourceContent content={JSON.stringify(scan)} />);
    const summary = screen.getByRole("region", { name: "Scan summary" });
    expect(summary).toHaveTextContent("recorded-revision");
    expect(summary).toHaveTextContent("Revision differs");
    expect(summary).toHaveTextContent("Incomplete");
    expect(within(summary).getByLabelText("Open findings by severity")).toHaveTextContent("CRITICAL1HIGH2");
    const raw = screen.getByLabelText("Raw scan data");
    expect(raw).not.toBeVisible();
    await userEvent.click(screen.getByText("Raw scan data", { selector: "summary" }));
    expect(raw).toBeVisible();
    expect(raw.textContent).toBe(JSON.stringify(scan, null, 2));
  });

  it("does not invent zero counts for absent or invalid evidence", () => {
    render(<ScanSourceContent content={JSON.stringify({ status: "partial", packageCount: -1, findingSummary: { totalOpen: "3" } })} />);
    expect(screen.getAllByText("Not reported").length).toBeGreaterThan(3);
    expect(screen.queryByLabelText("Open findings by severity")).not.toBeInTheDocument();
  });

  it.each(["malformed scan content", "null", "[]"])("preserves unsupported source data: %s", (content) => {
    render(<ScanSourceContent content={content} />);
    expect(screen.getByRole("status")).toHaveTextContent("structured scan summary is unavailable");
    expect(screen.getByLabelText("Raw scan data")).toBeVisible();
  });
});
