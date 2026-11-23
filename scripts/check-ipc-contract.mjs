import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const rust = [
  "src-tauri/src/core/models.rs",
  "src-tauri/src/core/github_auth.rs",
  "src-tauri/src/core/companion.rs",
  "src-tauri/src/core/fix_workspaces/mod.rs",
]
  .map((file) => fs.readFileSync(path.join(root, file), "utf8"))
  .join("\n");
const typescript = fs.readFileSync(path.join(root, "src/lib/types.ts"), "utf8");

const toCamelCase = (value) =>
  value.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());

const rustStructs = new Map();
for (const match of rust.matchAll(/pub struct (\w+)\s*\{([\s\S]*?)\n\}/g)) {
  const serializedBody = match[2].replace(
    /^\s*#\[serde\(skip\)\]\s*\n\s*pub\s+\w+\s*:[^\n]+$/gm,
    "",
  );
  const fields = [...serializedBody.matchAll(/^\s*pub\s+(\w+)\s*:/gm)].map((field) =>
    toCamelCase(field[1]),
  );
  if (fields.length > 0) {
    rustStructs.set(match[1], fields.sort());
  }
}

const tsInterfaces = new Map();
for (const match of typescript.matchAll(/export interface (\w+)\s*\{([\s\S]*?)\n\}/g)) {
  const fields = [...match[2].matchAll(/^\s*(\w+)\??\s*:/gm)].map((field) => field[1]);
  tsInterfaces.set(match[1], fields.sort());
}

const mismatches = [];
let compared = 0;
for (const [name, rustFields] of rustStructs) {
  const tsFields = tsInterfaces.get(name);
  if (!tsFields) {
    continue;
  }
  compared += 1;
  if (rustFields.join("\n") !== tsFields.join("\n")) {
    const missingInTypescript = rustFields.filter((field) => !tsFields.includes(field));
    const missingInRust = tsFields.filter((field) => !rustFields.includes(field));
    mismatches.push(
      `${name}: missing in TypeScript [${missingInTypescript.join(", ")}], missing in Rust [${missingInRust.join(", ")}]`,
    );
  }
}

if (compared === 0) {
  throw new Error("IPC contract check did not find any matching Rust/TypeScript types");
}
if (mismatches.length > 0) {
  console.error(`IPC contract drift detected:\n${mismatches.join("\n")}`);
  process.exit(1);
}

console.log(`IPC contract fields match for ${compared} shared types.`);
