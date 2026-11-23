import { readFile, readdir } from "node:fs/promises";
import path from "node:path";

const root = process.cwd();
const srcRoot = path.join(root, "src");
const strict = process.argv.includes("--strict");

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const absolute = path.join(directory, entry.name);
      return entry.isDirectory() ? walk(absolute) : absolute;
    }),
  );
  return nested.flat();
}

const sourceFiles = (await walk(srcRoot)).filter((file) =>
  /\.(?:css|tsx?|html)$/.test(file) &&
  !file.includes(`${path.sep}test${path.sep}`) &&
  !file.includes(".test."),
);
const contents = await Promise.all(
  sourceFiles.map(async (file) => ({
    file: path.relative(root, file),
    text: await readFile(file, "utf8"),
  })),
);
const combined = contents.map(({ text }) => text).join("\n");

const definitions = new Set(
  [...combined.matchAll(/(--[\w-]+)\s*:/g)].map((match) => match[1]),
);
// Base UI's popup positioner defines this variable at runtime.
definitions.add("--available-height");
const references = new Set(
  [...combined.matchAll(/var\((--[\w-]+)/g)].map((match) => match[1]),
);
const undefinedReferences = [...references]
  .filter((token) => !definitions.has(token))
  .sort();
const legacyReferences = [...combined.matchAll(/var\((--(?:tone|app)-[\w-]+)/g)]
  .map((match) => match[1]);
const arbitraryTextSizes = [...combined.matchAll(/text-\[(?:\d+(?:\.\d+)?(?:px|rem)|0\.625rem)\]/g)];
const arbitraryRadii = [...combined.matchAll(/rounded-\[[^\]]+\]/g)];
const rawButtons = [...combined.matchAll(/<button\b/g)];
const oldDefaultWidgets = [
  "metric-repos",
  "metric-new",
  "metric-critical",
  "metric-high",
  "workspace-status",
  "risk-radar-metrics",
  "intel-timeline",
  "ecosystem-heatmap",
  "cvss-epss-quadrant",
  "kev-watch",
  "top-cwe",
  "risk-trend",
  "repository-table",
];
const registry = contents.find(({ file }) =>
  file.endsWith("components/dashboard/dashboardRegistry.ts"),
)?.text ?? "";
const oldDefaultWidgetMatches = oldDefaultWidgets.filter((id) =>
  registry.includes(`id: "${id}"`) || registry.includes(`"${id}"`),
);

const report = {
  files: sourceFiles.length,
  tokenDefinitions: definitions.size,
  tokenReferences: references.size,
  undefinedReferences,
  legacyReferenceCount: legacyReferences.length,
  arbitraryTextSizeCount: arbitraryTextSizes.length,
  arbitraryRadiusCount: arbitraryRadii.length,
  rawButtonCount: rawButtons.length,
  oldDefaultWidgetMatches: oldDefaultWidgetMatches.length,
};

console.log(JSON.stringify(report, null, 2));

const hasUnexpectedInventory =
  undefinedReferences.length > 0 ||
  legacyReferences.length > 0 ||
  arbitraryTextSizes.length > 0 ||
  arbitraryRadii.length > 1 ||
  rawButtons.length > 4;

if (strict && hasUnexpectedInventory) {
  console.error("UI inventory failed: the semantic UI contract has regressed.");
  process.exitCode = 1;
}
