//! Anthropic (Claude) provider implementation.
//!
//! Supports Claude Sonnet 4 and Haiku 3.5 via the Anthropic Messages API.

use super::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    fn format_messages(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "user",
                    Role::System => unreachable!(),
                };

                match &m.content {
                    MessageContent::Text(text) => json!({
                        "role": role,
                        "content": text,
                    }),
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                    } => json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                        }],
                    }),
                }
            })
            .collect()
    }

    fn format_tools(&self, tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "Anthropic"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system_prompt: &str,
    ) -> anyhow::Result<CompletionResponse> {
        let formatted_messages = self.format_messages(messages);
        let formatted_tools = self.format_tools(tools);

        let mut body = json!({
            "model": self.model,
            "max_tokens": 8192,
            "system": system_prompt,
            "messages": formatted_messages,
        });

        if !formatted_tools.is_empty() {
            body["tools"] = json!(formatted_tools);
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let response_body: serde_json::Value = response.json().await?;

        if !status.is_success() {
            let error_msg = response_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {}",
                status,
                error_msg
            ));
        }

        // Parse the response
        let mut text = None;
        let mut tool_calls = Vec::new();

        if let Some(content) = response_body["content"].as_array() {
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        text = block["text"].as_str().map(|s| s.to_string());
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            arguments: block["input"].clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = match response_body["stop_reason"].as_str() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some(other) => StopReason::Error(other.to_string()),
            None => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: response_body["usage"]["input_tokens"].as_u64().unwrap_or(0) as usize,
            output_tokens: response_body["usage"]["output_tokens"]
                .as_u64()
                .unwrap_or(0) as usize,
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
            stop_reason,
        })
    }
}
