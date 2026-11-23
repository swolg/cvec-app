import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LOCAL_IDENTITY, signingConfiguration } from "./local-signing.mjs";

const scripts = path.dirname(fileURLToPath(import.meta.url));

test("signing helpers can be imported from stdin without running the CLI", () => {
  const source = `import {LOCAL_IDENTITY} from ${JSON.stringify(pathToFileURL(path.join(scripts, "local-signing.mjs")).href)}; console.log(LOCAL_IDENTITY);`;
  const result = spawnSync(process.execPath, ["--input-type=module", "-"], { input: source, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), LOCAL_IDENTITY);
});
function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "cvec signing fixture ")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "scripts"));
  fs.mkdirSync(path.join(root, "bin"));
  fs.mkdirSync(path.join(root, "src-tauri"));
  fs.writeFileSync(path.join(root, "src-tauri/tauri.conf.json"), JSON.stringify({ identifier: "dev.cvec.fixture" }));
  for (const file of ["cargo-local.sh", "run-signed.sh", "local-signing.mjs"]) fs.copyFileSync(path.join(scripts, file), path.join(root, "scripts", file));
  const env = { ...process.env, PATH: `${path.join(root, "bin")}${path.delimiter}${process.env.PATH}` };
  return { root, env, write(name, text) { const file = path.join(root, name); fs.writeFileSync(file, text, { mode: 0o700 }); return file; } };
}

test("signing keeps a fixed identity and honors an explicit profile identifier", () => {
  assert.equal(signingConfiguration({}).identity, LOCAL_IDENTITY);
  assert.equal(signingConfiguration({}).identifier, "dev.cvec.app");
  assert.deepEqual(signingConfiguration({ APPLE_SIGNING_IDENTITY: "Fixture Release", TAURI_CONFIG: JSON.stringify({ identifier: "dev.cvec.isolated-ui-qa" }) }), { identity: "Fixture Release", identifier: "dev.cvec.isolated-ui-qa" });
  assert.throws(() => signingConfiguration({ APPLE_SIGNING_IDENTITY: "-" }), /persistent certificate/);
  assert.throws(() => signingConfiguration({ TAURI_CONFIG: '{"identifier":"bad id"}' }), /Invalid application/);
});

test("Cargo shim preserves arguments and configures signing for both macOS architectures", (t) => {
  const f = fixture(t);
  f.write("bin/cargo", '#!/bin/sh\nprintf \'%s\\0\' "$@" > "$CVEC_TEST_LOG"\n');
  const log = path.join(f.root, "cargo-arguments");
  const args = ["run", "--manifest-path", "path with spaces/Cargo.toml", "--", "literal $value", "one;two"];
  execFileSync("/bin/sh", [path.join(f.root, "scripts/cargo-local.sh"), ...args], { env: { ...f.env, CVEC_TEST_LOG: log } });
  const seen = fs.readFileSync(log, "utf8").split("\0").slice(0, -1);
  assert.deepEqual(seen.slice(4), args);
  for (const [index, arch] of [[1, "aarch64"], [3, "x86_64"]]) {
    assert.ok(seen[index].startsWith(`target.${arch}-apple-darwin.runner=`));
    assert.deepEqual(JSON.parse(seen[index].slice(seen[index].indexOf("=") + 1)), [path.join(f.root, "scripts/run-signed.sh")]);
  }
  execFileSync("/bin/sh", [path.join(f.root, "scripts/cargo-local.sh"), "build", "--release"], { env: { ...f.env, CVEC_TEST_LOG: log } });
  assert.deepEqual(fs.readFileSync(log, "utf8").split("\0").slice(0, -1), ["build", "--release"]);
});

test("failed signing never launches the application", (t) => {
  const f = fixture(t);
  f.write("bin/node", "#!/bin/sh\nexit 23\n");
  const app = f.write("app", '#!/bin/sh\nprintf launched > "$CVEC_TEST_LOG"\n');
  const marker = path.join(f.root, "app-launched");
  const result = spawnSync("/bin/sh", [path.join(f.root, "scripts/run-signed.sh"), app], { env: { ...f.env, CVEC_TEST_LOG: marker } });
  assert.equal(result.status, 23);
  assert.equal(fs.existsSync(marker), false);
});

test("signed runner preserves application arguments and exit status", (t) => {
  const f = fixture(t);
  f.write("bin/node", "#!/bin/sh\nexit 0\n");
  const app = f.write("app", '#!/bin/sh\nprintf \'%s\\0\' "$@" > "$CVEC_TEST_LOG"\nexit 7\n');
  const marker = path.join(f.root, "arguments");
  const result = spawnSync("/bin/sh", [path.join(f.root, "scripts/run-signed.sh"), app, "a b", "$(literal)"], { env: { ...f.env, CVEC_TEST_LOG: marker } });
  assert.equal(result.status, 7);
  assert.deepEqual(fs.readFileSync(marker, "utf8").split("\0").slice(0, -1), ["a b", "$(literal)"]);
});

test("signed runner exec preserves direct termination of the running application", async (t) => {
  const f = fixture(t);
  f.write("bin/node", "#!/bin/sh\nexit 0\n");
  const app = f.write("app", '#!/bin/sh\nprintf ready > "$CVEC_TEST_LOG"\nexec /bin/sleep 30\n');
  const marker = path.join(f.root, "ready");
  const child = spawn("/bin/sh", [path.join(f.root, "scripts/run-signed.sh"), app], { env: { ...f.env, CVEC_TEST_LOG: marker }, stdio: "ignore" });
  t.after(() => { if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL"); });
  const closed = once(child, "exit");
  const deadline = Date.now() + 3000;
  while (!fs.existsSync(marker)) { assert.ok(Date.now() < deadline, "fixture did not start"); await new Promise((resolve) => setTimeout(resolve, 10)); }
  child.kill("SIGTERM");
  const [code, signal] = await closed;
  assert.equal(code, null);
  assert.equal(signal, "SIGTERM");
});
