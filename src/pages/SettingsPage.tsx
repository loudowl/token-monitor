import { useEffect, useState } from "react";
import {
  getPlanCatalog,
  getSettings,
  saveSettings,
  secretConfigured,
  setSecret,
  syncNow,
  type AppSettings,
  type ProviderPlan,
  PROVIDERS,
} from "../api";

const SECRETS: { key: string; label: string; hint: string }[] = [
  {
    key: "github_pat",
    label: "GitHub PAT (Copilot billing)",
    hint: "Fine-grained token with billing read",
  },
  {
    key: "openai_admin_key",
    label: "OpenAI admin key",
    hint: "Optional — org usage API",
  },
  {
    key: "anthropic_admin_key",
    label: "Anthropic admin key",
    hint: "Optional — org usage API",
  },
  { key: "xai_api_key", label: "xAI API key", hint: "Optional — usage API" },
];

export default function SettingsPage() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [catalog, setCatalog] = useState<ProviderPlan[]>([]);
  const [secretDraft, setSecretDraft] = useState<Record<string, string>>({});
  const [configured, setConfigured] = useState<Record<string, boolean>>({});
  const [saved, setSaved] = useState(false);

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
      <div className="min-h-screen bg-slate-950 text-slate-400 p-8">Loading settings…</div>
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
    await saveSettings(settings);
    for (const [key, val] of Object.entries(secretDraft)) {
      if (val.trim()) {
        await setSecret(key, val);
        setConfigured((c) => ({ ...c, [key]: true }));
      }
    }
    setSecretDraft({});
    setSaved(true);
    await syncNow();
    setTimeout(() => setSaved(false), 2000);
  };

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 p-6 max-w-xl mx-auto font-sans">
      <h1 className="text-2xl font-semibold mb-1">Settings</h1>
      <p className="text-sm text-slate-400 mb-6">
        Plans seeded from SubscriptionSentry-style catalog. Keys live in the OS keychain.
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
      </section>

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">
          Consumer plans & manual usage
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

      <section className="mb-8">
        <h2 className="text-sm font-semibold text-slate-300 mb-3">API keys</h2>
        {SECRETS.map(({ key, label, hint }) => (
          <label key={key} className="block mb-3 text-xs text-slate-400">
            {label}{" "}
            {configured[key] ? (
              <span className="text-emerald-500">· saved</span>
            ) : null}
            <input
              type="password"
              value={secretDraft[key] ?? ""}
              onChange={(e) =>
                setSecretDraft((d) => ({ ...d, [key]: e.target.value }))
              }
              placeholder={hint}
              className="mt-1 w-full bg-slate-900 border border-slate-700 rounded px-2 py-1.5 text-sm text-slate-200"
            />
          </label>
        ))}
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
