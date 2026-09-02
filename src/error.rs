//! Unified error types for the Loop harness.
//!
//! All errors propagate through the outer loop and are either
//! injected as observations for the LLM to self-correct, trigger
//! a checkpoint save, or are displayed to the user.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Tool execution failed: {tool_name} — {message}")]
    ToolExecution { tool_name: String, message: String },

    #[error("Context window exceeded (used {used} / {max} tokens)")]
    ContextOverflow { used: usize, max: usize },

    #[error("Max iterations reached ({0}). Checkpointing and pausing.")]
    IterationExhausted(usize),

    #[error("Checkpoint error: {0}")]
    Checkpoint(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("MCP connection error: {0}")]
    McpDisconnect(String),

    #[error("Model not configured: {0}")]
    ModelNotConfigured(String),

    #[error("API key missing for provider: {0}")]
    ApiKeyMissing(String),

    #[error("User cancelled operation")]
    UserCancelled,
}

impl LoopError {
    /// Convert to a string suitable for injecting into the LLM context
    /// as an observation, giving the model a chance to self-correct.
    pub fn as_observation(&self) -> String {
        format!("<observation type=\"error\">\n{}\n</observation>", self)
    }
}
