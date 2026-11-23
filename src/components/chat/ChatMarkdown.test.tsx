import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import ChatMarkdown, { clearMarkdownCache, getMarkdownCacheMetrics, projectMarkdown } from "./ChatMarkdown";
import { disablePerformanceCounters, enablePerformanceCounters, takePerformanceCounters } from "@/lib/performanceCounters";
import { openExternalUrl } from "@/lib/api";

vi.mock("@/lib/api", () => ({ openExternalUrl: vi.fn(async () => undefined) }));

describe("incremental answer Markdown", () => {
  it("reuses completed Markdown after unmount without parsing it again", () => {
    clearMarkdownCache();
    const text = "## Reopened synthetic answer\n\n| Package | Status |\n| --- | --- |\n| demo | Fictional |";
    const first = render(<ChatMarkdown text={text} />);
    first.unmount();
    enablePerformanceCounters();
    try {
      takePerformanceCounters();
      render(<ChatMarkdown text={text} />);
      expect(screen.getByRole("table")).toHaveTextContent("demoFictional");
      expect(takePerformanceCounters()).toMatchObject({ markdownParses: 0, markdownCacheHits: 1 });
    } finally { disablePerformanceCounters(); clearMarkdownCache(); }
  });

  it("bounds the compiled history cache and does not retain streaming revisions", () => {
    clearMarkdownCache();
    try {
      for (let i = 0; i < 530; i++) render(<ChatMarkdown text={`## Cached fixture ${i}\n\nFictional evidence.`} />).unmount();
      expect(getMarkdownCacheMetrics().entries).toBe(512);
      expect(getMarkdownCacheMetrics().estimatedBytes).toBeLessThanOrEqual(4 * 1024 * 1024);
      clearMarkdownCache();
      for (let i = 0; i < 6; i++) render(<ChatMarkdown text={`Synthetic large fixture ${i}: ${"x".repeat(200_000)}`} />).unmount();
      expect(getMarkdownCacheMetrics().entries).toBeLessThan(6);
      expect(getMarkdownCacheMetrics().estimatedBytes).toBeLessThanOrEqual(4 * 1024 * 1024);
      const before = getMarkdownCacheMetrics();
      const view = render(<ChatMarkdown text="Synthetic stream" streaming />);
      view.rerender(<ChatMarkdown text="Synthetic stream continues" streaming />);
      expect(getMarkdownCacheMetrics()).toEqual(before);
    } finally { clearMarkdownCache(); }
  });

  it("parses a completed multi-section answer once and reuses the AST when rendering blocks", () => {
    const text = Array.from({ length: 20 }, (_, i) => `## Synthetic section ${i}\n\nEvidence for demo-${i}.\n\n`).join("");
    enablePerformanceCounters();
    try {
      takePerformanceCounters();
      const view = render(<ChatMarkdown text={text} streaming />);
      expect(screen.getByRole("heading", { name: "Synthetic section 19" })).toBeInTheDocument();
      expect(takePerformanceCounters()).toMatchObject({ markdownParses: 1, markdownParsedCharacters: text.length });
      view.rerender(<ChatMarkdown text={text} streaming={false} />);
      expect(takePerformanceCounters().markdownParses).toBe(0);
    } finally { disablePerformanceCounters(); }
  });

  it("resolves reference definitions inside a later blockquote across blocks", () => {
    render(<ChatMarkdown text={"[Synthetic advisory][source]\n\n> [source]: https://example.test/synthetic"} />);
    expect(screen.getByRole("link", { name: "Synthetic advisory" })).toHaveAttribute("href", "https://example.test/synthetic");
  });

  it("reconciles reference definitions arriving after previously sealed blocks", () => {
    const prefix = "## Synthetic evidence\n\n[Advisory][source]\n\nLater paragraph.";
    const view = render(<ChatMarkdown text={prefix} streaming />);
    view.rerender(<ChatMarkdown text={`${prefix}\n\n> [source]: https://example.test/synthetic`} streaming />);
    expect(screen.getByRole("link", { name: "Advisory" })).toHaveAttribute("href", "https://example.test/synthetic");
  });

  it("keeps completed blocks and their DOM while a later block streams", () => {
    const first = "## Evidence\n\n- first finding\n- second";
    const projection = projectMarkdown(first);
    const next = projectMarkdown(`${first} finding\n`, projection);
    expect(next.sealed[0]).toBe(projection.sealed[0]);
    const view = render(<ChatMarkdown text={first} />);
    const heading = screen.getByRole("heading", { name: "Evidence" });
    view.rerender(<ChatMarkdown text={`${first} finding\n`} />);
    expect(screen.getByRole("heading", { name: "Evidence" })).toBe(heading);
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("renders security tables, fenced code, and reference links safely", () => {
    const text = "| Package | Fix |\n| --- | --- |\n| demo | 2.0 |\n\n```sh\nnpm update demo\n```\n\n[Advisory][a]\n\n[a]: https://example.test/advisory\n";
    const view = render(<ChatMarkdown text={text} />);
    expect(screen.getByRole("table")).toHaveTextContent("PackageFixdemo2.0");
    expect(screen.getByText("npm update demo")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Advisory" })).toHaveAttribute("href", "https://example.test/advisory");
    view.rerender(<ChatMarkdown text={'<script>alert("unsafe")</script>\n\n[unsafe](javascript:alert(1))'} />);
    expect(view.container.querySelector("script")).toBeNull();
    expect(view.container.querySelector('a[href^="javascript:"]')).toBeNull();
  });

  it("keeps footnote jumps inside the answer and opens external links through the native policy", () => {
    vi.mocked(openExternalUrl).mockClear();
    const view = render(<ChatMarkdown text={"Synthetic evidence[^a].\n\n[External advisory](https://example.test/advisory)\n\n[^a]: Synthetic footnote."} />);
    const fragments = view.container.querySelectorAll<HTMLAnchorElement>('a[href^="#"]');
    expect(fragments).toHaveLength(2);
    for (const link of fragments) expect(fireEvent.click(link)).toBe(true);
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(fireEvent.click(screen.getByRole("link", { name: "External advisory" }))).toBe(false);
    expect(openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://example.test/advisory");
  });

  it("scopes footnote targets and accessible labels to each answer", () => {
    const text = "Synthetic evidence[^a].\n\n[^a]: Synthetic footnote.";
    const view = render(<><section data-testid="first-answer"><ChatMarkdown text={text} /></section><section data-testid="second-answer"><ChatMarkdown text={text} /></section></>);
    const ids = [...view.container.querySelectorAll<HTMLElement>("[id]")].map((node) => node.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const answer of [screen.getByTestId("first-answer"), screen.getByTestId("second-answer")]) {
      const reference = answer.querySelector<HTMLAnchorElement>("[data-footnote-ref]")!;
      for (const link of answer.querySelectorAll<HTMLAnchorElement>('a[href^="#"]')) {
        expect(answer.contains(document.getElementById(decodeURIComponent(link.getAttribute("href")!.slice(1))))).toBe(true);
      }
      expect(answer.contains(document.getElementById(reference.getAttribute("aria-describedby")!))).toBe(true);
    }
  });

  it("keeps formatting in answers longer than 64,000 characters", () => {
    const text = "# Long security report\n\n" + "Evidence remains inspectable. ".repeat(90) + "\n\n";
    render(<ChatMarkdown text={text + Array.from({ length: 30 }, (_, i) => `## Finding ${i}\n\n${"Package evidence. ".repeat(160)}\n\n`).join("")} />);
    expect(screen.getByRole("heading", { name: "Long security report" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Finding 29" })).toBeInTheDocument();
  });

  it.each(["reference", "footnote"] as const)("keeps a long answer formatted when it contains a global %s definition", (kind) => {
    const sections = Array.from({ length: 35 }, (_, i) => `## Referenced finding ${i}\n\n${"Synthetic evidence. ".repeat(110)}\n\n`).join("");
    const reference = kind === "reference" ? "[Advisory][a]\n\n[a]: https://example.test/advisory" : "Evidence[^a]\n\n[^a]: Synthetic footnote";
    const text = sections + reference;
    expect(text.length).toBeGreaterThan(64_000);
    const view = render(<ChatMarkdown text={text} />);
    expect(screen.getByRole("heading", { name: "Referenced finding 34" })).toBeInTheDocument();
    if (kind === "reference") expect(screen.getByRole("link", { name: "Advisory" })).toHaveAttribute("href", "https://example.test/advisory");
    else expect(view.container.querySelector("[data-footnotes]")).toHaveTextContent("Synthetic footnote");
  });

  it("keeps the plain-text guard for an individual oversized block with reference definitions", () => {
    const text = `${"Oversized evidence. ".repeat(4_000)}\n\n[a]: https://example.test/advisory`;
    const view = render(<ChatMarkdown text={text} />);
    expect(view.container.querySelector("pre")).toHaveTextContent("Oversized evidence.");
  });

  it("bounds an oversized streaming tail and resolves its sections on completion", () => {
    const prefix = "## Evidence\n\nShort paragraph";
    const initial = projectMarkdown(prefix, undefined, true);
    const text = `${prefix}${" evidence".repeat(8_000)}\n\n## Verification\n\nRescan locally.`;
    const pending = projectMarkdown(text, initial, true);
    expect(pending.sealed[0]).toBe(initial.sealed[0]);
    expect(pending.blocks).toHaveLength(2);
    const view = render(<ChatMarkdown text={text} streaming />);
    view.rerender(<ChatMarkdown text={text} streaming={false} />);
    expect(screen.getByRole("heading", { name: "Verification" })).toBeInTheDocument();
  });
});
