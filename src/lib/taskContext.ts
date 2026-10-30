import type { RunRecord } from "../api";

/** Fallback when API did not populate task_context (older builds). */
export function deriveTaskContext(run: RunRecord): {
  label: string;
  inherited: boolean;
} {
  if (run.task_context?.trim()) {
    return {
      label: run.task_context,
      inherited: Boolean(run.task_inherited),
    };
  }
  const title = run.title?.trim() ?? "";
  const jira = extractJiraKey(title);
  if (jira) {
    return { label: jira, inherited: false };
  }
  return { label: taskThemeLabel(title), inherited: false };
}

export function taskContextTooltip(run: RunRecord): string {
  const { label, inherited } = deriveTaskContext(run);
  if (inherited) {
    return `${label} — Jira key from an earlier message in the same Cursor chat · click to open`;
  }
  if (/^[A-Z][A-Z0-9_]*-\d+$/.test(label)) {
    return `${label} — click to open in Jira`;
  }
  return `${label} — inferred task theme (no ticket key in prompt or thread)`;
}

function extractJiraKey(title: string): string | null {
  for (const word of title.split(/\s+/)) {
    const w = word.replace(/^[^A-Za-z0-9]+|[^A-Za-z0-9-]+$/g, "");
    if (
      w.length >= 6 &&
      w.includes("-") &&
      /^[A-Z]/.test(w) &&
      /\d/.test(w) &&
      /^[A-Z0-9_]+-\d/.test(w)
    ) {
      return w;
    }
  }
  return null;
}

function taskThemeLabel(title: string): string {
  const t = title.trim();
  if (!t) return "—";
  const lower = t.toLowerCase();
  const jira = extractJiraKey(t);
  if (jira) return jira;

  const mech: [string, string][] = [
    ["merge conflict", "merge conflict cleanup"],
    ["eslint", "ESLint / lint fixes"],
    ["lint", "lint fixes"],
    ["pr shepherd", "PR shepherd / CI follow-up"],
    ["shepherd", "PR shepherd / CI follow-up"],
  ];
  for (const [needle, label] of mech) {
    if (lower.includes(needle)) return label;
  }
  if (lower.includes("release") || /\b11\.\d/.test(lower)) {
    return "release-line work";
  }
  if (t.length <= 56) return t;
  return `${t.slice(0, 55)}…`;
}

export function isJiraKey(label: string): boolean {
  return /^[A-Z][A-Z0-9_]*-\d+$/.test(label);
}

/** Team Fox Jira — FTSWB, FTSMOB, FTSQA, etc. */
export function jiraBrowseUrl(issueKey: string): string {
  const key = issueKey.trim().toUpperCase();
  return `https://teamfox.atlassian.net/browse/${encodeURIComponent(key)}`;
}

export async function openJiraIssue(issueKey: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(jiraBrowseUrl(issueKey));
}
