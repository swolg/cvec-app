import fs from "node:fs";

const expectedScenarios = new Set([1_000, 10_000, 50_000].flatMap((count) => [10, 100].map((repos) => `${count}/${repos}`)));
const measurements = [
  "review_page_100", "review_next_page_100", "review_summary", "risk_trend_30",
  "review_one_repo", "repository_overview", "dependencies_80", "search_local_entities",
  "search_advisory_aliases", "persisted_snapshot",
];
const nonnegativeNumber = (value) => Number.isFinite(value) && value >= 0;
const records = fs.readFileSync(process.argv[2], "utf8").trim().split("\n").map((line) => JSON.parse(line));
const timings = process.argv.includes("--timing");
const failures = [];

for (const record of records) {
  const scenario = `${record?.findings}/${record?.repositories}`;
  if (!Number.isSafeInteger(record?.findings) || !Number.isSafeInteger(record?.repositories)
    || !expectedScenarios.delete(scenario) || record.kind !== "synthetic" || !Array.isArray(record.results)) {
    failures.push(`Invalid or duplicate performance scenario ${scenario}`);
    continue;
  }

  const missing = new Set(measurements);
  for (const result of record.results) {
    const operation = result?.operation;
    if (operation === "review_query_plan") continue;
    if (!missing.delete(operation)) {
      failures.push(`${scenario}: unexpected or duplicate operation ${operation}`);
      continue;
    }

    const timeFields = [result.first_call_ms, result.warm_p50_ms, result.warm_p95_ms, result.warm_p99_ms];
    if (!Number.isSafeInteger(result.warm_runs) || result.warm_runs < 30
      || !Number.isSafeInteger(result.payload_bytes) || result.payload_bytes < 1
      || !timeFields.every(nonnegativeNumber) || !Array.isArray(result.samples_ms)
      || result.samples_ms.length !== result.warm_runs || !result.samples_ms.every(nonnegativeNumber)) {
      failures.push(`${scenario}: invalid or incomplete ${operation} measurements`);
      continue;
    }

    const sorted = result.samples_ms.slice().sort((left, right) => left - right);
    const percentilesMatch = [0.5, 0.95, 0.99].every((percentile, index) =>
      sorted[Math.ceil(percentile * sorted.length) - 1] === timeFields[index + 1]);
    if (!percentilesMatch) {
      failures.push(`${scenario}: ${operation} percentiles do not match the samples`);
      continue;
    }

    if (operation === "repository_overview" && result.payload_bytes > 64 * 1024) failures.push(`${scenario}: overview exceeds 64 KiB`);
    if (operation.startsWith("review_") && result.payload_bytes > 160 * 1024) failures.push(`${scenario}: review page exceeds 160 KiB`);
    if (timings && ["review_page_100", "review_next_page_100"].includes(operation) && result.warm_p95_ms > 200) failures.push(`${scenario}: ${operation} p95 exceeds 200 ms`);
    if (timings && record.findings === 10_000 && operation === "review_summary" && result.warm_p95_ms > 50) failures.push(`${scenario}: summary p95 exceeds 50 ms`);
    if (timings && ["search_local_entities", "search_advisory_aliases"].includes(operation) && result.warm_p95_ms > 100) failures.push(`${scenario}: ${operation} p95 exceeds 100 ms`);
  }
  if (missing.size > 0) failures.push(`${scenario}: missing measurements for ${[...missing].join(", ")}`);
}

if (expectedScenarios.size > 0) failures.push(`Missing performance scenarios ${[...expectedScenarios].join(", ")}`);
if (failures.length) throw new Error(failures.join("\n"));
console.log(`Performance gates passed for ${records.length} scenarios (${timings ? "reference-hardware timing and payloads" : "payloads; timings retained for hardware-specific comparison"}).`);
