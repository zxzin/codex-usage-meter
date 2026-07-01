import { invoke } from "@tauri-apps/api/core";
import type { UsageSnapshot } from "./types";

let previewWarningShown = false;

export async function getUsageSnapshot(): Promise<UsageSnapshot> {
  try {
    return await invoke<UsageSnapshot>("get_usage_snapshot");
  } catch (error) {
    if (!previewWarningShown) {
      previewWarningShown = true;
      console.warn("Using browser preview mock data:", error);
    }
    return mockSnapshot();
  }
}

function mockSnapshot(): UsageSnapshot {
  const now = Math.floor(Date.now() / 1000);
  return {
    generatedAt: now,
    burnRatePerMin: 42000,
    animationBurnRatePerMin: 42000,
    state: "live",
    activeSessions: 3,
    activitySessions: 3,
    observedSessions: 7,
    windowSeconds: 60,
    activeGraceSeconds: 90,
    totalRecentTokens: 42000,
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
        recentTokens: 22000,
        burnRatePerMin: 22000,
        active: true,
      },
      {
        id: "preview-2",
        cwd: "/Users/zin/project-b",
        path: "~/.codex/sessions/preview-2.jsonl",
        lastSeen: now - 16,
        totalTokens: 180000,
        recentTokens: 12000,
        burnRatePerMin: 12000,
        active: true,
      },
      {
        id: "preview-3",
        cwd: "/Users/zin/project-c",
        path: "~/.codex/sessions/preview-3.jsonl",
        lastSeen: now - 31,
        totalTokens: 90000,
        recentTokens: 8000,
        burnRatePerMin: 8000,
        active: true,
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
