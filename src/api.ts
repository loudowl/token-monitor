import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface RunRecord {
  provider: string;
  run_id: string;
  started_at: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_tokens: number;
  cost_usd: number;
  title: string;
}

export interface QuotaSnapshot {
  provider: string;
  label: string;
  used_percent_month: number | null;
  used_percent_year: number | null;
  used_tokens_month: number | null;
  limit_tokens_month: number | null;
  used_usd_month: number | null;
  limit_usd_month: number | null;
  primary_window_label: string | null;
  primary_window_percent: number | null;
  secondary_window_label: string | null;
  secondary_window_percent: number | null;
  resets_at: string | null;
  status: string;
  message: string | null;
}

export interface UsageSummary {
  quotas: QuotaSnapshot[];
  projected_month_usd: number;
  month_spend_usd: number;
}

export interface ModelUsageRow {
  model: string;
  cost_usd: number;
  share_percent: number;
}

export interface ManualPlanEntry {
  provider: string;
  plan_id: string;
  period: string;
  renewal_day: number;
  manual_used_percent: number | null;
}

export interface AppSettings {
  enabled_providers: string[];
  billing_anchor_day: number;
  plans: ManualPlanEntry[];
  alert_threshold_percent: number;
  github_username: string | null;
  widget_x: number | null;
  widget_y: number | null;
}

export interface ProviderPlan {
  id: string;
  name: string;
  monthly_token_limit: number | null;
  monthly_usd_limit: number | null;
  period: string;
}

export const PROVIDERS = [
  { id: "codex", label: "Codex" },
  { id: "claude_code", label: "Claude Code" },
  { id: "cursor", label: "Cursor" },
  { id: "copilot", label: "Copilot" },
  { id: "chatgpt", label: "ChatGPT" },
  { id: "claude_ai", label: "Claude.ai" },
  { id: "grok", label: "Grok" },
] as const;

export function getUsageSummary() {
  return invoke<UsageSummary>("get_usage_summary");
}

export function syncNow() {
  return invoke<UsageSummary>("sync_now");
}

export function listRuns(opts?: {
  provider?: string;
  limit?: number;
  sort?: string;
}) {
  return invoke<RunRecord[]>("list_runs", { ...opts });
}

export function getProviderModelUsage(provider: string) {
  return invoke<ModelUsageRow[]>("get_provider_model_usage", { provider });
}

export function getSettings() {
  return invoke<AppSettings>("get_settings");
}

export function saveSettings(settings: AppSettings) {
  return invoke<void>("save_settings", { settings });
}

export function getPlanCatalog() {
  return invoke<ProviderPlan[]>("get_plan_catalog");
}

export function setSecret(provider: string, value: string) {
  return invoke<void>("set_secret", { provider, value });
}

export function secretConfigured(provider: string) {
  return invoke<boolean>("secret_configured", { provider });
}

export interface CursorProbeResult {
  ok: boolean;
  email: string | null;
  events: number;
  percent: number | null;
  error: string | null;
}

export function testCursorAuth() {
  return invoke<CursorProbeResult>("test_cursor_auth");
}

export function connectCursor() {
  return invoke<CursorProbeResult>("connect_cursor");
}

export function showWindow(
  label: "details" | "settings" | "widget",
  provider?: string,
) {
  return invoke<void>("show_window", { label, provider: provider ?? null });
}

export function saveWidgetPosition(x: number, y: number) {
  return invoke<void>("save_widget_position", { x, y });
}

export function onUsageUpdated(cb: (summary: UsageSummary) => void) {
  return listen<UsageSummary>("usage-updated", (e) => cb(e.payload));
}
