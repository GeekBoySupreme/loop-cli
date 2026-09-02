//! ASCII art, animations, and thinking spinners for Loop CLI.
//!
//! Provides funky visual feedback: animated init banner, thinking
//! spinner with real-time token counter, and manual page art.

use console::style;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

// ── ASCII art frames ────────────────────────────────────────────────

/// The big Loop init banner — displayed with typewriter reveal
pub fn print_init_banner() {
    let frames = [r#"
             ╭─────────────────────────────────────────╮
             │                                         │
             │    ██╗      ██████╗  ██████╗ ██████╗    │
             │    ██║     ██╔═══██╗██╔═══██╗██╔══██╗   │
             │    ██║     ██║   ██║██║   ██║██████╔╝   │
             │    ██║     ██║   ██║██║   ██║██╔═══╝    │
             │    ███████╗╚██████╔╝╚██████╔╝██║        │
             │    ╚══════╝ ╚═════╝  ╚═════╝ ╚═╝        │
             │                                         │
             │    ─── minimalist agent harness ───     │
             │                                         │
             ╰─────────────────────────────────────────╯
"#];

    // Typewriter effect: print char by char with small delays
    let frame = frames[0];
    let mut stdout = std::io::stdout();
    for ch in frame.chars() {
        print!("{}", style(ch).color256(69));
        let _ = stdout.flush();
        if ch == '\n' {
            std::thread::sleep(std::time::Duration::from_millis(20));
        } else if ch != ' ' {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    println!();
}

/// Small ASCII art for REPL startup
pub fn print_repl_header() {
    let art = r#"
   ╭──────────────────────────────────╮
   │   _     ___   ___  ____          │
   │  | |   / _ \ / _ \|  _ \         │
   │  | |  | | | | | | | |_) |        │
   │  | |__| |_| | |_| |  __/         │
   │  |____|\___/ \___/|_|      v0.1  │
   ╰──────────────────────────────────╯"#;

    let mut stdout = std::io::stdout();
    for line in art.lines() {
        if line.trim().is_empty() {
            continue;
        }
        println!("{}", style(line).color256(69));
        let _ = stdout.flush();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// Manual page header with wave animation
pub fn print_manual_header() {
    let art = r#"
    ╭──────────────────────────────────────────────────────╮
    │  ┌─┐                                                │
    │  │ │  ╔═╗╔═╗╔═╗  ╔╦╗╔═╗╔╗╔╦ ╦╔═╗╦              │
    │  │ │  ║ ║║ ║╠═╝  ║║║╠═╣║║║║ ║╠═╣║              │
    │  │ │  ╚═╝╚═╝╩    ╩ ╩╩ ╩╝╚╝╚═╝╩ ╩╩═╝            │
    │  └─┘                                                │
    ╰──────────────────────────────────────────────────────╯"#;

    // Print line by line with stagger
    for (i, line) in art.lines().enumerate() {
        if i == 0 && line.trim().is_empty() {
            continue;
        }
        println!("{}", style(line).color256(69));
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    println!();
}

/// Small "section complete" animation tick
pub fn print_section_done(section: &str) {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut stdout = std::io::stdout();

    for frame in &frames {
        print!("\r  {} {}...", style(frame).color256(69), section);
        let _ = stdout.flush();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("\r  {} {} ✓   ", style("✓").color256(111).bold(), section);
}

// ── Thinking spinner ────────────────────────────────────────────────

/// A thinking spinner that shows animated frames + live token count
pub struct ThinkingSpinner {
    running: Arc<AtomicBool>,
    input_tokens: Arc<AtomicUsize>,
    output_tokens: Arc<AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// The spinner frame sequences — cycles through these for a funky look
const SPINNER_FRAMES: &[&[&str]] = &[
    // DNA helix
    &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    // Orbit
    &["◐", "◓", "◑", "◒"],
    // Bounce
    &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
    // Wave
    &[
        "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂",
    ],
];

/// Flavor text that cycles during long thinking
const FLAVOR_TEXT: &[&str] = &[
    "thinking",
    "reasoning",
    "pondering",
    "computing",
    "analyzing",
    "processing",
    "deliberating",
    "contemplating",
    "evaluating",
    "synthesizing",
];

impl ThinkingSpinner {
    /// Start the spinner in a background thread
    pub fn start() -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let input_tokens = Arc::new(AtomicUsize::new(0));
        let output_tokens = Arc::new(AtomicUsize::new(0));

        let r = running.clone();
        let it = input_tokens.clone();
        let ot = output_tokens.clone();

        let handle = std::thread::spawn(move || {
            let mut frame_idx = 0usize;
            let mut set_idx = 0usize;
            let mut flavor_idx = 0usize;
            let mut ticks = 0u64;
            let mut stdout = std::io::stdout();

            while r.load(Ordering::Relaxed) {
                let set = SPINNER_FRAMES[set_idx % SPINNER_FRAMES.len()];
                let frame = set[frame_idx % set.len()];
                let flavor = FLAVOR_TEXT[flavor_idx % FLAVOR_TEXT.len()];
                let in_t = it.load(Ordering::Relaxed);
                let out_t = ot.load(Ordering::Relaxed);

                // Build the display line
                let token_info = if in_t > 0 || out_t > 0 {
                    format!(" │ {}↓ {}↑", format_tokens(in_t), format_tokens(out_t))
                } else {
                    String::new()
                };

                // Elapsed time
                let elapsed_secs = ticks as f64 * 0.08;
                let elapsed = format!("{:.1}s", elapsed_secs);

                print!(
                    "\r  {} {} {} {}{}   ",
                    style(frame).color256(69).bold(),
                    style(flavor).dim().italic(),
                    style("·").dim(),
                    style(&elapsed).dim(),
                    style(&token_info).yellow().dim(),
                );
                let _ = stdout.flush();

                std::thread::sleep(std::time::Duration::from_millis(80));
                frame_idx += 1;
                ticks += 1;

                // Switch spinner set every ~2 seconds
                if ticks % 25 == 0 {
                    set_idx += 1;
                }
                // Switch flavor text every ~3 seconds
                if ticks % 38 == 0 {
                    flavor_idx += 1;
                }
            }

            // Clear the spinner line
            print!("\r{}\r", " ".repeat(70));
            let _ = stdout.flush();
        });

        Self {
            running,
            input_tokens,
            output_tokens,
            handle: Some(handle),
        }
    }

    /// Update the token counts (called from the engine)
    pub fn update_tokens(&self, input: usize, output: usize) {
        self.input_tokens.store(input, Ordering::Relaxed);
        self.output_tokens.store(output, Ordering::Relaxed);
    }

    /// Stop the spinner
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ThinkingSpinner {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Don't join in drop — just signal stop
    }
}

/// Format token count compactly
fn format_tokens(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}

// ── Loading animation for MCP/startup ───────────────────────────────

/// Quick progress dots for short operations
pub fn loading_dots(label: &str, count: usize) {
    let mut stdout = std::io::stdout();
    for i in 0..count {
        print!(
            "\r  {} {}{}",
            style("·").color256(69),
            label,
            ".".repeat(i % 4)
        );
        let _ = stdout.flush();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    print!("\r{}\r", " ".repeat(60));
    let _ = stdout.flush();
}

/// Warp-drive animation for fun startup transitions
pub fn warp_animation() {
    let frames = [
        "  ·  ·  ·  ·  ·  ·  ·  ·",
        "  ·  · ·  · · ·  · ·  · ",
        "  · · · · · · · · · · · ·",
        "  ·-· ·-· ·-· ·-· ·-· ·-",
        "  ·=·=·=·=·=·=·=·=·=·=·=",
        "  ━━━━━━━━━━━━━━━━━━━━━━━",
        "  ═══════════════════════",
        "  ━━━━━━━━━━━━━━━━━━━━━━━",
        "  ·=·=·=·=·=·=·=·=·=·=·=",
        "  ·-· ·-· ·-· ·-· ·-· ·-",
        "  · · · · · · · · · · · ·",
        "  ·  · ·  · · ·  · ·  · ",
        "  ·  ·  ·  ·  ·  ·  ·  ·",
    ];

    let mut stdout = std::io::stdout();
    for frame in &frames {
        print!("\r{}", style(frame).color256(69).dim());
        let _ = stdout.flush();
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    print!("\r{}\r", " ".repeat(40));
    let _ = stdout.flush();
}

/// Completion celebration
pub fn celebration() {
    let msg = "  ✨ All set! ✨";
    let mut stdout = std::io::stdout();
    for (i, ch) in msg.chars().enumerate() {
        if ch == '✨' {
            print!("{}", style(ch).yellow());
        } else {
            let colors = [
                style(ch).color256(69),
                style(ch).color256(111),
                style(ch).magenta(),
                style(ch).yellow(),
            ];
            print!("{}", colors[i % colors.len()]);
        }
        let _ = stdout.flush();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    println!();
}

// ── Parallel progress display ───────────────────────────────────────

/// Distinct spinner sets for parallel tasks so they're visually distinguishable
const PARALLEL_SPINNERS: &[&[&str]] = &[
    &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    &["◐", "◓", "◑", "◒"],
    &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
    &[
        "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂",
    ],
    &["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"],
];

/// Colors for each parallel task row
const PARALLEL_COLORS: &[fn(&str) -> console::StyledObject<&str>] = &[
    |s| style(s).color256(69),
    |s| style(s).color256(111),
    |s| style(s).magenta(),
    |s| style(s).yellow(),
    |s| style(s).blue(),
];

/// Track state for each parallel task row
struct ParallelTaskState {
    label: String,
    status: String,
    tokens_in: usize,
    tokens_out: usize,
}

/// Multi-line progress display for parallel task execution.
/// Each task gets its own updating row with a different spinner.
#[derive(Clone)]
pub struct ParallelProgress {
    inner: Arc<ParallelProgressInner>,
}

struct ParallelProgressInner {
    running: AtomicBool,
    states: std::sync::Mutex<Vec<ParallelTaskState>>,
    handle: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ParallelProgress {
    /// Start the parallel progress display
    pub fn start(labels: Vec<String>) -> Self {
        let n = labels.len();
        let states: Vec<ParallelTaskState> = labels
            .into_iter()
            .map(|label| ParallelTaskState {
                label,
                status: "pending".into(),
                tokens_in: 0,
                tokens_out: 0,
            })
            .collect();

        let inner = Arc::new(ParallelProgressInner {
            running: AtomicBool::new(true),
            states: std::sync::Mutex::new(states),
            handle: std::sync::Mutex::new(None),
        });

        // Print initial empty lines
        let mut stdout = std::io::stdout();
        for _ in 0..n {
            println!();
        }
        let _ = stdout.flush();

        let inner_clone = inner.clone();
        let handle = std::thread::spawn(move || {
            let mut frame_idx = 0usize;
            let mut ticks = 0u64;
            let mut stdout = std::io::stdout();

            while inner_clone.running.load(Ordering::Relaxed) {
                let states = inner_clone.states.lock().unwrap();
                let count = states.len();

                // Move cursor up by N lines
                print!("\x1b[{}A", count);

                for (i, state) in states.iter().enumerate() {
                    let spinner_set = PARALLEL_SPINNERS[i % PARALLEL_SPINNERS.len()];
                    let frame = spinner_set[frame_idx % spinner_set.len()];
                    let color_fn = PARALLEL_COLORS[i % PARALLEL_COLORS.len()];

                    let elapsed_secs = ticks as f64 * 0.1;

                    let token_info = if state.tokens_in > 0 || state.tokens_out > 0 {
                        format!(
                            " │ {}↓ {}↑",
                            format_tokens(state.tokens_in),
                            format_tokens(state.tokens_out),
                        )
                    } else {
                        String::new()
                    };

                    let status_display = if state.status == "done ✓" {
                        format!("{}", style("done ✓").color256(111).bold())
                    } else {
                        format!("{}", style(&state.status).dim().italic())
                    };

                    // Pad to full width to clear any previous longer text
                    let line = format!(
                        "  {} [{}] {} · {} · {:.1}s{}",
                        color_fn(frame),
                        i + 1,
                        style(&state.label).bold(),
                        status_display,
                        elapsed_secs,
                        style(&token_info).yellow().dim(),
                    );

                    // Print line + clear to end of line
                    print!("\r{}\x1b[K\n", line);
                }

                drop(states);
                let _ = stdout.flush();

                std::thread::sleep(std::time::Duration::from_millis(100));
                frame_idx += 1;
                ticks += 1;
            }

            // Final render
            let states = inner_clone.states.lock().unwrap();
            let count = states.len();
            print!("\x1b[{}A", count);
            for (i, state) in states.iter().enumerate() {
                let status_icon = if state.status == "done ✓" {
                    style("✓").color256(111).bold().to_string()
                } else {
                    style("·").dim().to_string()
                };

                let token_info = if state.tokens_in > 0 || state.tokens_out > 0 {
                    format!(
                        " ({}↓ {}↑)",
                        format_tokens(state.tokens_in),
                        format_tokens(state.tokens_out),
                    )
                } else {
                    String::new()
                };

                print!(
                    "\r  {} [{}] {}{}\x1b[K\n",
                    status_icon,
                    i + 1,
                    state.label,
                    style(&token_info).dim(),
                );
            }
            let _ = stdout.flush();
        });

        *inner.handle.lock().unwrap() = Some(handle);

        Self { inner }
    }

    /// Update the status text for a specific task
    pub fn update_status(&self, index: usize, status: &str) {
        if let Ok(mut states) = self.inner.states.lock() {
            if let Some(state) = states.get_mut(index) {
                state.status = status.to_string();
            }
        }
    }

    /// Update token counts for a specific task
    pub fn update_tokens(&self, index: usize, tokens_in: usize, tokens_out: usize) {
        if let Ok(mut states) = self.inner.states.lock() {
            if let Some(state) = states.get_mut(index) {
                state.tokens_in = tokens_in;
                state.tokens_out = tokens_out;
            }
        }
    }

    /// Stop the progress display
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::Relaxed);
        if let Ok(mut handle) = self.inner.handle.lock() {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}
