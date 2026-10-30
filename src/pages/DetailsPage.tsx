import { useCallback, useEffect, useMemo, useState } from "react";
import { format } from "date-fns";
import { listen } from "@tauri-apps/api/event";
import {
  getUsageSummary,
  getProviderModelUsage,
  listRuns,
  syncNow,
  type ModelUsageRow,
  type RunRecord,
  type UsageSummary,
  PROVIDERS,
} from "../api";
import { formatPct, formatUsd, providerLabel, pctColor } from "../lib/ui";
import { promptSpecificityScore, specificityColor } from "../lib/specificity";
import { useResizableColumns } from "../lib/useResizableColumns";
import PromptCell from "../components/PromptCell";
import ResizableTh from "../components/ResizableTh";
import CursorModelUsageRings from "../components/CursorModelUsageRings";
import clsx from "clsx";

export type RunSort = "cost" | "cost_asc" | "started_at" | "model";

const DETAILS_COL_DEFAULTS = {
  when: 132,
  provider: 108,
  run: 320,
  specificity: 96,
  model: 148,
  tokens: 88,
  cost: 72,
} as const;

type DetailsCol = keyof typeof DETAILS_COL_DEFAULTS;

export default function DetailsPage() {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [modelUsage, setModelUsage] = useState<ModelUsageRow[]>([]);
  const [provider, setProvider] = useState<string>("all");
  const [sort, setSort] = useState<RunSort>("cost");
  const [loading, setLoading] = useState(true);

  const { widths, keys, onResizeStart } = useResizableColumns<DetailsCol>(
    "token-monitor:details-columns",
    { ...DETAILS_COL_DEFAULTS },
  );

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [s, r, models] = await Promise.all([
        getUsageSummary(),
        listRuns({
          provider: provider === "all" ? undefined : provider,
          limit: 250,
          sort,
        }),
        provider === "cursor"
          ? getProviderModelUsage("cursor")
          : Promise.resolve([] as ModelUsageRow[]),
      ]);
      setSummary(s);
      setRuns(r);
      setModelUsage(models);
    } finally {
      setLoading(false);
    }
  }, [provider, sort]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    const unlisten = listen<string>("details-filter", (e) => {
      if (e.payload) setProvider(e.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const totals = useMemo(() => {
    return runs.reduce(
      (acc, r) => {
        acc.cost += r.cost_usd;
        acc.tokens += r.input_tokens + r.output_tokens + r.cache_tokens;
        return acc;
      },
      { cost: 0, tokens: 0 },
    );
  }, [runs]);

  const sortLabel =
    sort === "cost"
      ? "most expensive"
      : sort === "cost_asc"
        ? "least expensive"
        : sort === "model"
          ? "model (A–Z)"
          : "most recent";

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 p-6 font-sans">
      <header className="flex flex-wrap items-end justify-between gap-4 mb-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Usage details</h1>
          <p className="text-sm text-slate-400 mt-1">
            Runs sorted by {sortLabel} · projected month{" "}
            <span className="text-slate-200 font-medium">
              {formatUsd(summary?.projected_month_usd)}
            </span>
          </p>
          <p className="text-xs text-slate-500 mt-1 max-w-3xl">
            Cursor rows are billed model calls inside a chat. The Run text is the
            nearest matching user prompt from local chat history, not the full
            request payload.
          </p>
        </div>
        <button
          type="button"
          onClick={() => syncNow().then(load)}
          className="px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-sm font-medium"
        >
          Sync now
        </button>
      </header>

      <div className="flex flex-wrap gap-2 mb-4">
        <Tab active={provider === "all"} onClick={() => setProvider("all")}>
          All
        </Tab>
        {PROVIDERS.map((p) => (
          <Tab key={p.id} active={provider === p.id} onClick={() => setProvider(p.id)}>
            {p.label}
          </Tab>
        ))}
      </div>

      {provider === "cursor" && !loading ? (
        <CursorModelUsageRings rows={modelUsage} />
      ) : null}

      <div className="flex gap-3 mb-4 text-sm">
        <label className="flex items-center gap-2 text-slate-400">
          Sort
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as RunSort)}
            className="bg-slate-900 border border-slate-700 rounded-md px-2 py-1 text-slate-200"
          >
            <option value="cost">Most expensive</option>
            <option value="cost_asc">Least expensive</option>
            <option value="started_at">Most recent</option>
            <option value="model">Model (A–Z)</option>
          </select>
        </label>
        <span className="text-slate-500">
          {totals.tokens.toLocaleString()} tokens · {formatUsd(totals.cost)} shown
        </span>
      </div>

      <div className="rounded-xl border border-slate-800 overflow-x-auto">
        <table className="w-full text-sm table-fixed min-w-[720px]">
          <colgroup>
            {keys.map((k) => (
              <col key={k} style={{ width: widths[k] }} />
            ))}
          </colgroup>
          <thead className="bg-slate-900/80 text-slate-400 text-left">
            <tr>
              <ResizableTh onResizeStart={(x) => onResizeStart("when", x)}>
                When
              </ResizableTh>
              <ResizableTh onResizeStart={(x) => onResizeStart("provider", x)}>
                Provider
              </ResizableTh>
              <ResizableTh onResizeStart={(x) => onResizeStart("run", x)}>Run</ResizableTh>
              <ResizableTh
                className="text-right"
                onResizeStart={(x) => onResizeStart("specificity", x)}
              >
                Specificity
              </ResizableTh>
              <ResizableTh onResizeStart={(x) => onResizeStart("model", x)}>
                Model
              </ResizableTh>
              <ResizableTh
                className="text-right"
                onResizeStart={(x) => onResizeStart("tokens", x)}
              >
                Tokens
              </ResizableTh>
              <ResizableTh
                className="text-right"
                onResizeStart={(x) => onResizeStart("cost", x)}
              >
                Cost
              </ResizableTh>
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr>
                <td colSpan={7} className="px-3 py-8 text-center text-slate-500">
                  Loading…
                </td>
              </tr>
            ) : runs.length === 0 ? (
              <tr>
                <td colSpan={7} className="px-3 py-8 text-center text-slate-500">
                  No runs for this provider yet. Add API keys in Settings and
                  click Sync now — Codex and Claude Code also ingest local
                  session logs automatically.
                </td>
              </tr>
            ) : (
              runs.map((r) => {
                const tokens = r.input_tokens + r.output_tokens + r.cache_tokens;
                const prompt = r.title || r.run_id;
                const specificity = promptSpecificityScore(prompt);
                let when = r.started_at;
                try {
                  when = format(new Date(r.started_at), "MMM d, yyyy HH:mm");
                } catch {
                  /* keep raw */
                }
                return (
                  <tr
                    key={`${r.provider}-${r.run_id}`}
                    className="border-t border-slate-800/80 hover:bg-slate-900/40"
                  >
                    <td className="px-3 py-2 whitespace-nowrap text-slate-300 font-mono text-xs">
                      {when}
                    </td>
                    <td className="px-3 py-2">{providerLabel(r.provider)}</td>
                    <td className="px-2 py-1.5 align-top min-w-0">
                      <PromptCell text={prompt} />
                    </td>
                    <td
                      className={clsx(
                        "px-3 py-2 text-right tabular-nums font-medium",
                        specificityColor(specificity),
                      )}
                      title="1 = vague · 10 = highly specific context"
                    >
                      {specificity}/10
                    </td>
                    <td className="px-3 py-2 font-mono text-xs text-slate-400">{r.model}</td>
                    <td className="px-3 py-2 text-right tabular-nums">
                      {tokens.toLocaleString()}
                    </td>
                    <td
                      className={clsx(
                        "px-3 py-2 text-right tabular-nums font-medium",
                        pctColor(r.cost_usd > 1 ? 85 : 40),
                      )}
                    >
                      {formatUsd(r.cost_usd)}
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      {summary ? (
        <section className="mt-8 grid sm:grid-cols-2 lg:grid-cols-3 gap-3">
          {summary.quotas.map((q) => (
            <div
              key={q.provider}
              className="rounded-lg border border-slate-800 bg-slate-900/50 p-3"
            >
              <p className="font-medium">{q.label}</p>
              <p className={clsx("text-lg", pctColor(q.used_percent_month))}>
                Month {formatPct(q.used_percent_month)}
                {q.used_percent_year != null ? ` · Year ${formatPct(q.used_percent_year)}` : ""}
                {q.used_usd_month != null && q.limit_usd_month != null
                  ? ` · ${formatUsd(q.used_usd_month)} / ${formatUsd(q.limit_usd_month)}`
                  : ""}
              </p>
              {q.message ? (
                <p className="text-xs text-slate-500 mt-1">{q.message}</p>
              ) : null}
            </div>
          ))}
        </section>
      ) : null}
    </div>
  );
}

function Tab({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={clsx(
        "px-3 py-1 rounded-full text-sm border transition-colors",
        active
          ? "bg-slate-100 text-slate-900 border-slate-100"
          : "border-slate-700 text-slate-400 hover:border-slate-500",
      )}
    >
      {children}
    </button>
  );
}
