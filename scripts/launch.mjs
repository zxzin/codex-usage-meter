#!/usr/bin/env node
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const command = process.argv[2] ?? "open";
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";

const macAppNames = ["Token Meter.app", "Codex Usage Meter.app"];
const macAppCandidates = macAppNames.flatMap((appName) => [
  join(root, "src-tauri", "target", "release", "bundle", "macos", appName),
  join(root, "src-tauri", "target", "debug", "bundle", "macos", appName),
]);

const winExeCandidates = [
  join(root, "src-tauri", "target", "release", "token-meter.exe"),
  join(root, "src-tauri", "target", "debug", "token-meter.exe"),
];

function run(cmd, args, options = {}) {
  const child = spawn(cmd, args, {
    cwd: root,
    stdio: "inherit",
    ...options,
  });

  child.on("error", (error) => {
    console.error(`Failed to run ${cmd}: ${error.message}`);
    process.exit(1);
  });

  return child;
}

function openMacApp() {
  const appPath = macAppCandidates.find((candidate) => existsSync(candidate));
  if (!appPath) {
    return false;
  }

  const child = run("open", [appPath]);
  child.on("exit", (code) => process.exit(code ?? 0));
  return true;
}

function openWindowsExe() {
  const exePath = winExeCandidates.find((candidate) => existsSync(candidate));
  if (!exePath) {
    return false;
  }

  const child = spawn(exePath, [], {
    cwd: root,
    detached: true,
    stdio: "ignore",
  });
  child.on("error", (error) => {
    console.error(`Failed to run ${exePath}: ${error.message}`);
    process.exit(1);
  });
  child.unref();
  process.exit(0);
}

function openBuiltApp() {
  if (process.platform === "darwin") {
    return openMacApp();
  }
  if (process.platform === "win32") {
    return openWindowsExe();
  }

  console.error("Token Meter currently ships desktop bundles for macOS and Windows.");
  console.error("For other Tauri targets, use `npm run tauri:dev` while porting.");
  process.exit(1);
}

function buildDebugApp() {
  if (process.platform === "darwin") {
    return run(npmCommand, ["run", "tauri", "--", "build", "--debug", "--bundles", "app"]);
  }
  if (process.platform === "win32") {
    return run(npmCommand, ["run", "tauri", "--", "build", "--debug", "--bundles", "nsis"]);
  }

  console.error("No debug bundle build command is configured for this platform.");
  process.exit(1);
}

if (command === "dev") {
  const child = run(npmCommand, ["run", "tauri:dev"]);
  child.on("exit", (code) => process.exit(code ?? 0));
} else if (!openBuiltApp()) {
  console.log("No built app found. Building a debug app first...");
  const build = buildDebugApp();
  build.on("exit", (code) => {
    if (code) {
      process.exit(code);
    }
    if (!openBuiltApp()) {
      console.error("Build finished, but no runnable desktop app was found.");
      process.exit(1);
    }
  });
}
