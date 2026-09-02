//! A rich ratatui dashboard for agent status and memory inspection.
use crate::engine::OuterLoop;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Wrap},
    Terminal,
};
use std::io;

/// Run the interactive TUI dashboard. Takes over the terminal until 'q' or 'Esc' is pressed.
pub fn run_dashboard(engine: &OuterLoop) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, engine);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    engine: &OuterLoop,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui(f, engine))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                    return Ok(());
                }
            }
        }
    }
}

fn ui(f: &mut ratatui::Frame, engine: &OuterLoop) {
    let size = f.area();
    let pink = Color::Rgb(255, 95, 162);
    let blue = Color::Rgb(84, 147, 255);
    let blue_soft = Color::Rgb(138, 182, 255);
    let yellow = Color::Rgb(249, 226, 175);
    let muted = Color::Rgb(127, 132, 156);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " LOOP / SESSION DASHBOARD ",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(pink));

    // Create the overall layout
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3), // Header/Status
                Constraint::Min(10),   // Main content
                Constraint::Length(3), // Footer (Tokens/Memory)
            ]
            .as_ref(),
        )
        .split(block.inner(size));

    f.render_widget(block, size);

    // ── Header: Model & Session ─────────────────────────────────────
    let (model, session) = engine.model_and_session();
    let header_text = vec![Line::from(vec![
        Span::styled(
            " MODEL ",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            model.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "   SESSION ",
            Style::default().fg(pink).add_modifier(Modifier::BOLD),
        ),
        Span::styled(session, Style::default().fg(blue)),
    ])];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(muted)),
    );
    f.render_widget(header, main_chunks[0]);

    // ── Main Content: Split into two columns ────────────────────────
    let middle_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(main_chunks[1]);

    // Left Column: Files
    let mut files_text = vec![];
    let (reads, writes) = engine.file_stats();
    files_text.push(Line::from(Span::styled(
        "Modified Files:",
        Style::default().fg(yellow).add_modifier(Modifier::BOLD),
    )));
    for w in &writes {
        files_text.push(Line::from(format!("  ⚡ {}", w)));
    }
    if writes.is_empty() {
        files_text.push(Line::from(Span::styled(
            "  (None)",
            Style::default().fg(muted),
        )));
    }
    files_text.push(Line::from(""));
    files_text.push(Line::from(Span::styled(
        "Read Files:",
        Style::default().fg(blue).add_modifier(Modifier::BOLD),
    )));
    for r in &reads {
        files_text.push(Line::from(format!("  🔍 {}", r)));
    }
    if reads.is_empty() {
        files_text.push(Line::from(Span::styled(
            "  (None)",
            Style::default().fg(muted),
        )));
    }

    let files_para = Paragraph::new(files_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(blue))
                .title(" WORKSPACE "),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(files_para, middle_chunks[0]);

    // Right Column: Directives & Stats
    let mut stats_text = vec![];
    let directive_count = engine.directive_count();
    stats_text.push(Line::from(vec![
        Span::styled("Saved Directives: ", Style::default().fg(pink)),
        Span::styled(
            directive_count.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    stats_text.push(Line::from(""));

    let (messages, context_tokens) = engine.memory_stats();
    stats_text.push(Line::from(format!("Messages in Context: {}", messages)));
    stats_text.push(Line::from(format!(
        "Estimated Context:   ~{} tokens",
        context_tokens
    )));

    let is_git = engine.git_auto_commit_enabled();
    stats_text.push(Line::from(vec![
        Span::raw("Git Auto-Commit:   "),
        if is_git {
            Span::styled(
                "ACTIVE",
                Style::default().fg(blue_soft).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("INACTIVE", Style::default().fg(muted))
        },
    ]));

    let stats_para = Paragraph::new(stats_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pink))
            .title(" AGENT STATE "),
    );
    f.render_widget(stats_para, middle_chunks[1]);

    // ── Footer: Token Usage ─────────────────────────────────────────
    let (tin, tout) = engine.token_stats();
    let total = tin + tout;
    let ratio = if total > 0 {
        tin as f64 / total as f64
    } else {
        0.5
    };

    let footer_text = format!(
        " Tokens Used: {} in / {} out  [ Press 'q' or 'Esc' to close ]",
        tin, tout
    );

    let footer = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(yellow)),
        )
        .gauge_style(Style::default().fg(blue).bg(muted))
        .ratio(ratio.clamp(0.0, 1.0))
        .label(Span::styled(
            footer_text,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));

    f.render_widget(footer, main_chunks[2]);
}
