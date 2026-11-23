import { test } from "node:test";
import assert from "node:assert/strict";
import { buildInventory, npmComponents, verifyInventory } from "./license-inventory.mjs";

function fixture() {
  const npm = Buffer.from(JSON.stringify({ packages: { "": { version: "0.0.0" }, "node_modules/example": { version: "1.0.0", license: "MIT", resolved: "https://registry.npmjs.org/example/-/example-1.0.0.tgz", integrity: "synthetic" } } }));
  const cargo = Buffer.from('version = 4\n\n[[package]]\nname = "fixture"\nversion = "0.0.0"\n\n[[package]]\nname = "fixture-lib"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n');
  const metadata = { packages: [{ name: "fixture", version: "0.0.0", source: null, license: "MIT", manifest_path: "/private/fixture/Cargo.toml" }, { name: "fixture-lib", version: "1.0.0", source: "registry+https://github.com/rust-lang/crates.io-index", license: "MIT/Apache-2.0", manifest_path: "/private/cache/Cargo.toml" }] };
  return { npm, cargo, metadata };
}

test("license inventory covers every locked identity without publishing local metadata paths", () => {
  const { npm, cargo, metadata } = fixture();
  const inventory = buildInventory(npm, cargo, metadata);
  assert.deepEqual(verifyInventory(inventory, npm, cargo), { npmInstallations: 1, cargoPackages: 2 });
  assert.equal(JSON.stringify(inventory).includes("/private/"), false);
  assert.equal(inventory.cargo.find((pkg) => pkg.name === "fixture-lib").license, "MIT/Apache-2.0");
  assert.throws(() => verifyInventory(inventory, Buffer.concat([npm, Buffer.from("\n")]), cargo), /stale/);
  inventory.cargo.pop();
  assert.throws(() => verifyInventory(inventory, npm, cargo), /complete lockfile/);
});

test("missing licenses, newly unreviewed declarations and alternate package sources block the inventory", () => {
  const { npm, cargo, metadata } = fixture();
  for (const license of [undefined, "LicenseRef-Private", "Commercial-only"]) {
    const lock = JSON.parse(npm); lock.packages["node_modules/example"].license = license;
    assert.throws(() => npmComponents(lock), /license|review/i);
  }
  metadata.packages.pop();
  assert.throws(() => buildInventory(npm, cargo, metadata), /missing locked package/);
  const lock = JSON.parse(npm); lock.packages["node_modules/example"].resolved = "https://unreviewed.example.test/package.tgz";
  assert.throws(() => npmComponents(lock), /Unreviewed npm source/);
});

test("bundled npm packages inherit the nearest verified parent archive without hiding their licenses", () => {
  const { npm } = fixture();
  const lock = JSON.parse(npm);
  lock.packages["node_modules/example/node_modules/bundled"] = { version: "2.0.0", inBundle: true, license: "ISC" };
  const child = npmComponents(lock).find((pkg) => pkg.name === "bundled");
  assert.equal(child.bundledIn, "node_modules/example");
  assert.equal(child.source, lock.packages["node_modules/example"].resolved);
  assert.equal(child.license, "ISC");
  delete lock.packages["node_modules/example"].integrity;
  assert.throws(() => npmComponents(lock), /Unreviewed npm source/);
});
