//! Multi-edit tool — batch file edits in a single tool call.
//!
//! Accepts an array of edit operations across multiple files,
//! each with path, old_string, and new_string. Executes all
//! edits atomically (all-or-nothing validation, then apply).

use super::*;
use async_trait::async_trait;
use serde_json::json;

pub struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn description(&self) -> &str {
        "Apply multiple surgical edits across one or more files in a single call. \
         Each edit specifies a file path, exact old_string to find, and new_string to replace it with. \
         All edits are validated before any are applied — if any old_string is not found, \
         no changes are made. Use this for coordinated changes across multiple files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "description": "Array of edit operations to apply",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to edit" },
                            "old_string": { "type": "string", "description": "Exact string to find" },
                            "new_string": { "type": "string", "description": "Replacement string" }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let edits = params["edits"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing 'edits' array"))?;

        if edits.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "No edits provided".to_string(),
                is_mutation: false,
            });
        }

        // ── Phase 1: Validate all edits ─────────────────────────────
        // Read all files and verify matches exist
        let mut file_contents: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut operations: Vec<(String, String, String)> = Vec::new(); // (path, old, new)
        let mut errors: Vec<String> = Vec::new();

        for (i, edit) in edits.iter().enumerate() {
            let path = edit["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Edit #{}: missing 'path'", i + 1))?;
            let old = edit["old_string"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Edit #{}: missing 'old_string'", i + 1))?;
            let new = edit["new_string"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Edit #{}: missing 'new_string'", i + 1))?;

            // Read file if we haven't already (or use the already-modified version)
            if !file_contents.contains_key(path) {
                match tokio::fs::read_to_string(path).await {
                    Ok(content) => {
                        file_contents.insert(path.to_string(), content);
                    }
                    Err(e) => {
                        errors.push(format!("Edit #{}: cannot read '{}': {}", i + 1, path, e));
                        continue;
                    }
                }
            }

            let content = file_contents.get(path).unwrap();
            let match_count = content.matches(old).count();

            if match_count == 0 {
                errors.push(format!(
                    "Edit #{}: no match for old_string in {} (first 50 chars: \"{}\")",
                    i + 1,
                    path,
                    &old[..old.len().min(50)]
                ));
            } else if match_count > 1 {
                errors.push(format!(
                    "Edit #{}: {} matches for old_string in {} — must be unique",
                    i + 1,
                    match_count,
                    path
                ));
            } else {
                // Apply the edit to our in-memory copy so subsequent edits on the same
                // file see the already-modified version
                let new_content = content.replacen(old, new, 1);
                file_contents.insert(path.to_string(), new_content);
                operations.push((path.to_string(), old.to_string(), new.to_string()));
            }
        }

        if !errors.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Validation failed — no changes applied:\n{}",
                    errors.join("\n")
                ),
                is_mutation: false,
            });
        }

        // ── Phase 2: Write all files ────────────────────────────────
        let mut results: Vec<String> = Vec::new();
        let mut files_written: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (path, _old, _new) in &operations {
            if files_written.contains(path) {
                continue; // Already written (multiple edits to same file)
            }
            let content = file_contents.get(path).unwrap();
            tokio::fs::write(path, content)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", path, e))?;
            files_written.insert(path.clone());
        }

        // Build summary
        for (path, old, _new) in &operations {
            let snippet = if old.len() > 40 {
                format!("\"{}...\"", &old[..40])
            } else {
                format!("\"{}\"", old)
            };
            results.push(format!("  ✓ {} — replaced {}", path, snippet));
        }

        let unique_files = files_written.len();
        let total_edits = operations.len();

        Ok(ToolResult {
            success: true,
            output: format!(
                "Successfully applied {} edit{} across {} file{}:\n{}",
                total_edits,
                if total_edits == 1 { "" } else { "s" },
                unique_files,
                if unique_files == 1 { "" } else { "s" },
                results.join("\n")
            ),
            is_mutation: true,
        })
    }
}
