//! Memory management: conversation history, token tracking, auto-compaction.

use crate::provider::{Message, MessageContent, Role};
use std::collections::HashSet;
use std::path::PathBuf;

/// Manages conversation context, file tracking, and auto-compaction
pub struct MemoryManager {
    /// Active conversation messages
    messages: Vec<Message>,
    /// The base system prompt (from skills + instructions)
    system_prompt: String,
    /// Max context window in tokens
    max_context_tokens: usize,
    /// Tokens reserved for LLM response
    reserve_tokens: usize,
    /// Recent tokens to always keep uncompressed
    keep_recent_messages: usize,
    /// Files read during this session
    pub read_files: HashSet<PathBuf>,
    /// Files modified during this session
    pub modified_files: HashSet<PathBuf>,
    /// Running token estimate
    estimated_tokens: usize,
}

impl MemoryManager {
    pub fn new(max_context_tokens: usize) -> Self {
        Self {
            messages: Vec::new(),
            system_prompt: String::new(),
            max_context_tokens,
            reserve_tokens: 4096,
            keep_recent_messages: 10,
            read_files: HashSet::new(),
            modified_files: HashSet::new(),
            estimated_tokens: 0,
        }
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
        });
        self.estimated_tokens += estimate_tokens(text);
    }

    pub fn add_assistant_message(&mut self, text: &str) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
        });
        self.estimated_tokens += estimate_tokens(text);
    }

    /// Add an assistant message that contains tool calls (stored as text summary)
    pub fn add_assistant_tool_use(&mut self, summary: &str) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(summary.to_string()),
        });
        self.estimated_tokens += estimate_tokens(summary);
    }

    pub fn add_tool_result(&mut self, tool_use_id: &str, content: &str) {
        // Truncate very large results before adding to context
        let truncated = if content.len() > 15_000 {
            format!(
                "{}...\n\n[Output truncated: {} total characters]",
                &content[..15_000],
                content.len()
            )
        } else {
            content.to_string()
        };

        self.messages.push(Message {
            role: Role::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: truncated.clone(),
            },
        });
        self.estimated_tokens += estimate_tokens(&truncated);
    }

    pub fn add_error_observation(&mut self, error: &str) {
        self.add_tool_result(
            "error",
            &format!("<observation type=\"error\">\n{}\n</observation>", error),
        );
    }

    /// Track a file read operation
    pub fn track_read(&mut self, path: PathBuf) {
        self.read_files.insert(path);
    }

    /// Track a file modification
    pub fn track_modification(&mut self, path: PathBuf) {
        self.modified_files.insert(path);
    }

    /// Estimated total tokens in the current context
    pub fn estimated_tokens(&self) -> usize {
        self.estimated_tokens + estimate_tokens(&self.system_prompt)
    }

    /// Check if compaction is needed and perform it
    pub fn needs_compaction(&self) -> bool {
        self.estimated_tokens() > self.max_context_tokens - self.reserve_tokens
    }

    /// Compact old messages into a summary.
    /// Returns the summary text that should be sent to the LLM for summarization.
    pub fn prepare_compaction(&self) -> Option<(String, usize)> {
        if !self.needs_compaction() || self.messages.len() <= self.keep_recent_messages {
            return None;
        }

        let split_point = self
            .messages
            .len()
            .saturating_sub(self.keep_recent_messages);
        let old_messages = &self.messages[..split_point];

        // Build a text representation of old messages for summarization
        let mut text = String::from("Summarize this conversation history, preserving:\n");
        text.push_str("- Key decisions made\n");
        text.push_str("- Files read and modified\n");
        text.push_str("- Current task context\n");
        text.push_str("- Any errors encountered\n\n");

        for msg in old_messages {
            let role = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
                Role::System => "System",
            };
            // Truncate individual messages during compaction
            let content = msg.content.as_text();
            let truncated = if content.len() > 2000 {
                format!("{}... [truncated]", &content[..2000])
            } else {
                content.to_string()
            };
            text.push_str(&format!("[{}]: {}\n\n", role, truncated));
        }

        Some((text, split_point))
    }

    /// Apply a compaction summary, replacing old messages
    pub fn apply_compaction(&mut self, summary: &str, split_point: usize) {
        // Build file tracking summary
        let files_summary = format!(
            "\n\nFiles read: {:?}\nFiles modified: {:?}",
            self.read_files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
            self.modified_files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
        );

        let combined_summary = format!("{}{}", summary, files_summary);

        // Replace old messages with a single summary message
        let recent = self.messages.split_off(split_point);
        self.messages.clear();
        self.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(format!(
                "[Previous conversation summary]\n{}",
                combined_summary
            )),
        });
        self.messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Text(
                "Understood. I have the context from the previous conversation. Let me continue."
                    .to_string(),
            ),
        });
        self.messages.extend(recent);

        // Recalculate token estimate
        self.estimated_tokens = self
            .messages
            .iter()
            .map(|m| estimate_tokens(m.content.as_text()))
            .sum();
    }

    /// Clear all context
    pub fn clear(&mut self) {
        self.messages.clear();
        self.estimated_tokens = 0;
    }
}

/// Rough token estimation: ~4 chars per token (good enough for management)
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}
