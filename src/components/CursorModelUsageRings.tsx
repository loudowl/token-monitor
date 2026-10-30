import { useState } from "react";
import type { ModelUsageRow } from "../api";
import { Ring, formatPct, formatUsd, formatUsdCompact, pctColor } from "../lib/ui";

function shortModelName(model: string, max = 24) {
  const m = model.trim() || "unknown";
  if (m.length <= max) return m;
  return `${m.slice(0, max - 1)}…`;
}

export default function CursorModelUsageRings({ rows }: { rows: ModelUsageRow[] }) {
  const [showCost, setShowCost] = useState(false);

  if (rows.length === 0) return null;

  const toggleAll = () => setShowCost((v) => !v);

  return (
    <section className="mb-6 rounded-xl border border-slate-800 bg-slate-900/30 p-4">
      <div className="mb-3">
        <h2 className="text-sm font-semibold text-slate-200">Models this billing period</h2>
        <p className="text-xs text-slate-500 mt-0.5">
          Ring fill = share of synced Cursor spend this cycle · click any ring to toggle
          all between % and cost
        </p>
      </div>
      <div className="flex flex-wrap gap-3 justify-center sm:justify-start">
        {rows.map((row) => {
          const pct = row.share_percent;
          return (
            <Ring
              key={row.model}
              size={56}
              label={shortModelName(row.model)}
              pct={pct}
              center={showCost ? formatUsdCompact(row.cost_usd) : formatPct(pct)}
              centerClassName={showCost ? "text-slate-200" : pctColor(pct)}
              sub={showCost ? formatPct(pct) : formatUsd(row.cost_usd)}
              title={row.model.trim() || "unknown"}
              styledTooltip
              onClick={toggleAll}
            />
          );
        })}
      </div>
    </section>
  );
}
