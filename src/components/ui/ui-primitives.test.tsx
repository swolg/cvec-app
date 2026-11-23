import * as React from "react";
import axe from "axe-core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { Alert, AlertDescription, AlertTitle, InlineNotice } from "./alert";
import { StatusBadge } from "./badge";
import { Button, IconButton } from "./button";
import { Card, CardContent, CardTitle, Panel, Section } from "./card";
import { EmptyState, Skeleton, Spinner } from "./feedback";
import { Input, SearchField, SegmentedControl, Select, Tabs } from "./form-controls";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "./overlays";
import { toast, ToastProvider } from "./toast";

function PrimitiveStateFixture({ theme }: { theme: "dark" | "light" }) {
  const [appearance, setAppearance] = React.useState("system");
  return (
    <main data-theme={theme} className="grid gap-4 bg-canvas p-4 text-ink">
      <Section aria-labelledby={`${theme}-section-title`}>
        <h2 id={`${theme}-section-title`}>Primitive states</h2>
        <Panel className="grid gap-3 p-3">
          <div className="flex flex-wrap gap-2">
            <Button>Primary</Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="ghost">Tertiary</Button>
            <Button variant="destructive">Delete</Button>
            <Button disabled>Disabled</Button>
            <Button loading loadingLabel="Saving">Save</Button>
            <IconButton aria-label="Close fixture" />
          </div>
          <div className="flex flex-wrap gap-2">
            <StatusBadge status="critical">Critical</StatusBadge>
            <StatusBadge status="warning">High</StatusBadge>
            <StatusBadge status="success">Ready</StatusBadge>
          </div>
        </Panel>
      </Section>
      <Card>
        <CardTitle>{theme} card</CardTitle>
        <CardContent className="grid gap-3 sm:grid-cols-2">
          <Input label={`${theme} repository`} description="Local workspace root" />
          <SearchField label={`${theme} search`} />
          <Select label={`${theme} severity`} defaultValue="high">
            <option value="high">High</option>
            <option value="critical">Critical</option>
          </Select>
          <SegmentedControl
            label={`${theme} appearance`}
            value={appearance}
            onValueChange={setAppearance}
            options={[
              { value: "system", label: "System" },
              { value: "dark", label: "Dark" },
              { value: "light", label: "Light" },
            ]}
          />
        </CardContent>
      </Card>
      <Alert variant="warning"><AlertTitle>Scanner unavailable</AlertTitle><AlertDescription>Install the scanner.</AlertDescription></Alert>
      <InlineNotice variant="success"><AlertTitle>Saved</AlertTitle><AlertDescription>Preferences are local.</AlertDescription></InlineNotice>
      <EmptyState title="No repositories" description="Add one to begin." action={<Button variant="secondary">Add repository</Button>} />
      <div className="flex items-center gap-3"><Skeleton className="h-4 w-24" /><Spinner label="Refreshing fixture" /></div>
    </main>
  );
}

describe("shared UI primitives", () => {
  it("provides semantic headings, statuses, notices, and async text", () => {
    render(<main>
      <Card><CardTitle>Workspace</CardTitle></Card>
      <StatusBadge status="critical">Critical</StatusBadge>
      <Alert variant="warning"><AlertTitle>Scanner unavailable</AlertTitle><AlertDescription>Install the scanner.</AlertDescription></Alert>
      <EmptyState title="No repositories" description="Add one to begin." />
      <Skeleton className="h-4" />
      <Spinner label="Refreshing repositories" />
    </main>);
    expect(screen.getByRole("heading", { name: "Workspace", level: 3 })).toBeInTheDocument();
    expect(screen.getByText("Critical")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Scanner unavailable");
    expect(screen.getByText("Refreshing repositories")).toHaveClass("sr-only");
  });

  it("exposes loading and disabled button state with desktop target classes", () => {
    render(<><Button loading loadingLabel="Saving">Save</Button><IconButton aria-label="Close" disabled /></>);
    expect(screen.getByRole("button", { name: /saving save/i })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Close" })).toHaveClass("size-10");
  });

  it("labels fields and segmented choices with validation and disabled reasons", async () => {
    const user = userEvent.setup();
    function Fixture() {
      const [theme, setTheme] = React.useState("system");
      return <><Input label="Repository URL" error="Enter a valid URL" /><SegmentedControl label="Appearance" value={theme} onValueChange={setTheme} options={[{ value: "system", label: "System" }, { value: "dark", label: "Dark" }]} /></>;
    }
    render(<Fixture />);
    expect(screen.getByLabelText("Repository URL")).toHaveAttribute("aria-invalid", "true");
    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(screen.getByRole("radio", { name: "Dark" })).toHaveAttribute("aria-checked", "true");
  });

  it("renders selects as styled listboxes with keyboard-accessible options", async () => {
    const user = userEvent.setup();
    function Fixture() {
      const [severity, setSeverity] = React.useState("high");
      return <Select label="Severity" value={severity} onValueChange={setSeverity}>
        <option value="high">High</option>
        <option value="critical">Critical</option>
      </Select>;
    }
    render(<Fixture />);
    const trigger = screen.getByRole("combobox", { name: "Severity" });
    expect(trigger).toHaveTextContent("High");
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(trigger).toHaveTextContent("Critical");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("exposes section tabs with a single programmatic selection", async () => {
    const user = userEvent.setup();
    function Fixture() {
      const [tab, setTab] = React.useState("findings");
      return <Tabs label="Repository details" value={tab} onValueChange={setTab} options={[{ value: "findings", label: "Findings" }, { value: "packages", label: "Packages" }]} />;
    }
    render(<Fixture />);
    expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("tab", { name: "Packages" }));
    expect(screen.getByRole("tab", { name: "Packages" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Findings" })).toHaveAttribute("tabindex", "-1");
  });

  it("traps modal interaction, closes on Escape, and restores trigger focus", async () => {
    const user = userEvent.setup();
    render(<Dialog><DialogTrigger render={<Button />}>Open details</DialogTrigger><DialogContent><DialogTitle>Finding details</DialogTitle><Button>Resolve</Button></DialogContent></Dialog>);
    const trigger = screen.getByRole("button", { name: "Open details" });
    await user.click(trigger);
    expect(screen.getByRole("dialog", { name: "Finding details" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("stops closed dialog layers from swallowing the next page click", async () => {
    const user = userEvent.setup();
    render(<Dialog><DialogTrigger render={<Button />}>Open details</DialogTrigger><DialogContent><DialogTitle>Finding details</DialogTitle></DialogContent></Dialog>);
    await user.click(screen.getByRole("button", { name: "Open details" }));
    const viewport = screen.getByRole("dialog", { name: "Finding details" }).parentElement;
    const backdrop = viewport?.previousElementSibling;
    expect(viewport).toHaveClass("data-[closed]:pointer-events-none", "data-[ending-style]:pointer-events-none");
    expect(backdrop).toHaveClass("data-[closed]:pointer-events-none", "data-[ending-style]:pointer-events-none");
  });

  it("dismisses popovers with Escape and exposes tooltip text", async () => {
    const user = userEvent.setup();
    render(<TooltipProvider><Popover><PopoverTrigger render={<Button />}>Open review</PopoverTrigger><PopoverContent><PopoverTitle>Review</PopoverTitle></PopoverContent></Popover><Tooltip><TooltipTrigger render={<Button />}>Help</TooltipTrigger><TooltipContent>Scanner status details</TooltipContent></Tooltip></TooltipProvider>);
    const popoverTrigger = screen.getByRole("button", { name: "Open review" });
    await user.click(popoverTrigger);
    expect(screen.getByText("Review")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByText("Review")).not.toBeInTheDocument();
    expect(popoverTrigger).toHaveFocus();
    await user.hover(screen.getByRole("button", { name: "Help" }));
    expect(await screen.findByText("Scanner status details")).toBeInTheDocument();
  });

  it("announces and dismisses shared toasts", async () => {
    const user = userEvent.setup();
    render(<ToastProvider><Button onClick={() => toast({ title: "Saved", description: "Layout updated", kind: "success" })}>Notify</Button></ToastProvider>);
    await user.click(screen.getByRole("button", { name: "Notify" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(screen.getByText("Layout updated")).toBeInTheDocument();
    const dismiss = document.querySelector('[aria-label="Dismiss notification"]');
    expect(dismiss).toBeInstanceOf(HTMLButtonElement);
    await user.click(dismiss as HTMLButtonElement);
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
  });

  it("has no obvious accessibility violations in the state fixture", async () => {
    const { container } = render(<main><Button>Continue</Button><Input label="Folder" description="Local workspace root" /><StatusBadge status="success">Ready</StatusBadge></main>);
    const results = await axe.run(container);
    expect(results.violations).toEqual([]);
  });

  it.each(["dark", "light"] as const)(
    "renders the complete %s primitive-state fixture without accessibility violations",
    async (theme) => {
      const { container } = render(<PrimitiveStateFixture theme={theme} />);
      expect(screen.getByRole("heading", { name: `${theme} card` })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Disabled" })).toBeDisabled();
      const results = await axe.run(container);
      expect(results.violations).toEqual([]);
    },
  );
});
