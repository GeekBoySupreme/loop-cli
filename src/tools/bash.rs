//! Bash tool — execute shell commands.

use super::*;
use async_trait::async_trait;
use serde_json::json;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return stdout/stderr. Use for running tests, \
         installing dependencies, checking file structure, git operations, etc."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to execute" },
                "cwd": { "type": "string", "description": "Optional working directory" },
                "timeout_secs": { "type": "integer", "description": "Optional timeout in seconds (default: 30)" }
            },
            "required": ["command"]
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let command = params["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'command'"))?;
        let cwd = params["cwd"].as_str();
        let timeout = params["timeout_secs"].as_u64().unwrap_or(30);

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd.env("PAGER", "cat");

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(timeout), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                // Truncate very long outputs to prevent context overflow
                let max_len = 10_000;
                let stdout_trunc = if stdout.len() > max_len {
                    format!(
                        "{}...\n[truncated, {} total chars]",
                        &stdout[..max_len],
                        stdout.len()
                    )
                } else {
                    stdout.to_string()
                };
                let stderr_trunc = if stderr.len() > max_len {
                    format!(
                        "{}...\n[truncated, {} total chars]",
                        &stderr[..max_len],
                        stderr.len()
                    )
                } else {
                    stderr.to_string()
                };

                let mut output_parts = Vec::new();
                output_parts.push(format!("Exit code: {}", exit_code));
                if !stdout_trunc.is_empty() {
                    output_parts.push(format!("stdout:\n{}", stdout_trunc));
                }
                if !stderr_trunc.is_empty() {
                    output_parts.push(format!("stderr:\n{}", stderr_trunc));
                }

                Ok(ToolResult {
                    success: exit_code == 0,
                    output: output_parts.join("\n\n"),
                    is_mutation: true,
                })
            }
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: format!("Failed to execute command: {}", e),
                is_mutation: false,
            }),
            Err(_) => Ok(ToolResult {
                success: false,
                output: format!("Command timed out after {} seconds", timeout),
                is_mutation: false,
            }),
        }
    }
}
