import { useCallback, useEffect, useMemo, useState } from "react";
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
  return undefined;
}

export default function WidgetPage() {
  const [summary, setSummary] = useState<UsageSummary | null>(null);

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

  const quotasById = useMemo(() => {
    const m = new Map<string, QuotaSnapshot>();
    summary?.quotas.forEach((q) => m.set(q.provider, q));
    return m;
  }, [summary]);

  const onMouseDown = async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("[data-no-drag]")) return;
    await getCurrentWindow().startDragging();
  };

  const startResize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    await getCurrentWindow().startResizeDragging("SouthEast");
  };

  const openDetails = (provider?: string) => showWindow("details", provider);
  const openSettings = (ev: React.MouseEvent) => {
    ev.stopPropagation();
    showWindow("settings");
  };

  return (
    <div
      className="h-screen w-screen p-2 select-none cursor-grab active:cursor-grabbing box-border"
      onMouseDown={onMouseDown}
      title="Drag to move · click a provider for details"
    >
      <div className="relative h-full w-full rounded-2xl border border-surface-border bg-surface shadow-2xl backdrop-blur-md px-3 py-2.5 flex flex-col">
        <div className="flex items-center justify-between gap-2 mb-2 shrink-0">
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
        <div className="flex flex-wrap gap-1 content-start overflow-auto flex-1 pb-3">
          {PROVIDERS.map(({ id, label }) => {
            const q = quotasById.get(id);
            const pct = q ? displayPct(q) : null;
            return (
              <Ring
                key={id}
                label={label}
                pct={pct}
                sub={q ? subLabel(q) : "…"}
                onClick={() => openDetails(id)}
              />
            );
          })}
        </div>
        <button
          type="button"
          data-no-drag
          aria-label="Resize"
          title="Drag to resize"
          onMouseDown={startResize}
          className="absolute right-1 bottom-1 w-3 h-3 cursor-se-resize opacity-60 hover:opacity-100"
          style={{
            background:
              "linear-gradient(135deg, transparent 50%, rgba(148,163,184,0.9) 50%)",
          }}
        />
      </div>
    </div>
  );
}
