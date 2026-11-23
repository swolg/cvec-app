import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WorkList } from "./work-list";

describe("WorkList", () => {
  const items = [{ id: "a", name: "Alpha" }, { id: "b", name: "Beta" }];
  const columns = [{ id: "name", label: "Name", sortable: true, render: (item: typeof items[number]) => item.name }];

  it("announces sort and activates headers", async () => {
    const onSort = vi.fn();
    render(<WorkList items={items} columns={columns} getKey={(item) => item.id} label="Findings" selectedKey="a" onSelect={() => {}} sort={{ column: "name", direction: "asc" }} onSort={onSort} />);
    expect(screen.getByRole("columnheader", { name: "Name" })).toHaveAttribute("aria-sort", "ascending");
    await userEvent.click(screen.getByRole("button", { name: "Name" }));
    expect(onSort).toHaveBeenCalledWith("name");
  });

  it("navigates and previews without stealing space from an inner control", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    render(<WorkList items={items} columns={columns} getKey={(item) => item.id} label="Findings" selectedKey="a" onSelect={onSelect} />);
    const row = screen.getByRole("row", { name: /Alpha/ });
    row.focus();
    await user.keyboard("j");
    expect(onSelect).toHaveBeenCalledWith("b", "preview");
    await user.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledWith("b", "open");
  });

  it("restores scroll and dismisses with Escape", () => {
    const onDismiss = vi.fn();
    const { container } = render(<WorkList items={items} columns={columns} getKey={(item) => item.id} label="Findings" selectedKey="a" onSelect={() => {}} initialScrollTop={42} onDismiss={onDismiss} />);
    expect(container.firstElementChild).toHaveProperty("scrollTop", 42);
    fireEvent.keyDown(screen.getByRole("row", { name: /Alpha/ }), { key: "Escape" });
    expect(onDismiss).toHaveBeenCalledOnce();
  });
});
