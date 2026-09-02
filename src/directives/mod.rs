//! Directives system: classify, store, and recall user-directed solutions.
//!
//! When a user tells the agent to do something specific (especially to fix
//! a bug), Loop captures the bug fingerprint, the directive, and the outcome
//! in `~/.loop/directives.md`. On future runs, the agent queries this file
//! (and all saved memories) for relevant prior knowledge before acting.

pub mod classifier;
pub mod embeddings;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single directive: a user-directed action with its outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directive {
    /// When this directive was recorded
    pub timestamp: String,
    /// Short fingerprint of the bug or situation
    pub fingerprint: String,
    /// What the user specifically told the agent to do
    pub user_directive: String,
    /// What the agent actually did
    pub action_taken: String,
    /// Whether it worked
    pub outcome: DirectiveOutcome,
    /// Files involved
    pub files_involved: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DirectiveOutcome {
    Worked,
    DidNotWork,
    Partial,
    Unknown,
}

impl std::fmt::Display for DirectiveOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectiveOutcome::Worked => write!(f, "✅ Worked"),
            DirectiveOutcome::DidNotWork => write!(f, "❌ Did not work"),
            DirectiveOutcome::Partial => write!(f, "⚠️ Partial"),
            DirectiveOutcome::Unknown => write!(f, "❓ Unknown"),
        }
    }
}

/// Manages the directives store at `~/.loop/directives.md`
pub struct DirectiveStore {
    directives_path: PathBuf,
    directives_json_path: PathBuf,
    directives: Vec<Directive>,
}

impl DirectiveStore {
    /// Load from `~/.loop/directives.md` and companion `.json`
    pub fn load(loop_home: &Path) -> Self {
        let directives_path = loop_home.join("directives.md");
        let directives_json_path = loop_home.join("directives.json");
        let mut directives = Vec::new();

        // Load from JSON if available (machine-reliable)
        if directives_json_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&directives_json_path) {
                if let Ok(parsed) = serde_json::from_str::<Vec<Directive>>(&content) {
                    directives = parsed;
                }
            }
        }

        Self {
            directives_path,
            directives_json_path,
            directives,
        }
    }

    /// Add a new directive and persist
    pub fn add(&mut self, directive: Directive) -> anyhow::Result<()> {
        self.directives.push(directive);
        self.persist()
    }

    /// Update the outcome of the most recent directive matching a fingerprint
    pub fn update_outcome(
        &mut self,
        fingerprint: &str,
        outcome: DirectiveOutcome,
    ) -> anyhow::Result<()> {
        for d in self.directives.iter_mut().rev() {
            if d.fingerprint == fingerprint {
                d.outcome = outcome;
                break;
            }
        }
        self.persist()
    }

    /// Get all directives
    pub fn all(&self) -> &[Directive] {
        &self.directives
    }

    /// Search directives by simple keyword matching (fast path)
    pub fn search_keywords(&self, query: &str) -> Vec<&Directive> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        self.directives
            .iter()
            .filter(|d| {
                let combined = format!(
                    "{} {} {} {}",
                    d.fingerprint.to_lowercase(),
                    d.user_directive.to_lowercase(),
                    d.action_taken.to_lowercase(),
                    d.tags.join(" ").to_lowercase(),
                );
                // At least 2 words must match, or any single word > 5 chars
                let matches: usize = query_words
                    .iter()
                    .filter(|w| combined.contains(**w))
                    .count();
                matches >= 2
                    || query_words
                        .iter()
                        .any(|w| w.len() > 5 && combined.contains(*w))
            })
            .collect()
    }

    /// Render all directives that worked into a context injection string
    pub fn relevant_context(&self, query: &str) -> Option<String> {
        let relevant = self.search_keywords(query);
        if relevant.is_empty() {
            return None;
        }

        let mut ctx = String::from("## Relevant Prior Directives\n\n");
        ctx.push_str(
            "The following are solutions that were previously attempted for similar problems:\n\n",
        );

        for d in &relevant {
            ctx.push_str(&format!("### {} — {}\n", d.fingerprint, d.outcome));
            ctx.push_str(&format!("- **User said**: {}\n", d.user_directive));
            ctx.push_str(&format!("- **Action taken**: {}\n", d.action_taken));
            if !d.files_involved.is_empty() {
                ctx.push_str(&format!("- **Files**: {}\n", d.files_involved.join(", ")));
            }
            ctx.push('\n');
        }

        if relevant
            .iter()
            .any(|d| d.outcome == DirectiveOutcome::DidNotWork)
        {
            ctx.push_str(
                "> ⚠️ Some of these approaches did NOT work previously. Avoid repeating them.\n\n",
            );
        }

        Some(ctx)
    }

    /// Persist to both `.md` and `.json`
    fn persist(&self) -> anyhow::Result<()> {
        // Write JSON
        let json = serde_json::to_string_pretty(&self.directives)?;
        std::fs::write(&self.directives_json_path, json)?;

        // Write Markdown
        let md = self.to_markdown();
        std::fs::write(&self.directives_path, md)?;

        Ok(())
    }

    /// Render all directives as a markdown document
    fn to_markdown(&self) -> String {
        let mut md = String::from("# Loop Directives\n\n");
        md.push_str(
            "_Auto-generated by Loop. Records user-directed solutions and their outcomes._\n\n",
        );
        md.push_str("---\n\n");

        if self.directives.is_empty() {
            md.push_str("_No directives recorded yet._\n");
            return md;
        }

        for (i, d) in self.directives.iter().enumerate() {
            md.push_str(&format!("## Directive #{} — {}\n\n", i + 1, d.timestamp));
            md.push_str(&format!("**Fingerprint**: `{}`\n\n", d.fingerprint));
            md.push_str(&format!("**User directive**: {}\n\n", d.user_directive));
            md.push_str(&format!("**Action taken**: {}\n\n", d.action_taken));
            md.push_str(&format!("**Outcome**: {}\n\n", d.outcome));

            if !d.files_involved.is_empty() {
                md.push_str("**Files involved**:\n");
                for f in &d.files_involved {
                    md.push_str(&format!("- `{}`\n", f));
                }
                md.push('\n');
            }

            if !d.tags.is_empty() {
                md.push_str(&format!(
                    "**Tags**: {}\n\n",
                    d.tags
                        .iter()
                        .map(|t| format!("`{}`", t))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            md.push_str("---\n\n");
        }

        md
    }
}

/// Create a directive from the LLM's structured response
pub fn create_directive_from_llm_response(
    fingerprint: &str,
    user_input: &str,
    action: &str,
    files: Vec<String>,
    tags: Vec<String>,
) -> Directive {
    Directive {
        timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        fingerprint: fingerprint.to_string(),
        user_directive: user_input.to_string(),
        action_taken: action.to_string(),
        outcome: DirectiveOutcome::Unknown,
        files_involved: files,
        tags,
    }
}
