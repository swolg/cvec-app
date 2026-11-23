import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { cargoIdentities, npmComponents } from "./license-inventory.mjs";

const root = path.resolve(import.meta.dirname, "..");
const lockfiles = ["package-lock.json", "src-tauri/Cargo.lock"];
const nonemptyString = (value) => typeof value === "string" && value.trim().length > 0;
const packageKey = ({ name, version }) => JSON.stringify([name, version]);

function lockedPackages(file) {
  const contents = fs.readFileSync(path.join(root, file), "utf8");
  const packages = file === "package-lock.json" ? npmComponents(JSON.parse(contents)) : cargoIdentities(contents);
  if (packages.length === 0) throw new Error(`No dependency inventory in ${file}`);
  return new Set(packages.map(packageKey));
}

function packageFindings(entry, source) {
  const { name, version } = entry?.package ?? {};
  if (!nonemptyString(name) || !nonemptyString(version)) {
    throw new Error(`OSV report contains an invalid package entry for ${source}`);
  }

  const { vulnerabilities = [], groups = [] } = entry;
  if (!Array.isArray(vulnerabilities) || !Array.isArray(groups)) {
    throw new Error(`OSV report contains invalid advisory entries for ${source}`);
  }

  const ids = vulnerabilities.map((advisory) => advisory?.id);
  for (const group of groups) {
    if (!Array.isArray(group?.ids) || group.ids.length === 0) {
      throw new Error(`OSV report contains an invalid advisory group for ${source}`);
    }
    ids.push(...group.ids);
  }
  if (!ids.every(nonemptyString)) {
    throw new Error(`OSV report contains an invalid advisory ID for ${source}`);
  }
  return Array.from(new Set(ids), (id) => ({ id, name, version, source }));
}

function reportFindings(report) {
  if (!Array.isArray(report?.results) || report.results.length === 0) {
    throw new Error("OSV report must include package results for both lockfiles");
  }

  const missing = new Map(lockfiles.map((file) => [path.resolve(root, file), file]));
  const findings = [];
  for (const result of report.results) {
    if (result?.source?.type !== "lockfile" || !nonemptyString(result.source.path)) {
      throw new Error("OSV report contains an invalid lockfile source");
    }
    const sourcePath = path.resolve(root, result.source.path);
    const lockfile = missing.get(sourcePath);
    if (!lockfile) throw new Error("OSV report contains an unexpected or duplicate lockfile source");
    if (!Array.isArray(result.packages) || result.packages.length === 0) {
      throw new Error(`OSV report contains no packages for ${lockfile}`);
    }
    missing.delete(sourcePath);
    const expected = lockedPackages(lockfile);
    const ecosystem = lockfile === "package-lock.json" ? "npm" : "crates.io";
    for (const entry of result.packages) {
      findings.push(...packageFindings(entry, lockfile));
      if (entry.package.ecosystem !== ecosystem || !expected.delete(packageKey(entry.package))) {
        throw new Error(`OSV report contains an unexpected or duplicate package in ${lockfile}`);
      }
    }
    if (expected.size > 0) {
      throw new Error(`OSV report is missing ${expected.size} locked package(s) in ${lockfile}`);
    }
  }
  if (missing.size) throw new Error(`OSV report is missing package results for ${[...missing.values()].join(", ")}`);
  return findings;
}

function main() {
  if (process.argv.length !== 2) throw new Error("Usage: npm run osv:check (requires osv-scanner on PATH)");

  const scan = spawnSync("osv-scanner", [
    "scan", "source", ...lockfiles.map((file) => `--lockfile=${file}`), "--all-packages", "--format=json",
  ], { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });

  if (scan.error) throw new Error(`Could not run OSV-Scanner: ${scan.error.message}`);
  if (scan.signal) throw new Error(`OSV-Scanner was terminated by ${scan.signal}`);
  // OSV uses exit 1 for findings. Operational errors must stop before report evaluation.
  if (scan.status !== 0 && scan.status !== 1) throw new Error(`OSV-Scanner failed with exit code ${scan.status}`);

  let report;
  try {
    report = JSON.parse(scan.stdout);
  } catch {
    throw new Error("OSV-Scanner returned an empty or invalid JSON report");
  }
  const findings = reportFindings(report);
  if ((scan.status === 1) !== (findings.length > 0)) {
    throw new Error(`OSV-Scanner exit code ${scan.status} disagrees with the advisory entries in its report`);
  }

  const policy = JSON.parse(fs.readFileSync(path.join(root, "security/dependency-policy.json"), "utf8"));
  const allowed = new Set(policy.allowedAdvisories);
  const unexpected = findings.filter(({ id }) => !allowed.has(id));
  if (unexpected.length) {
    const details = unexpected.map(({ id, name, version, source }) => `- ${id}: ${name}@${version} (${source})`);
    throw new Error(`OSV gate failed:\n${details.join("\n")}`);
  }
  console.log(`OSV gate passed for both lockfiles; ${findings.length} reviewed advisory ID(s) found.`);
}

try {
  main();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
