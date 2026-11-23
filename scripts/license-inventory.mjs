import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const inventoryPath = "security/licenses/inventory.json";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const reviewedIdentifiers = new Set(["0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "BSL-1.0", "BlueOak-1.0.0", "CC0-1.0", "CDLA-Permissive-2.0", "ISC", "LGPL-2.1-or-later", "MIT", "MIT-0", "MPL-2.0", "OFL-1.1", "Unicode-3.0", "Unlicense", "Zlib", "LLVM-exception"]);

export function checkExpression(expression) {
  if (typeof expression !== "string" || !expression.trim()) throw new Error("Missing declared license");
  // Retain declarations verbatim in the inventory, including legacy Cargo '/'.
  const identifiers = expression.replace(/[()/]/g, " ").split(/\s+/).filter((word) => word && !/^(?:AND|OR|WITH)$/i.test(word));
  if (!identifiers.length || identifiers.some((identifier) => !reviewedIdentifiers.has(identifier))) throw new Error(`License declaration needs review: ${expression}`);
  return expression;
}

export function npmComponents(lock) {
  if (!lock.packages?.[""]?.version) throw new Error("Expected an npm packages lockfile");
  return Object.entries(lock.packages).filter(([location]) => location).map(([location, pkg]) => {
    let origin = pkg;
    let bundledIn;
    if (!pkg.resolved && pkg.inBundle === true) {
      let parent = location;
      while (parent.includes("/node_modules/")) {
        parent = parent.slice(0, parent.lastIndexOf("/node_modules/"));
        if (lock.packages[parent]?.resolved) { origin = lock.packages[parent]; bundledIn = parent; break; }
      }
    }
    if (!pkg.version || !origin.resolved?.startsWith("https://registry.npmjs.org/") || (bundledIn && !origin.integrity)) throw new Error(`Unreviewed npm source: ${location}`);
    return { location, name: pkg.name ?? location.slice(location.lastIndexOf("node_modules/") + 13), version: pkg.version, license: checkExpression(pkg.license), source: origin.resolved, integrity: origin.integrity, ...(bundledIn ? { bundledIn } : {}) };
  }).sort((a, b) => a.location.localeCompare(b.location));
}

export function cargoIdentities(lock) {
  return lock.split(/^\[\[package\]\]\s*$/m).slice(1).map((section) => {
    const value = (field) => section.match(new RegExp(`^${field} = "([^"\\n]+)"`, "m"))?.[1];
    const name = value("name"), version = value("version");
    if (!name || !version) throw new Error("Invalid Cargo lockfile package");
    return { name, version, source: value("source") ?? null };
  });
}
const identity = ({ name, version, source }) => JSON.stringify([name, version, source]);

export function buildInventory(npmBytes, cargoBytes, metadata) {
  const identities = cargoIdentities(cargoBytes.toString("utf8"));
  const packages = new Map(metadata.packages.map((pkg) => [identity({ ...pkg, source: pkg.source ?? null }), pkg]));
  const cargo = identities.map((item) => {
    const pkg = packages.get(identity(item));
    if (!pkg) throw new Error(`Cargo metadata missing locked package: ${item.name}@${item.version}`);
    return { ...item, license: checkExpression(pkg.license) };
  }).sort((a, b) => identity(a).localeCompare(identity(b)));
  return { schemaVersion: 1, scope: "Declared licenses for all locked npm installations and Cargo packages, including development and non-macOS dependencies. No dependency source or binaries are bundled in this repository.", lockfiles: { "package-lock.json": hash(npmBytes), "src-tauri/Cargo.lock": hash(cargoBytes) }, npm: npmComponents(JSON.parse(npmBytes)), cargo };
}

export function verifyInventory(inventory, npmBytes, cargoBytes) {
  if (inventory.schemaVersion !== 1 || inventory.lockfiles?.["package-lock.json"] !== hash(npmBytes) || inventory.lockfiles?.["src-tauri/Cargo.lock"] !== hash(cargoBytes)) throw new Error("License inventory is stale; run npm run licenses:generate after reviewing dependency changes");
  if (JSON.stringify(inventory.npm) !== JSON.stringify(npmComponents(JSON.parse(npmBytes)))) throw new Error("npm license inventory does not match the complete lockfile");
  const expected = cargoIdentities(cargoBytes.toString("utf8")).map(identity).sort();
  const actual = inventory.cargo.map((pkg) => { checkExpression(pkg.license); return identity(pkg); }).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error("Cargo license inventory does not cover the complete lockfile");
  if (JSON.stringify(inventory).includes("manifest_path")) throw new Error("Do not publish local Cargo metadata paths");
  return { npmInstallations: inventory.npm.length, cargoPackages: inventory.cargo.length };
}

function main() {
  const action = process.argv[2];
  if (process.argv.length !== 3 || !["generate", "check"].includes(action)) throw new Error("Usage: license-inventory.mjs generate | check");
  const npmBytes = fs.readFileSync(path.join(root, "package-lock.json"));
  const cargoBytes = fs.readFileSync(path.join(root, "src-tauri/Cargo.lock"));
  if (action === "generate") {
    const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1", "--manifest-path", "src-tauri/Cargo.toml"], { cwd: root, maxBuffer: 32 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] }));
    const inventory = buildInventory(npmBytes, cargoBytes, metadata);
    fs.mkdirSync(path.join(root, path.dirname(inventoryPath)), { recursive: true });
    fs.writeFileSync(path.join(root, inventoryPath), `${JSON.stringify(inventory, null, 2)}\n`);
  }
  const inventory = JSON.parse(fs.readFileSync(path.join(root, inventoryPath), "utf8"));
  console.log(JSON.stringify({ status: "passed", ...verifyInventory(inventory, npmBytes, cargoBytes), scope: "source distribution; binary redistribution obligations require a separate release review" }));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
