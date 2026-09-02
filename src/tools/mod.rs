//! Tool system: trait, registry, and default tools.
//!
//! Tools are the agent's interface to the physical world.
//! The registry manages built-in, plugin, and MCP tools uniformly.

pub mod bash;
pub mod edit;
pub mod list_dir;
pub mod multi_edit;
pub mod read;
pub mod write;

use crate::config::LoopConfig;
use crate::provider::ToolDefinition;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Result of executing a tool
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Whether the tool succeeded
    pub success: bool,
    /// The output content (shown to the LLM)
    pub output: String,
    /// Whether this tool modified the filesystem
    pub is_mutation: bool,
}

/// The unified trait all tools must implement
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name
    fn name(&self) -> &str;
    /// Human-readable description (used in LLM prompt)
    fn description(&self) -> &str;
    /// JSON Schema for input parameters
    fn input_schema(&self) -> serde_json::Value;
    /// Whether this tool modifies the environment (requires approval)
    fn is_mutating(&self) -> bool;
    /// Execute the tool with the given parameters
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult>;
}

/// Central registry managing all available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    plugins_dir: PathBuf,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::with_plugins_dir(crate::config::plugins_dir())
    }

    fn with_plugins_dir(plugins_dir: PathBuf) -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            plugins_dir,
        }
    }

    /// Create registry with the default Pi-equivalent tool set + MCP tools
    pub fn default_tools(config: &LoopConfig) -> Self {
        let mut registry = Self::with_plugins_dir(config.plugins_dir.clone());
        registry.register(Box::new(read::ReadTool));
        registry.register(Box::new(write::WriteTool));
        registry.register(Box::new(edit::EditTool));
        registry.register(Box::new(multi_edit::MultiEditTool));
        registry.register(Box::new(bash::BashTool));
        registry.register(Box::new(list_dir::ListDirTool));

        // Load cached MCP tools
        let mcp_dir = crate::config::mcp_dir();
        crate::mcp::register_mcp_tools(&mut registry, &config.mcp_servers, &mcp_dir);
        registry.refresh_plugins();

        registry
    }

    /// Register a new tool
    pub fn register(&self, tool: Box<dyn Tool>) {
        self.tools
            .write()
            .expect("tool registry lock poisoned")
            .insert(tool.name().to_string(), Arc::from(tool));
    }

    /// Discover newly installed plugins without rebuilding the engine.
    pub fn refresh_plugins(&self) -> usize {
        let plugins = crate::plugin::discover_cli_plugins(&self.plugins_dir);
        let count = plugins.len();
        for plugin in plugins {
            self.register(Box::new(plugin));
        }
        count
    }

    /// Get tool definitions for sending to the LLM
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tools.read().expect("tool registry lock poisoned");
        let mut definitions: Vec<_> = tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    /// Execute a tool by name
    pub async fn execute(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let tool = self
            .tools
            .read()
            .expect("tool registry lock poisoned")
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
        tool.execute(params).await
    }

    /// Check if a tool is mutating (requires user approval)
    pub fn is_mutating(&self, name: &str) -> bool {
        self.tools
            .read()
            .expect("tool registry lock poisoned")
            .get(name)
            .map(|t| t.is_mutating())
            .unwrap_or(true)
    }

    /// List all tool names
    pub fn tool_names(&self) -> Vec<String> {
        let tools = self.tools.read().expect("tool registry lock poisoned");
        let mut names: Vec<_> = tools.keys().cloned().collect();
        names.sort_unstable();
        names
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::ToolRegistry;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn discovers_plugin_created_after_registry_startup() {
        let plugins_dir =
            std::env::temp_dir().join(format!("loop-plugin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let registry = ToolRegistry::with_plugins_dir(plugins_dir.clone());
        assert!(!registry
            .tool_names()
            .iter()
            .any(|name| name == "fresh_ping"));

        let plugin_path = plugins_dir.join("loop-plugin-fresh");
        std::fs::write(
            &plugin_path,
            "#!/bin/sh\nif [ \"$1\" = \"--manifest\" ]; then\n  printf '%s\\n' '{\"name\":\"fresh\",\"version\":\"1.0\",\"tools\":[{\"name\":\"ping\",\"description\":\"Ping\",\"input_schema\":{\"type\":\"object\"}}]}'\nfi\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&plugin_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, permissions).unwrap();

        registry.refresh_plugins();
        assert!(registry
            .tool_names()
            .iter()
            .any(|name| name == "fresh_ping"));
        std::fs::remove_dir_all(plugins_dir).unwrap();
    }
}
