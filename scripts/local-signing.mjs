import { spawnSync } from "node:child_process";
import { randomBytes, X509Certificate } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const LOCAL_IDENTITY = "CVEC Local Development";
const directory = path.dirname(fileURLToPath(import.meta.url));
const repository = path.dirname(directory);

function command(program, args, options = {}) {
  const result = spawnSync(program, args, { encoding: "utf8", timeout: 45_000, maxBuffer: 1024 * 1024, ...options });
  if (result.error || result.status !== 0) {
    // Never include arguments: setup passes private material to security import.
    throw new Error(`${path.basename(program)} failed: ${result.error?.message ?? result.stderr?.trim() ?? "operation did not complete"}`);
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

export function signingConfiguration(environment = process.env) {
  const base = JSON.parse(fs.readFileSync(path.join(repository, "src-tauri/tauri.conf.json"), "utf8"));
  const overlay = environment.TAURI_CONFIG ? JSON.parse(environment.TAURI_CONFIG) : {};
  const identity = environment.APPLE_SIGNING_IDENTITY || overlay.bundle?.macOS?.signingIdentity || LOCAL_IDENTITY;
  const identifier = overlay.identifier || base.identifier;
  if (!identity.trim() || identity === "-") throw new Error("A persistent certificate signing identity is required. Run npm run signing:setup.");
  if (!/^[A-Za-z0-9][A-Za-z0-9.-]+$/.test(identifier)) throw new Error("Invalid application signing identifier.");
  return { identity, identifier };
}

function requireMac() {
  if (process.platform !== "darwin") throw new Error("Local signing setup requires macOS.");
}

function defaultKeychain() {
  const output = command("/usr/bin/security", ["default-keychain", "-d", "user"]).trim();
  return output.replace(/^"(.*)"$/, "$1");
}

function localCertificates(keychain) {
  const result = spawnSync("/usr/bin/security", ["find-certificate", "-a", "-c", LOCAL_IDENTITY, "-p", keychain], { encoding: "utf8", timeout: 10_000, maxBuffer: 1024 * 1024 });
  if (result.error || (result.status !== 0 && result.status !== 44)) throw new Error("Could not inspect the local development certificate. Unlock the login Keychain and retry.");
  return (result.stdout?.match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g) ?? [])
    .map((pem) => new X509Certificate(pem))
    .filter((certificate) => certificate.subject.split("\n").includes(`CN=${LOCAL_IDENTITY}`));
}

export function setupLocalSigning() {
  requireMac();
  const keychain = defaultKeychain();
  const certificates = localCertificates(keychain);
  if (certificates.length > 1) throw new Error("Multiple CVEC Local Development certificates exist. Select the intended identity explicitly; setup will not replace them.");
  if (certificates.length === 1) {
    if (Date.now() > Date.parse(certificates[0].validTo) || Date.now() < Date.parse(certificates[0].validFrom)) {
      throw new Error("The existing CVEC Local Development certificate is outside its validity period. Setup preserved it; select a valid signing identity explicitly.");
    }
    const identities = command("/usr/bin/security", ["find-identity", "-p", "codesigning", keychain]);
    if (!identities.includes(certificates[0].fingerprint.replaceAll(":", ""))) {
      throw new Error("The CVEC development certificate exists, but its signing private key is unavailable. Setup preserved the certificate; restore the matching key or select another identity.");
    }
    console.log(`Reusing ${LOCAL_IDENTITY}. No certificate or credential was replaced.`);
    return;
  }
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "cvec-signing-setup-"));
  fs.chmodSync(temporary, 0o700);
  try {
    const configuration = path.join(temporary, "certificate.cnf");
    const key = path.join(temporary, "private.pem");
    const certificate = path.join(temporary, "certificate.pem");
    const identity = path.join(temporary, "identity.p12");
    const transportPassword = randomBytes(32).toString("hex");
    fs.writeFileSync(configuration, `[req]\nprompt = no\ndistinguished_name = subject\nx509_extensions = signing\n[subject]\nCN = ${LOCAL_IDENTITY}\n[signing]\nbasicConstraints = critical,CA:false\nkeyUsage = critical,digitalSignature\nextendedKeyUsage = codeSigning\nsubjectKeyIdentifier = hash\nauthorityKeyIdentifier = keyid\n`, { mode: 0o600 });
    command("/usr/bin/openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3650", "-config", configuration, "-keyout", key, "-out", certificate]);
    fs.chmodSync(key, 0o600);
    command("/usr/bin/openssl", ["pkcs12", "-export", "-inkey", key, "-in", certificate, "-name", LOCAL_IDENTITY, "-out", identity, "-passout", "stdin"], { input: `${transportPassword}\n` });
    fs.chmodSync(identity, 0o600);
    console.log(`Installing ${LOCAL_IDENTITY} in your default user Keychain. If macOS asks, approve this new key directly in its system dialog.`);
    command("/usr/bin/security", ["import", identity, "-k", keychain, "-f", "pkcs12", "-P", transportPassword, "-T", "/usr/bin/codesign"]);
    console.log(`Installed ${LOCAL_IDENTITY}; only /usr/bin/codesign was granted access to this new key. Existing credentials and global trust settings were unchanged.`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

export function signExecutable(target, environment = process.env) {
  requireMac();
  if (!target) throw new Error("Pass the executable or app bundle to sign.");
  const { identity, identifier } = signingConfiguration(environment);
  try {
    command("/usr/bin/codesign", ["--force", "--sign", identity, "--identifier", identifier, "--timestamp=none", path.resolve(target)]);
    command("/usr/bin/codesign", ["--verify", "--strict", path.resolve(target)]);
    const requirement = command("/usr/bin/codesign", ["--display", "--requirements", "-", "--verbose=2", path.resolve(target)]);
    if (requirement.includes("Signature=adhoc") || /designated => cdhash/.test(requirement)) throw new Error("The executable still has a build-specific ad-hoc identity.");
    const designated = requirement.split("\n").map((line) => line.replace(/^#\s*/, "")).find((line) => line.startsWith("designated =>"));
    if (!designated) throw new Error("The signed executable has no verifiable designated requirement.");
    return designated;
  } catch (cause) {
    throw new Error(`CVEC was not started because stable signing failed. Run npm run signing:setup; approve ${identity} for codesign in the macOS dialog if requested, then retry. ${cause.message}`);
  }
}

export function signingDoctor() {
  requireMac();
  const { identity, identifier } = signingConfiguration();
  const output = command("/usr/bin/security", ["find-identity", "-p", "codesigning"]);
  if (!output.includes(`"${identity}"`)) throw new Error(`${identity} is not installed. Run npm run signing:setup.`);
  console.log(JSON.stringify({ identity, identifier, status: "identity-present", nextCheck: "npm run signing:smoke" }));
}

export function signatureSmoke() {
  requireMac();
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "cvec-signing-smoke-"));
  try {
    fs.mkdirSync(path.join(temporary, "src"));
    fs.writeFileSync(path.join(temporary, "Cargo.toml"), '[package]\nname = "cvec-signing-fixture"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n');
    const source = path.join(temporary, "src/main.rs");
    const executable = path.join(temporary, "target/debug/cvec-signing-fixture");
    const requirements = [];
    for (const version of [1, 2]) {
      fs.writeFileSync(source, `fn main() { println!("fixture-version-${version}"); }\n`);
      const output = command("/bin/sh", [path.join(directory, "cargo-local.sh"), "run", "--offline", "--manifest-path", path.join(temporary, "Cargo.toml")], { cwd: temporary });
      if (!output.includes(`fixture-version-${version}`)) throw new Error("The signed Cargo fixture did not run.");
      command("/usr/bin/codesign", ["--verify", "--strict", executable]);
      const requirement = command("/usr/bin/codesign", ["--display", "--requirements", "-", executable]);
      if (/designated => cdhash/.test(requirement)) throw new Error("Cargo runner started an ad-hoc fixture.");
      requirements.push(requirement.split("\n").map((line) => line.replace(/^#\s*/, "")).find((line) => line.startsWith("designated =>")));
    }
    if (!requirements[0] || requirements[0] !== requirements[1]) throw new Error("Rebuilt fixtures did not keep the same signing identity.");
    console.log(JSON.stringify({ status: "passed", rebuiltFixtures: 2, cargoRunner: "sign-before-exec", stableDesignatedRequirement: true, cvecAppLaunched: false, applicationCredentialsRead: false }));
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && fs.existsSync(process.argv[1]) && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))) {
  try {
    switch (process.argv[2]) {
      case "setup": setupLocalSigning(); break;
      case "doctor": signingDoctor(); break;
      case "smoke": signatureSmoke(); break;
      case "sign": signExecutable(process.argv[3]); break;
      case "runner-path": console.log(JSON.stringify([path.join(directory, "run-signed.sh")])); break;
      default: throw new Error("Use setup, doctor, smoke, sign <path>, or runner-path.");
    }
  } catch (cause) {
    console.error(cause.message);
    process.exitCode = 1;
  }
}
