import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AppNotification } from "@/lib/types";

import { NotificationBell } from "./NotificationBell";

const notification: AppNotification = {
  id: 1,
  kind: "scan_alert",
  tone: "warning",
  title: "New critical finding",
  message: "A repository needs attention.",
  repoId: 7,
  repoName: "cvec",
  createdAt: "2026-08-15T07:00:00Z",
};

describe("NotificationBell", () => {
  it("anchors the unread badge to the bell without clipping it", () => {
    render(
      <NotificationBell
        isClearing={false}
        isOpen={false}
        notifications={[notification]}
        onClear={vi.fn()}
        onClearAll={vi.fn()}
        onClose={vi.fn()}
        onToggle={vi.fn()}
      />,
    );

    const trigger = screen.getByRole("button", { name: "Notifications, 1 unread" });
    const badge = within(trigger).getByText("1");

    expect(trigger).toHaveClass("relative", "overflow-visible");
    expect(badge).toHaveClass("absolute", "-right-1", "-top-1");
    expect(badge).toHaveAttribute("aria-hidden", "true");
  });
});
