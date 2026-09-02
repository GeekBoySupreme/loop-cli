//! Loop — A minimalist, Rust-native agent harness
//!
//! The CLI entry point dispatches between `loop init` (setup wizard)
//! and the default interactive REPL mode.

mod checkpoint;
mod cli;
mod config;
mod directives;
mod engine;
mod error;
mod git;
mod mcp;
mod memory;
mod plugin;
mod provider;
mod router;
mod tools;

use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "loop", version, about = "A minimalist agent harness", styles = cli_styles())]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Magenta.on_default().bold())
        .usage(AnsiColor::Blue.on_default().bold())
        .literal(AnsiColor::Blue.on_default().bold())
        .placeholder(AnsiColor::Yellow.on_default())
        .error(AnsiColor::Red.on_default().bold())
        .valid(AnsiColor::BrightBlue.on_default().bold())
        .invalid(AnsiColor::Red.on_default())
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Loop: configure models, API keys, and instruction files
    Init,
    /// Run the agent with a one-shot prompt (non-interactive)
    Run {
        /// The prompt to send to the agent
        #[arg(short, long)]
        prompt: String,
    },
    /// Show the full Loop manual: all commands, tools, features, and file locations
    Manual,
    /// Alias for `loop manual`
    Man,
    /// Inspect, refresh, and call Model Context Protocol servers
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured servers and cached tools
    List,
    /// Connect to every configured server and refresh its tools
    Refresh,
    /// Directly call a server tool without invoking a model
    Call {
        /// Configured MCP server name
        #[arg(long)]
        server: String,
        /// Tool name as advertised by the MCP server
        #[arg(long)]
        tool: String,
        /// Tool arguments as a JSON object
        #[arg(long, default_value = "{}")]
        arguments: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "loop_cli=info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init) => {
            cli::init::run_init_wizard().await?;
        }
        Some(Commands::Run { prompt }) => {
            let config = config::LoopConfig::load()?;
            cli::repl::run_one_shot(&config, &prompt).await?;
        }
        Some(Commands::Manual) | Some(Commands::Man) => {
            cli::manual::print_manual();
        }
        Some(Commands::Mcp { command }) => {
            let config = config::LoopConfig::load()?;
            match command {
                McpCommands::List => cli::mcp::list(&config),
                McpCommands::Refresh => cli::mcp::refresh(&config).await?,
                McpCommands::Call {
                    server,
                    tool,
                    arguments,
                } => {
                    cli::mcp::call(&config, &server, &tool, &arguments).await?;
                }
            }
        }
        None => {
            // Default: interactive REPL mode
            let config = match config::LoopConfig::load() {
                Ok(c) => c,
                Err(_) => {
                    eprintln!(
                        "{} No configuration found. Running setup wizard...\n",
                        console::style("⚡").yellow()
                    );
                    cli::init::run_init_wizard().await?;
                    config::LoopConfig::load()?
                }
            };
            cli::repl::run_interactive(&config).await?;
        }
    }

    Ok(())
}
