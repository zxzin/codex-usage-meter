import type { LimitWindow, UsageSnapshot } from "./types";

export function formatRate(tokensPerMinute: number): string {
  if (!Number.isFinite(tokensPerMinute) || tokensPerMinute <= 0) {
    return "0/min";
  }
  if (tokensPerMinute >= 1_000_000) {
    return `${trim(tokensPerMinute / 1_000_000)}M/min`;
  }
  if (tokensPerMinute >= 1_000) {
    return `${trim(tokensPerMinute / 1_000)}k/min`;
  }
  return `${Math.round(tokensPerMinute)}/min`;
}

export function formatPercent(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) {
    return "--";
  }
  return `${Math.round(value)}%`;
}

export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) {
    return `${trim(tokens / 1_000_000)}M`;
  }
  if (tokens >= 1_000) {
    return `${trim(tokens / 1_000)}k`;
  }
  return `${tokens}`;
}

export function formatReset(limit?: LimitWindow | null): string {
  if (!limit?.resetsAt) {
    return "reset --";
  }
  const deltaSeconds = limit.resetsAt - Math.floor(Date.now() / 1000);
  if (deltaSeconds <= 0) {
    return "reset now";
  }
  const minutes = Math.round(deltaSeconds / 60);
  if (minutes < 60) {
    return `${minutes}m reset`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return `${hours}h reset`;
  }
  return `${Math.round(hours / 24)}d reset`;
}

export function getStateLabel(snapshot: UsageSnapshot): string {
  switch (snapshot.state) {
    case "waiting":
      return "Waiting";
    case "idle":
      return "Idle";
    case "live":
      return "Live";
    case "warm":
      return "Warm";
    case "hot":
      return "Hot";
    case "limit_near":
      return "Limit";
    case "stale":
      return "Stale";
    default:
      return "Live";
  }
}

function trim(value: number): string {
  return value >= 10 ? value.toFixed(0) : value.toFixed(1).replace(/\.0$/, "");
}
