//! Commands for inspecting, refreshing, and directly calling MCP servers.

use crate::cli::charm;
use crate::config::{self, LoopConfig};
use crate::mcp::{self, McpConnection};

pub fn list(config: &LoopConfig) {
    charm::section("MCP servers", "external tools connected over stdio");

    if config.mcp_servers.is_empty() {
        println!(
            "{}\n",
            charm::panel(
                &charm::warning("No servers configured. Run `loop init`."),
                charm::YELLOW
            )
        );
        return;
    }

    let caches = mcp::load_cached_tools(&config::mcp_dir());
    for server in &config.mcp_servers {
        let cache = caches.iter().find(|cache| cache.server_name == server.name);
        let status = cache
            .map(|cache| format!("{} cached tools", cache.tools.len()))
            .unwrap_or_else(|| "not discovered".to_string());
        let mut lines = vec![format!(
            "{}  {}",
            charm::badge(&server.name.to_uppercase(), charm::PINK),
            charm::muted(&status)
        )];
        if let Some(cache) = cache {
            for tool in &cache.tools {
                lines.push(format!(
                    "{}  {}",
                    charm::command(&mcp::tool_name(&server.name, &tool.name)),
                    charm::muted(&tool.description)
                ));
            }
        }
        println!("{}", charm::panel(&lines.join("\n"), charm::PINK));
    }
    println!();
}

pub async fn refresh(config: &LoopConfig) -> anyhow::Result<()> {
    if config.mcp_servers.is_empty() {
        anyhow::bail!("No MCP servers configured. Run `loop init` first.");
    }

    charm::section("MCP refresh", "reconnecting configured servers");
    let results = mcp::refresh_all_servers(&config.mcp_servers, &config::mcp_dir()).await;
    println!();

    if results.len() != config.mcp_servers.len() {
        anyhow::bail!(
            "Connected to {} of {} MCP servers",
            results.len(),
            config.mcp_servers.len()
        );
    }

    Ok(())
}

pub async fn call(
    config: &LoopConfig,
    server_name: &str,
    tool_name: &str,
    arguments: &str,
) -> anyhow::Result<()> {
    let server = config
        .mcp_servers
        .iter()
        .find(|server| server.name == server_name)
        .ok_or_else(|| anyhow::anyhow!("MCP server '{}' is not configured", server_name))?;
    let arguments: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| anyhow::anyhow!("Invalid --arguments JSON: {}", error))?;
    if !arguments.is_object() {
        anyhow::bail!("--arguments must be a JSON object");
    }

    println!(
        "  {}  {}  {}",
        charm::badge("MCP", charm::PINK),
        charm::value(server_name),
        charm::command(tool_name)
    );
    let mut connection = McpConnection::connect(server).await?;
    let output = connection.call_tool(tool_name, arguments).await?;
    if !crate::cli::charm::render_markdown(&output) {
        println!("\n{}\n", output);
    }

    Ok(())
}
