import { invoke } from "@tauri-apps/api/core";
import type { UsageSnapshot } from "./types";

let previewWarningShown = false;

export async function getUsageSnapshot(): Promise<UsageSnapshot> {
  try {
    return await invoke<UsageSnapshot>("get_usage_snapshot");
  } catch (error) {
    const previewMode = new URLSearchParams(window.location.search).get("mock");
    if (previewMode != null) {
      if (!previewWarningShown) {
        previewWarningShown = true;
        console.warn("Using explicit browser preview mock data:", error);
      }
      return mockSnapshot(previewMode === "idle");
    }

    throw new Error(
      "Real token usage is only available in the Token Meter desktop app. Browser preview has no Tauri usage bridge; add ?mock=live or ?mock=idle only for visual preview.",
    );
  }
}

function mockSnapshot(idle = false): UsageSnapshot {
  const now = Math.floor(Date.now() / 1000);
  return {
    generatedAt: now,
    burnRatePerMin: idle ? 0 : 42000,
    animationBurnRatePerMin: idle ? 0 : 42000,
    state: idle ? "idle" : "live",
    activeSessions: idle ? 0 : 3,
    activitySessions: idle ? 0 : 3,
    observedSessions: 7,
    windowSeconds: 60,
    activeGraceSeconds: 90,
    totalRecentTokens: idle ? 0 : 42000,
    latestTotalTokens: 860000,
    primary: {
      usedPercent: 20,
      remainingPercent: 80,
      windowMinutes: 300,
      resetsAt: now + 68 * 60,
    },
    secondary: {
      usedPercent: 13,
      remainingPercent: 87,
      windowMinutes: 10080,
      resetsAt: now + 4 * 24 * 60 * 60,
    },
    resetCreditsAvailable: 2,
    sessions: [
      {
        id: "preview-1",
        cwd: "/Users/zin/project-a",
        path: "~/.codex/sessions/preview-1.jsonl",
        lastSeen: now - 8,
        totalTokens: 240000,
        recentTokens: idle ? 0 : 22000,
        burnRatePerMin: idle ? 0 : 22000,
        active: !idle,
      },
      {
        id: "preview-2",
        cwd: "/Users/zin/project-b",
        path: "~/.codex/sessions/preview-2.jsonl",
        lastSeen: now - 16,
        totalTokens: 180000,
        recentTokens: idle ? 0 : 12000,
        burnRatePerMin: idle ? 0 : 12000,
        active: !idle,
      },
      {
        id: "preview-3",
        cwd: "/Users/zin/project-c",
        path: "~/.codex/sessions/preview-3.jsonl",
        lastSeen: now - 31,
        totalTokens: 90000,
        recentTokens: idle ? 0 : 8000,
        burnRatePerMin: idle ? 0 : 8000,
        active: !idle,
      },
    ],
    source: {
      provider: "codex",
      providerLabel: "Codex",
      dataHome: "~/.codex",
      eventsPath: "~/.codex/sessions",
      codexHome: "~/.codex",
      sessionsDir: "~/.codex/sessions",
      scannedFiles: 7,
      message: "Preview mode: not real Codex account data",
    },
  };
}
