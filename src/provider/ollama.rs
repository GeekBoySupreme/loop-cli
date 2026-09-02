//! Ollama provider for local Gemma models.

use super::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct OllamaProvider {
    client: Client,
    endpoint: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(endpoint: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "Ollama"
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
        // Ollama supports OpenAI-compatible chat endpoint
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

        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "stream": false,
        });
        if !tool_defs.is_empty() {
            body["tools"] = json!(tool_defs);
        }

        let url = format!("{}/api/chat", self.endpoint);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama connection failed (is it running?): {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            return Err(anyhow::anyhow!("Ollama error ({}): {}", status, body));
        }

        let message = &body["message"];
        let text = message["content"].as_str().map(String::from);

        let mut tool_calls = Vec::new();
        if let Some(calls) = message["tool_calls"].as_array() {
            for c in calls {
                let func = &c["function"];
                tool_calls.push(ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: func["name"].as_str().unwrap_or("").into(),
                    arguments: func["arguments"].clone(),
                });
            }
        }

        let stop_reason = if !tool_calls.is_empty() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            stop_reason,
            usage: TokenUsage {
                input_tokens: body["prompt_eval_count"].as_u64().unwrap_or(0) as usize,
                output_tokens: body["eval_count"].as_u64().unwrap_or(0) as usize,
            },
        })
    }
}
