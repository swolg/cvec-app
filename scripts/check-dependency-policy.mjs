import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
const policy = JSON.parse(fs.readFileSync(path.join(root, "security/dependency-policy.json"), "utf8"));
const errors = [];
const deniedLifecycleScripts = [];
const exactVersion = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;

if (manifest.dependencies?.react !== manifest.dependencies?.["react-dom"]) {
  errors.push("react and react-dom must use the exact same version");
}

const cargoLock = fs.readFileSync(path.join(root, "src-tauri/Cargo.lock"), "utf8");
const tauriVersion = cargoLock.match(/^name = "tauri"\nversion = "([^"]+)"/m)?.[1];
const tauriApiVersion = lock.packages?.["node_modules/@tauri-apps/api"]?.version;
if (!tauriVersion || !tauriApiVersion || tauriVersion.split(".").slice(0, 2).join(".") !== tauriApiVersion.split(".").slice(0, 2).join(".")) {
  errors.push(`Tauri Rust (${tauriVersion ?? "missing"}) and JavaScript API (${tauriApiVersion ?? "missing"}) must use the same major/minor version`);
}

for (const [name, version] of Object.entries(manifest.dependencies ?? {})) {
  if (!exactVersion.test(version)) errors.push(`${name}: production dependency must use an exact version (${version})`);
  const locked = lock.packages?.[`node_modules/${name}`]?.version;
  if (locked !== version) errors.push(`${name}: package-lock version ${locked ?? "missing"} does not match ${version}`);
}

for (const [location, pkg] of Object.entries(lock.packages ?? {})) {
  if (!location || !pkg?.resolved) continue;
  if (!policy.allowedRegistries.some((registry) => pkg.resolved.startsWith(registry))) {
    errors.push(`${location}: unapproved package source ${pkg.resolved}`);
  }
  if (pkg.hasInstallScript) {
    const identity = `${pkg.name ?? location.replace(/^node_modules\//, "")}@${pkg.version}`;
    if (!policy.allowedLifecycleScripts.includes(identity)) deniedLifecycleScripts.push(identity);
  }
}

for (const identity of policy.allowedLifecycleScripts) {
  const separator = identity.lastIndexOf("@");
  const name = identity.slice(0, separator);
  const version = identity.slice(separator + 1);
  if (!name || !exactVersion.test(version) || lock.packages?.[`node_modules/${name}`]?.version !== version) {
    errors.push(`${identity}: lifecycle exception must name an exact locked package version`);
  }
}

for (const workflow of fs.readdirSync(path.join(root, ".github/workflows")).filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"))) {
  const source = fs.readFileSync(path.join(root, ".github/workflows", workflow), "utf8");
  for (const match of source.matchAll(/^\s*-?\s*uses:\s*[^@\s]+@([^\s#]+)/gm)) {
    if (!new RegExp(`^[0-9a-f]{${policy.actionPinLength}}$`).test(match[1])) errors.push(`${workflow}: mutable action reference @${match[1]}`);
  }
}

if (errors.length) {
  console.error(`Dependency policy failed:\n- ${errors.join("\n- ")}`);
  process.exit(1);
}
console.log(`Dependency policy passed for ${Object.keys(manifest.dependencies ?? {}).length} direct production dependencies; ${deniedLifecycleScripts.length} lifecycle script package(s) remain denied.`);
