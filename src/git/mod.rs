//! Git auto-checkpoint: commit accepted changes with semantic messages.
//!
//! When enabled, every time the user accepts a mutating tool execution,
//! Loop stages the changed files and creates a commit with an LLM-generated
//! semantic commit message. Only works in directories with git initialized.

use std::path::Path;

/// Check if git is initialized in the given directory (or any parent)
pub fn is_git_repo(dir: &Path) -> bool {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) => o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true",
        Err(_) => false,
    }
}

/// Get the git root directory
pub fn git_root(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Check if there are uncommitted changes
pub fn has_changes(dir: &Path) -> bool {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        Err(_) => false,
    }
}

/// Get a summary of staged/unstaged changes for commit message generation
pub fn diff_summary(dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["diff", "--stat"])
        .current_dir(dir)
        .output();

    let staged = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(dir)
        .output();

    let mut summary = String::new();

    if let Ok(o) = &output {
        let diff = String::from_utf8_lossy(&o.stdout);
        if !diff.trim().is_empty() {
            summary.push_str("Unstaged:\n");
            summary.push_str(&diff);
        }
    }

    if let Ok(o) = &staged {
        let diff = String::from_utf8_lossy(&o.stdout);
        if !diff.trim().is_empty() {
            summary.push_str("\nStaged:\n");
            summary.push_str(&diff);
        }
    }

    if summary.is_empty() {
        summary.push_str("(no changes detected)");
    }

    summary
}

/// Get a short diff of the actual content changes (for better commit messages)
pub fn diff_content_short(dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["diff", "--no-color", "-U2"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) => {
            let content = String::from_utf8_lossy(&o.stdout);
            // Truncate to avoid overwhelming the LLM
            if content.len() > 3000 {
                format!("{}...\n[diff truncated]", &content[..3000])
            } else {
                content.to_string()
            }
        }
        Err(_) => "(could not read diff)".to_string(),
    }
}

/// Stage all changes and commit with the given message
pub fn commit_changes(dir: &Path, message: &str) -> anyhow::Result<()> {
    // Stage all changes
    let add = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .output()?;

    if !add.status.success() {
        return Err(anyhow::anyhow!(
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }

    // Commit
    let commit = std::process::Command::new("git")
        .args(["commit", "-m", message, "--no-verify"])
        .current_dir(dir)
        .output()?;

    if !commit.status.success() {
        let stderr = String::from_utf8_lossy(&commit.stderr);
        // "nothing to commit" is not an error
        if stderr.contains("nothing to commit") {
            return Ok(());
        }
        return Err(anyhow::anyhow!("git commit failed: {}", stderr));
    }

    Ok(())
}

/// Stage specific files and commit
pub fn commit_files(dir: &Path, files: &[&str], message: &str) -> anyhow::Result<()> {
    for file in files {
        std::process::Command::new("git")
            .args(["add", file])
            .current_dir(dir)
            .output()?;
    }

    let commit = std::process::Command::new("git")
        .args(["commit", "-m", message, "--no-verify"])
        .current_dir(dir)
        .output()?;

    if !commit.status.success() {
        let stderr = String::from_utf8_lossy(&commit.stderr);
        if stderr.contains("nothing to commit") {
            return Ok(());
        }
        return Err(anyhow::anyhow!("git commit failed: {}", stderr));
    }

    Ok(())
}

/// Generate a prompt for the LLM to create a semantic commit message
pub fn commit_message_prompt(diff_summary: &str, diff_content: &str) -> String {
    format!(
        r#"Generate a single, concise git commit message for these changes.
Follow the Conventional Commits format: type(scope): description

Types: feat, fix, refactor, docs, style, test, chore, perf
Keep the message under 72 characters.
Output ONLY the commit message, nothing else.

Changes summary:
{}

Diff:
{}"#,
        diff_summary, diff_content
    )
}

/// Print git checkpoint status to the user
pub fn print_git_status(dir: &Path) {
    use console::style;

    if is_git_repo(dir) {
        let root = git_root(dir).unwrap_or_else(|| dir.to_string_lossy().to_string());
        println!(
            "  {} Git: enabled ({})",
            style("▸").color256(111),
            style(root).dim()
        );
    } else {
        println!(
            "  {} Git: {} — initialize with `git init` to enable auto-commits",
            style("▸").yellow(),
            style("not a git repo").dim()
        );
    }
}
