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
import {
  loadSubscriptionGate,
  openSubscriptionLink,
  purchaseSubscription,
  restoreSubscription,
  setSubscriptionWindowMode,
  subscriptionErrorMessage,
  subscriptionPrice,
  type SubscriptionGate,
} from "./subscription";
import beeBodyAsset from "./assets/living/bee-body-wingless-ui.png";
import beeWingAsset from "./assets/living/bee-wing-ui.png";
import appIconAsset from "../src-tauri/icons/128x128.png";
import beeMotionContract from "./bee-motion-contract.json";

const BEE_SPEED_FULL_RATE_PER_MIN = beeMotionContract.speedFullRatePerMin;
const BEE_SPEED_LOG_BASE_RATE_PER_MIN = beeMotionContract.speedLogBaseRatePerMin;
const BEE_ORBIT_IDLE_RATIO = beeMotionContract.orbitIdleRatio;
const BEE_ORBIT_SLOW_SECONDS = beeMotionContract.orbitSlowSeconds;
const BEE_ORBIT_FAST_SECONDS = beeMotionContract.orbitFastSeconds;
const BEE_TRAIL_START_RATIO = beeMotionContract.trailStartRatio;
const BEE_TRAIL_NEAR_MAX_OPACITY = beeMotionContract.trailNearMaxOpacity;
const BEE_TRAIL_FAR_MAX_OPACITY = beeMotionContract.trailFarMaxOpacity;
const BEE_WING_SLOW_MS = beeMotionContract.wingSlowMs;
const BEE_WING_FAST_MS = beeMotionContract.wingFastMs;
const BEE_ORBIT_BASE_RADIUS_PX = beeMotionContract.orbitBaseRadiusPx;
const BEE_ORBIT_FAST_RADIUS_PX = beeMotionContract.orbitFastRadiusPx;
const BEE_ORBIT_RADIUS_CURVE = beeMotionContract.orbitRadiusCurve;
const BEE_ORBIT_SMOOTHING_PER_SECOND = beeMotionContract.orbitSmoothingPerSecond;
const BEE_ORBIT_MAX_FRAME_SECONDS = beeMotionContract.maxFrameSeconds;
const BEE_MOTION_ACCELERATION_PER_SECOND = beeMotionContract.motionAccelerationPerSecond;
const BEE_MOTION_DECELERATION_PER_SECOND = beeMotionContract.motionDecelerationPerSecond;
const BEE_MOTION_IDLE_DECAY_PER_SECOND = beeMotionContract.motionIdleDecayPerSecond;
const BEE_MOTION_STOP_RATIO = beeMotionContract.motionStopRatio;
const BEE_MOTION_RENDER_EPSILON = beeMotionContract.motionRenderEpsilon;
const BEE_FACING_ROTATION_DEG = beeMotionContract.beeFacingRotationDeg;
const BEE_ACTIVE_SCALE = beeMotionContract.beeActiveScale;
const BEE_COUNT = beeMotionContract.beeCount;
const USAGE_POLL_INTERVAL_MS = 2000;
// Static bees perch inside the minimum active orbit radius, with feet against the frame.
const BEE_STATIC_PLACEMENTS = beeMotionContract.staticPlacements;

export function App() {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [subscriptionGate, setSubscriptionGate] = useState<SubscriptionGate>({ status: "checking" });
  const [subscriptionBusy, setSubscriptionBusy] = useState(false);
  const [subscriptionMessage, setSubscriptionMessage] = useState("");
  const subscriptionGateRef = useRef(subscriptionGate);
  const lastMenuOpenAtRef = useRef(0);
  const accessGranted = subscriptionGate.status === "active" || subscriptionGate.status === "not_required";

  useEffect(() => {
    subscriptionGateRef.current = subscriptionGate;
  }, [subscriptionGate]);

  const applySubscriptionGate = useCallback(async (next: SubscriptionGate) => {
    await setSubscriptionWindowMode(next.status !== "active" && next.status !== "not_required");
    setSubscriptionGate(next);
    setSubscriptionMessage("");
  }, []);

  const refreshSubscription = useCallback(async (preserveActiveOnError = false) => {
    try {
      const next = await loadSubscriptionGate();
      await applySubscriptionGate(next);
    } catch (err) {
      if (preserveActiveOnError && subscriptionGateRef.current.status === "active") {
        return;
      }
      await setSubscriptionWindowMode(true).catch(() => undefined);
      setSubscriptionGate({
        status: "unavailable",
        message: "The App Store could not verify the subscription. Please try again.",
      });
      setSubscriptionMessage(subscriptionErrorMessage(err));
    }
  }, [applySubscriptionGate]);

  useEffect(() => {
    void refreshSubscription();
  }, [refreshSubscription]);

  useEffect(() => {
    if (subscriptionGate.status !== "active") {
      return;
    }
    const timer = window.setInterval(() => {
      void refreshSubscription(true);
    }, 15 * 60 * 1000);
    return () => window.clearInterval(timer);
  }, [refreshSubscription, subscriptionGate.status]);

  const subscribe = useCallback(async () => {
    setSubscriptionBusy(true);
    setSubscriptionMessage("");
    try {
      const next = await purchaseSubscription();
      await applySubscriptionGate(next);
    } catch (err) {
      setSubscriptionMessage(subscriptionErrorMessage(err));
    } finally {
      setSubscriptionBusy(false);
    }
  }, [applySubscriptionGate]);

  const restore = useCallback(async () => {
    setSubscriptionBusy(true);
    setSubscriptionMessage("");
    try {
      const next = await restoreSubscription();
      await applySubscriptionGate(next);
      if (next.status === "inactive") {
        setSubscriptionMessage("No active subscription was found for this Apple Account.");
      }
    } catch (err) {
      setSubscriptionMessage(subscriptionErrorMessage(err));
    } finally {
      setSubscriptionBusy(false);
    }
  }, [applySubscriptionGate]);

  const reloadSnapshot = useCallback(async () => {
    try {
      const next = await getUsageSnapshot();
      setSnapshot(next);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const connectCodexFolder = useCallback(async () => {
    try {
      await invoke<boolean>("choose_codex_folder");
      await reloadSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [reloadSnapshot]);

  useEffect(() => {
    if (!accessGranted) {
      return;
    }

    let alive = true;
    let pollTimer = 0;

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

      if (alive) {
        pollTimer = window.setTimeout(poll, USAGE_POLL_INTERVAL_MS);
      }
    }

    async function start() {
      try {
        await invoke<boolean>("ensure_codex_access");
      } catch (err) {
        if (alive) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }

      if (alive) {
        await poll();
      }
    }

    void start();
    return () => {
      alive = false;
      window.clearTimeout(pollTimer);
    };
  }, [accessGranted]);

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
    if (!accessGranted) {
      return;
    }

    let unlistenReload: (() => void) | undefined;
    let unlistenConnect: (() => void) | undefined;
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

    void listen("context-menu-connect", () => {
      void connectCodexFolder();
    })
      .then((unlisten) => {
        if (alive) {
          unlistenConnect = unlisten;
        } else {
          unlisten();
        }
      })
      .catch((err) => {
        console.warn("Codex folder menu events unavailable:", err);
      });

    return () => {
      alive = false;
      window.removeEventListener("token-meter-reload", handleNativeReload);
      unlistenReload?.();
      unlistenConnect?.();
    };
  }, [accessGranted, connectCodexFolder, reloadSnapshot]);

  useEffect(() => {
    if (!accessGranted) {
      return;
    }

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
  }, [accessGranted, openNativeContextMenu]);

  const startWindowDrag = useCallback((event: React.MouseEvent<HTMLElement>) => {
    if (event.button !== 0) {
      return;
    }

    const target = event.target as HTMLElement;
    if (
      target.closest(
        "button, a, input, select, textarea, label, summary, [role='button'], [role='link']",
      )
    ) {
      return;
    }

    // Handle the drag ourselves during the capture phase. Tauri also installs a
    // document-level drag-region listener; allowing both handlers to run can
    // start two native drag operations for one press, which is unreliable on
    // Windows transparent windows.
    event.preventDefault();
    event.stopPropagation();
    void invoke("start_window_drag").catch(() => {
      void getCurrentWindow().startDragging().catch((err) => {
        console.warn("Window drag unavailable:", err);
      });
    });
  }, []);

  if (!accessGranted) {
    return (
      <SubscriptionPaywall
        gate={subscriptionGate}
        busy={subscriptionBusy}
        message={subscriptionMessage}
        onSubscribe={subscribe}
        onRestore={restore}
        onRetry={() => void refreshSubscription()}
        onMouseDown={startWindowDrag}
      />
    );
  }

  return (
    <main
      className={`app size-micro heat-${heat}`}
      data-tauri-drag-region="deep"
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void openNativeContextMenu(event.nativeEvent);
      }}
      onMouseDownCapture={startWindowDrag}
    >
      <section className="meter-window" data-tauri-drag-region="deep">
        <div className="meter-body" data-tauri-drag-region="deep">
          {snapshot ? (
            <HiveMeter snapshot={snapshot} />
          ) : (
            <LoadingMeter labels={labels} error={error} onConnect={connectCodexFolder} />
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

function SubscriptionPaywall({
  gate,
  busy,
  message,
  onSubscribe,
  onRestore,
  onRetry,
  onMouseDown,
}: {
  gate: SubscriptionGate;
  busy: boolean;
  message: string;
  onSubscribe: () => void;
  onRestore: () => void;
  onRetry: () => void;
  onMouseDown: (event: React.MouseEvent<HTMLElement>) => void;
}) {
  const stopDrag = (event: React.MouseEvent<HTMLElement>) => event.stopPropagation();
  const product = gate.status === "inactive" ? gate.product : null;

  return (
    <main
      className="subscription-shell"
      data-tauri-drag-region="deep"
      onMouseDownCapture={onMouseDown}
    >
      <section className="subscription-panel">
        <img className="subscription-icon" src={appIconAsset} alt="" />
        <div className="subscription-heading">
          <h1>Token Meter</h1>
          <p>Keep Codex usage visible while you work.</p>
        </div>

        {gate.status === "checking" ? (
          <div className="subscription-status" role="status">
            <span className="subscription-spinner" aria-hidden="true" />
            <span>Checking App Store subscription…</span>
          </div>
        ) : product ? (
          <>
            <div className="subscription-offer">
              <strong>7 days free</strong>
              <span>Then {subscriptionPrice(product)} per month</span>
            </div>
            <ul className="subscription-benefits">
              <li>Live 5-hour and weekly usage</li>
              <li>Motion that follows token burn rate</li>
              <li>Local processing, with no account to create</li>
            </ul>
            <button
              className="subscription-primary"
              type="button"
              disabled={busy}
              onMouseDown={stopDrag}
              onClick={onSubscribe}
            >
              {busy ? "Connecting to App Store…" : "Start 7-Day Free Trial"}
            </button>
          </>
        ) : (
          <div className="subscription-unavailable" role="alert">
            <strong>Subscription unavailable</strong>
            <span>{gate.status === "unavailable" ? gate.message : "Please try again."}</span>
            <button type="button" disabled={busy} onMouseDown={stopDrag} onClick={onRetry}>
              Try Again
            </button>
          </div>
        )}

        {message ? <p className="subscription-message">{message}</p> : null}

        <div className="subscription-footer">
          <button type="button" disabled={busy} onMouseDown={stopDrag} onClick={onRestore}>
            Restore Purchases
          </button>
          <p>
            Free trial for eligible new subscribers. Subscription renews automatically until canceled
            in App Store settings.
          </p>
          <nav aria-label="Legal">
            <button
              type="button"
              onMouseDown={stopDrag}
              onClick={() =>
                void openSubscriptionLink(
                  "https://github.com/zxzin/codex-usage-meter/blob/main/PRIVACY.md",
                )
              }
            >
              Privacy
            </button>
            <button
              type="button"
              onMouseDown={stopDrag}
              onClick={() =>
                void openSubscriptionLink(
                  "https://www.apple.com/legal/internet-services/itunes/dev/stdeula/",
                )
              }
            >
              Terms
            </button>
          </nav>
        </div>
      </section>
    </main>
  );
}

function LoadingMeter({
  labels,
  error,
  onConnect,
}: {
  labels: Labels;
  error: string | null;
  onConnect: () => void;
}) {
  if (error) {
    return (
      <div className="loading-meter loading-meter-error" role="alert" title={error}>
        <strong aria-hidden="true">!</strong>
        <button
          type="button"
          onMouseDown={(event) => event.stopPropagation()}
          onClick={onConnect}
        >
          Reconnect
        </button>
      </div>
    );
  }

  return (
    <div className="loading-meter">
      <span className="loading-ring" aria-hidden="true" />
      <span>{labels.loading}</span>
    </div>
  );
}

function HiveMeter({ snapshot }: { snapshot: UsageSnapshot }) {
  const meterRef = useRef<HTMLDivElement | null>(null);
  const tokenSpeedRatio = ratePercent(snapshot.animationBurnRatePerMin);
  const tokenMotionRatio = beeMotionRatio(tokenSpeedRatio);
  const motionRatio = useBeeMotionRatio(tokenMotionRatio);
  const activeMotion = motionRatio > BEE_MOTION_STOP_RATIO;
  const speed = motionRatio * 100;
  const primary = clampPercent(snapshot.primary?.remainingPercent);
  const secondary = clampPercent(snapshot.secondary?.remainingPercent);
  const live = activeMotion ? Math.max(0.08, motionRatio) : 0;
  const trailIntensity = beeTrailIntensity(motionRatio);
  const wingSpeed = `${beeWingDurationMs(motionRatio)}ms`;
  const orbitRadius = beeOrbitRadiusPx(motionRatio);
  const orbitDuration = beeOrbitDurationSeconds(motionRatio);
  const orbitTargetRef = useRef({
    durationSeconds: orbitDuration,
    radius: orbitRadius,
    active: activeMotion,
  });
  const orbitStateRef = useRef({
    angle: 0,
    currentDurationSeconds: orbitDuration,
    currentRadius: orbitRadius,
    lastTime: 0,
  });
  const beeCount = BEE_COUNT;
  const quotaLabel = `5H ${formatPercent(snapshot.primary?.remainingPercent)}; Weekly ${formatPercent(snapshot.secondary?.remainingPercent)}`;

  useEffect(() => {
    orbitTargetRef.current = {
      durationSeconds: orbitDuration,
      radius: orbitRadius,
      active: activeMotion,
    };
  }, [activeMotion, orbitDuration, orbitRadius]);

  useEffect(() => {
    let frameId = 0;

    const tick = (time: number) => {
      const state = orbitStateRef.current;
      if (!state.lastTime) {
        state.lastTime = time;
      }

      const frameSeconds = Math.min(
        BEE_ORBIT_MAX_FRAME_SECONDS,
        Math.max(0, (time - state.lastTime) / 1000),
      );
      state.lastTime = time;

      const target = orbitTargetRef.current;
      const smoothing = 1 - Math.exp(-frameSeconds * BEE_ORBIT_SMOOTHING_PER_SECOND);
      state.currentDurationSeconds += (target.durationSeconds - state.currentDurationSeconds) * smoothing;
      state.currentRadius += (target.radius - state.currentRadius) * smoothing;
      if (target.active) {
        state.angle = (
          state.angle
          + (frameSeconds * 360) / Math.max(0.2, state.currentDurationSeconds)
        ) % 360;
      }

      const bees = meterRef.current?.querySelectorAll<HTMLElement>(".bee-unit");
      bees?.forEach((bee) => {
        const staticIndex = Number(bee.dataset.staticIndex ?? 0);
        if (!target.active) {
          const placement = beeStaticPlacement(staticIndex);
          bee.style.left = "50%";
          bee.style.top = "50%";
          bee.style.transform = beeStaticTransform(placement);
          return;
        }

        bee.style.left = "50%";
        bee.style.top = "50%";
        const angleOffset = Number(bee.dataset.angleOffset ?? 0);
        const safeOffset = Number.isFinite(angleOffset) ? angleOffset : 0;
        bee.style.transform = beeOrbitTransform(state.angle + safeOffset, state.currentRadius);
      });

      frameId = window.requestAnimationFrame(tick);
    };

    frameId = window.requestAnimationFrame(tick);
    return () => {
      window.cancelAnimationFrame(frameId);
    };
  }, []);

  const renderBee = (index: number, angleOffset: number, layerClass = "") => {
    return (
      <span
        key={`${layerClass || "main"}-${index}`}
        className={`bee-unit b${index + 1}${layerClass ? ` ${layerClass}` : ""}`}
        data-angle-offset={angleOffset}
        data-static-index={index}
        style={
          {
            left: "50%",
            top: "50%",
            "--bee-angle-offset": `${angleOffset}deg`,
          } as CSSProperties
        }
      >
        <img className="bee-wing-layer wing-left" src={beeWingAsset} alt="" />
        <img className="bee-wing-layer wing-right" src={beeWingAsset} alt="" />
        <img className="bee-body-layer" src={beeBodyAsset} alt="" />
      </span>
    );
  };

  const bees = Array.from({ length: beeCount }, (_, index) => {
    const baseAngle = index * 120 - 18;
    return [
      renderBee(index, baseAngle - 24, "bee-ghost bee-ghost-far"),
      renderBee(index, baseAngle - 12, "bee-ghost bee-ghost-near"),
      renderBee(index, baseAngle),
    ];
  }).flat();

  return (
    <div
      ref={meterRef}
      className={`living-meter image-living hive-meter hive-compact${activeMotion ? "" : " bee-static"}`}
      data-tauri-drag-region="deep"
      style={
        {
          "--wing-speed": wingSpeed,
          "--bee-trail-near-opacity": (trailIntensity * BEE_TRAIL_NEAR_MAX_OPACITY).toFixed(3),
          "--bee-trail-far-opacity": (trailIntensity * BEE_TRAIL_FAR_MAX_OPACITY).toFixed(3),
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
  const safeRate = Number.isFinite(rate) ? Math.max(0, rate) : 0;
  const normalized =
    Math.log1p(safeRate / BEE_SPEED_LOG_BASE_RATE_PER_MIN)
    / Math.log1p(BEE_SPEED_FULL_RATE_PER_MIN / BEE_SPEED_LOG_BASE_RATE_PER_MIN);
  return Math.min(1, Math.max(0, normalized));
}

function useBeeMotionRatio(tokenMotionRatio: number) {
  const [visibleMotionRatio, setVisibleMotionRatio] = useState(tokenMotionRatio);
  const targetMotionRatioRef = useRef(tokenMotionRatio);
  const visibleMotionRatioRef = useRef(tokenMotionRatio);
  const renderedMotionRatioRef = useRef(tokenMotionRatio);

  useEffect(() => {
    targetMotionRatioRef.current = Math.min(1, Math.max(0, tokenMotionRatio));
  }, [tokenMotionRatio]);

  useEffect(() => {
    let frameId = 0;
    let lastTime = 0;

    const tick = (time: number) => {
      if (!lastTime) {
        lastTime = time;
      }

      const frameSeconds = Math.min(
        BEE_ORBIT_MAX_FRAME_SECONDS,
        Math.max(0, (time - lastTime) / 1000),
      );
      lastTime = time;

      const current = visibleMotionRatioRef.current;
      const target = targetMotionRatioRef.current;
      const next = nextBeeMotionRatio(current, target, frameSeconds);
      visibleMotionRatioRef.current = next;

      if (Math.abs(next - renderedMotionRatioRef.current) >= BEE_MOTION_RENDER_EPSILON || next === 0) {
        renderedMotionRatioRef.current = next;
        setVisibleMotionRatio(next);
      }

      frameId = window.requestAnimationFrame(tick);
    };

    frameId = window.requestAnimationFrame(tick);

    return () => {
      window.cancelAnimationFrame(frameId);
    };
  }, []);

  return visibleMotionRatio;
}

function nextBeeMotionRatio(current: number, target: number, frameSeconds: number) {
  const safeCurrent = Math.min(1, Math.max(0, current));
  const safeTarget = Math.min(1, Math.max(0, target));
  const safeFrameSeconds = Math.min(
    BEE_ORBIT_MAX_FRAME_SECONDS,
    Math.max(0, frameSeconds),
  );

  if (safeTarget <= 0) {
    const next = safeCurrent * Math.exp(-safeFrameSeconds * BEE_MOTION_IDLE_DECAY_PER_SECOND);
    return next <= BEE_MOTION_STOP_RATIO ? 0 : next;
  }

  const speed =
    safeTarget > safeCurrent
      ? BEE_MOTION_ACCELERATION_PER_SECOND
      : BEE_MOTION_DECELERATION_PER_SECOND;
  const smoothing = 1 - Math.exp(-safeFrameSeconds * speed);
  return safeCurrent + (safeTarget - safeCurrent) * smoothing;
}

function beeMotionRatio(speedRatio: number) {
  const safeRatio = Number.isFinite(speedRatio) ? Math.max(0, speedRatio) : 0;
  if (safeRatio <= 0) {
    return 0;
  }
  return Math.min(1, Math.max(BEE_ORBIT_IDLE_RATIO, safeRatio));
}

function beeOrbitDurationSeconds(motionRatio: number) {
  const safeRatio = Math.min(1, Math.max(0, motionRatio));
  return BEE_ORBIT_SLOW_SECONDS - safeRatio * (BEE_ORBIT_SLOW_SECONDS - BEE_ORBIT_FAST_SECONDS);
}

function beeTrailIntensity(motionRatio: number) {
  const safeRatio = Math.min(1, Math.max(0, motionRatio));
  return Math.min(1, Math.max(0, (safeRatio - BEE_TRAIL_START_RATIO) / (1 - BEE_TRAIL_START_RATIO)));
}

function beeWingDurationMs(motionRatio: number) {
  const safeRatio = Math.min(1, Math.max(0, motionRatio));
  return Math.round(BEE_WING_SLOW_MS - safeRatio * (BEE_WING_SLOW_MS - BEE_WING_FAST_MS));
}

function beeOrbitRadiusPx(motionRatio: number) {
  const safeRatio = Math.min(1, Math.max(0, motionRatio));
  const radiusIntensity = Math.pow(safeRatio, BEE_ORBIT_RADIUS_CURVE);
  return Number(
    (BEE_ORBIT_BASE_RADIUS_PX
      + radiusIntensity * (BEE_ORBIT_FAST_RADIUS_PX - BEE_ORBIT_BASE_RADIUS_PX)).toFixed(1),
  );
}

function beeOrbitTransform(angleDeg: number, radiusPx: number) {
  return `translate(-50%, -50%) rotate(${angleDeg.toFixed(3)}deg) translateX(${radiusPx.toFixed(2)}px) rotate(${BEE_FACING_ROTATION_DEG}deg) scale(${BEE_ACTIVE_SCALE})`;
}

function beeStaticPlacement(index: number) {
  const safeIndex = Number.isFinite(index) ? Math.max(0, Math.trunc(index)) : 0;
  return BEE_STATIC_PLACEMENTS[safeIndex % BEE_STATIC_PLACEMENTS.length];
}

function beeStaticTransform(placement: (typeof BEE_STATIC_PLACEMENTS)[number]) {
  const direction = placement.flipX ? -1 : 1;
  return `translate(-50%, -50%) translate(${placement.xPx}px, ${placement.yPx}px) rotate(${placement.rotationDeg}deg) scaleX(${direction}) scale(${placement.scale})`;
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
