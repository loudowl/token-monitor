use anyhow::anyhow;
use chrono::Utc;

use crate::db::billing_cycle_start;
use crate::db::Db;
use crate::models::{ModelUsageAdvice, ModelUsageRow, RunRecord};
use crate::task_context::{run_ticket_key, task_theme_label};

pub fn generate_cursor_advice(db: &Db, anchor_day: u8) -> anyhow::Result<ModelUsageAdvice> {
    let breakdown = db.provider_model_breakdown("cursor", anchor_day)?;
    if breakdown.is_empty() {
        return Err(anyhow!(
            "No Cursor runs in the current billing cycle yet. Sync usage, then try again."
        ));
    }

    let cycle_runs = db.provider_runs_in_billing_cycle("cursor", anchor_day, 5000)?;
    let top_runs: Vec<RunRecord> = cycle_runs.iter().take(40).cloned().collect();
    let cycle_start = billing_cycle_start(Utc::now(), anchor_day);
    let text = build_local_advice(
        &cycle_start.format("%Y-%m-%d").to_string(),
        &breakdown,
        &top_runs,
        &cycle_runs,
    );

    Ok(ModelUsageAdvice {
        text,
        generated_at: Utc::now().to_rfc3339(),
        model_used: "local-heuristics".into(),
        provider_used: "offline".into(),
    })
}

fn build_local_advice(
    cycle_start: &str,
    breakdown: &[ModelUsageRow],
    top_runs: &[RunRecord],
    cycle_runs: &[RunRecord],
) -> String {
    let total_cost: f64 = breakdown.iter().map(|r| r.cost_usd).sum();
    let total_runs: i64 = breakdown.iter().map(|r| r.run_count).sum();

    let premium_spend: f64 = breakdown
        .iter()
        .filter(|r| is_premium_model(&r.model))
        .map(|r| r.cost_usd)
        .sum();
    let premium_pct = if total_cost > 0.0 {
        (premium_spend / total_cost) * 100.0
    } else {
        0.0
    };

    let mut downgrade_candidates: Vec<(f64, String)> = Vec::new();
    for run in top_runs {
        if !is_premium_model(&run.model) {
            continue;
        }
        let kind = prompt_kind(&run.title);
        if kind == PromptKind::DeepWork {
            continue;
        }
        if run.cost_usd < 0.02 && kind == PromptKind::Neutral {
            continue;
        }
        let excerpt = truncate_prompt(&run.title, 120);
        downgrade_candidates.push((
            run.cost_usd,
            format!(
                "- **${:.2}** on **{}** — {} prompt: \"{}\"",
                run.cost_usd,
                run.model,
                kind.label(),
                excerpt
            ),
        ));
    }
    downgrade_candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    downgrade_candidates.truncate(8);

    let highest_avg = breakdown
        .iter()
        .filter(|r| r.run_count >= 3)
        .map(|r| {
            (
                r.model.clone(),
                r.cost_usd / r.run_count as f64,
                r.run_count,
            )
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let top_model = breakdown.first().map(|r| r.model.as_str()).unwrap_or("—");
    let top_share = breakdown.first().map(|r| r.share_percent).unwrap_or(0.0);

    let mut summary = format!(
        "Cycle from **{cycle_start}**: **{total_runs}** billed agent runs, **${total_cost:.2}** synced spend. "
    );
    if premium_pct >= 40.0 {
        summary.push_str(&format!(
            "**{premium_pct:.0}%** of spend went to premium-tier models (Opus/MAX/thinking/GPT-5-class). "
        ));
    } else {
        summary.push_str(&format!(
            "About **{premium_pct:.0}%** of spend is on premium-tier models. "
        ));
    }
    summary.push_str(&format!(
        "Most spend is on **{top_model}** ({top_share:.0}% of cycle)."
    ));

    let mut concentration = String::new();
    for row in breakdown.iter().take(5) {
        let avg = if row.run_count > 0 {
            row.cost_usd / row.run_count as f64
        } else {
            0.0
        };
        concentration.push_str(&format!(
            "- **{}**: ${:.2} ({:.0}%), {} runs, ${:.3}/run{}\n",
            row.model,
            row.cost_usd,
            row.share_percent,
            row.run_count,
            avg,
            if is_premium_model(&row.model) {
                " · premium tier"
            } else if is_fast_model(&row.model) {
                " · fast tier"
            } else {
                ""
            }
        ));
    }

    let mut patterns = String::new();
    if downgrade_candidates.is_empty() {
        patterns.push_str(
            "- No strong downgrade signals in the top expensive runs (premium use mostly paired with longer or architecture-style prompts).\n",
        );
    } else {
        patterns.push_str(
            "- Premium models on **short, vague, or mechanical** prompts are the main waste pattern this cycle.\n",
        );
        for (_, line) in &downgrade_candidates {
            patterns.push_str(line);
            patterns.push('\n');
        }
    }

    let mut cheaper = String::new();
    cheaper.push_str(
        "- Route **lint, formatting, search, small diffs, PR hygiene, and sync/conflict cleanup** to **Composer / fast** (or the cheapest model that passes your quality bar).\n",
    );
    cheaper.push_str(
        "- Keep **Sonnet-class or Opus/MAX** for ambiguous bugs, release risk, security, multi-file architecture, and product/AC judgment.\n",
    );
    if let Some((model, avg, runs)) = highest_avg {
        if is_premium_model(&model) && avg > 0.15 {
            cheaper.push_str(&format!(
                "- **{model}** averages **${avg:.2}/run** over **{runs}** runs — try downgrading the easy turns in those threads, not the hard ones.\n",
            ));
        }
    }

    let mut habits = String::new();
    habits.push_str(
        "- Before starting a long agent session, pick the **lowest tier** that can finish the task; escalate only when stuck.\n",
    );
    habits.push_str(
        "- Split **explore** (cheap) from **implement** (mid) from **review** (strong only if needed).\n",
    );
    habits.push_str(
        "- Re-sync after heavy days and re-check **Per run** — spikes there usually mean the wrong default model.\n",
    );
    habits.push_str(
        "- Advice here is **local only** (no external API). For nuance on a specific thread, paste one run into Cursor chat yourself.\n",
    );

    let executive = build_executive_summary(
        premium_pct,
        top_model,
        top_runs,
        &downgrade_candidates,
        &top_tickets_by_tokens(cycle_runs, 6),
    );

    format!(
        "## Executive summary\n{executive}\n\n\
         ## Summary\n{summary}\n\n\
         ## Where spend concentrated\n{concentration}\n\
         ## Prompt patterns driving cost\n{patterns}\n\
         ## Cheaper-model opportunities\n{cheaper}\n\
         ## Concrete habits for this cycle\n{habits}"
    )
}

fn build_executive_summary(
    premium_pct: f64,
    top_model: &str,
    top_runs: &[RunRecord],
    downgrade_candidates: &[(f64, String)],
    top_tickets: &[TicketTokenHeavy],
) -> String {
    let mut use_fast: Vec<String> = Vec::new();
    let mut use_premium: Vec<String> = Vec::new();

    for run in top_runs {
        let theme = task_theme_label(&run.title);
        let kind = prompt_kind(&run.title);
        if is_premium_model(&run.model)
            && (kind == PromptKind::Mechanical || kind == PromptKind::Vague)
        {
            push_unique(&mut use_fast, theme.clone());
        }
        if kind == PromptKind::DeepWork {
            push_unique(&mut use_premium, theme);
        }
    }

    if use_fast.is_empty() {
        for (_, line) in downgrade_candidates.iter().take(3) {
            if let Some(excerpt) = line.split("prompt: \"").nth(1) {
                let t = excerpt.trim_end_matches('"').trim();
                if !t.is_empty() {
                    push_unique(&mut use_fast, truncate_prompt(t, 55));
                }
            }
        }
    }

    if use_fast.is_empty() {
        use_fast = vec![
            "lint and formatting fixes".into(),
            "merge conflicts and PR hygiene".into(),
            "short follow-ups like \"continue\" or \"sync now\"".into(),
        ];
    }
    if use_premium.is_empty() {
        use_premium = vec![
            "ambiguous production bugs".into(),
            "release and AC judgment calls".into(),
            "multi-file architecture or security work".into(),
        ];
    }

    use_fast.truncate(3);
    use_premium.truncate(3);

    let fast_list = join_natural(&use_fast);
    let premium_list = join_natural(&use_premium);

    let ticket_block = format_top_tickets_remark(top_tickets);

    let opener = if premium_pct >= 45.0 {
        "You’re leaning on expensive models a lot this cycle."
    } else if premium_pct >= 25.0 {
        "Your mix is reasonable, but there’s still easy savings on the table."
    } else {
        "Your premium spend is relatively contained this cycle."
    };

    let rest = format!(
        "{opener} **TL;DR:** use **fast / Composer-class** models more on work like {fast_list}, \
         and reserve **mid-tier or premium** models for {premium_list}. \
         Most of your dollars still land on **{top_model}** ({premium_pct:.0}% of spend is premium-tier)."
    );

    format!("{ticket_block}\n\n{rest}")
}

/// Top ticket rollup for the executive summary.
struct TicketTokenHeavy {
    key: String,
    tokens: i64,
    cost_usd: f64,
    run_count: i64,
    /// Best-effort title from synced run prompts (not live Jira).
    title: String,
    /// Model that consumed the most tokens on this ticket this cycle.
    primary_model: String,
}

fn top_tickets_by_tokens(runs: &[RunRecord], limit: usize) -> Vec<TicketTokenHeavy> {
    use std::collections::HashMap;

    struct Agg {
        tokens: i64,
        cost_usd: f64,
        run_count: i64,
        prompts: Vec<String>,
        model_tokens: HashMap<String, i64>,
    }

    let mut by_key: HashMap<String, Agg> = HashMap::new();
    for run in runs {
        let Some(key) = run_ticket_key(run) else {
            continue;
        };
        let tokens = run.input_tokens + run.output_tokens + run.cache_tokens;
        let entry = by_key.entry(key).or_insert(Agg {
            tokens: 0,
            cost_usd: 0.0,
            run_count: 0,
            prompts: Vec::new(),
            model_tokens: HashMap::new(),
        });
        entry.tokens += tokens;
        entry.cost_usd += run.cost_usd;
        entry.run_count += 1;
        *entry
            .model_tokens
            .entry(run.model.clone())
            .or_insert(0) += tokens;
        if !run.title.trim().is_empty() {
            entry.prompts.push(run.title.clone());
        }
    }

    let mut rows: Vec<TicketTokenHeavy> = by_key
        .into_iter()
        .map(|(key, agg)| TicketTokenHeavy {
            title: infer_ticket_title(&key, &agg.prompts),
            primary_model: primary_model_for_ticket(&agg.model_tokens),
            key,
            tokens: agg.tokens,
            cost_usd: agg.cost_usd,
            run_count: agg.run_count,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal))
    });
    rows.truncate(limit);
    rows
}

fn primary_model_for_ticket(model_tokens: &std::collections::HashMap<String, i64>) -> String {
    model_tokens
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(b.0)))
        .map(|(m, _)| m.clone())
        .unwrap_or_else(|| "unknown".into())
}

fn infer_ticket_title(key: &str, prompts: &[String]) -> String {
    if prompts.is_empty() {
        return "Agent work this cycle".into();
    }

    let mut best: Option<&str> = None;
    let mut best_score = 0i32;
    for p in prompts {
        let trimmed = p.trim();
        let words = trimmed.split_whitespace().count() as i32;
        if words <= 3 {
            continue;
        }
        let mut score = words;
        if trimmed.to_uppercase().contains(&key.to_uppercase()) {
            score += 12;
        }
        if words >= 10 {
            score += 4;
        }
        if score > best_score {
            best_score = score;
            best = Some(trimmed);
        }
    }

    let source = best.unwrap_or(prompts[0].trim());
    let subtitle = strip_leading_ticket_key(key, source);
    if subtitle.chars().count() < 8 {
        return truncate_prompt(source, 88);
    }
    truncate_prompt(&subtitle, 88)
}

fn strip_leading_ticket_key(key: &str, text: &str) -> String {
    let t = text.trim();
    let upper = t.to_uppercase();
    let key_upper = key.to_uppercase();
    if upper.starts_with(&key_upper) {
        let rest = t[key.len()..].trim();
        return rest
            .trim_start_matches([':', '-', '–', '—', '.', ','])
            .trim()
            .to_string();
    }
    t.to_string()
}

fn format_top_tickets_remark(top: &[TicketTokenHeavy]) -> String {
    if top.is_empty() {
        return "**Heaviest tickets by token usage:** no Jira-style keys (e.g. **FTSWB-1234**) in synced run titles this cycle — check individual runs below for one-off spikes.".into();
    }

    let mut out = String::from("**Heaviest tickets by token usage:**\n");
    for row in top {
        out.push_str(&format!(
            "- **{}** — {} (~{} tokens, ${:.2}, {} run{}) · **{}**\n",
            row.key,
            row.title,
            format_token_count(row.tokens),
            row.cost_usd,
            row.run_count,
            if row.run_count == 1 { "" } else { "s" },
            row.primary_model,
        ));
    }
    out.trim_end().to_string()
}

fn format_token_count(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if item.is_empty() {
        return;
    }
    if list.iter().any(|x| x.eq_ignore_ascii_case(&item)) {
        return;
    }
    list.push(item);
}

fn join_natural(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => format!("**{}**", items[0]),
        2 => format!("**{}** and **{}**", items[0], items[1]),
        _ => format!(
            "**{}**, **{}**, and **{}**",
            items[0], items[1], items[2]
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Mechanical,
    Vague,
    DeepWork,
    Neutral,
}

impl PromptKind {
    fn label(self) -> &'static str {
        match self {
            PromptKind::Mechanical => "mechanical",
            PromptKind::Vague => "vague/short",
            PromptKind::DeepWork => "deep-work",
            PromptKind::Neutral => "neutral",
        }
    }
}

fn prompt_kind(title: &str) -> PromptKind {
    let t = title.trim();
    let lower = t.to_lowercase();
    let words = t.split_whitespace().count();

    if words <= 6 {
        return PromptKind::Vague;
    }

    const MECH: &[&str] = &[
        "lint",
        "eslint",
        "format",
        "prettier",
        "merge conflict",
        "sync now",
        "git status",
        "run test",
        "npm test",
        "cargo check",
        "fix ci",
        "pr comment",
        "shepherd",
        "typecheck",
        "whitespace",
        "trailing space",
    ];
    if MECH.iter().any(|p| lower.contains(p)) {
        return PromptKind::Mechanical;
    }

    const DEEP: &[&str] = &[
        "architecture",
        "root cause",
        "security",
        "release",
        "production",
        "acceptance criteria",
        "ambiguous",
        "design",
        "performance",
        "race condition",
    ];
    if DEEP.iter().any(|p| lower.contains(p)) || words >= 28 {
        return PromptKind::DeepWork;
    }

    const VAGUE: &[&str] = &[
        "continue",
        "go ahead",
        "thanks",
        "looks good",
        "say the word",
        "fix this",
        "help me",
        "never mind",
    ];
    if VAGUE.iter().any(|p| lower.contains(p)) {
        return PromptKind::Vague;
    }

    PromptKind::Neutral
}

fn is_premium_model(model: &str) -> bool {
    let m = model.to_lowercase();
    [
        "opus",
        "max",
        "thinking",
        "xhigh",
        "high-fast",
        "gpt-5",
        "o1",
        "o3",
        "sonnet-4.5",
        "claude-4-opus",
        "claude-opus",
    ]
    .iter()
    .any(|k| m.contains(k))
}

fn is_fast_model(model: &str) -> bool {
    let m = model.to_lowercase();
    [
        "composer",
        "haiku",
        "flash",
        "mini",
        "fast",
        "gpt-5-mini",
        "gemini-3-flash",
    ]
    .iter()
    .any(|k| m.contains(k))
}

fn truncate_prompt(s: &str, max: usize) -> String {
    let t = s.trim().replace('\n', " ");
    if t.chars().count() <= max {
        return t;
    }
    format!("{}…", t.chars().take(max).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_tickets_aggregate_tokens_and_cost() {
        let runs = vec![
            RunRecord {
                provider: "cursor".into(),
                run_id: "a".into(),
                started_at: "2026-01-01".into(),
                model: "gpt-5".into(),
                input_tokens: 1000,
                output_tokens: 500,
                cache_tokens: 0,
                cost_usd: 1.0,
                title: "FTSWB-1001 fix amp embed".into(),
                conversation_id: String::new(),
                task_context: String::new(),
                task_inherited: false,
            },
            RunRecord {
                provider: "cursor".into(),
                run_id: "b".into(),
                started_at: "2026-01-02".into(),
                model: "gpt-5".into(),
                input_tokens: 2000,
                output_tokens: 0,
                cache_tokens: 0,
                cost_usd: 0.5,
                title: "FTSWB-1001 follow up".into(),
                conversation_id: String::new(),
                task_context: String::new(),
                task_inherited: false,
            },
            RunRecord {
                provider: "cursor".into(),
                run_id: "c".into(),
                started_at: "2026-01-03".into(),
                model: "gpt-5".into(),
                input_tokens: 50_000,
                output_tokens: 0,
                cache_tokens: 0,
                cost_usd: 2.0,
                title: "FTSWB-2002 big refactor".into(),
                conversation_id: String::new(),
                task_context: String::new(),
                task_inherited: false,
            },
        ];
        let top = top_tickets_by_tokens(&runs, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].key, "FTSWB-2002");
        assert_eq!(top[0].tokens, 50_000);
        assert_eq!(top[1].key, "FTSWB-1001");
        assert_eq!(top[1].tokens, 3500);
        assert!(top[1].title.to_lowercase().contains("amp")
            || top[1].title.contains("1001"));
    }

    #[test]
    fn infer_ticket_title_strips_key_prefix() {
        let title = infer_ticket_title(
            "FTSWB-6018",
            &["continue".into(), "FTSWB-6018: AMP AP embed domain on iOS Safari".into()],
        );
        assert!(title.to_lowercase().contains("amp"));
        assert!(!title.starts_with("FTSWB-6018"));
    }
}
