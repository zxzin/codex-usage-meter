#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bridgePath = join(root, "scripts", "claude-statusline.mjs");
const claudeDir = process.env.CLAUDE_CONFIG_DIR || join(os.homedir(), ".claude");
const settingsPath = join(claudeDir, "settings.json");
const stateDir = resolveStateDir();
const backupPath = join(stateDir, "claude-statusline-backup.json");
const bridgeCommand = `node "${bridgePath.replaceAll("\"", "\\\"")}"`;

mkdirSync(claudeDir, { recursive: true });
mkdirSync(stateDir, { recursive: true });
chmodSync(bridgePath, 0o755);

const settings = readSettings();
const previousStatusLine = settings.statusLine;
const alreadyInstalled = statusLineCommand(previousStatusLine)?.includes("claude-statusline.mjs");

if (previousStatusLine && !alreadyInstalled) {
  writeFileSync(
    backupPath,
    `${JSON.stringify({
      installedAt: new Date().toISOString(),
      statusLine: previousStatusLine,
    }, null, 2)}\n`,
    "utf8",
  );
}

settings.statusLine = {
  type: "command",
  command: bridgeCommand,
  refreshInterval: 1,
  padding: typeof previousStatusLine?.padding === "number" ? previousStatusLine.padding : 0,
};

writeFileSync(settingsPath, `${JSON.stringify(settings, null, 2)}\n`, "utf8");

console.log("Claude Code Token Meter bridge installed.");
console.log(`Settings: ${settingsPath}`);
console.log(`Bridge: ${bridgePath}`);
console.log(`Meter state: ${process.env.CLAUDE_TOKEN_METER_STATE || process.env.CLAUDE_USAGE_METER_STATE || join(stateDir, "claude-status.json")}`);
if (previousStatusLine && !alreadyInstalled) {
  console.log(`Previous statusLine backed up and chained from: ${backupPath}`);
}

function readSettings() {
  if (!existsSync(settingsPath)) {
    return {};
  }

  try {
    return JSON.parse(readFileSync(settingsPath, "utf8"));
  } catch (error) {
    console.error(`Cannot parse Claude settings at ${settingsPath}: ${error.message}`);
    console.error("Fix the JSON file, then run this installer again.");
    process.exit(1);
  }
}

function statusLineCommand(statusLine) {
  return typeof statusLine?.command === "string" ? statusLine.command : null;
}

function resolveStateDir() {
  if (process.env.TOKEN_METER_HOME) {
    return process.env.TOKEN_METER_HOME;
  }
  if (process.env.CODEX_USAGE_METER_HOME) {
    return process.env.CODEX_USAGE_METER_HOME;
  }
  if (process.env.USAGE_METER_HOME) {
    return process.env.USAGE_METER_HOME;
  }
  const next = join(os.homedir(), ".token-meter");
  const legacy = join(os.homedir(), ".codex-usage-meter");
  return existsSync(legacy) && !existsSync(next) ? legacy : next;
}
