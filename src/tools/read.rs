//! Read tool — read file contents with optional line range.

use super::*;
use async_trait::async_trait;
use serde_json::json;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Optionally specify start_line and end_line for a range."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute or relative path to the file" },
                "start_line": { "type": "integer", "description": "Optional 1-indexed start line" },
                "end_line": { "type": "integer", "description": "Optional 1-indexed end line (inclusive)" }
            },
            "required": ["path"]
        })
    }

    fn is_mutating(&self) -> bool {
        false
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path'"))?;
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path, e))?;

        let lines: Vec<&str> = content.lines().collect();
        let start = params["start_line"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(1);
        let end = params["end_line"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(lines.len());

        let start = start.saturating_sub(1).min(lines.len());
        let end = end.min(lines.len());

        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4} │ {}", start + i + 1, line))
            .collect();

        let output = format!(
            "File: {} ({} lines total, showing {}-{})\n\n{}",
            path,
            lines.len(),
            start + 1,
            end,
            selected.join("\n")
        );

        Ok(ToolResult {
            success: true,
            output,
            is_mutation: false,
        })
    }
}
