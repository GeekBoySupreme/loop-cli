//! MCP (Model Context Protocol) client integration.
//!
//! Connects to MCP servers via stdio transport, downloads their tool
//! definitions, caches them locally at `~/.loop/mcp/`, and wraps them
//! as Loop `Tool` trait implementations for the engine.

use crate::tools::{Tool, ToolResult};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

// ── MCP JSON-RPC types ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

// ── MCP protocol types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Configuration for a single MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for this server
    pub name: String,
    /// Command to start the server (e.g. "npx" or "python")
    pub command: String,
    /// Arguments (e.g. ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"])
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A live connection to an MCP server
pub struct McpConnection {
    #[allow(dead_code)]
    config: McpServerConfig,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: AtomicU64,
}

/// Cached tool definitions from a server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCache {
    pub server_name: String,
    pub tools: Vec<McpToolDef>,
    pub cached_at: String,
}

// ── MCP Connection implementation ───────────────────────────────────

impl McpConnection {
    /// Spawn the MCP server process and perform the handshake
    pub async fn connect(config: &McpServerConfig) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to start MCP server '{}' ({}): {}",
                config.name,
                config.command,
                e
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdout not available"))?;

        let mut conn = Self {
            config: config.clone(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            request_id: AtomicU64::new(1),
        };

        // Send initialize request
        let init_result = conn
            .send_request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "loop-cli",
                        "version": "0.1.0"
                    }
                })),
            )
            .await?;

        tracing::debug!("MCP initialize response: {:?}", init_result);

        // Send initialized notification (no id, no response expected)
        conn.send_notification("notifications/initialized", None)
            .await?;

        Ok(conn)
    }

    /// List all available tools from the server
    pub async fn list_tools(&mut self) -> anyhow::Result<Vec<McpToolDef>> {
        let result = self.send_request("tools/list", None).await?;

        let tools: Vec<McpToolDef> = if let Some(tools_val) = result.get("tools") {
            serde_json::from_value(tools_val.clone())?
        } else {
            Vec::new()
        };

        Ok(tools)
    }

    /// Call a tool on the server
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<String> {
        let result = self
            .send_request(
                "tools/call",
                Some(serde_json::json!({
                    "name": name,
                    "arguments": arguments
                })),
            )
            .await?;

        if result.get("isError").and_then(|value| value.as_bool()) == Some(true) {
            let message = extract_content_text(&result)
                .unwrap_or_else(|| "MCP tool reported an error".to_string());
            return Err(anyhow::anyhow!(message));
        }

        if let Some(text) = extract_content_text(&result) {
            return Ok(text);
        }

        Ok(serde_json::to_string_pretty(&result)?)
    }

    /// Send a JSON-RPC request and wait for the response
    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let mut request_bytes = serde_json::to_vec(&request)?;
        request_bytes.push(b'\n');

        self.stdin.write_all(&request_bytes).await?;
        self.stdin.flush().await?;

        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                self.stdout.read_line(&mut line),
            )
            .await
            .map_err(|_| anyhow::anyhow!("MCP server response timeout (30s)"))?
            .map_err(|e| anyhow::anyhow!("Failed to read from MCP server: {}", e))?;

            if bytes_read == 0 {
                return Err(anyhow::anyhow!("MCP server closed connection"));
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Try to parse as JSON-RPC response
            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                if let Some(resp_id) = resp.id {
                    if resp_id == id {
                        if let Some(err) = resp.error {
                            return Err(anyhow::anyhow!("MCP error: {}", err.message));
                        }
                        return Ok(resp.result.unwrap_or(serde_json::Value::Null));
                    }
                }
                // Notification or different id — skip
            }
        }
    }

    /// Send a notification (no response expected)
    async fn send_notification(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Notification {
            jsonrpc: &'static str,
            method: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<serde_json::Value>,
        }

        let notif = Notification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
        };

        let mut bytes = serde_json::to_vec(&notif)?;
        bytes.push(b'\n');

        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;

        Ok(())
    }
}

fn extract_content_text(result: &serde_json::Value) -> Option<String> {
    let content = result.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter_map(|item| {
            (item.get("type").and_then(|value| value.as_str()) == Some("text"))
                .then(|| item.get("text").and_then(|value| value.as_str()))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        // Best-effort kill
        let _ = self.child.start_kill();
    }
}

// ── Tool cache ──────────────────────────────────────────────────────

/// Cache tool definitions to `~/.loop/mcp/<server_name>.json`
pub fn cache_tools(mcp_dir: &Path, server_name: &str, tools: &[McpToolDef]) -> anyhow::Result<()> {
    std::fs::create_dir_all(mcp_dir)?;
    let cache = McpToolCache {
        server_name: server_name.to_string(),
        tools: tools.to_vec(),
        cached_at: chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    };
    let path = mcp_dir.join(format!("{}.json", safe_name(server_name)));
    let content = serde_json::to_string_pretty(&cache)?;
    std::fs::write(&path, content)?;
    tracing::info!(
        "Cached {} MCP tools from '{}' to {}",
        tools.len(),
        server_name,
        path.display()
    );
    Ok(())
}

/// Load cached tool definitions
pub fn load_cached_tools(mcp_dir: &Path) -> Vec<McpToolCache> {
    let mut caches = Vec::new();
    if !mcp_dir.exists() {
        return caches;
    }
    if let Ok(entries) = std::fs::read_dir(mcp_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "json")
                .unwrap_or(false)
            {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(cache) = serde_json::from_str::<McpToolCache>(&content) {
                        caches.push(cache);
                    }
                }
            }
        }
    }
    caches
}

// ── MCP Tool adapter ────────────────────────────────────────────────

/// Wraps an MCP tool definition + server name as a Loop `Tool`
pub struct McpTool {
    name: String,
    server_name: String,
    tool_def: McpToolDef,
}

impl McpTool {
    pub fn new(server_name: &str, tool_def: McpToolDef) -> Self {
        Self {
            name: tool_name(server_name, &tool_def.name),
            server_name: server_name.to_string(),
            tool_def,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.tool_def.input_schema.clone()
    }

    fn is_mutating(&self) -> bool {
        // MCP tools are assumed mutating by default (safer)
        true
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<ToolResult> {
        let config = crate::config::LoopConfig::load()?;
        let server_config = config
            .mcp_servers
            .iter()
            .find(|s| s.name == self.server_name)
            .ok_or_else(|| {
                anyhow::anyhow!("MCP server '{}' not found in config", self.server_name)
            })?;

        let mut conn = McpConnection::connect(server_config).await?;
        let result = conn.call_tool(&self.tool_def.name, params).await;

        match result {
            Ok(output) => Ok(ToolResult {
                success: true,
                output,
                is_mutation: true,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("MCP tool call failed: {}", e),
                is_mutation: false,
            }),
        }
    }
}

/// Register cached tools belonging to currently configured MCP servers.
pub fn register_mcp_tools(
    registry: &mut crate::tools::ToolRegistry,
    servers: &[McpServerConfig],
    mcp_dir: &Path,
) {
    let caches = load_cached_tools(mcp_dir);
    for cache in caches {
        if !servers
            .iter()
            .any(|server| server.name == cache.server_name)
        {
            continue;
        }
        for tool_def in cache.tools {
            let name = tool_name(&cache.server_name, &tool_def.name);
            registry.register(Box::new(McpTool::new(&cache.server_name, tool_def)));
            tracing::debug!("Registered MCP tool: {}", name);
        }
    }
}

/// Stable provider-safe name exposed to the LLM and tool registry.
pub fn tool_name(server_name: &str, remote_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        safe_name(server_name),
        safe_name(remote_name)
    )
}

fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "unnamed".to_string()
    } else {
        value
    }
}

/// Connect to all configured MCP servers, refresh tool definitions, and cache
pub async fn refresh_all_servers(
    servers: &[McpServerConfig],
    mcp_dir: &Path,
) -> Vec<(String, Vec<McpToolDef>)> {
    let mut results = Vec::new();

    for server in servers {
        println!(
            "  {}  {}",
            crate::cli::charm::badge("CONNECT", crate::cli::charm::BLUE),
            crate::cli::charm::value(&server.name)
        );

        match McpConnection::connect(server).await {
            Ok(mut conn) => {
                match conn.list_tools().await {
                    Ok(tools) => {
                        println!(
                            "  {}",
                            crate::cli::charm::success(&format!(
                                "{} · {} tools available",
                                server.name,
                                tools.len()
                            ))
                        );

                        // Cache locally
                        if let Err(e) = cache_tools(mcp_dir, &server.name, &tools) {
                            tracing::warn!("Failed to cache tools for '{}': {}", server.name, e);
                        }

                        results.push((server.name.clone(), tools));
                    }
                    Err(e) => {
                        println!(
                            "  {}",
                            crate::cli::charm::error(&format!(
                                "{} · failed to list tools: {}",
                                server.name, e
                            ))
                        );
                    }
                }
            }
            Err(e) => {
                println!(
                    "  {}",
                    crate::cli::charm::error(&format!(
                        "{} · connection failed: {}",
                        server.name, e
                    ))
                );
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{read::ReadTool, ToolRegistry};
    use std::io::{BufRead, Write};

    #[test]
    fn fake_server() {
        if std::env::var_os("LOOP_MCP_TEST_SERVER").is_none() {
            return;
        }

        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let request: serde_json::Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(_) => continue,
            };
            let Some(id) = request.get("id") else {
                continue;
            };
            let result = match request.get("method").and_then(|value| value.as_str()) {
                Some("initialize") => serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "loop-test", "version": "1.0" }
                }),
                Some("tools/list") => serde_json::json!({
                    "tools": [{
                        "name": "echo",
                        "description": "Echo a message",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "message": { "type": "string" } }
                        }
                    }]
                }),
                Some("tools/call") => {
                    let message = request
                        .pointer("/params/arguments/message")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    serde_json::json!({
                        "content": [{ "type": "text", "text": message }]
                    })
                }
                _ => serde_json::Value::Null,
            };
            let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            writeln!(stdout, "{}", response).unwrap();
            stdout.flush().unwrap();
        }
    }

    #[tokio::test]
    async fn connects_lists_and_calls_stdio_tools() {
        let config = McpServerConfig {
            name: "test".to_string(),
            command: std::env::current_exe().unwrap().display().to_string(),
            args: vec![
                "--exact".to_string(),
                "mcp::tests::fake_server".to_string(),
                "--nocapture".to_string(),
            ],
            env: HashMap::from([("LOOP_MCP_TEST_SERVER".to_string(), "1".to_string())]),
        };

        let mut connection = McpConnection::connect(&config).await.unwrap();
        let tools = connection.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let output = connection
            .call_tool("echo", serde_json::json!({ "message": "hello from MCP" }))
            .await
            .unwrap();
        assert_eq!(output, "hello from MCP");
    }

    #[test]
    fn registered_mcp_tools_are_namespaced() {
        let cache_dir =
            std::env::temp_dir().join(format!("loop-mcp-test-{}", uuid::Uuid::new_v4()));
        let tools = vec![McpToolDef {
            name: "read".to_string(),
            description: "Remote read".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
        }];
        cache_tools(&cache_dir, "demo server", &tools).unwrap();

        let server = McpServerConfig {
            name: "demo server".to_string(),
            command: "unused".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
        };
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ReadTool));
        register_mcp_tools(&mut registry, &[server], &cache_dir);

        let names = registry.tool_names();
        assert!(names.iter().any(|name| name == "read"));
        assert!(names.iter().any(|name| name == "mcp__demo_server__read"));
        std::fs::remove_dir_all(cache_dir).unwrap();
    }
}
