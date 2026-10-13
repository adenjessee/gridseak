#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const text = readFileSync(
  resolve(root, "docs/workstreams/master-plan/evidence/12-LIVE_GATE-results.md"),
  "utf8",
);
if (text.includes("**NOT RUN**") || /\| — \| — \| — \| — \|/.test(text)) {
  console.error("results still marked NOT RUN");
  process.exit(1);
}
if (!text.includes("868ba299") || !text.includes("go-01")) {
  console.error("n=1 control/treatment ids missing");
  process.exit(1);
}
console.log("S3 n=1 row present");
