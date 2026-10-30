import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Settings } from "lucide-react";
import {
  getUsageSummary,
  onUsageUpdated,
  showWindow,
  syncNow,
  type QuotaSnapshot,
  type UsageSummary,
  PROVIDERS,
} from "../api";
import { Ring, formatUsd } from "../lib/ui";

type ResizeDir =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

function displayPct(q: QuotaSnapshot): number | null {
  if (q.primary_window_percent != null) return q.primary_window_percent;
  return q.used_percent_month;
}

function subLabel(q: QuotaSnapshot): string | undefined {
  if (q.status === "needs_session" || q.status === "needs_pat") return "setup";
  if (q.status === "manual") return "manual";
  if (q.primary_window_label && q.secondary_window_percent != null) {
    return `${q.secondary_window_label ?? "Alt"} ${q.secondary_window_percent.toFixed(0)}%`;
  }
  if (q.used_percent_year != null) {
    return `Yr ${q.used_percent_year.toFixed(0)}%`;
  }
  if (q.used_usd_month != null && q.limit_usd_month != null) {
    return `${formatUsd(q.used_usd_month)}/${formatUsd(q.limit_usd_month)}`;
  }
  return undefined;
}

function startResize(e: React.MouseEvent, direction: ResizeDir) {
  e.stopPropagation();
  e.preventDefault();
  void getCurrentWindow().startResizeDragging(direction);
}

function Edge({
  direction,
  className,
  label,
}: {
  direction: ResizeDir;
  className: string;
  label: string;
}) {
  return (
    <div
      data-no-drag
      role="separator"
      aria-label={label}
      title="Drag to resize"
      onMouseDown={(e) => startResize(e, direction)}
      className={`absolute z-20 ${className}`}
    />
  );
}

const RING_MIN = 34;
const RING_MAX = 112;
const RING_LABEL_STACK = 36;

function ringSizeForArea(width: number, height: number, count: number) {
  if (width < 1 || height < 1) return 44;
  const minCell = 52;
  const cols = Math.max(1, Math.min(count, Math.floor(width / minCell)));
  const rows = Math.ceil(count / cols);
  const cellW = width / cols;
  const cellH = height / rows;
  const fromW = cellW - 6;
  const fromH = cellH - RING_LABEL_STACK;
  return Math.floor(Math.max(RING_MIN, Math.min(RING_MAX, fromW, fromH)));
}

export default function WidgetPage() {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const ringsHostRef = useRef<HTMLDivElement>(null);
  const [ringSize, setRingSize] = useState(44);

  const refresh = useCallback(async () => {
    try {
      setSummary(await getUsageSummary());
    } catch {
      setSummary(await syncNow());
    }
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = onUsageUpdated(setSummary);
    const id = window.setInterval(refresh, 60_000);
    return () => {
      unlisten.then((fn) => fn());
      window.clearInterval(id);
    };
  }, [refresh]);

  useEffect(() => {
    const el = ringsHostRef.current;
    if (!el) return;
    const update = () => {
      const { width, height } = el.getBoundingClientRect();
      setRingSize(ringSizeForArea(width, height, PROVIDERS.length));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const quotasById = useMemo(() => {
    const m = new Map<string, QuotaSnapshot>();
    summary?.quotas.forEach((q) => m.set(q.provider, q));
    return m;
  }, [summary]);

  const onMouseDown = async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("[data-no-drag]")) return;
    await getCurrentWindow().startDragging();
  };

  const openDetails = (provider?: string) => showWindow("details", provider);
  const openSettings = (ev: React.MouseEvent) => {
    ev.stopPropagation();
    showWindow("settings");
  };

  return (
    <div
      className="h-screen w-screen p-1 select-none cursor-grab active:cursor-grabbing box-border"
      onMouseDown={onMouseDown}
      title="Drag to move · click a provider for details"
    >
      <div className="relative h-full w-full rounded-2xl border border-surface-border bg-surface shadow-2xl backdrop-blur-md px-3 py-2.5 flex flex-col">
        <div className="relative z-30 flex items-center justify-between gap-2 mb-2 shrink-0">
          <div
            className="cursor-pointer"
            data-no-drag
            onClick={() => openDetails()}
            onMouseDown={(e) => e.stopPropagation()}
          >
            <p className="text-xs font-semibold text-slate-100 tracking-tight">
              Token Monitor
            </p>
            <p className="text-[10px] text-slate-500">
              Mo {formatUsd(summary?.month_spend_usd)} · proj{" "}
              {formatUsd(summary?.projected_month_usd)}
            </p>
          </div>
          <button
            type="button"
            data-no-drag
            onClick={openSettings}
            onMouseDown={(e) => e.stopPropagation()}
            className="p-1 rounded-md text-slate-400 hover:text-slate-200 hover:bg-white/5"
            aria-label="Settings"
          >
            <Settings size={14} />
          </button>
        </div>
        <div
          ref={ringsHostRef}
          className="flex-1 min-h-0 flex items-center justify-center overflow-hidden pb-1"
        >
          <div className="flex flex-wrap gap-x-1 gap-y-1.5 justify-center items-center content-center max-w-full max-h-full overflow-auto px-1">
            {PROVIDERS.map(({ id, label }) => {
              const q = quotasById.get(id);
              const pct = q ? displayPct(q) : null;
              return (
                <Ring
                  key={id}
                  label={label}
                  pct={pct}
                  size={ringSize}
                  sub={q ? subLabel(q) : "…"}
                  onClick={() => openDetails(id)}
                />
              );
            })}
          </div>
        </div>

        <Edge direction="North" label="Resize top" className="top-0 left-3 right-3 h-2 cursor-n-resize" />
        <Edge direction="South" label="Resize bottom" className="bottom-0 left-3 right-3 h-2 cursor-s-resize" />
        <Edge direction="West" label="Resize left" className="left-0 top-3 bottom-3 w-2 cursor-w-resize" />
        <Edge direction="East" label="Resize right" className="right-0 top-3 bottom-3 w-2 cursor-e-resize" />
        <Edge direction="NorthWest" label="Resize top-left" className="top-0 left-0 w-3 h-3 cursor-nw-resize" />
        <Edge direction="NorthEast" label="Resize top-right" className="top-0 right-0 w-3 h-3 cursor-ne-resize" />
        <Edge direction="SouthWest" label="Resize bottom-left" className="bottom-0 left-0 w-3 h-3 cursor-sw-resize" />

        <button
          type="button"
          data-no-drag
          aria-label="Resize from corner"
          title="Drag to resize"
          onMouseDown={(e) => startResize(e, "SouthEast")}
          className="absolute right-0 bottom-0 z-30 w-8 h-8 cursor-se-resize flex items-end justify-end p-1.5 text-slate-400 hover:text-slate-200"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
            <path
              d="M5 13h8M9 13h4M13 9v4M13 5v8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
            />
            <path
              d="M8 13h5M13 8v5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              opacity="0.45"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}
