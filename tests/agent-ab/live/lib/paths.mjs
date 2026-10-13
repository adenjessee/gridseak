import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const LIB_DIR = dirname(fileURLToPath(import.meta.url));
export const LIVE_DIR = resolve(LIB_DIR, "..");
export const REPO_ROOT = resolve(LIVE_DIR, "../../..");

export function corpusRoot(corpus) {
  const root = join(REPO_ROOT, ".cache", "g1", corpus);
  if (!existsSync(root)) {
    throw new Error(`corpus missing: ${root}`);
  }
  return root;
}

export function homeDir() {
  const h = process.env.HOME;
  if (!h) throw new Error("HOME is unset");
  return h;
}

export function ledgerPath(home = homeDir()) {
  return join(home, ".gridseak", "gate.jsonl");
}

export function cursorAgentBin() {
  const fromEnv = process.env.CURSOR_AGENT;
  if (fromEnv && fromEnv.includes("/") && existsSync(fromEnv)) {
    return fromEnv;
  }
  const fallback = join(homeDir(), ".local", "bin", "cursor-agent");
  if (!existsSync(fallback)) {
    throw new Error(`cursor-agent not found at ${fallback}`);
  }
  return fallback;
}
