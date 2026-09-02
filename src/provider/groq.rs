//! Groq provider implementation.
//!
//! Supports Llama models via Groq's OpenAI-compatible API.

use super::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct GroqProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl GroqProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for GroqProvider {
    fn name(&self) -> &str {
        "Groq"
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
        // Groq uses OpenAI-compatible format
        let mut msgs = vec![json!({"role": "system", "content": system_prompt})];
        for m in messages {
            match (&m.role, &m.content) {
                (Role::User, MessageContent::Text(text)) => {
                    msgs.push(json!({"role": "user", "content": text}));
                }
                (Role::Assistant, MessageContent::Text(text)) => {
                    msgs.push(json!({"role": "assistant", "content": text}));
                }
                (
                    Role::Tool,
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                    },
                ) => {
                    msgs.push(
                        json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}),
                    );
                }
                _ => {}
            }
        }

        let tool_defs: Vec<serde_json::Value> = tools.iter().map(|t| {
            json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}})
        }).collect();

        let mut body = json!({"model": self.model, "messages": msgs, "max_tokens": 8192});
        if !tool_defs.is_empty() {
            body["tools"] = json!(tool_defs);
        }

        let resp = self
            .client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Groq API error ({}): {}",
                status,
                body["error"]["message"].as_str().unwrap_or("unknown")
            ));
        }

        let choice = &body["choices"][0];
        let message = &choice["message"];
        let text = message["content"].as_str().map(String::from);

        let mut tool_calls = Vec::new();
        if let Some(calls) = message["tool_calls"].as_array() {
            for c in calls {
                tool_calls.push(ToolCall {
                    id: c["id"].as_str().unwrap_or("").into(),
                    name: c["function"]["name"].as_str().unwrap_or("").into(),
                    arguments: serde_json::from_str(
                        c["function"]["arguments"].as_str().unwrap_or("{}"),
                    )
                    .unwrap_or(json!({})),
                });
            }
        }

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            stop_reason,
            usage: TokenUsage {
                input_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as usize,
                output_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or(0) as usize,
            },
        })
    }
}
