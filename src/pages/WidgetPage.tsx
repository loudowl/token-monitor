import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import FloatingWindowChrome from "../components/FloatingWindowChrome";
import { Ring, formatUsd } from "../lib/ui";

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

  const openDetails = (provider?: string) => showWindow("details", provider);
  const openSettings = (ev: React.MouseEvent) => {
    ev.stopPropagation();
    showWindow("settings");
  };

  return (
    <FloatingWindowChrome>
      <div className="flex flex-col flex-1 min-h-0 px-3 py-2.5 select-none text-slate-100">
        <div
          data-tauri-drag-region
          className="flex items-center justify-between gap-2 mb-2 shrink-0 cursor-grab active:cursor-grabbing"
        >
          <div
            className="cursor-pointer"
            data-tauri-drag-region="false"
            onClick={() => openDetails()}
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
            data-tauri-drag-region="false"
            onClick={openSettings}
            className="p-1 rounded-md text-slate-400 hover:text-slate-200 hover:bg-white/5"
            aria-label="Settings"
          >
            <Settings size={14} />
          </button>
        </div>
        <div
          ref={ringsHostRef}
          data-tauri-drag-region="false"
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
      </div>
    </FloatingWindowChrome>
  );
}
