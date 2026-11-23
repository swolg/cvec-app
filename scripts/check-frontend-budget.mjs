import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const outputIndex = args.indexOf("--output");
const output = outputIndex >= 0 ? args[outputIndex + 1] : undefined;
if (outputIndex >= 0 && !output) throw new Error("--output requires a path");
const directory = path.resolve(args[0] && args[0] !== "--output" ? args[0] : "dist");
const manifest = JSON.parse(fs.readFileSync(path.join(directory, ".vite/manifest.json"), "utf8"));
const entryKey = Object.keys(manifest).find(key => manifest[key].isEntry && manifest[key].file.endsWith(".js"));
const homeKey = Object.keys(manifest).find(key => key.endsWith("components/dashboard/DashboardOverview.tsx"));
if (!entryKey || !homeKey) throw new Error("Build manifest must contain the application entry and Home workspace");

function closure(keys) {
  const files = new Map();
  const seen = new Set();
  function visit(key) {
    if (seen.has(key)) return;
    seen.add(key);
    const chunk = manifest[key];
    if (!chunk) throw new Error(`Missing manifest import: ${key}`);
    const resolved = path.resolve(directory, chunk.file);
    if (!resolved.startsWith(directory + path.sep)) throw new Error("Build asset escapes the output directory");
    if (chunk.file.endsWith(".js")) files.set(chunk.file, fs.statSync(resolved).size);
    for (const dependency of chunk.imports ?? []) visit(dependency);
  }
  for (const key of keys) visit(key);
  return { files: Object.fromEntries([...files].sort()), bytes: [...files.values()].reduce((sum, bytes) => sum + bytes, 0) };
}

const entry = closure([entryKey]);
const home = closure([entryKey, homeKey]);
const result = {
  definition: "Uncompressed emitted JavaScript bytes, exact Vite manifest static-import closures; shared chunks counted once. Home includes entry and Home dependencies. CSS, assets and unopened dynamic workspaces excluded.",
  entry: { ...entry, budgetBytes: 900 * 1024 },
  home: { ...home, budgetBytes: 1024 * 1024 },
};
if (entry.bytes > result.entry.budgetBytes) throw new Error(`Entry JavaScript ${entry.bytes} exceeds 900 KiB`);
if (home.bytes > result.home.budgetBytes) throw new Error(`Home JavaScript ${home.bytes} exceeds 1024 KiB`);
if (output) fs.writeFileSync(output, JSON.stringify(result, null, 2) + "\n");
console.log(JSON.stringify(result, null, 2));
