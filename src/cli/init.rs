//! `loop init` — Interactive setup wizard.
//!
//! Walks the user through configuring model providers, API keys,
//! instruction files, and plugin discovery. Saves to `~/.loop/config.toml`.

use crate::cli::animation;
use crate::config::{self, *};
use crate::mcp::{self, McpServerConfig};
use console::style;
use dialoguer::{Confirm, Input, MultiSelect, Password, Select};
use std::path::PathBuf;

/// Run the full init wizard
pub async fn run_init_wizard() -> anyhow::Result<()> {
    animation::print_init_banner();

    println!(
        "  {} Setting up your Loop agent harness.\n",
        style("⚙").color256(69)
    );

    // ── Step 1: Select providers ────────────────────────────────────
    let provider_choices = vec![
        "Anthropic (Claude Sonnet / Haiku)",
        "OpenAI (GPT-4o)",
        "Google Gemini",
        "Groq (Llama)",
        "Ollama — Gemma (local)",
        "OpenRouter (bring any supported model slug)",
    ];

    let selected = MultiSelect::new()
        .with_prompt("Which model providers would you like to configure?")
        .items(&provider_choices)
        .interact()?;

    if selected.is_empty() {
        println!(
            "\n  {} You need at least one provider. Run `loop init` again.",
            style("✗").red()
        );
        return Ok(());
    }

    let mut model_config = ModelConfig::default();
    let mut all_models: Vec<(String, &str)> = Vec::new();

    // ── Step 2: Configure each selected provider ────────────────────
    for &idx in &selected {
        match idx {
            0 => {
                println!("\n  {} Anthropic Configuration", style("▸").magenta());
                let api_key = Password::new()
                    .with_prompt("  Anthropic API key")
                    .interact()?;
                let models = default_anthropic_models();
                for m in &models {
                    all_models.push((m.clone(), "Anthropic"));
                }
                model_config.anthropic = Some(AnthropicConfig { api_key, models });
                println!("  {} Anthropic configured ✓", style("✓").color256(111));
            }
            1 => {
                println!("\n  {} OpenAI Configuration", style("▸").magenta());
                let api_key = Password::new().with_prompt("  OpenAI API key").interact()?;
                let models = default_openai_models();
                for m in &models {
                    all_models.push((m.clone(), "OpenAI"));
                }
                model_config.openai = Some(OpenAIConfig { api_key, models });
                println!("  {} OpenAI configured ✓", style("✓").color256(111));
            }
            2 => {
                println!("\n  {} Google Gemini Configuration", style("▸").magenta());
                let api_key = Password::new().with_prompt("  Gemini API key").interact()?;
                let models = default_gemini_models();
                for m in &models {
                    all_models.push((m.clone(), "Gemini"));
                }
                model_config.gemini = Some(GeminiConfig { api_key, models });
                println!("  {} Gemini configured ✓", style("✓").color256(111));
            }
            3 => {
                println!("\n  {} Groq Configuration", style("▸").magenta());
                let api_key = Password::new().with_prompt("  Groq API key").interact()?;
                let models = default_groq_models();
                for m in &models {
                    all_models.push((m.clone(), "Groq"));
                }
                model_config.groq = Some(GroqConfig { api_key, models });
                println!("  {} Groq configured ✓", style("✓").color256(111));
            }
            4 => {
                println!(
                    "\n  {} Ollama (Local Gemma) Configuration",
                    style("▸").magenta()
                );

                // Check if ollama is installed
                let ollama_installed = which::which("ollama").is_ok();
                if !ollama_installed {
                    println!(
                        "  {} Ollama not found on PATH. Install it from https://ollama.com",
                        style("⚠").yellow()
                    );
                    println!("  After installing, run: ollama pull gemma3");
                } else {
                    println!("  {} Ollama found ✓", style("✓").color256(111));
                    // Prompt to pull the model
                    if Confirm::new()
                        .with_prompt("  Pull gemma3 model now? (requires ~3GB download)")
                        .default(true)
                        .interact()?
                    {
                        println!("  Pulling gemma3... (this may take a few minutes)");
                        let status = tokio::process::Command::new("ollama")
                            .args(["pull", "gemma3"])
                            .status()
                            .await;
                        match status {
                            Ok(s) if s.success() => {
                                println!(
                                    "  {} gemma3 pulled successfully ✓",
                                    style("✓").color256(111)
                                )
                            }
                            _ => println!(
                                "  {} Failed to pull. Run `ollama pull gemma3` manually.",
                                style("⚠").yellow()
                            ),
                        }
                    }
                }

                let endpoint: String = Input::new()
                    .with_prompt("  Ollama endpoint")
                    .default("http://localhost:11434".into())
                    .interact_text()?;

                let models = default_ollama_models();
                for m in &models {
                    all_models.push((m.clone(), "Ollama"));
                }
                model_config.ollama = Some(OllamaConfig { endpoint, models });
                println!("  {} Ollama configured ✓", style("✓").color256(111));
            }
            5 => {
                println!("\n  {} OpenRouter Configuration", style("▸").magenta());
                let api_key = loop {
                    let candidate = Password::new()
                        .with_prompt("  OpenRouter API key")
                        .interact()?;
                    animation::loading_dots("Validating OpenRouter key", 5);
                    match crate::provider::validate_openrouter_key(&candidate).await {
                        Ok(()) => {
                            println!("  {} OpenRouter key verified", style("✓").color256(111));
                            break candidate;
                        }
                        Err(error) => {
                            println!("  {} {}", style("✗").red(), error);
                            println!("    Enter a valid key or press Ctrl+C to cancel setup.");
                        }
                    }
                };
                let model: String = Input::new()
                    .with_prompt("  OpenRouter model slug")
                    .validate_with(|value: &String| -> Result<(), &str> {
                        if value.trim().contains('/') {
                            Ok(())
                        } else {
                            Err("Use a full OpenRouter slug such as anthropic/claude-sonnet-4")
                        }
                    })
                    .interact_text()?;
                let model = model.trim().to_string();
                all_models.push((model.clone(), "OpenRouter"));
                model_config.openrouter = Some(OpenRouterConfig { api_key, model });
                println!("  {} OpenRouter configured ✓", style("✓").color256(111));
            }
            _ => {}
        }
    }

    // ── Step 3: Select default model ────────────────────────────────
    println!();
    let model_labels: Vec<String> = all_models
        .iter()
        .map(|(m, p)| format!("{} ({})", m, p))
        .collect();

    let default_idx = Select::new()
        .with_prompt("Select your default model")
        .items(&model_labels)
        .default(0)
        .interact()?;

    let default_model = all_models[default_idx].0.clone();

    // ── Step 4: Instruction files ───────────────────────────────────
    println!();
    let mut instructions: Vec<PathBuf> = Vec::new();

    if Confirm::new()
        .with_prompt("Add instruction files (.md)?")
        .default(false)
        .interact()?
    {
        loop {
            let path: String = Input::new()
                .with_prompt("  Path to .md file (or 'done' to finish)")
                .interact_text()?;

            if path == "done" {
                break;
            }

            let path = PathBuf::from(shellexpand(&path));
            if path.exists() {
                instructions.push(path.clone());
                println!("  {} Added: {}", style("✓").color256(111), path.display());
            } else {
                println!("  {} File not found: {}", style("✗").red(), path.display());
            }
        }
    }

    // ── Step 5: Tool approval mode ──────────────────────────────────
    println!();
    let require_approval = Confirm::new()
        .with_prompt("Require approval before executing tools? (recommended)")
        .default(true)
        .interact()?;

    // ── Step 6: Git auto-commit ─────────────────────────────────────
    println!();
    println!(
        "  {} Git auto-checkpoint: when enabled, Loop will create a git commit",
        style("📝").color256(69)
    );
    println!("    with a semantic commit message every time you accept a file change.");
    println!(
        "    {}",
        style("⚠ This feature only works in directories where git is initialized.").yellow()
    );
    println!("    Run `git init` in your project folder first if you haven't already.\n");

    let git_auto_commit = Confirm::new()
        .with_prompt("Enable git auto-commit for accepted changes?")
        .default(false)
        .interact()?;

    // ── Step 7: MCP Servers ───────────────────────────────────────
    println!();
    println!(
        "  {} MCP (Model Context Protocol) lets you connect to external tool servers.",
        style("🔌").color256(69)
    );
    println!("    Examples: filesystem, database, GitHub, Slack, custom APIs");
    println!();

    let mut mcp_servers: Vec<McpServerConfig> = Vec::new();

    if Confirm::new()
        .with_prompt("Configure MCP servers?")
        .default(false)
        .interact()?
    {
        loop {
            println!();
            let name: String = Input::new()
                .with_prompt("  Server name (or 'done' to finish)")
                .interact_text()?;

            if name == "done" {
                break;
            }

            let command: String = Input::new()
                .with_prompt("  Command to start server")
                .interact_text()?;

            let args_str: String = Input::new()
                .with_prompt("  Arguments (space-separated, or empty)")
                .default(String::new())
                .interact_text()?;

            let args: Vec<String> = if args_str.trim().is_empty() {
                Vec::new()
            } else {
                args_str.split_whitespace().map(String::from).collect()
            };

            let server = McpServerConfig {
                name: name.clone(),
                command,
                args,
                env: std::collections::HashMap::new(),
            };

            // Try connecting and fetching tools
            animation::loading_dots(&format!("Connecting to {}", name), 5);
            match mcp::McpConnection::connect(&server).await {
                Ok(mut conn) => {
                    match conn.list_tools().await {
                        Ok(tools) => {
                            println!(
                                "  {} '{}' connected — {} tools found",
                                style("✓").color256(111),
                                name,
                                style(tools.len()).bold()
                            );
                            for t in &tools {
                                println!("    {} {}", style("•").dim(), style(&t.name).bold());
                            }
                            // Cache tools
                            let _ = mcp::cache_tools(&config::mcp_dir(), &name, &tools);
                        }
                        Err(e) => {
                            println!(
                                "  {} '{}' connected but couldn't list tools: {}",
                                style("⚠").yellow(),
                                name,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    println!(
                        "  {} Failed to connect to '{}': {}",
                        style("✗").red(),
                        name,
                        e
                    );
                    println!("    Server config will still be saved — you can retry later.");
                }
            }

            mcp_servers.push(server);
        }
    }

    // ── Step 8: Save config ─────────────────────────────────────────
    animation::warp_animation();

    let config = LoopConfig {
        default_model,
        models: model_config,
        instructions,
        plugins_dir: config::plugins_dir(),
        checkpoints_dir: config::checkpoints_dir(),
        max_iterations: 25,
        max_context_tokens: 128_000,
        require_tool_approval: require_approval,
        git_auto_commit,
        mcp_servers,
    };

    config.save()?;
    install_default_skills()?;

    animation::print_section_done("Configuration saved");

    println!(
        "  {} {}",
        style("✓").color256(111).bold(),
        style(config::config_path().display()).dim()
    );

    animation::celebration();

    println!(
        "\n  {} Run {} to start.\n",
        style("🔁").color256(69),
        style("loop").bold()
    );

    Ok(())
}

/// Expand `~` in paths
fn shellexpand(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// Install default skill files to `~/.loop/skills/`
fn install_default_skills() -> anyhow::Result<()> {
    let skills_dir = config::skills_dir();
    std::fs::create_dir_all(&skills_dir)?;

    // General skill (fallback)
    std::fs::write(
        skills_dir.join("general.md"),
        r#"# skill: general
## triggers: *
## tools: read, write, edit, bash, list_dir

You are a helpful coding assistant. You have access to the user's filesystem
and can execute commands. Follow these principles:

1. **Understand first**: Read relevant files before making changes.
2. **Be precise**: Make surgical edits rather than rewriting entire files.
3. **Verify**: After making changes, verify they work by running tests or checks.
4. **Communicate**: Explain what you're doing and why.

When working on a task:
- Start by understanding the codebase structure
- Read the most relevant files
- Plan your approach
- Execute changes incrementally
- Verify each change
"#,
    )?;

    // Debug skill
    std::fs::write(
        skills_dir.join("debug.md"),
        r#"# skill: debug
## triggers: debug, fix, error, bug, crash, failing, broken
## tools: read, edit, bash

You are debugging a codebase issue. Follow this procedure:

1. **Read the error** carefully — understand the exact failure.
2. **Identify files** — determine which files are involved.
3. **Form a hypothesis** — what could cause this behavior?
4. **Verify** — read the code to confirm or reject your hypothesis.
5. **Fix** — implement the minimal fix.
6. **Test** — run tests or the failing command to verify the fix.
7. **Explain** — tell the user what was wrong and what you changed.
"#,
    )?;

    // Code review skill
    std::fs::write(
        skills_dir.join("review.md"),
        r#"# skill: review
## triggers: review, audit, check, inspect, analyze
## tools: read, bash, list_dir

You are performing a code review. Focus on:

1. **Correctness** — does the code do what it claims?
2. **Security** — are there any vulnerabilities?
3. **Performance** — are there obvious inefficiencies?
4. **Style** — does it follow project conventions?
5. **Tests** — is the code adequately tested?

Read all relevant files, then provide a structured review.
"#,
    )?;

    Ok(())
}

// ── Default model lists (kept in sync with config/types.rs) ─────────

fn default_anthropic_models() -> Vec<String> {
    vec![
        "claude-sonnet-4-20250514".into(),
        "claude-haiku-3-5-20241022".into(),
    ]
}

fn default_openai_models() -> Vec<String> {
    vec!["gpt-4o".into(), "gpt-4o-mini".into()]
}

fn default_gemini_models() -> Vec<String> {
    vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()]
}

fn default_groq_models() -> Vec<String> {
    vec!["llama-3.3-70b-versatile".into()]
}

fn default_ollama_models() -> Vec<String> {
    vec!["gemma3".into()]
}

fn print_banner() {
    // Kept as fallback; the animated banner is used by default
    let banner = r#"
    ╭──────────────────────────────────────╮
    │                                      │
    │      🔁  L O O P                     │
    │      ─────────────────               │
    │      Minimalist Agent Harness        │
    │                                      │
    ╰──────────────────────────────────────╯
"#;
    println!("{}", style(banner).color256(69));
}
