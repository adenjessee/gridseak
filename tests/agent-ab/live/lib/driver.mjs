import { spawn } from "node:child_process";
import { cursorAgentBin } from "./paths.mjs";

export function runAgent({
  workspace,
  prompt,
  pluginDir,
  extraEnv = {},
  timeoutMs = 180_000,
}) {
  const bin = cursorAgentBin();
  const args = [
    "-p",
    "--force",
    "--trust",
    "--output-format",
    "json",
    "--workspace",
    workspace,
  ];
  if (pluginDir) {
    args.push("--plugin-dir", pluginDir);
  }
  args.push(prompt);

  return new Promise((resolve) => {
    const child = spawn(bin, args, {
      env: { ...process.env, ...extraEnv },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => {
      child.kill("SIGTERM");
    }, timeoutMs);
    child.stdout.on("data", (b) => {
      stdout += b.toString();
    });
    child.stderr.on("data", (b) => {
      stderr += b.toString();
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({
        code,
        signal,
        stdout,
        stderr,
        parsed: parseAgentJson(stdout),
      });
    });
    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({
        code: 127,
        signal: null,
        stdout,
        stderr: String(err),
        parsed: null,
      });
    });
  });
}

export function parseAgentJson(stdout) {
  const text = stdout.trim();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    const start = text.lastIndexOf("{");
    if (start < 0) return null;
    try {
      return JSON.parse(text.slice(start));
    } catch {
      return null;
    }
  }
}
