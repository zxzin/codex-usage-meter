import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { type CSSProperties, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { getUsageSnapshot } from "./data";
import {
  formatPercent,
  formatRate,
  formatReset,
  formatTokens,
} from "./format";
import type { UsageSnapshot } from "./types";
import beeBodyAsset from "./assets/living/bee-body-wingless-ui.png";
import beeWingAsset from "./assets/living/bee-wing-ui.png";

export function App() {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const lastMenuOpenAtRef = useRef(0);

  const reloadSnapshot = useCallback(async () => {
    try {
      const next = await getUsageSnapshot();
      setSnapshot(next);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    let alive = true;

    async function poll() {
      try {
        const next = await getUsageSnapshot();
        if (alive) {
          setSnapshot(next);
          setError(null);
        }
      } catch (err) {
        if (alive) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    }

    poll();
    const timer = window.setInterval(poll, 1000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const labels = enLabels;
  const heat = snapshot?.state ?? "waiting";

  const refreshWindowChrome = useCallback(() => {
    void invoke("refresh_window_chrome").catch((err) => {
      console.warn("Window chrome refresh unavailable:", err);
    });
  }, []);

  useEffect(() => {
    const refreshSoon = () => {
      refreshWindowChrome();
      window.setTimeout(refreshWindowChrome, 80);
    };

    refreshSoon();
    window.addEventListener("focus", refreshSoon);
    window.addEventListener("pageshow", refreshSoon);
    document.addEventListener("visibilitychange", refreshSoon);
    return () => {
      window.removeEventListener("focus", refreshSoon);
      window.removeEventListener("pageshow", refreshSoon);
      document.removeEventListener("visibilitychange", refreshSoon);
    };
  }, [refreshWindowChrome]);

  const openNativeContextMenu = useCallback(
    async (event?: Pick<MouseEvent, "clientX" | "clientY">) => {
      const now = Date.now();
      if (now - lastMenuOpenAtRef.current < 220) {
        return;
      }
      lastMenuOpenAtRef.current = now;

      try {
        await invoke("show_context_menu", {
          x: Math.max(0, event?.clientX ?? 0),
          y: Math.max(0, event?.clientY ?? 0),
        });
      } catch (err) {
        console.warn("Native context menu unavailable:", err);
      }
    },
    [],
  );

  useEffect(() => {
    let unlistenReload: (() => void) | undefined;
    let alive = true;

    const handleNativeReload = () => {
      void reloadSnapshot();
    };

    window.addEventListener("token-meter-reload", handleNativeReload);

    void listen("context-menu-reload", () => {
      void reloadSnapshot();
    })
      .then((unlisten) => {
        if (alive) {
          unlistenReload = unlisten;
        } else {
          unlisten();
        }
      })
      .catch((err) => {
        console.warn("Native reload menu events unavailable:", err);
      });

    return () => {
      alive = false;
      window.removeEventListener("token-meter-reload", handleNativeReload);
      unlistenReload?.();
    };
  }, [reloadSnapshot]);

  useEffect(() => {
    const handleContextMenu = (event: globalThis.MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      void openNativeContextMenu(event);
    };

    const handleMouseDown = (event: globalThis.MouseEvent) => {
      if (event.button !== 2) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      void openNativeContextMenu(event);
    };

    document.addEventListener("contextmenu", handleContextMenu, true);
    window.addEventListener("contextmenu", handleContextMenu, true);
    document.addEventListener("mousedown", handleMouseDown, true);
    window.addEventListener("mousedown", handleMouseDown, true);
    return () => {
      document.removeEventListener("contextmenu", handleContextMenu, true);
      window.removeEventListener("contextmenu", handleContextMenu, true);
      document.removeEventListener("mousedown", handleMouseDown, true);
      window.removeEventListener("mousedown", handleMouseDown, true);
    };
  }, [openNativeContextMenu]);

  const startWindowDrag = useCallback((event: React.MouseEvent<HTMLElement>) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    void invoke("start_window_drag").catch(() => {
      void getCurrentWindow().startDragging().catch((err) => {
        console.warn("Window drag unavailable:", err);
      });
    });
  }, []);

  return (
    <main
      className={`app size-micro heat-${heat}`}
      data-tauri-drag-region="deep"
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void openNativeContextMenu(event.nativeEvent);
      }}
      onMouseDown={startWindowDrag}
    >
      <section className="meter-window" data-tauri-drag-region="deep">
        <div className="meter-body" data-tauri-drag-region="deep">
          {snapshot ? (
            <HiveMeter snapshot={snapshot} />
          ) : (
            <LoadingMeter labels={labels} />
          )}
        </div>
      </section>

      <aside className="detail-popover">
        {snapshot ? (
          <Details snapshot={snapshot} labels={labels} />
        ) : (
          <p>{error ?? labels.loading}</p>
        )}
      </aside>
    </main>
  );
}

function LoadingMeter({ labels }: { labels: Labels }) {
  return (
    <div className="loading-meter">
      <span className="loading-ring" aria-hidden="true" />
      <span>{labels.loading}</span>
    </div>
  );
}

function HiveMeter({ snapshot }: { snapshot: UsageSnapshot }) {
  const animationRate = snapshot.animationBurnRatePerMin;
  const speed = ratePercent(animationRate) * 100;
  const primary = clampPercent(snapshot.primary?.remainingPercent);
  const secondary = clampPercent(snapshot.secondary?.remainingPercent);
  const speedRatio = ratePercent(animationRate);
  const visualSpeedRatio = Math.max(0.16, speedRatio);
  const live = Math.max(0.08, visualSpeedRatio);
  const flySpeed = `${Math.max(0.56, 3.6 - visualSpeedRatio * 2.95)}s`;
  const beeCount = 3;
  const quotaLabel = `5H ${formatPercent(snapshot.primary?.remainingPercent)}; Weekly ${formatPercent(snapshot.secondary?.remainingPercent)}`;

  const bees = Array.from({ length: beeCount }, (_, index) => {
    const orbitRadius = 29;
    return (
      <span
        key={index}
        className={`bee-unit b${index + 1}`}
        style={
          {
            left: "50%",
            top: "50%",
            "--bee-angle-offset": `${index * 120 - 18}deg`,
            "--orbit-radius": `${orbitRadius}px`,
          } as CSSProperties
        }
      >
        <img className="bee-wing-layer wing-left" src={beeWingAsset} alt="" />
        <img className="bee-wing-layer wing-right" src={beeWingAsset} alt="" />
        <img className="bee-body-layer" src={beeBodyAsset} alt="" />
      </span>
    );
  });

  return (
    <div
      className="living-meter image-living hive-meter hive-compact"
      data-tauri-drag-region="deep"
      style={
        {
          "--life-speed": flySpeed,
          "--speed-pct": speed,
          "--hive-glow": 0.22 + live * 0.62,
          "--quota-5h": primary,
          "--quota-total": secondary,
          "--quota-5h-color": quotaTone(primary, "#1667e8"),
          "--quota-total-color": quotaTone(secondary, "#16aa73"),
        } as CSSProperties
      }
    >
      <div className="living-refraction" />
      <HiveImageCore
        primary={primary}
        secondary={secondary}
        quotaLabel={quotaLabel}
      />
      {bees}
    </div>
  );
}

function HiveImageCore({
  primary,
  secondary,
  quotaLabel,
}: {
  primary: number;
  secondary: number;
  quotaLabel: string;
}) {
  return (
    <div
      className="hive-reference-wrap hive-reference-compact"
      data-tauri-drag-region="deep"
      role="img"
      aria-label={quotaLabel}
    >
      <HiveCompactQuotaSlots primary={primary} secondary={secondary} />
    </div>
  );
}

const compactQuotaSlots = {
  clusterBackplate:
    "M 8 54 L 25 24 H 282 L 302 54 L 285 85 L 302 116 L 282 146 H 25 L 8 116 L 24 85 Z",
  labels: [
    { label: "5H", d: hexPath(42, 54, 35, 30), x: 42, y: 64 },
    { label: "WK", d: hexPath(42, 116, 35, 30), x: 42, y: 126 },
  ],
  primary: [
    {
      d: hexPath(116, 54, 35, 30),
      innerD: hexPath(116, 54, 24, 19),
      x: 81,
      y: 24,
      w: 70,
      h: 60,
      ix: 92,
      iy: 35,
      iw: 48,
      ih: 38,
    },
    {
      d: hexPath(190, 54, 35, 30),
      innerD: hexPath(190, 54, 24, 19),
      x: 155,
      y: 24,
      w: 70,
      h: 60,
      ix: 166,
      iy: 35,
      iw: 48,
      ih: 38,
    },
    {
      d: hexPath(264, 54, 35, 30),
      innerD: hexPath(264, 54, 24, 19),
      x: 229,
      y: 24,
      w: 70,
      h: 60,
      ix: 240,
      iy: 35,
      iw: 48,
      ih: 38,
    },
  ],
  secondary: [
    {
      d: hexPath(116, 116, 35, 30),
      innerD: hexPath(116, 116, 24, 19),
      x: 81,
      y: 86,
      w: 70,
      h: 60,
      ix: 92,
      iy: 97,
      iw: 48,
      ih: 38,
    },
    {
      d: hexPath(190, 116, 35, 30),
      innerD: hexPath(190, 116, 24, 19),
      x: 155,
      y: 86,
      w: 70,
      h: 60,
      ix: 166,
      iy: 97,
      iw: 48,
      ih: 38,
    },
    {
      d: hexPath(264, 116, 35, 30),
      innerD: hexPath(264, 116, 24, 19),
      x: 229,
      y: 86,
      w: 70,
      h: 60,
      ix: 240,
      iy: 97,
      iw: 48,
      ih: 38,
    },
  ],
} as const;

function HiveCompactQuotaSlots({
  primary,
  secondary,
}: {
  primary: number;
  secondary: number;
}) {
  const rawId = useId();
  const clipPrefix = rawId.replace(/[^a-zA-Z0-9_-]/g, "");

  return (
    <svg
      className="hive-compact-quota-slots"
      viewBox="0 0 310 170"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <path className="hive-cluster-backplate" d={compactQuotaSlots.clusterBackplate} />
      <HiveCompactSlotRow
        kind="primary"
        percent={primary}
        slots={compactQuotaSlots.primary}
        clipPrefix={`${clipPrefix}-5h`}
      />
      <HiveCompactSlotRow
        kind="secondary"
        percent={secondary}
        slots={compactQuotaSlots.secondary}
        clipPrefix={`${clipPrefix}-wk`}
      />
      <g className="hive-compact-labels">
        {compactQuotaSlots.labels.map((cell) => (
          <g key={cell.label} className="hive-compact-label-cell">
            <path className="hive-slot-label-base" d={cell.d} />
            <text className={`hive-slot-label-text label-${cell.label.toLowerCase()}`} x={cell.x} y={cell.y}>
              {cell.label}
            </text>
          </g>
        ))}
      </g>
    </svg>
  );
}

function HiveCompactSlotRow({
  kind,
  percent,
  slots,
  clipPrefix,
}: {
  kind: "primary" | "secondary";
  percent: number;
  slots: readonly {
    d: string;
    x: number;
    y: number;
    w: number;
    h: number;
    innerD: string;
    ix: number;
    iy: number;
    iw: number;
    ih: number;
  }[];
  clipPrefix: string;
}) {
  return (
    <g className={`hive-compact-slot-row ${kind}${percent <= 20 ? " critical" : ""}`}>
      <defs>
        {slots.map((slot, index) => (
          <clipPath id={`${clipPrefix}-${index}`} key={`${clipPrefix}-clip-${index}`}>
            <path d={slot.innerD} />
          </clipPath>
        ))}
      </defs>
      {slots.map((slot, index) => {
        const fill = quotaSlotFill(percent, index, slots.length);
        const fillWidth = slot.iw * fill;
        return (
          <g className="hive-compact-slot" key={`${clipPrefix}-slot-${index}`}>
            <path className="hive-slot-base" d={slot.d} />
            <path className="hive-slot-inner-base" d={slot.innerD} />
            <g clipPath={`url(#${clipPrefix}-${index})`}>
              <rect
                className="hive-slot-fill"
                x={slot.ix}
                y={slot.iy}
                width={fillWidth}
                height={slot.ih}
              />
              <rect
                className="hive-slot-fill-shade"
                x={slot.ix}
                y={slot.iy + slot.ih * 0.62}
                width={fillWidth}
                height={slot.ih * 0.38}
              />
              <rect
                className="hive-slot-fill-gloss"
                x={slot.ix + 6}
                y={slot.iy + 7}
                width={Math.max(0, Math.min(fillWidth - 8, slot.iw * 0.46))}
                height={7}
              />
            </g>
            <path className="hive-slot-inner-outline" d={slot.innerD} />
            <path className="hive-slot-outline" d={slot.d} />
          </g>
        );
      })}
    </g>
  );
}

function Details({ snapshot, labels }: { snapshot: UsageSnapshot; labels: Labels }) {
  const activeSessions = useMemo(
    () => snapshot.sessions.filter((session) => session.active),
    [snapshot.sessions],
  );

  return (
    <div className="details">
      <div className="detail-grid">
        <Detail label={labels.speed} value={formatRate(snapshot.burnRatePerMin)} />
        <Detail label="5h" value={`${formatPercent(snapshot.primary?.remainingPercent)} · ${formatReset(snapshot.primary)}`} />
        <Detail label="7d" value={`${formatPercent(snapshot.secondary?.remainingPercent)} · ${formatReset(snapshot.secondary)}`} />
        <Detail label={labels.active} value={`${snapshot.activeSessions}/${snapshot.observedSessions}`} />
        <Detail label="Resets" value={snapshot.resetCreditsAvailable == null ? "--" : String(snapshot.resetCreditsAvailable)} />
      </div>
      <div className="session-list">
        {activeSessions.length === 0 ? (
          <span>{labels.noActive}</span>
        ) : (
          activeSessions.slice(0, 3).map((session) => (
            <div className="session-row" key={session.id}>
              <span>{session.cwd?.split("/").filter(Boolean).pop() ?? session.id.slice(0, 8)}</span>
              <strong>{formatRate(session.burnRatePerMin)}</strong>
            </div>
          ))
        )}
      </div>
      <p className="source-note">
        {snapshot.source.providerLabel} · {snapshot.source.message} · {snapshot.source.scannedFiles} files · {formatTokens(snapshot.totalRecentTokens)} recent
      </p>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ratePercent(rate: number) {
  return Math.min(1, Math.max(0, rate / 220_000));
}

function clampPercent(value?: number | null) {
  if (value == null || !Number.isFinite(value)) {
    return 0;
  }
  return Math.min(100, Math.max(0, value));
}

function quotaTone(percent: number, healthy: string) {
  if (percent <= 15) {
    return "#e33f39";
  }
  if (percent <= 35) {
    return "#f4c83e";
  }
  return healthy;
}

function hexPath(cx: number, cy: number, rx: number, ry: number) {
  return [
    `M ${cx - rx} ${cy}`,
    `L ${cx - rx / 2} ${cy - ry}`,
    `H ${cx + rx / 2}`,
    `L ${cx + rx} ${cy}`,
    `L ${cx + rx / 2} ${cy + ry}`,
    `H ${cx - rx / 2}`,
    "Z",
  ].join(" ");
}

function quotaSlotFill(percent: number, index: number, count: number) {
  const slotSize = 100 / count;
  const filled = (percent - index * slotSize) / slotSize;
  return Math.min(1, Math.max(0, filled));
}

interface Labels {
  loading: string;
  speed: string;
  active: string;
  noActive: string;
}

const enLabels: Labels = {
  loading: "Waiting for usage data",
  speed: "Speed",
  active: "Active",
  noActive: "No active sessions",
};
