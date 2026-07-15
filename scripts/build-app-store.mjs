#!/usr/bin/env node
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const cargoTomlPath = join(root, "src-tauri", "Cargo.toml");
const cargoToml = readFileSync(cargoTomlPath, "utf8");
const releaseDir = join(root, ".release", "appstore");
const targetDir = resolve(
  process.env.TOKEN_METER_APPSTORE_TARGET_DIR ?? "/tmp/token-meter-appstore-target",
);
const generatedEntitlements = join(releaseDir, "Entitlements.generated.plist");
const generatedConfig = join(releaseDir, "tauri.generated.conf.json");
const staticConfig = join(root, "src-tauri", "tauri.appstore.conf.json");
const appPath = join(targetDir, "release", "bundle", "macos", "Token Meter.app");
const pkgPath = join(releaseDir, `Token Meter_${packageJson.version}.pkg`);

const teamId = process.env.APPLE_TEAM_ID?.trim() ?? "";
const signingIdentity = process.env.APPLE_SIGNING_IDENTITY?.trim() || "-";
const installerIdentity = process.env.APPLE_INSTALLER_IDENTITY?.trim() ?? "";
const profilePath = process.env.TOKEN_METER_PROVISIONING_PROFILE?.trim() ?? "";
const submissionBuild = Boolean(profilePath);

if (/^tauri\s*=.*features\s*=\s*\[[^\]]*"macos-private-api"/m.test(cargoToml)) {
  fail(
    "The App Store build cannot enable tauri/macos-private-api directly. Keep it only in the direct-download feature.",
  );
}

if (submissionBuild && (!teamId || signingIdentity === "-" || !installerIdentity)) {
  fail(
    "A submission build requires APPLE_TEAM_ID, APPLE_SIGNING_IDENTITY, APPLE_INSTALLER_IDENTITY, and TOKEN_METER_PROVISIONING_PROFILE.",
  );
}

mkdirSync(releaseDir, { recursive: true });
writeFileSync(generatedEntitlements, entitlementsPlist(teamId), "utf8");

const macOsBundle = {
  entitlements: generatedEntitlements,
  minimumSystemVersion: "12.0",
  signingIdentity,
};
if (profilePath) {
  macOsBundle.files = {
    "embedded.provisionprofile": resolve(profilePath),
  };
}

writeFileSync(
  generatedConfig,
  `${JSON.stringify({ bundle: { macOS: macOsBundle } }, null, 2)}\n`,
  "utf8",
);

run(
  "npm",
  [
    "run",
    "tauri",
    "--",
    "build",
    "--features",
    "app-store",
    "--bundles",
    "app",
    "--config",
    staticConfig,
    "--config",
    generatedConfig,
  ],
  {
    ...process.env,
    CARGO_TARGET_DIR: targetDir,
  },
);

const binaryPath = join(appPath, "Contents", "MacOS", "token-meter");
const binaryStrings = spawnSync("strings", [binaryPath], { encoding: "utf8" });
if (binaryStrings.error || binaryStrings.status !== 0) {
  fail(`Could not scan the App Store binary: ${binaryStrings.error?.message ?? "strings failed"}`);
}
if (/setDrawsBackground:|drawsBackground/.test(binaryStrings.stdout)) {
  fail("The App Store binary contains a private WKWebView background selector.");
}

if (!submissionBuild) {
  process.stdout.write(`App Store sandbox candidate: ${appPath}\n`);
  process.stdout.write("No .pkg was created because no provisioning profile was supplied.\n");
  process.exit(0);
}

rmSync(pkgPath, { force: true });
run("productbuild", [
  "--component",
  appPath,
  "/Applications",
  "--sign",
  installerIdentity,
  pkgPath,
]);
run("pkgutil", ["--check-signature", pkgPath]);
process.stdout.write(`Mac App Store package: ${pkgPath}\n`);

function entitlementsPlist(team) {
  const identityEntitlements = team
    ? `
  <key>com.apple.application-identifier</key>
  <string>${xml(team)}.com.zin.token-meter</string>
  <key>com.apple.developer.team-identifier</key>
  <string>${xml(team)}</string>`
    : "";

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.app-sandbox</key>
  <true/>
  <key>com.apple.security.network.client</key>
  <true/>
  <key>com.apple.security.files.user-selected.read-only</key>
  <true/>
  <key>com.apple.security.files.bookmarks.app-scope</key>
  <true/>${identityEntitlements}
</dict>
</plist>
`;
}

function xml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, {
    cwd: root,
    env,
    stdio: "inherit",
  });
  if (result.error) {
    fail(`${command} failed: ${result.error.message}`);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(1);
}
