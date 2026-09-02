//! Configuration type definitions for Loop.

use crate::mcp::McpServerConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root configuration for the Loop harness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopConfig {
    /// Which model ID to use by default (e.g., "claude-sonnet-4-20250514")
    pub default_model: String,

    /// Model provider configurations
    pub models: ModelConfig,

    /// Paths to local .md instruction files injected into system prompt
    #[serde(default)]
    pub instructions: Vec<PathBuf>,

    /// Directory for CLI plugins
    #[serde(default = "default_plugins_dir")]
    pub plugins_dir: PathBuf,

    /// Directory for checkpoint files
    #[serde(default = "default_checkpoints_dir")]
    pub checkpoints_dir: PathBuf,

    /// Maximum iterations before forced checkpoint (inner loop guard)
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,

    /// Maximum context tokens before auto-compaction triggers
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,

    /// Whether tools require explicit user approval
    #[serde(default = "default_require_approval")]
    pub require_tool_approval: bool,

    /// Whether to auto-commit accepted changes via git
    #[serde(default)]
    pub git_auto_commit: bool,

    /// Configured MCP servers
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

/// All model provider configurations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelConfig {
    pub anthropic: Option<AnthropicConfig>,
    pub openai: Option<OpenAIConfig>,
    pub gemini: Option<GeminiConfig>,
    pub groq: Option<GroqConfig>,
    pub ollama: Option<OllamaConfig>,
    #[serde(default)]
    pub openrouter: Option<OpenRouterConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default = "default_anthropic_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default = "default_openai_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default = "default_gemini_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroqConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default = "default_groq_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_ollama_models")]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,
    /// OpenRouter model slug, for example `anthropic/claude-sonnet-4`.
    pub model: String,
}

/// Enum identifying which provider backend to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    Gemini,
    Groq,
    Ollama,
    OpenRouter,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => write!(f, "Anthropic"),
            ProviderKind::OpenAI => write!(f, "OpenAI"),
            ProviderKind::Gemini => write!(f, "Google Gemini"),
            ProviderKind::Groq => write!(f, "Groq"),
            ProviderKind::Ollama => write!(f, "Ollama (local)"),
            ProviderKind::OpenRouter => write!(f, "OpenRouter"),
        }
    }
}

// ── Default value functions ─────────────────────────────────────────

fn default_plugins_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".loop").join("plugins")
}

fn default_checkpoints_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".loop").join("checkpoints")
}

fn default_max_iterations() -> usize {
    25
}

fn default_max_context_tokens() -> usize {
    128_000
}

fn default_require_approval() -> bool {
    true
}

fn default_anthropic_models() -> Vec<String> {
    vec![
        "claude-sonnet-4-20250514".into(),
        "claude-haiku-3-5-20241022".into(),
    ]
}

fn default_openai_models() -> Vec<String> {
    vec!["gpt-4o".into(), "gpt-4o-mini".into()]
}

fn default_gemini_models() -> Vec<String> {
    vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()]
}

fn default_groq_models() -> Vec<String> {
    vec!["llama-3.3-70b-versatile".into()]
}

fn default_ollama_endpoint() -> String {
    "http://localhost:11434".into()
}

fn default_ollama_models() -> Vec<String> {
    vec!["gemma3".into()]
}
