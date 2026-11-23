import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const checker = fileURLToPath(new URL("./check-performance.mjs", import.meta.url));

function records() {
  const operations = ["review_page_100", "review_next_page_100", "review_summary", "risk_trend_30", "review_one_repo", "repository_overview", "dependencies_80", "search_local_entities", "search_advisory_aliases", "persisted_snapshot"];
  return [1_000, 10_000, 50_000].flatMap((findings) => [10, 100].map((repositories) => ({
    kind: "synthetic", findings, repositories,
    results: operations.map((operation) => ({ operation, payload_bytes: 100, warm_runs: 30,
      first_call_ms: 1, warm_p50_ms: 1, warm_p95_ms: 1, warm_p99_ms: 1, samples_ms: Array(30).fill(1) })),
  })));
}

test("requires all six scenarios and their measured operations", (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cvec-perf-gate-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "matrix.jsonl");
  function check(input) {
    fs.writeFileSync(filename, input.map((record) => JSON.stringify(record)).join("\n"));
    return spawnSync(process.execPath, [checker, filename], { encoding: "utf8" });
  }
  assert.equal(check(records()).status, 0);
  assert.match(check(Array.from({ length: 6 }, () => ({}))).stderr, /missing performance scenarios/i);
  const repeated = records();
  repeated[5] = repeated[0];
  assert.match(check(repeated).stderr, /duplicate performance scenario/);
  const missing = records();
  missing[0].results.pop();
  assert.match(check(missing).stderr, /missing measurements for persisted_snapshot/);
  const truncated = records();
  truncated[0].results[0].samples_ms.pop();
  assert.match(check(truncated).stderr, /incomplete review_page_100 measurements/);
  const inconsistent = records();
  inconsistent[0].results[0].warm_p95_ms = 0;
  assert.match(check(inconsistent).stderr, /percentiles do not match the samples/);
});
