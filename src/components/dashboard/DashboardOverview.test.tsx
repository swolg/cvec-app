import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentType, ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { READY_SNAPSHOT } from "@/test/fixtures";

import { DASHBOARD_STORAGE_KEY, loadDashboardState, saveDashboardState } from "./dashboardStorage";
import { createDefaultDashboardState } from "./dashboardRegistry";
import { DashboardOverview } from "./DashboardOverview";
import type { DashboardDataContext } from "./types";

vi.mock("react-grid-layout/legacy", () => ({
  Responsive: ({
    children,
    containerPadding,
  }: {
    children: ReactNode;
    containerPadding?: [number, number];
  }) => (
    <div
      data-container-padding={containerPadding?.join(",")}
      data-testid="responsive-dashboard-grid"
    >
      {children}
    </div>
  ),
  WidthProvider: (Component: ComponentType) => Component,
}));

vi.mock("./DashboardWidgets", () => ({
  DashboardWidgetRenderer: () => <div data-testid="dashboard-widget" />,
}));

function context(): DashboardDataContext {
  return {
    snapshot: READY_SNAPSHOT,
    riskTrend: { inventoryRevision: 0, day: "2026-08-12", hasCompletedScan: true, read: async () => [] },
    riskTrendRange: 7,
    externalIntelItems: [],
    isScanning: false,
    selectedRepoId: null,
    onAddGithubRepository: () => {},
    onChooseFolder: () => {},
    onRangeChange: () => {},
    onScanAll: () => {},
    onSelectRepo: () => {},
    onOpenReview: () => {},
    onOpenFailedScans: () => {},
  };
}

describe("DashboardOverview", () => {
  it("keeps customization in the page header and exposes a compact edit toolbar", () => {
    render(
      <DashboardOverview
        context={context()}
        renderHeader={(actions) => <header>{actions}</header>}
      />,
    );

    expect(screen.getByRole("button", { name: "Customize" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Dashboard options" })).toBeNull();
    expect(screen.getByTestId("responsive-dashboard-grid")).toHaveAttribute(
      "data-container-padding",
      "0,0",
    );

    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit layout" }));

    expect(screen.getByRole("group", { name: "Dashboard editing controls" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Add widget" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Reset layout" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByRole("button", { name: "Customize" })).toBeVisible();
  });

  it("keeps edits private until Done and cancels the entire editing session", async () => {
    const defaults = createDefaultDashboardState();
    saveDashboardState(defaults);
    const baseline = loadDashboardState(defaults);
    saveDashboardState(baseline);
    render(<DashboardOverview context={context()} renderHeader={(actions) => <header>{actions}</header>} />);
    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit layout" }));
    fireEvent.keyDown(screen.getByRole("button", { name: "Arrange Repos watched" }), { key: "ArrowRight" });
    fireEvent.click(screen.getByRole("button", { name: "Remove Open high" }));
    expect(screen.queryByRole("button", { name: "Remove Open high" })).toBeNull();
    // Waiting past autosave also covers the debounce being disabled in edit mode.
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(JSON.parse(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)!)).toEqual(baseline);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getAllByTestId("dashboard-widget")).toHaveLength(baseline.widgets.length);
    expect(JSON.parse(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)!)).toEqual(baseline);
  });

  it("undoes individual moves, resizes and removals before saving the final layout", async () => {
    const baseline = createDefaultDashboardState();
    render(<DashboardOverview context={context()} renderHeader={(actions) => <header>{actions}</header>} />);
    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit layout" }));
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("button", { name: "Arrange Repos watched" }), { key: "ArrowRight" });
    fireEvent.keyDown(screen.getByRole("button", { name: "Arrange Workspace status" }), { key: "ArrowRight", shiftKey: true });
    fireEvent.click(screen.getByRole("button", { name: "Remove Open high" }));
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.getByRole("button", { name: "Remove Open high" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)).not.toBeNull());
    const saved = JSON.parse(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)!);
    expect(saved.widgets).toEqual(baseline.widgets);
    expect(saved.layouts.lg.find((item: { i: string }) => item.i === "metric-repos").x).toBe(1);
    expect(saved.layouts.lg.find((item: { i: string }) => item.i === "workspace-status").w).toBe(5);
  });

  it("can undo adding from Customize and reset without destroying the saved layout", () => {
    const defaults = createDefaultDashboardState();
    saveDashboardState(defaults);
    const baseline = loadDashboardState(defaults);
    saveDashboardState(baseline);
    render(<DashboardOverview context={context()} renderHeader={(actions) => <header>{actions}</header>} />);
    fireEvent.click(screen.getByRole("button", { name: "Customize" }));
    fireEvent.click(screen.getByRole("button", { name: "Add widget" }));
    fireEvent.click(screen.getByRole("button", { name: /^Risk quadrant Prioritize/ }));
    expect(screen.getByRole("button", { name: "Arrange Risk quadrant" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Reset layout" }));
    expect(screen.queryByRole("button", { name: "Arrange Risk quadrant" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.getByRole("button", { name: "Arrange Risk quadrant" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.queryByRole("button", { name: "Arrange Risk quadrant" })).toBeNull();
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(JSON.parse(window.localStorage.getItem(DASHBOARD_STORAGE_KEY)!)).toEqual(baseline);
  });

});
