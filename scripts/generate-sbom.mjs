import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const hash = (value) => createHash("sha256").update(value).digest("hex");
const purl = (type, name, version) => `pkg:${type}/${name.split("/").map(encodeURIComponent).join("/")}@${encodeURIComponent(version)}`;
const envelope = (metadata, components, dependencies) => ({ bomFormat: "CycloneDX", specVersion: "1.6", serialNumber: `urn:uuid:${randomUUID()}`, version: 1, metadata, components, dependencies });

// Each installation has its own identity. Equal purls with different hashes,
// sources or dependency resolution must not be collapsed into one component.
export function buildNpmSbom(lock, provenance = {}) {
  if (!lock.packages?.[""]?.version) throw new Error("Expected an npm packages lockfile with a versioned root");
  const packages = lock.packages;
  const refs = new Map(Object.entries(packages).filter(([key, value]) => key && value.version).map(([key]) => [key, `urn:cvec:npm:${hash(key)}`]));
  function refAt(key, seen = new Set()) {
    if (seen.has(key)) throw new Error(`Cyclic npm link at ${key}`);
    seen.add(key);
    return refs.get(key) ?? (packages[key]?.link ? refAt(packages[key].resolved, seen) : undefined);
  }
  function resolve(from, name) {
    let directory = from;
    for (;;) {
      const ref = refAt(path.posix.join(directory, "node_modules", name));
      if (ref) return ref;
      if (!directory || directory === ".") return undefined;
      directory = path.posix.dirname(directory);
    }
  }
  const components = [...refs].sort(([a], [b]) => a.localeCompare(b)).map(([key, ref]) => {
    const value = packages[key];
    const name = value.name ?? key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    const hashes = (value.integrity?.trim().split(/\s+/) ?? []).map((integrity) => {
      const match = /^(sha1|sha256|sha384|sha512)-([A-Za-z0-9+/]+={0,2})$/.exec(integrity);
      if (!match) throw new Error(`Unsupported npm integrity at ${key}`);
      const bytes = Buffer.from(match[2], "base64");
      const bits = match[1] === "sha1" ? 160 : Number(match[1].slice(3));
      if (bytes.length * 8 !== bits) throw new Error(`Invalid npm integrity at ${key}`);
      return { alg: `SHA-${bits === 160 ? 1 : bits}`, content: bytes.toString("hex") };
    });
    return { type: "library", "bom-ref": ref, name, version: value.version, purl: purl("npm", name, value.version), ...(hashes.length ? { hashes } : {}), properties: [{ name: "cvec:npm-install-path", value: key }, ...(value.resolved ? [{ name: "cvec:npm-resolved", value: value.resolved }] : [])] };
  });
  const rootRef = "urn:cvec:npm:root";
  function dependenciesFor(key, value) {
    const optional = value.optionalDependencies ?? {};
    const required = { ...value.dependencies, ...(key === "" ? value.devDependencies : {}) };
    const peers = value.peerDependencies ?? {};
    const names = { ...required, ...optional, ...peers };
    return [...new Set(Object.keys(names).sort().flatMap((name) => {
      const ref = resolve(key, name);
      const requiredDependency = Object.hasOwn(required, name) && !Object.hasOwn(optional, name);
      const requiredPeer = Object.hasOwn(peers, name) && value.peerDependenciesMeta?.[name]?.optional !== true;
      if (!ref && (requiredDependency || requiredPeer)) throw new Error(`Unresolved npm dependency ${name} from ${key || "root"}`);
      return ref ? [ref] : [];
    }))].sort();
  }
  const bom = envelope({ ...provenance, component: { type: "application", "bom-ref": rootRef, name: packages[""].name ?? lock.name, version: packages[""].version } }, components, [{ ref: rootRef, dependsOn: dependenciesFor("", packages[""]) }, ...[...refs].sort(([a], [b]) => a.localeCompare(b)).map(([key, ref]) => ({ ref, dependsOn: dependenciesFor(key, packages[key]) }))]);
  validateSbom(bom);
  return bom;
}

export function buildCargoSbom(cargo, provenance = {}) {
  const rootId = cargo.resolve?.root;
  const rootPackage = cargo.packages.find((pkg) => pkg.id === rootId);
  if (!rootPackage) throw new Error("Cargo metadata has no resolved root package");
  const refs = new Map(cargo.packages.map((pkg) => [pkg.id, `urn:cvec:cargo:${hash(pkg.id)}`]));
  function component(pkg, type = "library") {
    return { type, "bom-ref": refs.get(pkg.id), name: pkg.name, version: pkg.version, purl: purl("cargo", pkg.name, pkg.version), ...(pkg.license ? { licenses: [{ expression: pkg.license }] } : {}), ...(pkg.source ? { properties: [{ name: "cvec:cargo-source", value: pkg.source }] } : {}) };
  }
  const bom = envelope({ ...provenance, component: component(rootPackage, "application") }, cargo.packages.filter((pkg) => pkg.id !== rootId).sort((a, b) => a.id.localeCompare(b.id)).map((pkg) => component(pkg)), cargo.resolve.nodes.map((node) => ({ ref: refs.get(node.id), dependsOn: [...new Set(node.dependencies.map((id) => { if (!refs.has(id)) throw new Error(`Unresolved Cargo dependency ${id}`); return refs.get(id); }))].sort() })).sort((a, b) => a.ref.localeCompare(b.ref)));
  validateSbom(bom);
  return bom;
}

export function validateSbom(bom) {
  if (bom.bomFormat !== "CycloneDX" || bom.specVersion !== "1.6" || !bom.metadata?.component) throw new Error("Expected a CycloneDX 1.6 BOM with a root component");
  const refs = new Set();
  const identities = new Set();
  function inspect(value) {
    if (!value || typeof value !== "object") return;
    if ("bom-ref" in value) {
      const ref = value["bom-ref"];
      if (typeof ref !== "string" || !ref || identities.has(ref)) throw new Error(`Missing or duplicate bom-ref: ${ref}`);
      identities.add(ref);
    }
    for (const child of Object.values(value)) inspect(child);
  }
  inspect(bom);
  function register(component) {
    const ref = component["bom-ref"];
    if (typeof ref !== "string" || !ref || refs.has(ref)) throw new Error(`Missing or duplicate bom-ref: ${ref}`);
    if (!component.name || !component.version || !["application", "library"].includes(component.type)) throw new Error(`Invalid component: ${ref}`);
    refs.add(ref);
    for (const child of component.components ?? []) register(child);
  }
  register(bom.metadata.component);
  for (const component of bom.components ?? []) register(component);
  const nodes = new Set();
  for (const dependency of bom.dependencies ?? []) {
    if (!refs.has(dependency.ref) || nodes.has(dependency.ref)) throw new Error(`Unresolved or duplicate dependency node: ${dependency.ref}`);
    nodes.add(dependency.ref);
    if (!Array.isArray(dependency.dependsOn) || new Set(dependency.dependsOn).size !== dependency.dependsOn.length || dependency.dependsOn.some((ref) => !refs.has(ref))) throw new Error(`Invalid dependency references from: ${dependency.ref}`);
  }
  if (nodes.size !== refs.size) throw new Error("Every component must have an explicit dependency node");
  return bom;
}

export function provenance(lockfile, repository = root) {
  const file = path.join(repository, lockfile);
  // Never inherit an unrelated parent directory's Git commit when .git is absent.
  const commit = fs.existsSync(path.join(repository, ".git"))
    ? execFileSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).trim()
    : "unavailable";
  return { timestamp: new Date().toISOString(), properties: [
    { name: "cvec:git-commit", value: commit },
    { name: "cvec:lockfile", value: lockfile },
    { name: "cvec:lockfile-sha256", value: hash(fs.readFileSync(file)) },
  ] };
}

export function main(args = process.argv.slice(2)) {
  const output = path.join(root, "artifacts/security");
  if (args.length) {
    if (args[0] !== "--check" || args.length !== 1) throw new Error("Usage: generate-sbom.mjs [--check]");
    for (const name of ["frontend", "rust"]) validateSbom(JSON.parse(fs.readFileSync(path.join(output, `${name}.cdx.json`), "utf8")));
    console.log("Both SBOMs have unique identities and complete, resolved dependency graphs.");
    return;
  }
  const npm = buildNpmSbom(JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8")), provenance("package-lock.json"));
  const cargo = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1", "--manifest-path", "src-tauri/Cargo.toml"], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }));
  const rust = buildCargoSbom(cargo, provenance("src-tauri/Cargo.lock"));
  // Validate both before replacing either artifact.
  fs.mkdirSync(output, { recursive: true });
  for (const [name, bom] of [["frontend", npm], ["rust", rust]]) fs.writeFileSync(path.join(output, `${name}.cdx.json`), `${JSON.stringify(bom, null, 2)}\n`);
  console.log(`Wrote ${npm.components.length} npm and ${rust.components.length} Cargo components to artifacts/security.`);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
