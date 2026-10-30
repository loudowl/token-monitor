import type { RunRecord } from "../api";
import { deriveTaskContext, isJiraKey } from "./taskContext";
import { promptSpecificityScore } from "./specificity";

export type TurnRole = "kickoff" | "steer" | "approve" | "mechanical" | "recovery";

export function classifyTurnRole(text: string): TurnRole {
  const t = text.trim();
  const lower = t.toLowerCase();
  const words = t.split(/\s+/).filter(Boolean).length;

  if (
    words <= 5
    || /^(yes|no|ok|okay|sure|thanks|continue|go ahead|looks good|say the word)\.?$/i.test(t)
    || /\b(tpr|tprn|fcpr|prstat)\b/i.test(lower)
  ) {
    return "approve";
  }

  const mech = [
    "lint",
    "eslint",
    "prettier",
    "merge conflict",
    "whitespace",
    "cargo check",
    "npm test",
    "typecheck",
    "shepherd",
    "sync now",
    "pr comment",
  ];
  if (mech.some((p) => lower.includes(p))) {
    return "mechanical";
  }

  if (
    /\b(still broken|failed again|regression|fix ci|ci fail|not working)\b/i.test(lower)
  ) {
    return "recovery";
  }

  if (
    words >= 22
    || /\b(architecture|acceptance criteria|root cause|ambiguous|release risk|design doc)\b/i.test(
      lower,
    )
  ) {
    return "kickoff";
  }

  return "steer";
}

function roleLabel(role: TurnRole): string {
  switch (role) {
    case "kickoff":
      return "kickoff";
    case "steer":
      return "steer";
    case "approve":
      return "approve";
    case "mechanical":
      return "mech";
    case "recovery":
      return "recovery";
  }
}

function threadPeakSpecificity(runs: RunRecord[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const run of runs) {
    const id = run.conversation_id?.trim();
    if (!id) continue;
    const local = promptSpecificityScore(run.title);
    map.set(id, Math.max(map.get(id) ?? 0, local));
  }
  return map;
}

export function promptAdequacy(
  run: RunRecord,
  _runsInView: RunRecord[],
  threadPeaks: Map<string, number>,
): { score: number; role: TurnRole; roleShort: string; tooltip: string } {
  const role = classifyTurnRole(run.title);
  const local = promptSpecificityScore(run.title);
  const conv = run.conversation_id?.trim();
  const threadHigh = conv ? (threadPeaks.get(conv) ?? local) : local;
  const task = deriveTaskContext(run);
  const anchored = isJiraKey(task.label);

  let blended: number;
  switch (role) {
    case "kickoff":
      blended = 0.6 * local + 0.25 * threadHigh + (anchored ? 1.5 : 0);
      break;
    case "approve":
      blended = 0.12 * local + 0.88 * threadHigh;
      break;
    case "steer":
      blended = 0.35 * local + 0.5 * threadHigh + (anchored ? 0.75 : 0);
      break;
    case "mechanical":
      blended = 0.25 * local + 0.35 * threadHigh + 2.5;
      break;
    case "recovery":
      blended = 0.55 * local + 0.35 * threadHigh + (local >= 6 ? 0.75 : 0);
      break;
  }

  const score = Math.max(1, Math.min(10, Math.round(blended)));
  const roleShort = roleLabel(role);

  const tooltipParts = [
    `Turn adequacy ${score}/10 for a ${role} message`,
    anchored
      ? task.inherited
        ? "ticket anchored via earlier chat context"
        : "ticket anchored in prompt"
      : "no Jira key on this thread",
    `local detail ${local}/10 · thread peak ${threadHigh}/10`,
  ];

  return {
    score,
    role,
    roleShort,
    tooltip: tooltipParts.join(" · "),
  };
}

export function buildThreadPeakMap(runs: RunRecord[]): Map<string, number> {
  return threadPeakSpecificity(runs);
}

export function adequacyColor(score: number): string {
  if (score >= 8) return "text-emerald-400";
  if (score >= 5) return "text-slate-300";
  return "text-amber-400";
}
