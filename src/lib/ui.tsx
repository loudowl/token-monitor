import clsx from "clsx";

export function pctColor(pct: number | null | undefined) {
  if (pct == null) return "text-slate-400";
  if (pct >= 90) return "text-rose-400";
  if (pct >= 75) return "text-amber-400";
  return "text-emerald-400";
}

export function formatPct(pct: number | null | undefined) {
  if (pct == null) return "—";
  return `${pct.toFixed(0)}%`;
}

export function formatUsd(n: number | null | undefined) {
  if (n == null) return "—";
  return `$${n.toFixed(2)}`;
}

export function providerLabel(id: string) {
  const map: Record<string, string> = {
    codex: "Codex",
    claude_code: "Claude Code",
    cursor: "Cursor",
    copilot: "Copilot",
    chatgpt: "ChatGPT",
    claude_ai: "Claude.ai",
    grok: "Grok",
  };
  return map[id] ?? id;
}

export function Ring({
  pct,
  label,
  sub,
  size = 44,
  onClick,
}: {
  pct: number | null;
  label: string;
  sub?: string;
  size?: number;
  onClick?: () => void;
}) {
  const r = (size - 6) / 2;
  const c = 2 * Math.PI * r;
  const v = pct == null ? 0 : Math.min(pct, 100);
  const offset = c - (v / 100) * c;

  return (
    <div
      className={clsx(
        "flex flex-col items-center gap-0.5 min-w-[52px] rounded-md px-0.5",
        onClick && "cursor-pointer hover:bg-white/5",
      )}
      data-no-drag={onClick ? true : undefined}
      onClick={onClick}
      onMouseDown={(e) => onClick && e.stopPropagation()}
      role={onClick ? "button" : undefined}
      title={onClick ? `Open ${label} details` : undefined}
    >
      <div className="relative" style={{ width: size, height: size }}>
        <svg width={size} height={size} className="-rotate-90">
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="rgba(148,163,184,0.25)"
            strokeWidth={4}
          />
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="currentColor"
            strokeWidth={4}
            strokeDasharray={c}
            strokeDashoffset={offset}
            strokeLinecap="round"
            className={clsx(pctColor(pct))}
          />
        </svg>
        <span
          className={clsx(
            "absolute inset-0 flex items-center justify-center text-[10px] font-semibold",
            pctColor(pct),
          )}
        >
          {formatPct(pct)}
        </span>
      </div>
      <span className="text-[10px] font-medium text-slate-200 leading-tight text-center">
        {label}
      </span>
      {sub ? (
        <span className="text-[9px] text-slate-500 text-center leading-tight">{sub}</span>
      ) : null}
    </div>
  );
}
