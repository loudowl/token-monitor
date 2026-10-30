/**
 * Heuristic 1–10 score: how specific the prompt context is (10 = very specific).
 */
export function promptSpecificityScore(text: string): number {
  const t = text.trim();
  if (!t) return 1;

  let score = 4;
  const lower = t.toLowerCase();
  const words = t.split(/\s+/).filter(Boolean);

  if (words.length >= 12) score += 1;
  if (words.length >= 24) score += 1;
  if (words.length <= 4) score -= 2;
  else if (words.length <= 8) score -= 1;

  if (t.length >= 120) score += 1;
  if (t.length < 20) score -= 1;

  if (/[\\/][\w.-]+/.test(t) || /\b[\w-]+\.(tsx?|jsx?|rs|py|md|json|yaml|yml|sql)\b/i.test(t)) {
    score += 2;
  }
  if (/https?:\/\/|\bgithub\.com\//i.test(t)) score += 2;
  if (/\b[A-Z][A-Z0-9_]*-\d+\b/.test(t)) score += 2;
  if (/\b\d+\.\d+\.\d+\b/.test(t)) score += 1;
  if (/[`"'][^`"']{8,}[`"']/.test(t)) score += 1;
  if (/\b(?:must|should|only|exactly|before|after|when|unless|without)\b/i.test(t)) {
    score += 1;
  }
  if (/\b(?:error|stack trace|repro|steps to reproduce|acceptance criteria)\b/i.test(t)) {
    score += 1;
  }

  const generic = [
    "implement the plan as specified",
    "briefly inform the user",
    "never mind",
    "neverr mind",
    "help me",
    "fix this",
    "what happened",
    "can you help",
    "thanks",
    "thank you",
    "looks good",
    "continue",
    "go ahead",
    "say the word",
  ];
  if (generic.some((phrase) => lower.includes(phrase))) score -= 2;

  if (/^(yes|no|ok|okay|sure|thanks)\.?$/i.test(t)) score = 1;

  return Math.max(1, Math.min(10, Math.round(score)));
}

export function specificityColor(score: number): string {
  if (score >= 8) return "text-emerald-400";
  if (score >= 5) return "text-slate-300";
  return "text-amber-400";
}
