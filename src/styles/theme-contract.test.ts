import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const projectRoot = process.cwd();
const srcRoot = path.join(projectRoot, "src");
const primitives = readFileSync(path.join(srcRoot, "styles/primitives.css"), "utf8");
const themes = readFileSync(path.join(srcRoot, "styles/themes.css"), "utf8");

function themeDeclarations(theme: "dark" | "light"): string {
  return themes.split(`:root[data-theme="${theme}"] {`)[1]?.split("\n}")[0] ?? "";
}

function themeColors(theme: "dark" | "light") {
  const declarations = `${primitives}\n${themeDeclarations(theme)}`;
  const colors = new Map(
    [...declarations.matchAll(/(--[\w-]+):\s*(#[\da-f]{6}|var\((--[\w-]+)\));/gi)].map(
      ([, token, value, reference]) => [token, reference ?? value],
    ),
  );

  function color(token: string): string {
    const value = colors.get(token);
    if (!value) throw new Error(`Missing color ${token} for the ${theme} theme`);
    return value.startsWith("--") ? color(value) : value;
  }

  return color;
}

function walk(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = path.join(directory, entry.name);
    return entry.isDirectory() ? walk(absolute) : absolute;
  });
}

function sourceFiles(): Array<{ file: string; text: string }> {
  return walk(srcRoot)
    .filter(
      (file) =>
        /\.(?:css|tsx?)$/.test(file) &&
        !file.includes(`${path.sep}test${path.sep}`) &&
        !file.includes(".test."),
    )
    .map((file) => ({ file, text: readFileSync(file, "utf8") }));
}

function luminance(hex: string): number {
  const channels = hex
    .replace("#", "")
    .match(/.{2}/g)!
    .map((channel) => Number.parseInt(channel, 16) / 255)
    .map((channel) =>
      channel <= 0.04045
        ? channel / 12.92
        : ((channel + 0.055) / 1.055) ** 2.4,
    );
  return (
    channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
  );
}

function contrast(foreground: string, background: string): number {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

describe("theme contract", () => {
  it("defines every referenced custom property", () => {
    const combined = sourceFiles()
      .map(({ text }) => text)
      .join("\n");
    const definitions = new Set(
      [...combined.matchAll(/(--[\w-]+)\s*:/g)].map((match) => match[1]),
    );
    // Base UI Popover.Positioner supplies this measured viewport bound at runtime.
    definitions.add("--available-height");
    const references = new Set(
      [...combined.matchAll(/var\((--[\w-]+)/g)].map((match) => match[1]),
    );
    expect([...references].filter((token) => !definitions.has(token))).toEqual(
      [],
    );
  });

  it("keeps primitive palette consumption inside the theme mapping", () => {
    const violations = sourceFiles()
      .filter(({ file }) => !file.endsWith("styles/themes.css"))
      .filter(({ text }) => text.includes("var(--primitive-"))
      .map(({ file }) => path.relative(projectRoot, file));
    expect(violations).toEqual([]);
  });

  it("uses one data-theme source instead of duplicate dark-mode blocks", () => {
    const css = sourceFiles()
      .filter(({ file }) => file.endsWith(".css"))
      .map(({ text }) => text)
      .join("\n");
    expect(css).not.toMatch(/\.dark\s*\{/);
    expect(css).not.toMatch(/prefers-color-scheme:\s*dark/);
    expect(css).toContain(':root[data-theme="dark"]');
    expect(css).toContain(':root[data-theme="light"]');
  });

  it("keeps product primary separate from security severity", () => {
    expect(themes).not.toMatch(
      /--severity-(?:critical|high|medium|low):\s*var\(--primary\)/,
    );
    expect(themes).not.toMatch(/--primary:\s*var\(--severity-/);
  });

  it("keeps light shell roles explicit instead of collapsing every region to gray", () => {
    const lightTheme = themeDeclarations("light");

    expect(primitives).toContain("--primitive-light-canvas: #f3f2ef");
    expect(primitives).toContain("--primitive-light-surface-1: #fdfcf9");
    expect(lightTheme).toContain("--navigation-surface: var(--surface-1)");
    expect(lightTheme).toContain("--chart-surface: var(--surface-1)");
    expect(lightTheme).toContain("--row-selected-surface: #e8e7fa");
    expect(lightTheme).toContain("--navigation-active-surface: #e5e5f9");
    expect(lightTheme).not.toMatch(
      /--row-selected-surface:\s*var\(--surface-[234]\)/,
    );
  });
});

describe.each(["dark", "light"] as const)("%s theme contrast", (theme) => {
  const color = themeColors(theme);
  const normalTextPairs = [
    ["--ink", "--canvas"],
    ["--ink-muted", "--surface-1"],
    ["--ink-subtle", "--surface-3"],
    ["--ink-tertiary", "--surface-4"],
    ["--ink", "--navigation-active-surface"],
    ["--on-primary", "--primary"],
    ["--on-primary", "--primary-hover"],
    ["--primary", "--surface-4"],
    ["--severity-critical", "--surface-4"],
    ["--severity-high", "--surface-4"],
    ["--severity-medium", "--surface-4"],
    ["--severity-low", "--surface-4"],
    ["--status-success", "--surface-4"],
  ];

  it.each(normalTextPairs)(
    "%s on %s reaches 4.5:1 for normal text",
    (foreground, background) => {
      expect(contrast(color(foreground), color(background))).toBeGreaterThanOrEqual(4.5);
    },
  );

  it.each(["--canvas", "--surface-4"])("focus ring on %s reaches 3:1", (background) => {
    expect(contrast(color("--primary-focus"), color(background))).toBeGreaterThanOrEqual(3);
  });
});
