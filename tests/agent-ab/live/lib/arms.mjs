import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { homeDir, REPO_ROOT } from "./paths.mjs";

function writeJson(path, doc) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(doc, null, 2) + "\n");
}

function emptyCursorHooks(doc) {
  const next = structuredClone(doc);
  if (next.hooks && typeof next.hooks === "object") {
    next.hooks.preToolUse = [];
    next.hooks.beforeShellExecution = [];
    next.hooks.PreToolUse = [];
  }
  return next;
}

function emptyClaudeSettings(doc) {
  const next = structuredClone(doc);
  if (next.hooks && typeof next.hooks === "object") {
    next.hooks.PreToolUse = [];
  }
  return next;
}

function emptyPlugin(doc) {
  const next = structuredClone(doc);
  if (next.hooks?.hooks) {
    next.hooks.hooks.preToolUse = [];
    next.hooks.hooks.beforeShellExecution = [];
    next.hooks.hooks.PreToolUse = [];
  } else if (next.hooks) {
    next.hooks.preToolUse = [];
    next.hooks.beforeShellExecution = [];
    next.hooks.PreToolUse = [];
  }
  return next;
}

function backupAndEmptyJson(path, emptyFn, backups) {
  if (!existsSync(path)) return;
  const original = readFileSync(path, "utf8");
  const bak = `${path}.livebak`;
  writeFileSync(bak, original);
  backups.push({ path, bak, original });
  try {
    writeJson(path, emptyFn(JSON.parse(original)));
  } catch {
    writeFileSync(path, original);
  }
}

const STUB_WRAPPER = `#!/usr/bin/env bash
printf '%s\\n' '{"permission":"allow"}'
exit 0
`;

export function prepareArm(arm) {
  if (arm === "treatment") {
    return {
      arm,
      home: homeDir(),
      pluginDir: join(REPO_ROOT, "plugin"),
      extraEnv: {},
      cleanup: () => {},
    };
  }
  if (arm !== "control") {
    throw new Error(`unknown arm: ${arm}`);
  }

  const backups = [];
  backupAndEmptyJson(join(homeDir(), ".cursor", "hooks.json"), emptyCursorHooks, backups);
  backupAndEmptyJson(join(homeDir(), ".claude", "settings.json"), emptyClaudeSettings, backups);
  backupAndEmptyJson(join(REPO_ROOT, "plugin", "plugin.json"), emptyPlugin, backups);
  backupAndEmptyJson(
    join(REPO_ROOT, "plugin", ".cursor-plugin", "plugin.json"),
    emptyPlugin,
    backups,
  );
  backupAndEmptyJson(join(homeDir(), ".cursor", "mcp.json"), (doc) => {
    const next = structuredClone(doc);
    if (next.mcpServers && next.mcpServers.gridseak) {
      delete next.mcpServers.gridseak;
    }
    return next;
  }, backups);

  const wrapper = join(homeDir(), ".cursor", "hooks", "gridseak-gate.sh");
  if (existsSync(wrapper)) {
    const original = readFileSync(wrapper, "utf8");
    const bak = `${wrapper}.livebak`;
    writeFileSync(bak, original);
    writeFileSync(wrapper, STUB_WRAPPER);
    chmodSync(wrapper, 0o755);
    backups.push({ path: wrapper, bak, original });
  }

  const skill = join(homeDir(), ".cursor", "skills", "gridseak-gate");
  if (existsSync(skill)) {
    const moved = `${skill}.liveoff`;
    renameSync(skill, moved);
    backups.push({ path: skill, bak: moved, original: null, restoreDir: true });
  }

  return {
    arm,
    home: homeDir(),
    pluginDir: null,
    extraEnv: {},
    cleanup: () => {
      for (const b of backups) {
        if (b.restoreDir) {
          if (existsSync(b.bak) && !existsSync(b.path)) {
            renameSync(b.bak, b.path);
          }
          continue;
        }
        writeFileSync(b.path, b.original);
        if (b.path.endsWith("gridseak-gate.sh")) {
          chmodSync(b.path, 0o755);
        }
        try {
          renameSync(b.bak, `${b.bak}.done`);
        } catch {
          // keep .livebak
        }
      }
    },
  };
}
