#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const dir = resolve(process.argv[2] || "target/live-ab");
const report = resolve(dir, "report.json");
if (!existsSync(report)) {
  console.error(`missing ${report}`);
  process.exit(1);
}
const doc = JSON.parse(readFileSync(report, "utf8"));
const arms = new Set((doc.rows || []).map((r) => r.arm));
if (!arms.has("treatment") || !arms.has("control")) {
  console.error("report missing both arms");
  process.exit(1);
}
if ((doc.rows || []).length < 2) {
  console.error("report too small");
  process.exit(1);
}
console.log("LIVE_AB_BATCH_OK");
