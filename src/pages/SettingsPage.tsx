import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  getPlanCatalog,
  getSettings,
  saveSettings,
  secretConfigured,
  setSecret,
  syncNow,
  hideCurrentWindow,
  testCursorAuth,
  testCopilotAuth,
  connectCursor,
  type CopilotProbeResult,
  type CursorProbeResult,
  type AppSettings,
  type ProviderPlan,
  PROVIDERS,
} from "../api";

const SECRETS: {
  key: string;
  label: string;
  hint: string;
  steps: string[];
  url?: string;
  urlLabel?: string;
}[] = [
  {
    key: "cursor_session_token",
    label: "Cursor session cookie (WorkosCursorSessionToken)",
    hint: "Paste the cookie value from cursor.com",
    url: "https://cursor.com/dashboard/usage",
    urlLabel: "Open Cursor usage dashboard",
    steps: [
      "Prefer the Sign in to Cursor button above — no DevTools required.",
      "Manual fallback: in Chrome, Application → Cookies → https://cursor.com → double-click the Value of WorkosCursorSessionToken → ⌘A ⌘C, then paste below.",
      "You do not need to find usage-summary in the Network tab. Those request names change and are often hidden.",
    ],
  },
  {
    key: "github_pat",
    label: "GitHub PAT (Copilot billing)",
    hint: "Fine-grained token with billing / plan read",
    url: "https://github.com/settings/personal-access-tokens/new",
    urlLabel: "Create a GitHub token",
    steps: [
      "GitHub → Settings → Developer settings → Personal access tokens.",
      "Create a fine-grained token with Account → Plan / Billing read (or classic token with read:user).",
      "If Copilot is billed through your employer org, set GitHub org below and use a token with org billing read.",
      "Paste the token here and set your GitHub username (or leave blank to use the token owner).",
    ],
  },
  {
    key: "openai_admin_key",
    label: "OpenAI admin / API key (ChatGPT org usage)",
    hint: "Admin key for organization costs, or a standard API key",
    url: "https://platform.openai.com/settings/organization/admin-keys",
    urlLabel: "OpenAI admin keys",
    steps: [
      "Go to platform.openai.com → Organization → Admin keys (or API keys).",
      "Create a key that can read usage/costs for your org.",
      "This tracks API spend, not the consumer ChatGPT Plus request quota.",
    ],
  },
  {
    key: "anthropic_admin_key",
    label: "Anthropic admin / API key (Claude usage)",
    hint: "Console API key with usage access",
    url: "https://console.anthropic.com/settings/keys",
    urlLabel: "Anthropic console keys",
    steps: [
      "Go to console.anthropic.com → Settings → API keys.",
      "Create a key for the org whose usage you want to see.",
      "This is Claude API usage, not the claude.ai chat plan unless you also set manual %.",
    ],
  },
  {
    key: "xai_api_key",
    label: "xAI API key (Grok)",
    hint: "API key from console.x.ai",
    url: "https://console.x.ai/",
    urlLabel: "xAI console",
    steps: [
      "Sign in at console.x.ai and create an API key.",
      "Paste it here to pull Grok API usage when the endpoint is available.",
      "For grok.x.com consumer quota, also set Manual used % below.",
    ],
  },
];

export default function SettingsPage() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [catalog, setCatalog] = useState<ProviderPlan[]>([]);
  const [secretDraft, setSecretDraft] = useState<Record<string, string>>({});
  const [configured, setConfigured] = useState<Record<string, boolean>>({});
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [cursorProbe, setCursorProbe] = useState<CursorProbeResult | null>(null);
  const [copilotProbe, setCopilotProbe] = useState<CopilotProbeResult | null>(null);
  const [cursorConnecting, setCursorConnecting] = useState(false);
  const [copilotTesting, setCopilotTesting] = useState(false);

  useEffect(() => {
    Promise.all([getSettings(), getPlanCatalog()]).then(([s, c]) => {
      setSettings(s);
      setCatalog(c);
    });
    SECRETS.forEach(({ key }) => {
      secretConfigured(key).then((ok) =>
        setConfigured((prev) => ({ ...prev, [key]: ok })),
      );
    });
  }, []);

  if (!settings) {
    return (
      <div className="min-h-screen bg-slate-950 text-slate-400 p-8">
        Loading settings…
      </div>
    );
  }

  const toggleProvider = (id: string) => {
    const enabled = settings.enabled_providers.includes(id)
      ? settings.enabled_providers.filter((p) => p !== id)
      : [...settings.enabled_providers, id];
    setSettings({ ...settings, enabled_providers: enabled });
  };

  const updatePlan = (provider: string, patch: Partial<(typeof settings.plans)[0]>) => {
    setSettings({
      ...settings,
      plans: settings.plans.map((p) =>
        p.provider === provider ? { ...p, ...patch } : p,
      ),
    });
  };

  const onSave = async () => {
    setSaveError(null);
    try {
      await saveSettings(settings);
      for (const [key, val] of Object.entries(secretDraft)) {
        if (val.trim()) {
          await setSecret(key, val);
          setConfigured((c) => ({ ...c, [key]: true }));
        }
      }
      setSecretDraft({});
      await syncNow();
      const probe = await testCursorAuth();
      setCursorProbe(probe);
      if (!probe.ok) {
        setSaveError(probe.error || "Cursor cookie was not accepted.");
      }
      setSaved(true);
      setTimeout(() => setSaved(false), 800);
      await hideCurrentWindow();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 p-6 max-w-xl mx-auto font-sans">
      <h1 className="text-2xl font-semibold mb-1">Settings</h1>
      <p className="text-sm text-slate-400 mb-6">
        Keys stay in the macOS Keychain. Codex and Claude Code need no keys — they
        read local session logs.
      </p>

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">Providers</h2>
        <div className="space-y-2">
          {PROVIDERS.map((p) => (
            <label
              key={p.id}
              className="flex items-center gap-3 rounded-lg border border-slate-800 px-3 py-2"
            >
              <input
                type="checkbox"
                checked={settings.enabled_providers.includes(p.id)}
                onChange={() => toggleProvider(p.id)}
              />
              <span>{p.label}</span>
            </label>
          ))}
        </div>
      </section>

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">Billing cycle</h2>
        <label className="flex items-center gap-3 text-sm">
          Anchor day (1–28)
          <input
            type="number"
            min={1}
            max={28}
            value={settings.billing_anchor_day}
            onChange={(e) =>
              setSettings({
                ...settings,
                billing_anchor_day: Number(e.target.value),
              })
            }
            className="w-20 bg-slate-900 border border-slate-700 rounded px-2 py-1"
          />
        </label>
        <label className="flex items-center gap-3 text-sm mt-3">
          Alert threshold %
          <input
            type="number"
            min={50}
            max={100}
            value={settings.alert_threshold_percent}
            onChange={(e) =>
              setSettings({
                ...settings,
                alert_threshold_percent: Number(e.target.value),
              })
            }
            className="w-20 bg-slate-900 border border-slate-700 rounded px-2 py-1"
          />
        </label>
        <label className="flex items-center gap-3 text-sm mt-3">
          GitHub username (Copilot)
          <input
            type="text"
            value={settings.github_username ?? ""}
            onChange={(e) =>
              setSettings({ ...settings, github_username: e.target.value || null })
            }
            className="flex-1 bg-slate-900 border border-slate-700 rounded px-2 py-1"
            placeholder="loudowl"
          />
        </label>
        <label className="flex items-center gap-3 text-sm mt-3">
          GitHub org (org-billed Copilot)
          <input
            type="text"
            value={settings.github_org ?? ""}
            onChange={(e) =>
              setSettings({ ...settings, github_org: e.target.value || null })
            }
            className="flex-1 bg-slate-900 border border-slate-700 rounded px-2 py-1"
            placeholder="my-company (optional)"
          />
        </label>
        <p className="text-xs text-slate-500 mt-2">
          Personal Copilot plans use your user billing API. If your license is through a
          company org, enter the org slug and use a PAT that can read that org&apos;s billing.
        </p>
      </section>

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">
          API keys &amp; session cookies
        </h2>
        <p className="text-xs text-slate-500 mb-3">
          Cursor has no personal usage API key. Use <span className="text-slate-300">Sign in to Cursor</span> —
          a browser window opens, you log in, and Token Monitor captures the session. You do not need DevTools or request names like usage-summary.
        </p>
        <button
          type="button"
          disabled={cursorConnecting}
          onClick={async () => {
            setSaveError(null);
            setCursorConnecting(true);
            try {
              const probe = await connectCursor();
              setCursorProbe(probe);
              if (probe.ok) {
                setConfigured((c) => ({ ...c, cursor_session_token: true }));
                await syncNow();
              } else {
                setSaveError(probe.error || "Cursor sign-in did not produce a session.");
              }
            } catch (err) {
              setSaveError(err instanceof Error ? err.message : String(err));
            } finally {
              setCursorConnecting(false);
            }
          }}
          className="mb-4 w-full py-2 rounded-lg bg-slate-100 text-slate-900 hover:bg-white font-medium text-sm disabled:opacity-60"
        >
          {cursorConnecting ? "Waiting for Cursor sign-in…" : "Sign in to Cursor"}
        </button>
        {cursorProbe ? (
          <div
            className={`mb-4 rounded-lg border px-3 py-2 text-sm ${
              cursorProbe.ok
                ? "border-emerald-800 bg-emerald-950/40 text-emerald-300"
                : "border-rose-800 bg-rose-950/40 text-rose-300"
            }`}
          >
            {cursorProbe.ok
              ? `Cursor connected${cursorProbe.email ? ` as ${cursorProbe.email}` : ""}. ${cursorProbe.events} runs · ${cursorProbe.percent != null ? `${cursorProbe.percent.toFixed(0)}%` : "—"} of cycle`
              : `Cursor failed: ${cursorProbe.error}`}
          </div>
        ) : null}
        {saveError ? (
          <p className="mb-3 text-sm text-rose-400">{saveError}</p>
        ) : null}
        {SECRETS.map(({ key, label, hint, steps, url, urlLabel }) => (
          <div key={key} className="mb-5 rounded-lg border border-slate-800 p-3">
            <p className="text-sm font-medium text-slate-200">
              {label}{" "}
              {configured[key] ? (
                <span className="text-emerald-500 text-xs font-normal">· saved</span>
              ) : (
                <span className="text-amber-400 text-xs font-normal">· not set</span>
              )}
            </p>
            <ol className="mt-2 list-decimal list-inside space-y-1 text-xs text-slate-400">
              {steps.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ol>
            {url ? (
              <button
                type="button"
                className="mt-2 text-xs text-indigo-400 hover:text-indigo-300 underline"
                onClick={() => openUrl(url)}
              >
                {urlLabel ?? url}
              </button>
            ) : null}
            <input
              type="password"
              value={secretDraft[key] ?? ""}
              onChange={(e) =>
                setSecretDraft((d) => ({ ...d, [key]: e.target.value }))
              }
              placeholder={hint}
              className="mt-2 w-full bg-slate-900 border border-slate-700 rounded px-2 py-1.5 text-sm text-slate-200"
            />
          </div>
        ))}
        <button
          type="button"
          disabled={copilotTesting}
          onClick={async () => {
            setSaveError(null);
            setCopilotTesting(true);
            try {
              const probe = await testCopilotAuth();
              setCopilotProbe(probe);
              if (probe.ok) {
                await syncNow();
              }
            } catch (err) {
              setSaveError(err instanceof Error ? err.message : String(err));
            } finally {
              setCopilotTesting(false);
            }
          }}
          className="mt-2 w-full py-2 rounded-lg border border-slate-600 text-slate-200 hover:bg-slate-800 font-medium text-sm disabled:opacity-60"
        >
          {copilotTesting ? "Testing Copilot billing…" : "Test GitHub Copilot billing"}
        </button>
        {copilotProbe ? (
          <div
            className={`mt-3 rounded-lg border px-3 py-2 text-sm ${
              copilotProbe.ok
                ? "border-emerald-800 bg-emerald-950/40 text-emerald-300"
                : "border-rose-800 bg-rose-950/40 text-rose-300"
            }`}
          >
            {copilotProbe.ok
              ? `Copilot ${copilotProbe.used_requests} / ${copilotProbe.limit_requests} premium requests${copilotProbe.percent != null ? ` · ${copilotProbe.percent.toFixed(0)}%` : ""}${copilotProbe.source ? ` · ${copilotProbe.source}` : ""}`
              : `Copilot billing failed: ${copilotProbe.error}`}
          </div>
        ) : null}
      </section>

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">
          Consumer plans &amp; manual usage
        </h2>
        {(["chatgpt", "claude_ai", "grok"] as const).map((provider) => {
          const plan = settings.plans.find((p) => p.provider === provider);
          if (!plan) return null;
          const plansFor = catalog.filter((c) =>
            provider === "chatgpt"
              ? c.id.startsWith("chatgpt")
              : provider === "claude_ai"
                ? c.id.startsWith("claude")
                : c.id.startsWith("grok"),
          );
          return (
            <div
              key={provider}
              className="mb-4 rounded-lg border border-slate-800 p-3 space-y-2"
            >
              <p className="font-medium capitalize">{provider.replace("_", ".")}</p>
              <label className="block text-xs text-slate-400">
                Plan
                <select
                  value={plan.plan_id}
                  onChange={(e) => updatePlan(provider, { plan_id: e.target.value })}
                  className="mt-1 w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-sm"
                >
                  {plansFor.map((pl) => (
                    <option key={pl.id} value={pl.id}>
                      {pl.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block text-xs text-slate-400">
                Manual used % (optional)
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={plan.manual_used_percent ?? ""}
                  onChange={(e) =>
                    updatePlan(provider, {
                      manual_used_percent: e.target.value
                        ? Number(e.target.value)
                        : null,
                    })
                  }
                  className="mt-1 w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-sm"
                  placeholder="From app settings UI"
                />
              </label>
              <label className="block text-xs text-slate-400">
                Period
                <select
                  value={plan.period}
                  onChange={(e) => updatePlan(provider, { period: e.target.value })}
                  className="mt-1 w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-sm"
                >
                  <option value="month">Monthly plan</option>
                  <option value="year">Yearly plan</option>
                </select>
              </label>
            </div>
          );
        })}
      </section>

      <button
        type="button"
        onClick={onSave}
        className="w-full py-2 rounded-lg bg-indigo-600 hover:bg-indigo-500 font-medium"
      >
        {saved ? "Saved" : "Save & sync"}
      </button>
    </div>
  );
}
