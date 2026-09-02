//! Parallel task decomposition and execution.
//!
//! When a user request contains multiple independent subtasks, the engine
//! asks the LLM to decompose it into a plan. Independent subtasks are
//! dispatched as parallel LLM inference threads. Each gets its own
//! spinner row in the terminal, showing live progress in a flat list.

use crate::cli::animation::ParallelProgress;
use crate::provider::{LlmProvider, Message, MessageContent, Role, ToolDefinition};
use crate::tools::ToolRegistry;

use console::style;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A subtask identified by the planner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Short label (e.g. "Update API handler")
    pub label: String,
    /// The detailed prompt to send to the LLM
    pub prompt: String,
    /// Whether this task depends on any other (index), or is independent
    #[serde(default)]
    pub depends_on: Option<usize>,
}

/// Result from planning: the decomposed task list + whether parallelism is viable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Should we parallelize at all?
    pub should_parallelize: bool,
    /// Why (brief explanation)
    pub reasoning: String,
    /// The subtasks
    pub tasks: Vec<SubTask>,
}

/// Ask the LLM to analyze the user input and decide if it should be decomposed
pub async fn plan_tasks(
    provider: &Arc<dyn LlmProvider>,
    user_input: &str,
    system_context: &str,
) -> anyhow::Result<TaskPlan> {
    let prompt = format!(
        r#"Analyze this user request and determine if it can be broken into independent, parallelizable subtasks.

User request: "{}"

Current context:
{}

Respond with valid JSON only, using this schema:
{{
  "should_parallelize": true/false,
  "reasoning": "Brief explanation of why/why not",
  "tasks": [
    {{
      "label": "Short label for this subtask",
      "prompt": "Detailed prompt for the agent to execute this subtask",
      "depends_on": null  // or index of a task this depends on (0-indexed)
    }}
  ]
}}

Rules:
- Only parallelize if there are 2+ truly independent subtasks (no data dependencies)
- Each subtask must be self-contained — it should not need results from another subtask
- If tasks MUST be sequential (e.g. "read file then modify it"), set should_parallelize to false
- If the request is simple/singular, return should_parallelize: false with a single task
- Maximum 5 parallel subtasks
- Each subtask's prompt should be specific and actionable"#,
        user_input, system_context
    );

    let messages = vec![Message {
        role: Role::User,
        content: MessageContent::Text(prompt),
    }];

    let response = provider
        .complete(
            &messages,
            &[],
            "You are a task planner. Output ONLY valid JSON, no markdown.",
        )
        .await?;

    let text = response.text.unwrap_or_default();

    // Try to parse the JSON (handle potential markdown wrapping)
    let json_text = extract_json(&text);

    match serde_json::from_str::<TaskPlan>(&json_text) {
        Ok(plan) => {
            // Sanity checks
            if plan.tasks.is_empty() {
                return Ok(TaskPlan {
                    should_parallelize: false,
                    reasoning: "Planner returned empty tasks".into(),
                    tasks: vec![SubTask {
                        label: "Execute request".into(),
                        prompt: user_input.to_string(),
                        depends_on: None,
                    }],
                });
            }
            if plan.tasks.len() > 5 {
                // Too many — fall back to sequential
                return Ok(TaskPlan {
                    should_parallelize: false,
                    reasoning: "Too many subtasks for safe parallelism".into(),
                    tasks: vec![SubTask {
                        label: "Execute request".into(),
                        prompt: user_input.to_string(),
                        depends_on: None,
                    }],
                });
            }
            Ok(plan)
        }
        Err(_) => {
            // Fallback: single sequential task
            Ok(TaskPlan {
                should_parallelize: false,
                reasoning: "Could not parse plan".into(),
                tasks: vec![SubTask {
                    label: "Execute request".into(),
                    prompt: user_input.to_string(),
                    depends_on: None,
                }],
            })
        }
    }
}

/// Execute independent subtasks in parallel with live progress display
pub async fn execute_parallel(
    provider: &Arc<dyn LlmProvider>,
    tools: &Arc<ToolRegistry>,
    subtasks: &[SubTask],
    system_prompt: &str,
    tool_defs: &[ToolDefinition],
    require_approval: bool,
) -> Vec<SubTaskResult> {
    let n = subtasks.len();

    // Print the plan header
    println!(
        "\n  {} Executing {} independent subtasks in parallel:\n",
        style("⚡").yellow().bold(),
        style(n).bold()
    );

    for (i, task) in subtasks.iter().enumerate() {
        println!(
            "  {} {} {}",
            style(format!("[{}]", i + 1)).color256(69).bold(),
            style("→").dim(),
            task.label
        );
    }
    println!();

    // Start parallel progress display
    let labels: Vec<String> = subtasks.iter().map(|t| t.label.clone()).collect();
    let progress = ParallelProgress::start(labels);

    // Spawn parallel tasks
    let mut handles = Vec::new();

    for (i, task) in subtasks.iter().enumerate() {
        let provider = provider.clone();
        let tools = tools.clone();
        let system_prompt = system_prompt.to_string();
        let tool_defs: Vec<ToolDefinition> = tool_defs.to_vec();
        let prompt = task.prompt.clone();
        let label = task.label.clone();
        let progress = progress.clone();

        let handle = tokio::spawn(async move {
            progress.update_status(i, "thinking");

            let messages = vec![Message {
                role: Role::User,
                content: MessageContent::Text(prompt.clone()),
            }];

            let mut all_text = String::new();
            let mut all_tool_outputs = Vec::new();
            let mut total_in = 0usize;
            let mut total_out = 0usize;
            let mut iteration = 0;
            let mut current_messages = messages;

            // Inner loop for this subtask
            loop {
                if iteration >= 10 {
                    break;
                }

                let response = match provider
                    .complete(&current_messages, &tool_defs, &system_prompt)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        progress.update_status(i, &format!("error: {}", e));
                        return SubTaskResult {
                            label,
                            success: false,
                            output: format!("API error: {}", e),
                            tokens_in: total_in,
                            tokens_out: total_out,
                            tool_calls: Vec::new(),
                        };
                    }
                };

                total_in += response.usage.input_tokens;
                total_out += response.usage.output_tokens;
                progress.update_tokens(i, total_in, total_out);

                // Handle text
                if let Some(text) = &response.text {
                    if !text.is_empty() {
                        all_text.push_str(text);
                    }
                }

                // Handle tool calls
                if !response.tool_calls.is_empty() {
                    for tc in &response.tool_calls {
                        progress.update_status(i, &format!("tool: {}", tc.name));

                        // Execute tool (parallel tasks skip approval for read-only tools)
                        let tool_result = tools.execute(&tc.name, tc.arguments.clone()).await;
                        match tool_result {
                            Ok(result) => {
                                all_tool_outputs.push(format!(
                                    "{}({}) → {}",
                                    tc.name,
                                    format_args_short_parallel(&tc.arguments),
                                    if result.output.len() > 100 {
                                        format!("{}...", &result.output[..100])
                                    } else {
                                        result.output.clone()
                                    }
                                ));

                                // Add tool result to messages for next iteration
                                current_messages.push(Message {
                                    role: Role::Assistant,
                                    content: MessageContent::Text(format!(
                                        "I called tool '{}' with args {}.",
                                        tc.name, tc.arguments
                                    )),
                                });
                                current_messages.push(Message {
                                    role: Role::Tool,
                                    content: MessageContent::ToolResult {
                                        tool_use_id: tc.id.clone(),
                                        content: result.output,
                                    },
                                });
                            }
                            Err(e) => {
                                current_messages.push(Message {
                                    role: Role::Tool,
                                    content: MessageContent::ToolResult {
                                        tool_use_id: tc.id.clone(),
                                        content: format!("Error: {}", e),
                                    },
                                });
                            }
                        }
                    }

                    iteration += 1;
                    progress.update_status(i, "thinking");
                    continue;
                }

                // End turn or max tokens
                match response.stop_reason {
                    crate::provider::StopReason::EndTurn => break,
                    crate::provider::StopReason::ToolUse => {
                        iteration += 1;
                        continue;
                    }
                    _ => break,
                }
            }

            progress.update_status(i, "done ✓");

            SubTaskResult {
                label,
                success: true,
                output: all_text,
                tokens_in: total_in,
                tokens_out: total_out,
                tool_calls: all_tool_outputs,
            }
        });

        handles.push(handle);
    }

    // Await all tasks
    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(SubTaskResult {
                label: "unknown".into(),
                success: false,
                output: format!("Task panicked: {}", e),
                tokens_in: 0,
                tokens_out: 0,
                tool_calls: Vec::new(),
            }),
        }
    }

    // Stop the progress display
    progress.stop();

    // Print summary
    println!();
    println!(
        "  {} All {} subtasks complete:",
        style("✓").color256(111).bold(),
        n
    );
    for result in &results {
        let status = if result.success {
            style("✓").color256(111)
        } else {
            style("✗").red()
        };
        println!(
            "  {} {} ({}↓ {}↑)",
            status,
            result.label,
            format_tokens_compact(result.tokens_in),
            format_tokens_compact(result.tokens_out),
        );
    }
    println!();

    results
}

/// Result from a parallel subtask execution
#[derive(Debug, Clone)]
pub struct SubTaskResult {
    pub label: String,
    pub success: bool,
    pub output: String,
    pub tokens_in: usize,
    pub tokens_out: usize,
    pub tool_calls: Vec<String>,
}

/// Extract JSON from potentially markdown-wrapped text
fn extract_json(text: &str) -> String {
    // Try to find ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    // Try finding JSON object directly
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return text[start..=end].to_string();
        }
    }
    text.trim().to_string()
}

fn format_args_short_parallel(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        let parts: Vec<String> = obj
            .iter()
            .take(2)
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) if s.len() > 30 => format!("\"{}...\"", &s[..30]),
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    other => {
                        let s = other.to_string();
                        if s.len() > 30 {
                            format!("{}...", &s[..30])
                        } else {
                            s
                        }
                    }
                };
                format!("{}={}", k, val)
            })
            .collect();
        parts.join(", ")
    } else {
        "...".into()
    }
}

fn format_tokens_compact(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}
