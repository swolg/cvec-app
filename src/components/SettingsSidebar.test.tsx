import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SettingsSidebar } from "./SettingsSidebar";

describe("SettingsSidebar", () => {
  it("replaces the app navigation with settings destinations and a return action", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    render(<SettingsSidebar activePage="providers" onBack={onBack} onSelect={onSelect} />);

    expect(screen.getByRole("navigation", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "AI Providers" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("button", { name: "Home" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "General" }));
    expect(onSelect).toHaveBeenCalledWith("general");
    fireEvent.click(screen.getByRole("button", { name: "Back to cvec" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
