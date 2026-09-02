//! OpenAI provider implementation.
//!
//! Supports GPT-4o and GPT-4o-mini via the OpenAI Chat Completions API.

use super::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    model: String,
    endpoint: String,
    provider_name: &'static str,
}

impl OpenAIProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            provider_name: "OpenAI",
        }
    }

    pub fn openrouter(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".to_string(),
            provider_name: "OpenRouter",
        }
    }

    fn format_messages(&self, messages: &[Message], system_prompt: &str) -> Vec<serde_json::Value> {
        let mut result = vec![json!({
            "role": "system",
            "content": system_prompt,
        })];

        for m in messages {
            match (&m.role, &m.content) {
                (Role::User, MessageContent::Text(text)) => {
                    result.push(json!({"role": "user", "content": text}));
                }
                (Role::Assistant, MessageContent::Text(text)) => {
                    result.push(json!({"role": "assistant", "content": text}));
                }
                (
                    Role::Tool,
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                    },
                ) => {
                    result.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    }));
                }
                _ => {}
            }
        }

        result
    }

    fn format_tools(&self, tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    fn name(&self) -> &str {
        self.provider_name
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
        let formatted_messages = self.format_messages(messages, system_prompt);
        let formatted_tools = self.format_tools(tools);

        let mut body = json!({
            "model": self.model,
            "messages": formatted_messages,
            "max_tokens": 8192,
        });

        if !formatted_tools.is_empty() {
            body["tools"] = json!(formatted_tools);
        }

        let response = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://github.com/loop-cli/loop")
            .header("X-Title", "Loop CLI")
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
                "{} API error ({}): {}",
                self.provider_name,
                status,
                error_msg
            ));
        }

        let choice = &response_body["choices"][0];
        let message = &choice["message"];

        let text = message["content"].as_str().map(|s| s.to_string());

        let mut tool_calls = Vec::new();
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                tool_calls.push(ToolCall {
                    id: call["id"].as_str().unwrap_or("").to_string(),
                    name: call["function"]["name"].as_str().unwrap_or("").to_string(),
                    arguments: serde_json::from_str(
                        call["function"]["arguments"].as_str().unwrap_or("{}"),
                    )
                    .unwrap_or(json!({})),
                });
            }
        }

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some(other) => StopReason::Error(other.to_string()),
            None => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input_tokens: response_body["usage"]["prompt_tokens"]
                .as_u64()
                .unwrap_or(0) as usize,
            output_tokens: response_body["usage"]["completion_tokens"]
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
