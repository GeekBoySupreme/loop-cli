//! Charm-inspired terminal theme, backed by Lip Gloss with optional Gum/Glow.

use lipgloss::{Border, Style};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

pub const PINK: &str = "#FF5FA2";
pub const BLUE: &str = "#5493FF";
pub const BLUE_SOFT: &str = "#8AB6FF";
pub const YELLOW: &str = "#F9E2AF";
pub const RED: &str = "#FF6B6B";
pub const TEXT: &str = "#CDD6F4";
pub const MUTED: &str = "#7F849C";
pub const SURFACE: &str = "#262638";

pub fn print_header() -> bool {
    if !std::io::stdout().is_terminal() {
        return false;
    }

    if which::which("gum").is_ok()
        && Command::new("gum")
            .args([
                "style",
                "--border",
                "rounded",
                "--border-foreground",
                "212",
                "--foreground",
                "#5493FF",
                "--padding",
                "1 4",
                "--bold",
                "LOOP  /  BUILD WITH MOMENTUM",
            ])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    {
        return true;
    }

    let wordmark = format!(
        "{}\n{}",
        Style::new().foreground(PINK).bold().render("LOOP"),
        Style::new()
            .foreground(MUTED)
            .render("agent harness  ·  build with momentum")
    );
    println!(
        "{}",
        Style::new()
            .border(Border::rounded())
            .border_foreground(PINK)
            .padding((1, 3))
            .width(content_width())
            .render(&wordmark)
    );
    true
}

pub fn section(label: &str, detail: &str) {
    let heading = Style::new()
        .foreground(SURFACE)
        .background(PINK)
        .padding((0, 1))
        .bold()
        .render(&label.to_uppercase());
    if detail.is_empty() {
        println!("\n  {}", heading);
    } else {
        println!(
            "\n  {}  {}",
            heading,
            Style::new().foreground(MUTED).render(detail)
        );
    }
}

pub fn badge(label: &str, color: &str) -> String {
    Style::new()
        .foreground(SURFACE)
        .background(color)
        .padding((0, 1))
        .bold()
        .render(label)
}

pub fn command(command: &str) -> String {
    Style::new().foreground(BLUE).bold().render(command)
}

pub fn key(label: &str) -> String {
    Style::new().foreground(PINK).bold().render(label)
}

pub fn value(value: &str) -> String {
    Style::new().foreground(TEXT).render(value)
}

pub fn muted(value: &str) -> String {
    Style::new().foreground(MUTED).render(value)
}

pub fn success(message: &str) -> String {
    format!("{}  {}", badge("DONE", BLUE_SOFT), value(message))
}

pub fn warning(message: &str) -> String {
    format!("{}  {}", badge("NOTE", YELLOW), value(message))
}

pub fn error(message: &str) -> String {
    format!("{}  {}", badge("ERROR", RED), value(message))
}

pub fn panel(content: &str, accent: &str) -> String {
    Style::new()
        .foreground(TEXT)
        .border(Border::rounded())
        .border_foreground(accent)
        .padding((0, 2))
        .width(content_width())
        .render(content)
}

pub fn prompt() -> String {
    format!("{} {}", badge("LOOP", PINK), command("›"))
}

fn content_width() -> u16 {
    crossterm::terminal::size()
        .map(|(width, _)| width.saturating_sub(8).clamp(52, 92))
        .unwrap_or(72)
}

pub fn render_markdown(markdown: &str) -> bool {
    if !std::io::stdout().is_terminal() || which::which("glow").is_err() {
        return false;
    }

    let width = crossterm::terminal::size()
        .map(|(width, _)| width.saturating_sub(4).max(40).to_string())
        .unwrap_or_else(|_| "80".to_string());
    let mut child = match Command::new("glow")
        .args(["--style", "dark", "--width", &width, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let wrote_input = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(markdown.as_bytes()).is_ok())
        .unwrap_or(false);
    wrote_input && child.wait().map(|status| status.success()).unwrap_or(false)
}
