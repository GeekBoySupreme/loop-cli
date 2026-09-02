//! Edit tool — surgical string replacement in files.

use super::*;
use async_trait::async_trait;
use serde_json::json;

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Perform a surgical edit on a file by replacing an exact string match with new content. \
         The old_string must match exactly (including whitespace). Use this instead of rewriting entire files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit" },
                "old_string": { "type": "string", "description": "Exact string to find and replace" },
                "new_string": { "type": "string", "description": "Replacement string" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path'"))?;
        let old = params["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'old_string'"))?;
        let new = params["new_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'new_string'"))?;

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path, e))?;

        let count = content.matches(old).count();
        if count == 0 {
            return Ok(ToolResult {
                success: false,
                output: format!("No match found for the specified old_string in {}", path),
                is_mutation: false,
            });
        }
        if count > 1 {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Found {} matches for old_string in {}. Please provide a more unique string.",
                    count, path
                ),
                is_mutation: false,
            });
        }

        let new_content = content.replacen(old, new, 1);
        tokio::fs::write(path, &new_content)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", path, e))?;

        Ok(ToolResult {
            success: true,
            output: format!("Successfully edited {} (1 replacement made)", path),
            is_mutation: true,
        })
    }
}
