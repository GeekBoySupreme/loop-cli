//! `loop manual` / `loop man` — comprehensive command reference.

use crate::cli::{animation, charm};

pub fn print_manual() {
    println!();
    if !charm::print_header() {
        animation::print_manual_header();
    }

    charm::section("Start here", "commands you can run from your shell");
    let commands = [
        ("loop", "Start the interactive agent"),
        ("loop init", "Set up providers and integrations"),
        ("loop run -p <prompt>", "Run one task and exit"),
        ("loop mcp list", "Show MCP servers and tools"),
        ("loop mcp refresh", "Refresh MCP definitions"),
        ("loop mcp call --help", "Call an MCP tool directly"),
        ("loop manual", "Open this command guide"),
    ];
    println!("{}", charm::panel(&rows(&commands, 32), charm::BLUE));

    charm::section("Inside Loop", "fast controls for the interactive session");
    let repl = [
        ("/status", "Open the live session dashboard"),
        ("/tools", "Browse built-in, plugin, and MCP tools"),
        ("/model", "Show the active model"),
        ("/clear", "Reset conversation context"),
        ("/quit", "Checkpoint and leave the session"),
        ("-t · -t2 · -t3", "Add one to three self-review passes"),
    ];
    println!("{}", charm::panel(&rows(&repl, 22), charm::PINK));

    charm::section(
        "Tool belt",
        "read-only tools are blue; mutations require approval",
    );
    let tools = [
        ("READ  read", "Read a file with an optional line range"),
        ("READ  list_dir", "Explore directory contents recursively"),
        ("WRITE write", "Create or replace a file"),
        ("WRITE edit", "Apply an exact surgical replacement"),
        ("WRITE multi_edit", "Apply an atomic batch of edits"),
        ("WRITE bash", "Run a shell command with a timeout"),
    ];
    println!("{}", charm::panel(&rows(&tools, 22), charm::YELLOW));

    charm::section("Built in", "the useful machinery behind every task");
    let features = [
        ("CHECKPOINTS", "Done / Doing / Next session recovery"),
        ("DIRECTIVES", "Remembered corrections and proven approaches"),
        ("THINKING", "Directive-aware refinement with -t through -t3"),
        ("MCP", "Namespaced tools from external MCP servers"),
        ("PLUGINS", "Hot-loaded loop-plugin-* executables"),
        ("GIT", "Optional semantic commits for changes"),
    ];
    println!("{}", charm::panel(&rows(&features, 20), charm::BLUE_SOFT));

    charm::section(
        "Local data",
        "credentials stay in your operating system vault",
    );
    let paths = [
        ("~/.loop/config.toml", "Provider metadata and settings"),
        ("~/.loop/checkpoints/", "Resumable session checkpoints"),
        ("~/.loop/directives.*", "Learned directives and outcomes"),
        ("~/.loop/mcp/", "Cached MCP tool definitions"),
        ("~/.loop/plugins/", "Locally installed plugins"),
    ];
    println!("{}", charm::panel(&rows(&paths, 30), charm::BLUE));

    println!(
        "\n  {}  {}\n",
        charm::badge("READY", charm::BLUE_SOFT),
        charm::muted("Run `loop init`, then type `loop` to begin.")
    );
}

fn rows(items: &[(&str, &str)], command_width: usize) -> String {
    items
        .iter()
        .map(|(command, description)| {
            format!(
                "{}{}  {}",
                charm::command(command),
                " ".repeat(command_width.saturating_sub(command.chars().count())),
                charm::muted(description)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
