import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { ChatComposer, type ChatComposerProps } from "./ChatComposer";

const props: ChatComposerProps = {
  draft: "", selection: { repositoryId: 1, connectionId: "codex_default", modelId: null, effort: null },
  models: [], connections: [], selectionLoading: false, threadUsage: null, providerUsage: null,
  contextControls: null, contextChips: null, statusMessage: "", placeholder: "Draft fixture", disabled: false,
  sending: false, canSend: false, historyCachedOnly: false, retryableError: false,
  onDraftChange: vi.fn(), onSelectModel: vi.fn(), onEffortChange: vi.fn(), onSend: vi.fn(), onRetry: vi.fn(), onStop: vi.fn(),
};

function ComposerFixture({ initial = "", ...overrides }: Partial<ChatComposerProps> & { initial?: string }) {
  const [draft, setDraft] = useState(initial);
  return <><ChatComposer {...props} {...overrides} draft={draft} onDraftChange={setDraft} /><button onClick={() => setDraft("")}>Reset fixture draft</button></>;
}

function pasteNativeMultiline(input: HTMLElement, clipboardText: string, valueAfterPaste: string) {
  // WebKit normalizes textarea line endings before enforcing maxLength.
  // user-event instead clips the raw clipboard before normalizing it.
  fireEvent.paste(input, { clipboardData: { getData: () => clipboardText } });
  fireEvent.change(input, { target: { value: valueAfterPaste } });
}

describe("ChatComposer input and context", () => {
  it("shortens an oversized Unicode paste safely, explains the change and clears the notice with a new draft", async () => {
    const user = userEvent.setup();
    render(<ComposerFixture />);
    const input = screen.getByRole("textbox");
    await user.click(input);
    await user.paste("A".repeat(65_535) + "👋");
    expect(input).toHaveValue("A".repeat(65_535));
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("Text was shortened to fit the input limit.");
    expect(input.getAttribute("aria-describedby")?.split(" ")).toContain(notice.id);
    await user.click(screen.getByRole("button", { name: "Reset fixture draft" }));
    expect(input).toHaveValue("");
    expect(screen.queryByText("Text was shortened to fit the input limit.")).not.toBeInTheDocument();
  });

  it("preserves an emoji fitting exactly at the bound without a shortening notice", async () => {
    const user = userEvent.setup();
    render(<ComposerFixture />);
    const input = screen.getByRole("textbox");
    const text = "B".repeat(65_534) + "👋";
    await user.click(input);
    await user.paste(text);
    expect(input).toHaveValue(text);
    expect(screen.queryByText("Text was shortened to fit the input limit.")).not.toBeInTheDocument();
  });

  it("accounts for replaced text when checking a smaller Fix-style input bound", async () => {
    const user = userEvent.setup();
    render(<ComposerFixture initial="abcdefgh" maxLength={8} />);
    const input = screen.getByRole("textbox") as HTMLTextAreaElement;
    await user.click(input);
    input.setSelectionRange(2, 6);
    await user.paste("👋xy");
    expect(input).toHaveValue("ab👋xygh");
    expect(screen.queryByText("Text was shortened to fit the input limit.")).not.toBeInTheDocument();
  });

  it.each(["\r\n", "\r"])("counts normalized %j line endings before warning about a fitting paste", async (lineEnding) => {
    const user = userEvent.setup();
    render(<ComposerFixture maxLength={6} />);
    const input = screen.getByRole("textbox");
    await user.click(input);
    pasteNativeMultiline(input, `a${lineEnding}b${lineEnding}cd`, "a\nb\ncd");
    expect(input).toHaveValue("a\nb\ncd");
    expect(screen.queryByText("Text was shortened to fit the input limit.")).not.toBeInTheDocument();
  });

  it("still warns when normalized multiline content exceeds the input limit", async () => {
    const user = userEvent.setup();
    render(<ComposerFixture maxLength={6} />);
    const input = screen.getByRole("textbox");
    await user.click(input);
    pasteNativeMultiline(input, "a\r\nb\r\ncd!", "a\nb\ncd");
    expect(input).toHaveValue("a\nb\ncd");
    expect(screen.getByRole("status")).toHaveTextContent("Text was shortened to fit the input limit.");
  });

  it("counts normalized pasted text against the replaced selection", async () => {
    const user = userEvent.setup();
    render(<ComposerFixture initial="abcdefgh" maxLength={8} />);
    const input = screen.getByRole("textbox") as HTMLTextAreaElement;
    await user.click(input);
    input.setSelectionRange(2, 6);
    pasteNativeMultiline(input, "X\r\nYZ", "abX\nYZgh");
    expect(input).toHaveValue("abX\nYZgh");
    expect(screen.queryByText("Text was shortened to fit the input limit.")).not.toBeInTheDocument();
  });

  it("exposes a keyboard context region and lets a selected attachment be removed without changing the draft", async () => {
    const user = userEvent.setup();
    function ManyContexts() {
      const [attachments, setAttachments] = useState(Array.from({ length: 32 }, (_, index) => index + 1));
      return <ComposerFixture initial="Keep this draft" contextChips={<>{attachments.map((id) => <button key={id} onClick={() => setAttachments((current) => current.filter((entry) => entry !== id))}>Remove file {id}</button>)}</>} />;
    }
    render(<ManyContexts />);
    const region = screen.getByRole("region", { name: "Attached context" });
    expect(within(region).getAllByRole("button")).toHaveLength(32);
    await user.tab();
    expect(region).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: /^Remove file 1$/ })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(within(region).getAllByRole("button")).toHaveLength(31);
    expect(screen.getByRole("textbox")).toHaveValue("Keep this draft");
  });
});
