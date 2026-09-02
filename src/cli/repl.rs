//! Interactive REPL and one-shot execution mode.
//!
//! After `loop init`, the REPL is the primary interface. It displays
//! a status bar, accepts user input, and drives the outer loop engine.

use crate::checkpoint::CheckpointManager;
use crate::cli::charm;
use crate::config::{self, LoopConfig};
use crate::directives::embeddings::MemoryIndex;
use crate::directives::DirectiveStore;
use crate::engine::OuterLoop;
use crate::git;
use crate::memory::MemoryManager;
use crate::provider;
use crate::router::Router;
use crate::tools::ToolRegistry;

use console::style;
use dialoguer::Input;

/// Build the engine from config (shared between REPL and one-shot)
fn build_engine(config: &LoopConfig) -> anyhow::Result<OuterLoop> {
    let llm = provider::create_provider(config)?;
    let tools = ToolRegistry::default_tools(config);
    let checkpoint_mgr = CheckpointManager::new(&config.checkpoints_dir);
    let router = Router::load_skills()?;
    let memory = MemoryManager::new(config.max_context_tokens);

    // Load directive store and memory index from ~/.loop/
    let loop_home = config::loop_home();
    let directive_store = DirectiveStore::load(&loop_home);
    let memory_index = MemoryIndex::build(&loop_home);

    Ok(OuterLoop::new(
        llm,
        tools,
        memory,
        checkpoint_mgr,
        router,
        directive_store,
        memory_index,
        config.instructions.clone(),
        config.max_iterations,
        config.require_tool_approval,
        config.git_auto_commit,
    ))
}

/// Run in interactive REPL mode (default)
pub async fn run_interactive(config: &LoopConfig) -> anyhow::Result<()> {
    print_welcome(config);

    let mut engine = build_engine(config)?;

    // Check for existing checkpoint to resume
    if let Some(checkpoint) = engine.checkpoint_manager().latest_checkpoint()? {
        println!(
            "  {} Found checkpoint from {}",
            style("📌").yellow(),
            style(&checkpoint.timestamp.format("%Y-%m-%d %H:%M")).dim()
        );
        println!("  Context: {}", style(&checkpoint.current_context).italic());
        println!();

        let resume = dialoguer::Confirm::new()
            .with_prompt("  Resume from this checkpoint?")
            .default(true)
            .interact()?;

        if resume {
            engine.resume_from_checkpoint(&checkpoint)?;
            println!(
                "  {} Checkpoint loaded. Resuming...\n",
                style("✓").color256(111)
            );
        }
    }

    // ── Main REPL loop ──────────────────────────────────────────────
    loop {
        let input: String = Input::new()
            .with_prompt(charm::prompt())
            .allow_empty(false)
            .interact_text()?;

        let trimmed = input.trim();

        // Handle special commands
        match trimmed {
            "/quit" | "/exit" | "/q" => {
                println!(
                    "\n  {} Saving checkpoint and exiting...",
                    style("📌").yellow()
                );
                engine.force_checkpoint().await?;
                println!("  {} Goodbye!\n", style("👋").color256(69));
                break;
            }
            "/status" | "/dash" | "/dashboard" => {
                if let Err(e) = crate::cli::dashboard::run_dashboard(&engine) {
                    println!(
                        "  {} Failed to open dashboard: {}",
                        style("Error:").red(),
                        e
                    );
                } else {
                    println!("  {} Dashboard closed.", style("✓").color256(111));
                }
                continue;
            }
            "/clear" => {
                engine.clear_context();
                println!("  {} Context cleared.\n", style("✓").color256(111));
                continue;
            }
            "/model" => {
                println!("  Active model: {}", style(engine.active_model()).bold());
                continue;
            }
            "/tools" => {
                engine.print_tools();
                continue;
            }
            "/help" => {
                print_help();
                continue;
            }
            _ => {}
        }

        // Route and execute through the engine
        match engine.execute(trimmed).await {
            Ok(()) => {}
            Err(e) => {
                println!("\n  {}\n", charm::error(&e.to_string()));
            }
        }
    }

    Ok(())
}

/// Run a single prompt non-interactively
pub async fn run_one_shot(config: &LoopConfig, prompt: &str) -> anyhow::Result<()> {
    let mut engine = build_engine(config)?;
    engine.execute(prompt).await?;
    Ok(())
}

fn print_welcome(config: &LoopConfig) {
    let (model, provider_kind) = config
        .default_provider_info()
        .unwrap_or(("unknown", crate::config::ProviderKind::Anthropic));

    println!();
    if !crate::cli::charm::print_header() {
        crate::cli::animation::print_repl_header();
    }

    let builtin_count = 6;
    let mcp_caches = crate::mcp::load_cached_tools(&config::mcp_dir());
    let mcp_count: usize = mcp_caches.iter().map(|c| c.tools.len()).sum();
    let tools = if mcp_count > 0 {
        format!("{} built-in + {} MCP", builtin_count, mcp_count)
    } else {
        format!("{} built-in", builtin_count)
    };
    let approval = if config.require_tool_approval {
        charm::badge("ASK", charm::YELLOW)
    } else {
        charm::badge("AUTO", charm::RED)
    };
    let mut metadata = vec![
        format!(
            "{}  {}  {}",
            charm::key("MODEL"),
            charm::value(model),
            charm::muted(&format!("via {}", provider_kind))
        ),
        format!("{}  {}", charm::key("TOOLS"), charm::value(&tools)),
        format!("{}  {}", charm::key("APPROVAL"), approval),
    ];

    if !config.instructions.is_empty() {
        metadata.push(format!(
            "{}  {}",
            charm::key("CONTEXT"),
            charm::value(&format!("{} instruction files", config.instructions.len()))
        ));
    }
    println!("{}", charm::panel(&metadata.join("\n"), charm::BLUE));

    // Git status
    if config.git_auto_commit {
        let cwd = std::env::current_dir().unwrap_or_default();
        git::print_git_status(&cwd);
    }

    // Directives count
    let loop_home = config::loop_home();
    let directive_store = DirectiveStore::load(&loop_home);
    let count = directive_store.all().len();
    if count > 0 {
        println!(
            "  {}  {}",
            charm::badge("MEMORY", charm::PINK),
            charm::muted(&format!("{} saved directives ready", count))
        );
    }

    println!(
        "  {}  {}\n",
        charm::command("/help"),
        charm::muted("commands  ·  /status dashboard  ·  /quit exit")
    );
}

fn print_help() {
    charm::section("Quick controls", "stay in flow");
    let content = [
        ("/status", "open dashboard"),
        ("/tools", "show tools"),
        ("/model", "active model"),
        ("/clear", "reset context"),
        ("/quit", "checkpoint + exit"),
        ("-t · -t2 · -t3", "deeper self-review"),
    ]
    .iter()
    .map(|(command, description)| {
        format!(
            "{}{}  {}",
            charm::command(command),
            " ".repeat(20usize.saturating_sub(command.chars().count())),
            charm::muted(description)
        )
    })
    .collect::<Vec<_>>()
    .join("\n");
    println!("{}\n", charm::panel(&content, charm::PINK));
}
