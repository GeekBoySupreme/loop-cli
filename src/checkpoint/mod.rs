//! Tri-state checkpoint system: Done / Doing / Next.
//!
//! Checkpoints are persisted as human-readable `.md` files in
//! `~/.loop/checkpoints/` so that any follow-up run can seamlessly
//! resume without data loss or format confusion.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A checkpoint capturing the full tri-state context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    /// Unique session identifier
    pub session_id: String,
    /// When this checkpoint was created
    pub timestamp: DateTime<Utc>,
    /// The active skill/profile at suspension
    pub active_skill: String,
    /// The active model ID
    pub active_model: String,
    /// DONE: immutable ledger of completed actions
    pub completed_actions: Vec<String>,
    /// DOING: exact context at moment of suspension
    pub current_context: String,
    /// NEXT: prioritized pending operations
    pub pending_tasks: Vec<String>,
    /// Tracked file operations
    pub file_operations: FileOperations,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileOperations {
    pub read_files: HashSet<PathBuf>,
    pub modified_files: HashSet<PathBuf>,
}

/// Manages checkpoint persistence as `.md` files
pub struct CheckpointManager {
    checkpoints_dir: PathBuf,
}

impl CheckpointManager {
    pub fn new(dir: &Path) -> Self {
        Self {
            checkpoints_dir: dir.to_path_buf(),
        }
    }

    /// Save a checkpoint as a structured `.md` file
    pub fn save(&self, checkpoint: &WorkflowCheckpoint) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.checkpoints_dir)?;

        let md_content = checkpoint.to_markdown();
        let filename = format!("checkpoint_{}.md", checkpoint.session_id);
        let md_path = self.checkpoints_dir.join(&filename);
        std::fs::write(&md_path, &md_content)?;

        // Also write a companion `.json` for machine-reliable parsing
        let json_path = self
            .checkpoints_dir
            .join(format!("checkpoint_{}.json", checkpoint.session_id));
        let json_content = serde_json::to_string_pretty(checkpoint)?;
        std::fs::write(&json_path, json_content)?;

        tracing::info!("Checkpoint saved to {}", md_path.display());
        Ok(())
    }

    /// Load the most recent checkpoint (reads JSON companion, falls back to MD parsing)
    pub fn latest_checkpoint(&self) -> anyhow::Result<Option<WorkflowCheckpoint>> {
        if !self.checkpoints_dir.exists() {
            return Ok(None);
        }

        // Look for .json files first (machine-reliable)
        let mut json_entries: Vec<_> = std::fs::read_dir(&self.checkpoints_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        if json_entries.is_empty() {
            // Fall back to .md files
            return self.latest_from_md();
        }

        // Sort by modification time, newest first
        json_entries.sort_by(|a, b| {
            b.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .cmp(
                    &a.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                )
        });

        let content = std::fs::read_to_string(json_entries[0].path())?;
        let checkpoint: WorkflowCheckpoint = serde_json::from_str(&content)?;
        Ok(Some(checkpoint))
    }

    /// Parse the latest .md checkpoint (fallback when no JSON exists)
    fn latest_from_md(&self) -> anyhow::Result<Option<WorkflowCheckpoint>> {
        let mut md_entries: Vec<_> = std::fs::read_dir(&self.checkpoints_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .collect();

        if md_entries.is_empty() {
            return Ok(None);
        }

        md_entries.sort_by(|a, b| {
            b.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .cmp(
                    &a.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                )
        });

        let content = std::fs::read_to_string(md_entries[0].path())?;
        let checkpoint = WorkflowCheckpoint::from_markdown(&content)?;
        Ok(Some(checkpoint))
    }

    /// Delete a checkpoint after successful resumption
    pub fn clear_checkpoint(&self, session_id: &str) -> anyhow::Result<()> {
        for ext in &["md", "json"] {
            let path = self
                .checkpoints_dir
                .join(format!("checkpoint_{}.{}", session_id, ext));
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// Generate a tri-state prompt for the LLM to produce a checkpoint
    pub fn checkpoint_extraction_prompt() -> &'static str {
        r#"You must now generate a structured checkpoint of your current progress. 
Output ONLY valid JSON in this exact format, with no other text:

{
    "completed_actions": ["list of completed actions as strings"],
    "current_context": "description of what you were working on at the moment of suspension",
    "pending_tasks": ["ordered list of remaining tasks"]
}

Be specific and actionable. Include file paths, function names, and exact details."#
    }
}

impl WorkflowCheckpoint {
    /// Render checkpoint as a structured markdown file
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!(
            "# Loop Checkpoint — {}\n\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!("- **Session**: `{}`\n", self.session_id));
        md.push_str(&format!("- **Model**: `{}`\n", self.active_model));
        md.push_str(&format!("- **Skill**: `{}`\n\n", self.active_skill));

        md.push_str("---\n\n");

        // DONE
        md.push_str("## ✅ DONE (completed — do NOT repeat)\n\n");
        if self.completed_actions.is_empty() {
            md.push_str("_No actions completed yet._\n\n");
        } else {
            for action in &self.completed_actions {
                md.push_str(&format!("- [x] {}\n", action));
            }
            md.push('\n');
        }

        // DOING
        md.push_str("## 🔄 DOING (context at suspension)\n\n");
        md.push_str(&self.current_context);
        md.push_str("\n\n");

        // NEXT
        md.push_str("## 📋 NEXT (pending tasks — execute in order)\n\n");
        if self.pending_tasks.is_empty() {
            md.push_str("_No pending tasks._\n\n");
        } else {
            for (i, task) in self.pending_tasks.iter().enumerate() {
                md.push_str(&format!("{}. {}\n", i + 1, task));
            }
            md.push('\n');
        }

        // File operations
        md.push_str("---\n\n");
        md.push_str("## 📁 File Operations\n\n");

        md.push_str("### Files Read\n\n");
        if self.file_operations.read_files.is_empty() {
            md.push_str("_None._\n\n");
        } else {
            for f in &self.file_operations.read_files {
                md.push_str(&format!("- `{}`\n", f.display()));
            }
            md.push('\n');
        }

        md.push_str("### Files Modified\n\n");
        if self.file_operations.modified_files.is_empty() {
            md.push_str("_None._\n\n");
        } else {
            for f in &self.file_operations.modified_files {
                md.push_str(&format!("- `{}`\n", f.display()));
            }
            md.push('\n');
        }

        md
    }

    /// Parse a checkpoint from its markdown representation
    pub fn from_markdown(content: &str) -> anyhow::Result<Self> {
        let mut session_id = String::new();
        let mut active_model = String::new();
        let mut active_skill = String::new();
        let mut timestamp = Utc::now();
        let mut completed_actions = Vec::new();
        let mut current_context = String::new();
        let mut pending_tasks = Vec::new();
        let mut read_files = HashSet::new();
        let mut modified_files = HashSet::new();

        #[derive(PartialEq)]
        enum Section {
            None,
            Done,
            Doing,
            Next,
            FilesRead,
            FilesModified,
        }
        let mut section = Section::None;

        for line in content.lines() {
            let trimmed = line.trim();

            // Parse metadata
            if trimmed.starts_with("- **Session**:") {
                session_id = extract_backtick_value(trimmed);
            } else if trimmed.starts_with("- **Model**:") {
                active_model = extract_backtick_value(trimmed);
            } else if trimmed.starts_with("- **Skill**:") {
                active_skill = extract_backtick_value(trimmed);
            }
            // Parse section headers
            else if trimmed.contains("DONE") && trimmed.starts_with("##") {
                section = Section::Done;
            } else if trimmed.contains("DOING") && trimmed.starts_with("##") {
                section = Section::Doing;
            } else if trimmed.contains("NEXT") && trimmed.starts_with("##") {
                section = Section::Next;
            } else if trimmed.starts_with("### Files Read") {
                section = Section::FilesRead;
            } else if trimmed.starts_with("### Files Modified") {
                section = Section::FilesModified;
            } else if trimmed.starts_with("## ") || trimmed == "---" {
                if section == Section::Doing {
                    // End of DOING section
                    section = Section::None;
                }
            }
            // Parse content within sections
            else if !trimmed.is_empty() && !trimmed.starts_with('_') {
                match section {
                    Section::Done => {
                        if let Some(action) = trimmed.strip_prefix("- [x] ") {
                            completed_actions.push(action.to_string());
                        }
                    }
                    Section::Doing => {
                        if !current_context.is_empty() {
                            current_context.push('\n');
                        }
                        current_context.push_str(trimmed);
                    }
                    Section::Next => {
                        // Strip leading "1. ", "2. " etc
                        if let Some(pos) = trimmed.find(". ") {
                            pending_tasks.push(trimmed[pos + 2..].to_string());
                        }
                    }
                    Section::FilesRead => {
                        if let Some(path) = extract_backtick_list_item(trimmed) {
                            read_files.insert(PathBuf::from(path));
                        }
                    }
                    Section::FilesModified => {
                        if let Some(path) = extract_backtick_list_item(trimmed) {
                            modified_files.insert(PathBuf::from(path));
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(WorkflowCheckpoint {
            session_id,
            timestamp,
            active_skill,
            active_model,
            completed_actions,
            current_context,
            pending_tasks,
            file_operations: FileOperations {
                read_files,
                modified_files,
            },
        })
    }

    /// Convert checkpoint into a system prompt injection for resumption
    pub fn to_resume_prompt(&self) -> String {
        // Reuse the markdown format — it's already structured for LLM consumption
        let mut prompt = String::from("## Resuming from checkpoint\n\n");
        prompt.push_str(&self.to_markdown());
        prompt
    }
}

fn extract_backtick_value(line: &str) -> String {
    if let Some(start) = line.find('`') {
        if let Some(end) = line[start + 1..].find('`') {
            return line[start + 1..start + 1 + end].to_string();
        }
    }
    String::new()
}

fn extract_backtick_list_item(line: &str) -> Option<String> {
    let trimmed = line.strip_prefix("- ")?;
    let inner = trimmed.trim_start_matches('`').trim_end_matches('`');
    Some(inner.to_string())
}
