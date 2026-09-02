//! ListDir tool — list directory contents recursively.

use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the contents of a directory. Shows files and subdirectories with sizes. \
         Set max_depth to control recursion (default: 2)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the directory to list" },
                "max_depth": { "type": "integer", "description": "Maximum recursion depth (default: 2)" }
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
        let max_depth = params["max_depth"].as_u64().unwrap_or(2) as usize;

        let path = Path::new(path);
        if !path.exists() {
            return Ok(ToolResult {
                success: false,
                output: format!("Directory not found: {}", path.display()),
                is_mutation: false,
            });
        }

        let mut entries = Vec::new();
        list_recursive(path, "", 0, max_depth, &mut entries)?;

        let output = if entries.is_empty() {
            format!("{} (empty directory)", path.display())
        } else {
            format!("{}\n\n{}", path.display(), entries.join("\n"))
        };

        Ok(ToolResult {
            success: true,
            output,
            is_mutation: false,
        })
    }
}

fn list_recursive(
    dir: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    entries: &mut Vec<String>,
) -> anyhow::Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    let mut items: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());

    for entry in &items {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Skip hidden files and common noise
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }

        let metadata = entry.metadata()?;
        let indent = "  ".repeat(depth);

        if metadata.is_dir() {
            entries.push(format!("{}{}📁 {}/", indent, prefix, name));
            list_recursive(&entry.path(), prefix, depth + 1, max_depth, entries)?;
        } else {
            let size = format_size(metadata.len());
            entries.push(format!("{}{}   {} ({})", indent, prefix, name, size));
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    if bytes < 1024 * 1024 {
        return format!("{:.1} KB", bytes as f64 / 1024.0);
    }
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
