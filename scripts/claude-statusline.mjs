#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync, openSync, closeSync, unlinkSync, renameSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import os from "node:os";

const STATE_VERSION = 1;
const MAX_EVENTS_PER_SESSION = 300;
const MAX_SESSION_AGE_SECONDS = 7 * 24 * 60 * 60;
const LOCK_RETRIES = 24;
const LOCK_SLEEP_MS = 12;

const rawInput = readStdin();
const input = parseJson(rawInput);
const stateDir = resolveStateDir();
const statePath = process.env.CLAUDE_TOKEN_METER_STATE || process.env.CLAUDE_USAGE_METER_STATE || join(stateDir, "claude-status.json");
const backupPath = join(stateDir, "claude-statusline-backup.json");

try {
  if (input) {
    mkdirSync(stateDir, { recursive: true });
    withLock(`${statePath}.lock`, () => recordStatusLineInput(input));
  }
} catch (error) {
  writeDiagnostic(error);
}

const previousOutput = runPreviousStatusLine(rawInput);
if (previousOutput != null) {
  process.stdout.write(previousOutput);
} else if (process.env.CLAUDE_TOKEN_METER_SILENT !== "1" && process.env.CLAUDE_USAGE_METER_SILENT !== "1") {
  process.stdout.write(defaultStatusLine(input));
}

function recordStatusLineInput(data) {
  const now = Math.floor(Date.now() / 1000);
  const state = readState();
  const sessionId = stringValue(data.session_id) || stringValue(data.transcript_path) || "claude-session";
  const current = state.sessions[sessionId] ?? {
    id: sessionId,
    totalObservedTokens: 0,
    events: [],
  };
  const usageTokens = usageTokenTotal(data.context_window?.current_usage);
  const signature = usageSignature(data);

  current.id = sessionId;
  current.sessionName = stringValue(data.session_name) || null;
  current.cwd = stringValue(data.workspace?.current_dir) || stringValue(data.cwd) || null;
  current.projectDir = stringValue(data.workspace?.project_dir) || null;
  current.transcriptPath = stringValue(data.transcript_path) || null;
  current.model = stringValue(data.model?.display_name) || stringValue(data.model?.id) || null;
  current.updatedAt = now;
  current.rateLimits = normalizeRateLimits(data.rate_limits);

  if (usageTokens > 0 && signature && signature !== current.lastSignature) {
    current.lastSignature = signature;
    current.lastUsageTokens = usageTokens;
    current.lastEventAt = now;
    current.totalObservedTokens = positiveInteger(current.totalObservedTokens) + usageTokens;
    current.events = [...(Array.isArray(current.events) ? current.events : []), {
      ts: now,
      tokens: usageTokens,
      signature,
    }].slice(-MAX_EVENTS_PER_SESSION);
  }

  state.sessions[sessionId] = current;
  state.version = STATE_VERSION;
  state.updatedAt = now;
  pruneSessions(state, now);
  atomicWriteJson(statePath, state);
}

function readState() {
  if (!existsSync(statePath)) {
    return { version: STATE_VERSION, updatedAt: 0, sessions: {} };
  }

  try {
    const parsed = JSON.parse(readFileSync(statePath, "utf8"));
    return {
      version: parsed.version ?? STATE_VERSION,
      updatedAt: parsed.updatedAt ?? 0,
      sessions: parsed.sessions && typeof parsed.sessions === "object" ? parsed.sessions : {},
    };
  } catch {
    return { version: STATE_VERSION, updatedAt: 0, sessions: {} };
  }
}

function normalizeRateLimits(rateLimits) {
  if (!rateLimits || typeof rateLimits !== "object") {
    return null;
  }

  return {
    fiveHour: normalizeLimit(rateLimits.five_hour),
    sevenDay: normalizeLimit(rateLimits.seven_day),
  };
}

function normalizeLimit(limit) {
  if (!limit || typeof limit !== "object") {
    return null;
  }

  return {
    usedPercent: numberOrNull(limit.used_percentage),
    resetsAt: integerOrNull(limit.resets_at),
  };
}

function usageTokenTotal(usage) {
  if (!usage || typeof usage !== "object") {
    return 0;
  }

  return [
    usage.input_tokens,
    usage.output_tokens,
    usage.cache_creation_input_tokens,
    usage.cache_read_input_tokens,
  ].reduce((sum, value) => sum + positiveInteger(value), 0);
}

function usageSignature(data) {
  const usage = data.context_window?.current_usage;
  if (!usage || typeof usage !== "object") {
    return null;
  }

  return JSON.stringify({
    sessionId: data.session_id ?? null,
    usage,
    apiMs: data.cost?.total_api_duration_ms ?? null,
    durationMs: data.cost?.total_duration_ms ?? null,
    cost: data.cost?.total_cost_usd ?? null,
    totalInput: data.context_window?.total_input_tokens ?? null,
    totalOutput: data.context_window?.total_output_tokens ?? null,
  });
}

function runPreviousStatusLine(inputText) {
  const previous = readPreviousStatusLine();
  const command = stringValue(previous?.command);
  if (!command || command.includes("claude-statusline.mjs")) {
    return null;
  }

  const result = spawnSync(command, {
    input: inputText,
    shell: true,
    encoding: "utf8",
    timeout: 1200,
    env: process.env,
  });

  if (result.error || result.status !== 0) {
    return null;
  }

  return result.stdout || null;
}

function readPreviousStatusLine() {
  if (!existsSync(backupPath)) {
    return null;
  }

  try {
    const parsed = JSON.parse(readFileSync(backupPath, "utf8"));
    return parsed.statusLine ?? null;
  } catch {
    return null;
  }
}

function defaultStatusLine(data) {
  if (!data) {
    return "Token Meter";
  }

  const fiveHour = remainingPercent(data.rate_limits?.five_hour);
  const sevenDay = remainingPercent(data.rate_limits?.seven_day);
  const parts = ["Token Meter"];
  if (fiveHour != null) {
    parts.push(`5H ${fiveHour}%`);
  }
  if (sevenDay != null) {
    parts.push(`WK ${sevenDay}%`);
  }
  return parts.join(" | ");
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

function remainingPercent(limit) {
  const used = numberOrNull(limit?.used_percentage);
  if (used == null) {
    return null;
  }
  return Math.round(Math.max(0, Math.min(100, 100 - used)));
}

function pruneSessions(state, now) {
  for (const [sessionId, session] of Object.entries(state.sessions)) {
    const updatedAt = positiveInteger(session.updatedAt);
    if (updatedAt > 0 && now - updatedAt <= MAX_SESSION_AGE_SECONDS) {
      continue;
    }
    delete state.sessions[sessionId];
  }
}

function atomicWriteJson(path, value) {
  const tmpPath = `${path}.${process.pid}.tmp`;
  writeFileSync(tmpPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  renameSync(tmpPath, path);
}

function withLock(lockPath, fn) {
  mkdirSync(dirname(lockPath), { recursive: true });
  let fd = null;
  for (let attempt = 0; attempt < LOCK_RETRIES; attempt += 1) {
    try {
      fd = openSync(lockPath, "wx");
      break;
    } catch {
      sleepSync(LOCK_SLEEP_MS);
    }
  }

  if (fd == null) {
    fn();
    return;
  }

  try {
    fn();
  } finally {
    closeSync(fd);
    try {
      unlinkSync(lockPath);
    } catch {
      // The next statusline tick can recover if cleanup races.
    }
  }
}

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function readStdin() {
  try {
    return readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

function parseJson(text) {
  if (!text.trim()) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value : null;
}

function numberOrNull(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function integerOrNull(value) {
  const number = Number(value);
  return Number.isFinite(number) ? Math.trunc(number) : null;
}

function positiveInteger(value) {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? Math.trunc(number) : 0;
}

function writeDiagnostic(error) {
  try {
    mkdirSync(stateDir, { recursive: true });
    writeFileSync(join(stateDir, "claude-statusline-error.log"), `${new Date().toISOString()} ${error?.stack ?? error}\n`, {
      flag: "a",
    });
  } catch {
    // Statusline scripts should never break Claude Code rendering.
  }
}
