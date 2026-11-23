import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

for (const [provider, script] of [
  ["CODEX", "companion-live-smoke.sh"],
  ["GROK", "grok-live-smoke.sh"],
]) {
  test(`${provider} live smoke requires confirmation and an explicit Git root`, (t) => {
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "cvec-live-smoke-")));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    const bin = path.join(root, "bin");
    fs.mkdirSync(bin);
    const repository = "disposable repository";
    execFileSync("git", ["init", "--quiet", "--template=", repository], {
      cwd: root,
      env: { PATH: process.env.PATH, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null" },
    });

    const repositoryVar = `CVEC_LIVE_${provider}_REPOSITORY`;
    const optInVar = `CVEC_RUN_LIVE_${provider}_SMOKE`;
    fs.writeFileSync(path.join(bin, "cargo"), [
      "#!/bin/bash",
      "set -eu",
      `printf '%s\\n' 'cargo stub' "$${repositoryVar}" "$${optInVar}" "$@"`,
      "",
    ].join("\n"), { mode: 0o755 });

    const scriptPath = fileURLToPath(new URL(script, import.meta.url));
    const run = (variables) => spawnSync("/bin/bash", [scriptPath], {
      cwd: root,
      encoding: "utf8",
      env: { PATH: bin, ...variables },
    });
    const confirmation = { CVEC_LIVE_SMOKE_CONFIRM: "1" };
    const validRepository = { [repositoryVar]: repository };
    for (const [variables, message] of [
      [validRepository, /without explicit confirmation/],
      [confirmation, new RegExp(repositoryVar)],
      [{ ...confirmation, [repositoryVar]: "" }, new RegExp(repositoryVar)],
      [{ ...confirmation, [repositoryVar]: "." }, new RegExp(repositoryVar)],
    ]) {
      const result = run(variables);
      assert.equal(result.status, 2, result.stderr);
      assert.match(result.stderr, message);
      assert.equal(result.stdout, "");
    }

    const result = run({ ...confirmation, ...validRepository });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(result.stdout.includes([
      "cargo stub", path.join(root, repository), "1", "test",
      "--manifest-path", "src-tauri/Cargo.toml", "--locked",
    ].join("\n")), result.stdout);
  });
}
