import { useCallback, useEffect, useMemo, useState } from "react";
import { eachDayOfInterval, format, parseISO } from "date-fns";
import type { ModelUsageAdvice, ModelUsageRow, ModelUsageSeries } from "../api";
import { generateCursorModelAdvice } from "../api";
import {
  Ring,
  formatPct,
  formatUsd,
  formatUsdCompact,
  formatUsdPerRun,
  pctColor,
} from "../lib/ui";
import clsx from "clsx";
import { isJiraKey, openJiraIssue } from "../lib/taskContext";

const RING_SIZE = 56;
const COL_MIN_PX = 92;
const PIE_SIZE = 220;
const PIE_R = 96;

const MODEL_COLORS = [
  "#818cf8",
  "#34d399",
  "#fbbf24",
  "#f472b6",
  "#38bdf8",
  "#a78bfa",
  "#fb7185",
  "#4ade80",
  "#f97316",
  "#22d3ee",
  "#e879f9",
  "#84cc16",
];

type ViewMode = "rings" | "pie" | "trend" | "perRun" | "advice";
type TrendMetric = "runs" | "cost";

function modelColor(_model: string, index: number) {
  return MODEL_COLORS[index % MODEL_COLORS.length];
}

function polar(cx: number, cy: number, r: number, angle: number) {
  const a = angle - Math.PI / 2;
  return [cx + r * Math.cos(a), cy + r * Math.sin(a)] as const;
}

function piePath(cx: number, cy: number, r: number, start: number, end: number) {
  const [x1, y1] = polar(cx, cy, r, start);
  const [x2, y2] = polar(cx, cy, r, end);
  const large = end - start > Math.PI ? 1 : 0;
  return `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2} Z`;
}

function ViewToggle({
  mode,
  onChange,
}: {
  mode: ViewMode;
  onChange: (mode: ViewMode) => void;
}) {
  const options: { id: ViewMode; label: string }[] = [
    { id: "rings", label: "Rings" },
    { id: "pie", label: "Pie" },
    { id: "trend", label: "Over time" },
    { id: "perRun", label: "Per run" },
    { id: "advice", label: "Advice" },
  ];
  return (
    <div className="flex rounded-lg border border-slate-700 p-0.5 bg-slate-950/60">
      {options.map((opt) => (
        <button
          key={opt.id}
          type="button"
          aria-pressed={mode === opt.id}
          onClick={() => onChange(opt.id)}
          className={clsx(
            "px-2.5 py-1 text-xs font-medium rounded-md",
            mode === opt.id
              ? "bg-slate-700 text-slate-100"
              : "text-slate-400 hover:text-slate-200",
          )}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function PieView({
  rows,
  showCost,
  onToggle,
}: {
  rows: ModelUsageRow[];
  showCost: boolean;
  onToggle: () => void;
}) {
  const [hover, setHover] = useState<string | null>(null);
  const slices = useMemo(() => {
    const total = rows.reduce((s, r) => s + r.cost_usd, 0);
    let acc = 0;
    return rows.map((row, i) => {
      const frac = total > 0 ? row.cost_usd / total : 0;
      const start = acc * 2 * Math.PI;
      acc += frac;
      const end = acc * 2 * Math.PI;
      return { row, frac, start, end, color: modelColor(row.model, i) };
    });
  }, [rows]);

  const cx = PIE_SIZE / 2;
  const cy = PIE_SIZE / 2;
  const full = slices.length === 1 && slices[0].frac >= 0.999;

  return (
    <div className="flex flex-col md:flex-row items-center gap-5">
      <svg
        width={PIE_SIZE}
        height={PIE_SIZE}
        viewBox={`0 0 ${PIE_SIZE} ${PIE_SIZE}`}
        role="img"
        aria-label="Model spend share this billing period"
        className="shrink-0 cursor-pointer"
        onClick={onToggle}
      >
        {full ? (
          <circle
            cx={cx}
            cy={cy}
            r={PIE_R}
            fill={slices[0].color}
            onMouseEnter={() => setHover(slices[0].row.model)}
            onMouseLeave={() => setHover(null)}
          />
        ) : (
          slices.map((s) =>
            s.frac <= 0 ? null : (
              <path
                key={s.row.model}
                d={piePath(cx, cy, PIE_R, s.start, s.end)}
                fill={s.color}
                opacity={hover && hover !== s.row.model ? 0.45 : 1}
                onMouseEnter={() => setHover(s.row.model)}
                onMouseLeave={() => setHover(null)}
              >
                <title>
                  {s.row.model} · {formatPct(s.row.share_percent)} ·{" "}
                  {formatUsd(s.row.cost_usd)}
                </title>
              </path>
            ),
          )
        )}
      </svg>
      <ul className="w-full min-w-0 space-y-1.5">
        {slices.map((s) => {
          const name = s.row.model.trim() || "unknown";
          const primary = showCost
            ? formatUsdCompact(s.row.cost_usd)
            : formatPct(s.row.share_percent);
          const secondary = showCost
            ? formatPct(s.row.share_percent)
            : formatUsd(s.row.cost_usd);
          return (
            <li
              key={s.row.model}
              className={clsx(
                "flex items-center gap-2 text-xs",
                hover === s.row.model && "text-slate-100",
              )}
              onMouseEnter={() => setHover(s.row.model)}
              onMouseLeave={() => setHover(null)}
            >
              <span
                className="w-2.5 h-2.5 rounded-sm shrink-0"
                style={{ backgroundColor: s.color }}
              />
              <span className="truncate text-slate-200 flex-1" title={name}>
                {name}
              </span>
              <span className="font-medium text-slate-100 tabular-nums">
                {primary}
              </span>
              <span className="text-slate-500 tabular-nums">{secondary}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function TrendView({
  rows,
  series,
}: {
  rows: ModelUsageRow[];
  series: ModelUsageSeries | null;
}) {
  const [metric, setMetric] = useState<TrendMetric>("runs");
  const [hoverModel, setHoverModel] = useState<string | null>(null);
  const [hoverPoint, setHoverPoint] = useState<{
    day: string;
    model: string;
  } | null>(null);

  const chart = useMemo(() => {
    const models = rows.map((r) => r.model);
    const start = parseISO(series?.cycle_start || format(new Date(), "yyyy-MM-dd"));
    const end = new Date();
    const days =
      start <= end
        ? eachDayOfInterval({ start, end }).map((d) => format(d, "yyyy-MM-dd"))
        : [format(end, "yyyy-MM-dd")];
    const key = (day: string, model: string) => `${day}\t${model}`;
    const map = new Map<string, { run_count: number; cost_usd: number }>();
    for (const p of series?.points ?? []) {
      map.set(key(p.day, p.model), {
        run_count: p.run_count,
        cost_usd: p.cost_usd,
      });
    }
    const value = (day: string, model: string) => {
      const cell = map.get(key(day, model));
      if (!cell) return 0;
      return metric === "runs" ? cell.run_count : cell.cost_usd;
    };
    let max = 0;
    for (const day of days) {
      for (const model of models) {
        max = Math.max(max, value(day, model));
      }
    }
    if (max <= 0) max = 1;
    return { models, days, value, max };
  }, [rows, series, metric]);

  const W = 640;
  const H = 220;
  const pad = { l: 36, r: 12, t: 12, b: 28 };
  const innerW = W - pad.l - pad.r;
  const innerH = H - pad.t - pad.b;
  const n = chart.days.length;
  const xAt = (i: number) =>
    pad.l + (n <= 1 ? innerW / 2 : (i / (n - 1)) * innerW);
  const yAt = (v: number) => pad.t + innerH - (v / chart.max) * innerH;
  const yTicks = [0, 0.5, 1].map((t) => t * chart.max);
  const xStep = n <= 8 ? 1 : n <= 16 ? 2 : Math.ceil(n / 8);

  return (
    <div>
      <div className="flex justify-end mb-2">
        <div className="flex rounded-lg border border-slate-700 p-0.5">
          {(["runs", "cost"] as TrendMetric[]).map((id) => (
            <button
              key={id}
              type="button"
              aria-pressed={metric === id}
              onClick={() => setMetric(id)}
              className={clsx(
                "px-2 py-0.5 text-[11px] font-medium rounded-md",
                metric === id
                  ? "bg-slate-700 text-slate-100"
                  : "text-slate-400 hover:text-slate-200",
              )}
            >
              {id === "runs" ? "Frequency" : "Cost"}
            </button>
          ))}
        </div>
      </div>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full h-auto"
        role="img"
        aria-label={
          metric === "runs"
            ? "Model call frequency by day this billing period"
            : "Model spend by day this billing period"
        }
      >
        {yTicks.map((tick) => (
          <g key={tick}>
            <line
              x1={pad.l}
              x2={W - pad.r}
              y1={yAt(tick)}
              y2={yAt(tick)}
              stroke="rgba(148,163,184,0.18)"
            />
            <text
              x={pad.l - 6}
              y={yAt(tick) + 3}
              textAnchor="end"
              className="fill-slate-500"
              fontSize="10"
            >
              {metric === "cost"
                ? formatUsdCompact(tick)
                : String(Math.round(tick))}
            </text>
          </g>
        ))}
        {chart.days.map((day, i) =>
          i % xStep === 0 || i === n - 1 ? (
            <text
              key={day}
              x={xAt(i)}
              y={H - 8}
              textAnchor="middle"
              className="fill-slate-500"
              fontSize="10"
            >
              {format(parseISO(day), "M/d")}
            </text>
          ) : null,
        )}
        {[...chart.models]
          .map((model, mi) => ({ model, mi }))
          .sort((a, b) => {
            if (a.model === hoverModel) return 1;
            if (b.model === hoverModel) return -1;
            return 0;
          })
          .map(({ model, mi }) => {
          const pts = chart.days.map((day, i) => {
            const v = chart.value(day, model);
            return { day, x: xAt(i), y: yAt(v), v };
          });
          const d = pts
            .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x.toFixed(1)} ${p.y.toFixed(1)}`)
            .join(" ");
          const color = modelColor(model, mi);
          const dimmed = hoverModel != null && hoverModel !== model;
          const emphasized = hoverModel === model;
          return (
            <g
              key={model}
              opacity={dimmed ? 0.18 : 1}
              onMouseEnter={() => setHoverModel(model)}
              onMouseLeave={() => {
                setHoverModel(null);
                setHoverPoint(null);
              }}
            >
              <path
                d={d}
                fill="none"
                stroke={color}
                strokeWidth={emphasized ? 3 : 1.75}
                strokeLinejoin="round"
                strokeLinecap="round"
              />
              {pts.map((p) => (
                <circle
                  key={`${model}-${p.day}`}
                  cx={p.x}
                  cy={p.y}
                  r={
                    hoverPoint?.day === p.day && hoverPoint?.model === model
                      ? 4.5
                      : emphasized
                        ? 3.25
                        : 2.25
                  }
                  fill={color}
                  className="cursor-pointer"
                  onMouseEnter={() => {
                    setHoverModel(model);
                    setHoverPoint({ day: p.day, model });
                  }}
                >
                  <title>
                    {model} · {format(parseISO(p.day), "MMM d")} ·{" "}
                    {metric === "cost"
                      ? formatUsd(p.v)
                      : `${p.v} call${p.v === 1 ? "" : "s"}`}
                  </title>
                </circle>
              ))}
            </g>
          );
        })}
      </svg>
      <ul className="mt-3 flex flex-wrap gap-x-3 gap-y-1">
        {chart.models.map((model, i) => (
          <li
            key={model}
            className={clsx(
              "flex items-center gap-1.5 text-[11px] cursor-pointer rounded px-1 py-0.5 -mx-1",
              hoverModel === model
                ? "text-slate-100 bg-slate-800/80"
                : hoverModel
                  ? "text-slate-500"
                  : "text-slate-300",
            )}
            onMouseEnter={() => setHoverModel(model)}
            onMouseLeave={() => setHoverModel(null)}
          >
            <span
              className="w-2.5 h-0.5 rounded-full shrink-0"
              style={{
                backgroundColor: modelColor(model, i),
                height: hoverModel === model ? 3 : 2,
              }}
            />
            <span className="truncate max-w-[10rem]" title={model}>
              {model.trim() || "unknown"}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PerRunBars({ rows }: { rows: ModelUsageRow[] }) {
  const [hover, setHover] = useState<string | null>(null);
  const ranked = useMemo(() => {
    return rows
      .map((row, colorIndex) => ({
        model: row.model,
        run_count: row.run_count,
        cost_usd: row.cost_usd,
        perRun:
          row.run_count > 0 ? row.cost_usd / row.run_count : 0,
        colorIndex,
      }))
      .sort((a, b) => b.perRun - a.perRun);
  }, [rows]);
  const max = ranked[0]?.perRun || 1;

  return (
    <ol className="space-y-2">
      {ranked.map((row, rank) => {
        const name = row.model.trim() || "unknown";
        const width = max > 0 ? (row.perRun / max) * 100 : 0;
        const active = hover === row.model;
        return (
          <li
            key={row.model}
            className="grid grid-cols-[1.25rem_minmax(0,8.5rem)_1fr_auto] items-center gap-2"
            onMouseEnter={() => setHover(row.model)}
            onMouseLeave={() => setHover(null)}
          >
            <span className="text-[11px] text-slate-500 tabular-nums text-right">
              {rank + 1}
            </span>
            <span
              className={clsx(
                "truncate text-xs",
                active ? "text-slate-100" : "text-slate-300",
              )}
              title={name}
            >
              {name}
            </span>
            <div className="h-4 rounded-sm bg-slate-800/80 overflow-hidden">
              <div
                className="h-full rounded-sm transition-[width,opacity]"
                style={{
                  width: `${Math.max(width, row.perRun > 0 ? 1.5 : 0)}%`,
                  backgroundColor: modelColor(row.model, row.colorIndex),
                  opacity: hover && !active ? 0.28 : 1,
                }}
              />
            </div>
            <span className="text-xs font-medium text-slate-100 tabular-nums whitespace-nowrap">
              {formatUsdPerRun(row.perRun)}
              <span className="ml-1.5 text-slate-500 font-normal">
                {row.run_count} run{row.run_count === 1 ? "" : "s"}
              </span>
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function renderPlainWithTokenHighlight(text: string, keyPrefix: string) {
  const tokenPattern = /(~[\d.]+[kM]? tokens)/g;
  const parts = text.split(tokenPattern);
  return parts.map((part, i) =>
    /^~[\d.]+[kM]? tokens$/.test(part) ? (
      <span
        key={`${keyPrefix}-tok-${i}`}
        className="text-emerald-400 tabular-nums font-medium"
      >
        {part}
      </span>
    ) : (
      part
    ),
  );
}

function renderInline(text: string) {
  const parts = text.split(/(\*\*[^*]+\*\*)/g);
  return parts.map((part, i) => {
    if (part.startsWith("**") && part.endsWith("**")) {
      const inner = part.slice(2, -2);
      if (isJiraKey(inner)) {
        return (
          <button
            key={i}
            type="button"
            onClick={() => void openJiraIssue(inner)}
            className="font-mono text-indigo-300 hover:text-indigo-200 hover:underline font-medium cursor-pointer align-baseline"
          >
            {inner}
          </button>
        );
      }
      return (
        <strong key={i} className="text-slate-100 font-medium">
          {inner}
        </strong>
      );
    }
    return (
      <span key={i}>{renderPlainWithTokenHighlight(part, `p-${i}`)}</span>
    );
  });
}

function AdviceMarkdown({ text }: { text: string }) {
  const blocks = useMemo(() => {
    const lines = text.split("\n");
    const out: { type: "h" | "p" | "ul"; content: string | string[] }[] = [];
    let bullets: string[] = [];
    const flushBullets = () => {
      if (bullets.length) {
        out.push({ type: "ul", content: bullets });
        bullets = [];
      }
    };
    for (const line of lines) {
      const t = line.trim();
      if (!t) {
        flushBullets();
        continue;
      }
      if (t.startsWith("## ")) {
        flushBullets();
        out.push({ type: "h", content: t.slice(3) });
      } else if (t.startsWith("- ") || t.startsWith("* ")) {
        bullets.push(t.slice(2));
      } else {
        flushBullets();
        out.push({ type: "p", content: t });
      }
    }
    flushBullets();
    return out;
  }, [text]);

  return (
    <div className="text-sm text-slate-300 space-y-3 leading-relaxed">
      {blocks.map((block, i) => {
        if (block.type === "h") {
          const heading = block.content as string;
          const isExec = heading === "Executive summary";
          return (
            <h3
              key={i}
              className={clsx(
                "font-semibold mt-4 first:mt-0",
                isExec
                  ? "text-base text-indigo-200 border-b border-slate-700/80 pb-2 mb-1"
                  : "text-slate-100 text-sm",
              )}
            >
              {heading}
            </h3>
          );
        }
        if (block.type === "ul") {
          return (
            <ul key={i} className="list-disc pl-5 space-y-1 text-slate-300">
              {(block.content as string[]).map((item, j) => (
                <li key={j}>{renderInline(item)}</li>
              ))}
            </ul>
          );
        }
        const paragraph = block.content as string;
        const execLead =
          paragraph.includes("Heaviest tickets by token usage") ||
          paragraph.includes("TL;DR:");
        return (
          <p
            key={i}
            className={clsx(
              execLead
                ? "text-[15px] leading-relaxed text-slate-200"
                : "text-slate-300",
            )}
          >
            {renderInline(paragraph)}
          </p>
        );
      })}
    </div>
  );
}

function AdviceView({ cycleStart }: { cycleStart: string | null }) {
  const [advice, setAdvice] = useState<ModelUsageAdvice | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await generateCursorModelAdvice();
      setAdvice(result);
    } catch (e) {
      setAdvice(null);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    run();
  }, [run]);

  return (
    <div>
      <p className="text-xs text-slate-500 mb-3">
        Offline analysis of synced runs this cycle — no external API, no Cursor
        agent, and no prompt data leaves your machine.
        {cycleStart ? (
          <>
            {" "}
            Cycle from {format(parseISO(cycleStart), "MMM d, yyyy")}.
          </>
        ) : null}
      </p>
      <div className="flex flex-wrap items-center gap-2 mb-4">
        <button
          type="button"
          onClick={run}
          disabled={loading}
          className="px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-sm font-medium"
        >
          {loading ? "Analyzing…" : "Refresh advice"}
        </button>
        {advice ? (
          <span className="text-[11px] text-slate-500">
            local · {format(parseISO(advice.generated_at), "MMM d, h:mm a")}
          </span>
        ) : null}
      </div>
      {error ? (
        <p className="text-sm text-rose-400/90 whitespace-pre-wrap">{error}</p>
      ) : null}
      {loading && !advice ? (
        <p className="text-sm text-slate-400">Reviewing billing-cycle usage…</p>
      ) : null}
      {advice ? <AdviceMarkdown text={advice.text} /> : null}
      {!advice && !loading && !error ? (
        <p className="text-sm text-slate-500">Waiting for analysis…</p>
      ) : null}
    </div>
  );
}

export default function CursorModelUsageRings({
  rows,
  series = null,
}: {
  rows: ModelUsageRow[];
  series?: ModelUsageSeries | null;
}) {
  const [showCost, setShowCost] = useState(false);
  const [mode, setMode] = useState<ViewMode>("rings");

  const gridColumns = useMemo(() => {
    const n = rows.length;
    if (n === 0) return undefined;
    return `repeat(${n}, minmax(${COL_MIN_PX}px, 1fr))`;
  }, [rows.length]);

  if (rows.length === 0) return null;

  const toggleAll = () => setShowCost((v) => !v);
  const hint =
    mode === "rings"
      ? "Ring fill = share of synced Cursor spend this cycle · click any ring to toggle all between % and cost"
      : mode === "pie"
        ? "Slice size = share of synced Cursor spend this cycle · click the pie to toggle % and cost"
        : mode === "trend"
          ? "Each line is one model this billing cycle · frequency is billed calls per day"
          : mode === "perRun"
            ? "Ranked by average cost per billed agent run this cycle (total spend ÷ run count)"
            : "Local heuristics on this cycle's usage — premium vs fast models and costly prompt patterns";

  return (
    <section className="mb-6 rounded-xl border border-slate-800 bg-slate-900/30 p-4">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-slate-200">
            Models this billing period
          </h2>
          <p className="text-xs text-slate-500 mt-0.5">{hint}</p>
        </div>
        <ViewToggle mode={mode} onChange={setMode} />
      </div>
      {mode === "rings" ? (
        <div
          className="w-full grid gap-x-1 gap-y-4 justify-items-center items-start"
          style={{ gridTemplateColumns: gridColumns }}
        >
          {rows.map((row) => {
            const pct = row.share_percent;
            const name = row.model.trim() || "unknown";
            return (
              <Ring
                key={row.model}
                size={RING_SIZE}
                fillColumn
                className="max-w-[10.5rem]"
                label={name}
                labelClassName="line-clamp-2 whitespace-normal break-all hyphens-auto"
                pct={pct}
                center={showCost ? formatUsdCompact(row.cost_usd) : formatPct(pct)}
                centerClassName={showCost ? "text-slate-200" : pctColor(pct)}
                sub={showCost ? formatPct(pct) : formatUsd(row.cost_usd)}
                title={name}
                styledTooltip
                onClick={toggleAll}
              />
            );
          })}
        </div>
      ) : null}
      {mode === "pie" ? (
        <PieView rows={rows} showCost={showCost} onToggle={toggleAll} />
      ) : null}
      {mode === "trend" ? <TrendView rows={rows} series={series} /> : null}
      {mode === "perRun" ? <PerRunBars rows={rows} /> : null}
      {mode === "advice" ? (
        <AdviceView cycleStart={series?.cycle_start ?? null} />
      ) : null}
    </section>
  );
}
