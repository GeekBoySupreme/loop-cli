//! Google Gemini provider implementation.
//!
//! Supports Gemini 2.5 Pro and Flash via the Generative Language API.

use super::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

pub struct GeminiProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    fn format_contents(
        &self,
        messages: &[Message],
        system_prompt: &str,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let system = json!({
            "parts": [{"text": system_prompt}]
        });

        let contents: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| {
                let role = match m.role {
                    Role::User | Role::Tool => "user",
                    Role::Assistant => "model",
                    Role::System => unreachable!(),
                };

                match &m.content {
                    MessageContent::Text(text) => json!({
                        "role": role,
                        "parts": [{"text": text}],
                    }),
                    MessageContent::ToolResult {
                        tool_use_id,
                        content,
                    } => json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": tool_use_id,
                                "response": {
                                    "content": content,
                                }
                            }
                        }],
                    }),
                }
            })
            .collect();

        (system, contents)
    }

    fn format_tools(&self, tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
        if tools.is_empty() {
            return vec![];
        }

        let function_declarations: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();

        vec![json!({
            "functionDeclarations": function_declarations,
        })]
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        "Google Gemini"
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
        let (system_instruction, contents) = self.format_contents(messages, system_prompt);
        let formatted_tools = self.format_tools(tools);

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let mut body = json!({
            "system_instruction": system_instruction,
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": 8192,
            },
        });

        if !formatted_tools.is_empty() {
            body["tools"] = json!(formatted_tools);
        }

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
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
                "Gemini API error ({}): {}",
                status,
                error_msg
            ));
        }

        let candidate = &response_body["candidates"][0];
        let parts = candidate["content"]["parts"].as_array();

        let mut text = None;
        let mut tool_calls = Vec::new();

        if let Some(parts) = parts {
            for part in parts {
                if let Some(t) = part["text"].as_str() {
                    text = Some(t.to_string());
                }
                if let Some(fc) = part.get("functionCall") {
                    tool_calls.push(ToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fc["name"].as_str().unwrap_or("").to_string(),
                        arguments: fc["args"].clone(),
                    });
                }
            }
        }

        let stop_reason = match candidate["finishReason"].as_str() {
            Some("STOP") => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            _ => {
                if !tool_calls.is_empty() {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                }
            }
        };

        let usage = TokenUsage {
            input_tokens: response_body["usageMetadata"]["promptTokenCount"]
                .as_u64()
                .unwrap_or(0) as usize,
            output_tokens: response_body["usageMetadata"]["candidatesTokenCount"]
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
