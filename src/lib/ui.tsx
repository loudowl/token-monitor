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

/** Compact dollars for small ring labels */
export function formatUsdCompact(n: number | null | undefined) {
  if (n == null) return "—";
  if (n >= 100) return `$${n.toFixed(0)}`;
  if (n >= 10) return `$${n.toFixed(1)}`;
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
  center,
  centerClassName,
  title,
  styledTooltip,
  onClick,
}: {
  pct: number | null;
  label: string;
  sub?: string;
  size?: number;
  center?: string;
  centerClassName?: string;
  title?: string;
  /** Use a styled hover tooltip instead of the native `title` attribute. */
  styledTooltip?: boolean;
  onClick?: () => void;
}) {
  const stroke = Math.max(3, Math.round(size * 0.09));
  const pad = stroke + 2;
  const r = (size - pad * 2) / 2;
  const c = 2 * Math.PI * r;
  const v = pct == null ? 0 : Math.min(pct, 100);
  const offset = c - (v / 100) * c;
  const pctFont = Math.max(9, Math.round(size * 0.22));
  const labelFont = Math.max(9, Math.round(size * 0.2));
  const subFont = Math.max(8, Math.round(size * 0.17));
  const centerDisplay = center ?? formatPct(pct);
  const ringTitle =
    title ?? (onClick && !center ? `Open ${label} details` : undefined);
  const nativeTitle = styledTooltip ? undefined : ringTitle;

  return (
    <div
      className={clsx(
        "flex flex-col items-center gap-0.5 rounded-md px-0.5 shrink-0",
        onClick && "cursor-pointer hover:bg-white/5",
        styledTooltip && title && "relative group",
      )}
      style={{ width: size, maxWidth: "100%" }}
      data-no-drag={onClick ? true : undefined}
      onClick={onClick}
      onMouseDown={(e) => onClick && e.stopPropagation()}
      role={onClick ? "button" : undefined}
      title={nativeTitle}
    >
      {styledTooltip && title ? (
        <div
          role="tooltip"
          className="pointer-events-none absolute left-1/2 bottom-full -translate-x-1/2 mb-1.5 z-50 rounded-md border border-slate-600 bg-slate-800 px-2.5 py-1.5 text-[13px] leading-snug text-slate-100 shadow-lg opacity-0 group-hover:opacity-100 transition-opacity max-w-[min(20rem,calc(100vw-2rem))] text-center whitespace-normal"
        >
          {title}
        </div>
      ) : null}
      <div className="relative" style={{ width: size, height: size }}>
        <svg width={size} height={size} className="-rotate-90">
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="rgba(148,163,184,0.25)"
            strokeWidth={stroke}
          />
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="currentColor"
            strokeWidth={stroke}
            strokeDasharray={c}
            strokeDashoffset={offset}
            strokeLinecap="round"
            className={clsx(pctColor(pct))}
          />
        </svg>
        <span
          className={clsx(
            "absolute inset-0 flex items-center justify-center font-semibold px-0.5 text-center leading-none",
            centerClassName ?? pctColor(pct),
          )}
          style={{ fontSize: pctFont }}
        >
          {centerDisplay}
        </span>
      </div>
      <span
        className="font-medium text-slate-200 leading-tight text-center w-full truncate px-0.5"
        style={{ fontSize: labelFont }}
      >
        {label}
      </span>
      {sub ? (
        <span
          className="text-slate-500 text-center leading-tight w-full truncate px-0.5"
          style={{ fontSize: subFont }}
        >
          {sub}
        </span>
      ) : null}
    </div>
  );
}
