use anyhow::Result;
use async_trait::async_trait;

use crate::db::Db;
use crate::models::{AppSettings, QuotaSnapshot};

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn sync(&self, db: &Db, settings: &AppSettings) -> Result<()>;
    async fn quota(&self, db: &Db, settings: &AppSettings) -> Result<QuotaSnapshot>;
}

pub mod anthropic_api;
pub mod claude_code;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod manual;
pub mod openai_api;
pub mod xai_api;

pub fn all_providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(codex::CodexProvider),
        Box::new(claude_code::ClaudeCodeProvider),
        Box::new(cursor::CursorProvider),
        Box::new(copilot::CopilotProvider),
        Box::new(openai_api::OpenAiProvider),
        Box::new(anthropic_api::AnthropicProvider),
        Box::new(xai_api::XaiProvider),
    ]
}
