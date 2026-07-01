export type MeterState =
  | "waiting"
  | "idle"
  | "live"
  | "warm"
  | "hot"
  | "limit_near"
  | "stale";

export interface LimitWindow {
  usedPercent: number;
  remainingPercent: number;
  windowMinutes?: number | null;
  resetsAt?: number | null;
}

export interface SessionSummary {
  id: string;
  cwd?: string | null;
  path: string;
  lastSeen: number;
  totalTokens: number;
  recentTokens: number;
  burnRatePerMin: number;
  active: boolean;
}

export interface SourceStatus {
  provider: "codex" | "claude";
  providerLabel: string;
  dataHome: string;
  eventsPath: string;
  codexHome: string;
  sessionsDir: string;
  scannedFiles: number;
  message: string;
}

export interface UsageSnapshot {
  generatedAt: number;
  burnRatePerMin: number;
  animationBurnRatePerMin: number;
  state: MeterState;
  activeSessions: number;
  activitySessions: number;
  observedSessions: number;
  windowSeconds: number;
  activeGraceSeconds: number;
  totalRecentTokens: number;
  latestTotalTokens: number;
  primary?: LimitWindow | null;
  secondary?: LimitWindow | null;
  resetCreditsAvailable?: number | null;
  sessions: SessionSummary[];
  source: SourceStatus;
}
