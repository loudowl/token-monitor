use std::collections::{HashMap, HashSet};

use crate::models::RunRecord;

pub fn extract_jira_key(title: &str) -> Option<String> {
    for word in title.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        if w.len() >= 6
            && w.contains('-')
            && w.chars().next()?.is_ascii_uppercase()
            && w.chars().any(|c| c.is_ascii_digit())
        {
            let parts: Vec<&str> = w.split('-').collect();
            if parts.len() >= 2 && parts[0].chars().all(|c| c.is_ascii_uppercase()) {
                return Some(w.to_string());
            }
        }
    }
    None
}

pub fn task_theme_label(title: &str) -> String {
    let t = title.trim();
    let lower = t.to_lowercase();

    if let Some(ticket) = extract_jira_key(t) {
        let rest = t
            .split_whitespace()
            .skip(1)
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        if rest.is_empty() {
            return format!("{ticket} follow-ups");
        }
        return truncate_text(&format!("{ticket} {rest}"), 56);
    }

    const MECH_LABELS: &[(&str, &str)] = &[
        ("merge conflict", "merge conflict cleanup"),
        ("eslint", "ESLint / lint fixes"),
        ("lint", "lint fixes"),
        ("npm test", "running tests locally"),
        ("cargo check", "Rust compile checks"),
        ("pr shepherd", "PR shepherd / CI follow-up"),
        ("shepherd", "PR shepherd / CI follow-up"),
        ("sync now", "usage sync and housekeeping"),
        ("typecheck", "typecheck / build fixes"),
        ("whitespace", "whitespace and formatting"),
        ("hotfix pr", "hotfix PR prep"),
        ("create pr", "PR creation and description"),
    ];
    for (needle, label) in MECH_LABELS {
        if lower.contains(needle) {
            return (*label).into();
        }
    }

    if lower.contains("release") || lower.contains("11.") {
        return "release-line / collab branch work".into();
    }
    if lower.contains("stage") || lower.contains("qa") {
        return "stage / QA verification".into();
    }
    if lower.contains("advisor") || lower.contains("architecture") {
        return "planning and architecture".into();
    }
    if lower.contains("security") {
        return "security-sensitive changes".into();
    }

    truncate_text(t, 56)
}

pub fn resolve_task_context(title: &str, thread_ticket: Option<&str>) -> (String, bool) {
    if let Some(key) = extract_jira_key(title) {
        return (key, false);
    }
    if let Some(key) = thread_ticket {
        return (key.to_string(), true);
    }
    (task_theme_label(title), false)
}

pub fn run_ticket_key(run: &RunRecord) -> Option<String> {
    extract_jira_key(&run.title).or_else(|| {
        if run.task_inherited {
            Some(run.task_context.clone())
        } else {
            extract_jira_key(&run.task_context)
        }
    })
}

pub fn jira_from_thread_titles(titles: &[(String, String)]) -> Option<String> {
    let mut sorted = titles.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, title) in &sorted {
        if let Some(k) = extract_jira_key(title) {
            return Some(k);
        }
    }
    None
}

pub fn enrich_runs_task_context(
    runs: &mut [RunRecord],
    thread_titles: &HashMap<String, Vec<(String, String)>>,
) {
    for run in runs.iter_mut() {
        let thread_ticket = if run.conversation_id.is_empty() {
            None
        } else {
            thread_titles
                .get(&run.conversation_id)
                .and_then(|rows| jira_from_thread_titles(rows))
        };
        let (ctx, inherited) = resolve_task_context(&run.title, thread_ticket.as_deref());
        run.task_context = ctx;
        run.task_inherited = inherited;
    }
}

pub fn collect_conversation_ids(runs: &[RunRecord]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for run in runs {
        if run.conversation_id.is_empty() {
            continue;
        }
        if seen.insert(run.conversation_id.clone()) {
            out.push(run.conversation_id.clone());
        }
    }
    out
}

fn truncate_text(s: &str, max: usize) -> String {
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
    fn inherits_ticket_from_thread() {
        let (ctx, inherited) = resolve_task_context("continue", Some("FTSWB-6018"));
        assert_eq!(ctx, "FTSWB-6018");
        assert!(inherited);
    }

    #[test]
    fn prompt_ticket_wins_over_thread() {
        let (ctx, inherited) = resolve_task_context("FTSWB-9999 fix", Some("FTSWB-6018"));
        assert_eq!(ctx, "FTSWB-9999");
        assert!(!inherited);
    }
}
