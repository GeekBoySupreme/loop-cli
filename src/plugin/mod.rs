//! Plugin system for extending Loop with external tools.
//!
//! Supports two plugin types:
//! - CLI plugins: executables on $PATH matching `loop-plugin-*`
//! - Self-written plugins: compiled by the agent into ~/.loop/plugins/

use crate::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Plugin manifest returned by `loop-plugin-* --manifest`
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub tools: Vec<PluginToolDef>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool backed by a CLI plugin executable
pub struct CliPluginTool {
    pub executable: String,
    pub tool_name: String,
    pub tool_description: String,
    pub schema: serde_json::Value,
}

#[async_trait]
impl Tool for CliPluginTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn description(&self) -> &str {
        &self.tool_description
    }
    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }
    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let input = serde_json::to_string(&params)?;

        let output = std::process::Command::new(&self.executable)
            .args(["--execute", &self.tool_name])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(input.as_bytes()).ok();
                }
                child.wait_with_output()
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(ToolResult {
                success: true,
                output: stdout.to_string(),
                is_mutation: true,
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: format!("Plugin error: {}\n{}", stdout, stderr),
                is_mutation: false,
            })
        }
    }
}

/// Discover CLI plugins in the configured directory and on `PATH`.
pub fn discover_cli_plugins(plugins_dir: &Path) -> Vec<CliPluginTool> {
    let mut plugins = Vec::new();
    let mut directories = vec![plugins_dir.to_path_buf()];
    if let Some(path_var) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path_var));
    }
    directories.sort();
    directories.dedup();

    for directory in directories {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("loop-plugin-") {
                    if let Ok(output) = std::process::Command::new(entry.path())
                        .arg("--manifest")
                        .output()
                    {
                        if output.status.success() {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            if let Ok(manifest) = serde_json::from_str::<PluginManifest>(&stdout) {
                                for tool_def in manifest.tools {
                                    plugins.push(CliPluginTool {
                                        executable: entry.path().to_string_lossy().to_string(),
                                        tool_name: format!("{}_{}", manifest.name, tool_def.name),
                                        tool_description: tool_def.description,
                                        schema: tool_def.input_schema,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    plugins
}

/// Returns a Rust source template for self-written plugins.
/// The agent can write this to a file, compile it, and register the result.
pub fn plugin_template() -> String {
    let manifest_json = r#"{"name":"myplugin","version":"0.1.0","tools":[{"name":"my_tool","description":"Description","input_schema":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}}]}"#;
    let result_json = r#"{"success": true, "output": "Result here"}"#;

    format!(
        r#"//! Loop CLI Plugin Template
//! Compile: rustc plugin_name.rs -o loop-plugin-name

use std::io::{{self, Read}};

fn main() {{
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--manifest" {{
        println!("{}");
        return;
    }}
    if args.len() > 2 && args[1] == "--execute" {{
        let mut input = String::new();
        io::stdin().read_to_string(&mut input).unwrap();
        // Your tool logic here
        println!("{}");
        return;
    }}
    eprintln!("Usage: loop-plugin-name --manifest | --execute <tool_name>");
    std::process::exit(1);
}}
"#,
        manifest_json, result_json
    )
}
