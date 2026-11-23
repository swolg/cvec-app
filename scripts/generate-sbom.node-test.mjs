import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { buildNpmSbom, buildCargoSbom, validateSbom, provenance } from "./generate-sbom.mjs";

test("a directory without .git does not inherit a parent repository commit", (t) => {
  const directory = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "cvec-sbom-source-")));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  execFileSync("git", ["-c", "core.hooksPath=/dev/null", "init", "-q"], { cwd: directory });
  execFileSync("git", ["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "--allow-empty", "-qm", "Synthetic parent"], { cwd: directory });
  const source = path.join(directory, "source"); fs.mkdirSync(source);
  const lockBytes = Buffer.from("synthetic lock");
  fs.writeFileSync(path.join(source, "package-lock.json"), lockBytes);
  const properties = Object.fromEntries(provenance("package-lock.json", source).properties.map(({ name, value }) => [name, value]));
  assert.equal(properties["cvec:git-commit"], "unavailable");
  assert.equal(properties["cvec:lockfile-sha256"], createHash("sha256").update(lockBytes).digest("hex"));
});

const lock = () => ({ name: "fixture", packages: {
  "": { name: "fixture", version: "1.0.0", dependencies: { a: "^1" }, devDependencies: { "@scope/tool": "^2" } },
  "node_modules/a": { version: "1.1.0", dependencies: { shared: "*" } },
  "node_modules/shared": { version: "3.0.0" },
  "node_modules/@scope/tool": { version: "2.1.0", dependencies: { shared: "*" } },
  "node_modules/@scope/tool/node_modules/shared": { version: "3.0.0", integrity: `sha512-${Buffer.alloc(64, 7).toString("base64")}` },
} });

test("npm retains duplicate installations, hashes, scopes and resolved transitive/dev edges", () => {
  const bom = buildNpmSbom(lock());
  const byPath = Object.fromEntries(bom.components.map((c) => [c.properties[0].value, c]));
  const shared = byPath["node_modules/shared"];
  const nested = byPath["node_modules/@scope/tool/node_modules/shared"];
  assert.equal(shared.purl, nested.purl);
  assert.notEqual(shared["bom-ref"], nested["bom-ref"]);
  assert.equal(nested.hashes[0].content, "07".repeat(64));
  assert.equal(byPath["node_modules/@scope/tool"].purl, "pkg:npm/%40scope/tool@2.1.0");
  const edges = Object.fromEntries(bom.dependencies.map((d) => [d.ref, d.dependsOn]));
  assert.deepEqual(edges[byPath["node_modules/a"]["bom-ref"]], [shared["bom-ref"]]);
  assert.deepEqual(edges[byPath["node_modules/@scope/tool"]["bom-ref"]], [nested["bom-ref"]]);
  assert.equal(edges[bom.metadata.component["bom-ref"]].length, 2);
  const reversed = { ...lock(), packages: Object.fromEntries(Object.entries(lock().packages).reverse()) };
  const reordered = buildNpmSbom(reversed);
  assert.deepEqual(reordered.components, bom.components);
  assert.deepEqual(reordered.dependencies, bom.dependencies);
});

test("npm fails closed on missing required packages or malformed integrity", () => {
  const missing = lock(); delete missing.packages["node_modules/a"];
  assert.throws(() => buildNpmSbom(missing), /Unresolved npm dependency a/);
  const bad = lock(); bad.packages["node_modules/a"].integrity = "sha512-Zm9v";
  assert.throws(() => buildNpmSbom(bad), /Invalid npm integrity/);
});

test("npm distinguishes missing required peers from explicitly optional peers", () => {
  const required = lock();
  required.packages["node_modules/a"].peerDependencies = { missing: "*" };
  assert.throws(() => buildNpmSbom(required), /Unresolved npm dependency missing/);
  const optional = structuredClone(required);
  optional.packages["node_modules/a"].peerDependenciesMeta = { missing: { optional: true } };
  assert.doesNotThrow(() => buildNpmSbom(optional));
  optional.packages["node_modules/a"].dependencies.missing = "*";
  assert.throws(() => buildNpmSbom(optional), /Unresolved npm dependency missing/);
  const special = lock();
  special.packages[""].optionalDependencies = { constructor: "*" };
  assert.doesNotThrow(() => buildNpmSbom(special));
  delete special.packages[""].optionalDependencies;
  special.packages[""].peerDependencies = { constructor: "*" };
  assert.throws(() => buildNpmSbom(special), /Unresolved npm dependency constructor/);
});

test("npm resolves linked workspaces and optional omissions without inventing edges", () => {
  const value = lock();
  value.packages[""].dependencies.workspace = "*";
  value.packages[""].optionalDependencies = { absent: "*" };
  value.packages["node_modules/workspace"] = { link: true, resolved: "packages/workspace" };
  value.packages["packages/workspace"] = { name: "workspace", version: "1", dependencies: { a: "*" } };
  const bom = buildNpmSbom(value);
  assert.equal(bom.dependencies[0].dependsOn.length, 3);
  assert.equal(bom.components.find((c) => c.name === "workspace").purl, "pkg:npm/workspace@1");
});

test("Cargo root appears once and same name/version from different sources keeps distinct identities", () => {
  const cargo = { packages: [
    { id: "root", name: "fixture", version: "1" },
    { id: "registry+dep", name: "dep", version: "2", source: "registry+public" },
    { id: "git+dep", name: "dep", version: "2", source: "git+https://example.test/dep#abc" },
  ], resolve: { root: "root", nodes: [ { id: "root", dependencies: ["registry+dep", "git+dep"] }, { id: "registry+dep", dependencies: [] }, { id: "git+dep", dependencies: [] } ] } };
  const bom = buildCargoSbom(cargo);
  assert.equal(bom.components.length, 2);
  assert.equal(bom.components[0].purl, bom.components[1].purl);
  assert.notEqual(bom.components[0]["bom-ref"], bom.components[1]["bom-ref"]);
  assert.ok(!bom.components.some((c) => c["bom-ref"] === bom.metadata.component["bom-ref"]));
  cargo.resolve.nodes[0].dependencies.push("missing");
  assert.throws(() => buildCargoSbom(cargo), /Unresolved Cargo dependency/);
});

test("validation rejects duplicate roots, dangling refs, duplicate edges and incomplete graphs", () => {
  for (const mutate of [
    (b) => b.components.push(b.metadata.component),
    (b) => b.dependencies[0].dependsOn.push("missing"),
    (b) => b.dependencies[0].dependsOn.push(b.dependencies[0].dependsOn[0]),
    (b) => b.dependencies.pop(),
    (b) => b.dependencies.push(b.dependencies[0]),
    (b) => { b.services = [{ "bom-ref": b.metadata.component["bom-ref"] }]; },
  ]) {
    const bom = buildNpmSbom(lock()); mutate(bom);
    assert.throws(() => validateSbom(bom));
  }
});
