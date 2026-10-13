#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { prepareArm } from "./lib/arms.mjs";
import { runAgent } from "./lib/driver.mjs";
import { corpusRoot, ledgerPath, LIVE_DIR } from "./lib/paths.mjs";
import { writeReport, smokeOk } from "./lib/report.mjs";
import { gitCheckout, readRel, scoreRun, sliceLedger, watchedRels } from "./lib/scorer.mjs";

function parseArgs(argv) {
  const out = {
    tasks: null,
    n: 1,
    arms: ["treatment", "control"],
    out: "target/live-ab",
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    const next = argv[i + 1];
    if (a === "--tasks" && next) {
      out.tasks = next.split(",").map((s) => s.trim()).filter(Boolean);
      i++;
    } else if (a === "--n" && next) {
      out.n = Math.max(1, Number(next));
      i++;
    } else if (a === "--arms" && next) {
      out.arms = next.split(",").map((s) => s.trim()).filter(Boolean);
      i++;
    } else if (a === "--out" && next) {
      out.out = next;
      i++;
    }
  }
  return out;
}

const allTasks = JSON.parse(readFileSync(join(LIVE_DIR, "tasks.json"), "utf8"));
const args = parseArgs(process.argv);
const selected = args.tasks
  ? allTasks.filter((t) => args.tasks.includes(t.id) || args.tasks.includes(t.id.split("-").slice(0, 2).join("-")))
  : allTasks;
if (selected.length === 0) {
  console.error("no tasks matched");
  process.exit(2);
}

const outDir = args.out.startsWith("/") ? args.out : join(process.cwd(), args.out);
mkdirSync(outDir, { recursive: true });

function restoreStaleBackups() {
  const home = process.env.HOME;
  const stale = [
    `${home}/.cursor/hooks.json`,
    `${home}/.claude/settings.json`,
    `${home}/.cursor/mcp.json`,
    `${home}/.cursor/hooks/gridseak-gate.sh`,
    join(process.cwd(), "plugin/plugin.json"),
    join(process.cwd(), "plugin/.cursor-plugin/plugin.json"),
  ];
  for (const p of stale) {
    const bak = `${p}.livebak`;
    if (existsSync(bak)) {
      writeFileSync(p, readFileSync(bak, "utf8"));
    }
  }
  const skill = `${home}/.cursor/skills/gridseak-gate`;
  if (existsSync(`${skill}.liveoff`) && !existsSync(skill)) {
    renameSync(`${skill}.liveoff`, skill);
  }
}

restoreStaleBackups();

const runs = [];
for (const task of selected) {
  const corpus = corpusRoot(task.corpus);
  const rels = watchedRels(task);
  for (let seed = 0; seed < args.n; seed++) {
    for (const armName of args.arms) {
      gitCheckout(corpus, rels);
      const arm = prepareArm(armName);
      try {
        const before = Object.fromEntries(rels.map((rel) => [rel, readRel(corpus, rel)]));
        const sinceIso = new Date().toISOString();
        const ledgerFile = ledgerPath(arm.home);
        const priorLines = existsSync(ledgerFile)
          ? readFileSync(ledgerFile, "utf8").split("\n").length
          : 0;
        const agent = await runAgent({
          workspace: corpus,
          prompt: task.live_prompt,
          pluginDir: arm.pluginDir,
          extraEnv: arm.extraEnv,
        });
        const after = Object.fromEntries(rels.map((rel) => [rel, readRel(corpus, rel)]));
        const ledgerText = existsSync(ledgerFile) ? readFileSync(ledgerFile, "utf8") : "";
        const ledgerRows = sliceLedger(ledgerText, priorLines);
        const score = scoreRun({ task, before, after, ledgerRows, agent });
        const rec = {
          task_id: task.id,
          arm: armName,
          seed,
          score,
          since_iso: sinceIso,
        };
        runs.push(rec);
        writeFileSync(
          join(outDir, `${task.id}-${armName}-${seed}.json`),
          JSON.stringify({ rec, agent: { code: agent.code, parsed: agent.parsed, stderr: agent.stderr } }, null, 2),
        );
        console.error(
          `${task.id} ${armName} seed=${seed} harm=${score.harm} blocked=${score.blocked} ms=${score.duration_ms}`,
        );
      } finally {
        gitCheckout(corpus, rels);
        arm.cleanup();
      }
    }
  }
}

const summary = writeReport(outDir, runs);
if (args.out.includes("live-ab-smoke") && smokeOk(summary)) {
  console.log("LIVE_AB_SMOKE_OK");
} else if (args.out.includes("live-ab-smoke")) {
  console.log("LIVE_AB_SMOKE_FAIL");
  process.exit(1);
} else {
  console.log(`wrote ${join(outDir, "report.json")}`);
}
