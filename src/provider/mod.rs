//! LLM Provider abstraction layer.
//!
//! Defines the `LlmProvider` trait that all model backends implement,
//! and a factory function to create the right provider from config.

pub mod anthropic;
pub mod gemini;
pub mod groq;
pub mod ollama;
pub mod openai;

use crate::config::{LoopConfig, ProviderKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A single message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Content can be plain text or structured (for tool results etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

impl MessageContent {
    pub fn as_text(&self) -> &str {
        match self {
            MessageContent::Text(t) => t,
            MessageContent::ToolResult { content, .. } => content,
        }
    }
}

/// A tool call request emitted by the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool definition sent to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result from a completion request
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// Any text content the model produced
    pub text: Option<String>,
    /// Tool calls the model wants to make
    pub tool_calls: Vec<ToolCall>,
    /// Tokens used in the request
    pub usage: TokenUsage,
    /// Whether the model wants to stop
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Error(String),
}

/// The unified trait all LLM providers must implement
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Display name of this provider
    fn name(&self) -> &str;

    /// The model ID currently configured
    fn model_id(&self) -> &str;

    /// Send a completion request
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system_prompt: &str,
    ) -> anyhow::Result<CompletionResponse>;
}

/// Factory: create the appropriate provider from the user's config
pub fn create_provider(config: &LoopConfig) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let (model_id, kind) = config
        .default_provider_info()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    match kind {
        ProviderKind::Anthropic => {
            let cfg = config.models.anthropic.as_ref().unwrap();
            require_api_key("Anthropic", &cfg.api_key)?;
            Ok(Arc::new(anthropic::AnthropicProvider::new(
                &cfg.api_key,
                model_id,
            )))
        }
        ProviderKind::OpenAI => {
            let cfg = config.models.openai.as_ref().unwrap();
            require_api_key("OpenAI", &cfg.api_key)?;
            Ok(Arc::new(openai::OpenAIProvider::new(
                &cfg.api_key,
                model_id,
            )))
        }
        ProviderKind::Gemini => {
            let cfg = config.models.gemini.as_ref().unwrap();
            require_api_key("Gemini", &cfg.api_key)?;
            Ok(Arc::new(gemini::GeminiProvider::new(
                &cfg.api_key,
                model_id,
            )))
        }
        ProviderKind::Groq => {
            let cfg = config.models.groq.as_ref().unwrap();
            require_api_key("Groq", &cfg.api_key)?;
            Ok(Arc::new(groq::GroqProvider::new(&cfg.api_key, model_id)))
        }
        ProviderKind::Ollama => {
            let cfg = config.models.ollama.as_ref().unwrap();
            Ok(Arc::new(ollama::OllamaProvider::new(
                &cfg.endpoint,
                model_id,
            )))
        }
        ProviderKind::OpenRouter => {
            let cfg = config.models.openrouter.as_ref().unwrap();
            require_api_key("OpenRouter", &cfg.api_key)?;
            Ok(Arc::new(openai::OpenAIProvider::openrouter(
                &cfg.api_key,
                model_id,
            )))
        }
    }
}

fn require_api_key(provider: &str, api_key: &str) -> anyhow::Result<()> {
    if api_key.is_empty() {
        anyhow::bail!(
            "{} API key is missing from the OS credential store. Run `loop init` to configure it.",
            provider
        );
    }
    Ok(())
}

/// Verify an OpenRouter key before asking the user to choose a model slug.
pub async fn validate_openrouter_key(api_key: &str) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .get("https://openrouter.ai/api/v1/key")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body: serde_json::Value = response.json().await.unwrap_or_default();
    let message = body
        .pointer("/error/message")
        .and_then(|value| value.as_str())
        .unwrap_or("key validation failed");
    anyhow::bail!("OpenRouter API error ({}): {}", status, message)
}
