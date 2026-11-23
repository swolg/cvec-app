import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

const reviewed = "RUSTSEC-2099-0001";

function cleanReport() {
  return {
    results: [
      { source: { type: "lockfile", path: "package-lock.json" }, packages: [
        { package: { name: "fixture-npm", version: "1.0.0", ecosystem: "npm" } },
        { package: { name: "fixture-transitive", version: "2.0.0", ecosystem: "npm" } },
      ] },
      { source: { type: "lockfile", path: "src-tauri/Cargo.lock" }, packages: [
        { package: { name: "fixture-cargo", version: "1.0.0", ecosystem: "crates.io" } },
      ] },
    ],
  };
}

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "cvec-osv-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const directory of ["bin", "scripts", "security", "src-tauri"]) fs.mkdirSync(path.join(root, directory));
  const check = path.join(root, "scripts/check-osv-results.mjs");
  fs.copyFileSync(new URL("./check-osv-results.mjs", import.meta.url), check);
  fs.copyFileSync(new URL("./license-inventory.mjs", import.meta.url), path.join(root, "scripts/license-inventory.mjs"));
  const npmPackage = (name, version) => ({ version, license: "MIT", resolved: `https://registry.npmjs.org/${name}/-/${name}-${version}.tgz` });
  fs.writeFileSync(path.join(root, "package-lock.json"), JSON.stringify({ packages: {
    "": { version: "1.0.0" },
    "node_modules/fixture-npm": npmPackage("fixture-npm", "1.0.0"),
    "node_modules/fixture-transitive": npmPackage("fixture-transitive", "2.0.0"),
  } }));
  fs.writeFileSync(path.join(root, "src-tauri/Cargo.lock"), 'version = 4\n\n[[package]]\nname = "fixture-cargo"\nversion = "1.0.0"\n');
  fs.writeFileSync(path.join(root, "security/dependency-policy.json"), JSON.stringify({ allowedAdvisories: [reviewed] }));
  const bin = path.join(root, "bin");
  fs.writeFileSync(path.join(bin, "osv-scanner"), `#!/usr/bin/env node
const assert = require("node:assert/strict");
assert.deepEqual(process.argv.slice(2), ["scan", "source", "--lockfile=package-lock.json", "--lockfile=src-tauri/Cargo.lock", "--all-packages", "--format=json"]);
if (process.env.CVEC_TEST_OSV_SIGNAL) process.kill(process.pid, process.env.CVEC_TEST_OSV_SIGNAL);
process.stdout.write(process.env.CVEC_TEST_OSV_REPORT);
process.exitCode = Number(process.env.CVEC_TEST_OSV_EXIT);
`, { mode: 0o755 });

  return ({ stdout = JSON.stringify(cleanReport()), exitCode = 0, signal = "", scannerAvailable = true } = {}) => spawnSync(process.execPath, [check], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: scannerAvailable ? [bin, path.dirname(process.execPath)].join(path.delimiter) : path.join(root, "missing-bin"),
      CVEC_TEST_OSV_REPORT: stdout,
      CVEC_TEST_OSV_EXIT: String(exitCode),
      CVEC_TEST_OSV_SIGNAL: signal,
    },
  });
}

function assertFailed(result, message) {
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, message);
  assert.doesNotMatch(result.stdout, /OSV gate passed/);
}

test("accepts a complete OSV report with zero findings", (t) => {
  const run = fixture(t);
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /both lockfiles; 0 reviewed/);
});

test("rejects empty, malformed and incomplete OSV reports", (t) => {
  const run = fixture(t);
  for (const stdout of ["", "{"]) {
    assertFailed(run({ stdout }), /empty or invalid JSON report/);
  }
  for (const stdout of ["{}", "null", '{"results":[]}']) {
    assertFailed(run({ stdout }), /must include package results for both lockfiles/);
  }

  const partial = cleanReport();
  partial.results.pop();
  assertFailed(run({ stdout: JSON.stringify(partial) }), /missing package results for src-tauri\/Cargo.lock/);

  const emptyPackages = cleanReport();
  emptyPackages.results[1].packages = [];
  assertFailed(run({ stdout: JSON.stringify(emptyPackages) }), /contains no packages for src-tauri\/Cargo.lock/);

  const duplicate = cleanReport();
  duplicate.results[1].source.path = "package-lock.json";
  assertFailed(run({ stdout: JSON.stringify(duplicate) }), /unexpected or duplicate lockfile source/);

  const truncated = cleanReport();
  truncated.results[0].packages.pop();
  assertFailed(run({ stdout: JSON.stringify(truncated) }), /missing 1 locked package/);

  const repeated = cleanReport();
  repeated.results[0].packages[1] = repeated.results[0].packages[0];
  assertFailed(run({ stdout: JSON.stringify(repeated) }), /unexpected or duplicate package/);
});

test("rejects scanner failures even when stdout contains a complete report", (t) => {
  const run = fixture(t);
  assertFailed(run({ exitCode: 127 }), /failed with exit code 127/);
  assertFailed(run({ exitCode: 128, stdout: "" }), /failed with exit code 128/);
  assertFailed(run({ signal: "SIGTERM" }), /terminated by SIGTERM/);
  assertFailed(run({ scannerAvailable: false }), /Could not run OSV-Scanner/);
  assertFailed(run({ exitCode: 1 }), /exit code 1 disagrees with the advisory entries/);
});

test("applies the allowlist to advisory entries and grouped IDs", (t) => {
  const run = fixture(t);
  const report = cleanReport();
  const entry = report.results[1].packages[0];
  entry.vulnerabilities = [{ id: reviewed }];
  entry.groups = [{ ids: [reviewed] }];
  const result = run({ stdout: JSON.stringify(report), exitCode: 1 });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /1 reviewed advisory/);
  assertFailed(run({ stdout: JSON.stringify(report) }), /exit code 0 disagrees with the advisory entries/);

  entry.groups = [{ ids: [reviewed, "GHSA-example-unreviewed"] }];
  assertFailed(run({ stdout: JSON.stringify(report), exitCode: 1 }), /GHSA-example-unreviewed: fixture-cargo@1.0.0/);

  entry.groups = [{ ids: [] }];
  assertFailed(run({ stdout: JSON.stringify(report), exitCode: 1 }), /invalid advisory group/);

  delete entry.groups;
  entry.vulnerabilities = [{ id: "MAL-2099-0001" }];
  assertFailed(run({ stdout: JSON.stringify(report), exitCode: 1 }), /MAL-2099-0001: fixture-cargo@1.0.0/);
});
