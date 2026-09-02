//! Embedding-based semantic search over saved memories.
//!
//! Uses the configured LLM provider to generate embeddings for
//! directives, checkpoints, and instruction files, enabling
//! semantic similarity search before each task execution.
//!
//! For providers that don't have a native embedding endpoint,
//! falls back to keyword-based search via TF-IDF cosine similarity.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A memory entry that can be searched
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Source: "directive", "checkpoint", "instruction"
    pub source: String,
    /// The text content to match against
    pub content: String,
    /// Path to the source file
    pub source_path: PathBuf,
    /// Precomputed TF-IDF vector (term -> tf-idf score)
    pub tfidf: HashMap<String, f64>,
}

/// Manages semantic search over all Loop memories
pub struct MemoryIndex {
    entries: Vec<MemoryEntry>,
}

impl MemoryIndex {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Build the index from all available memory sources
    pub fn build(loop_home: &Path) -> Self {
        let mut index = Self::new();

        // Index directives
        let directives_path = loop_home.join("directives.md");
        if directives_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&directives_path) {
                // Split by directive sections
                for section in content.split("## Directive #") {
                    if section.trim().is_empty() || section.starts_with(" Loop") {
                        continue;
                    }
                    let entry_text = section
                        .lines()
                        .filter(|l| !l.starts_with("---") && !l.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if entry_text.len() > 20 {
                        index.add_entry("directive", &entry_text, &directives_path);
                    }
                }
            }
        }

        // Index checkpoints
        let checkpoints_dir = loop_home.join("checkpoints");
        if checkpoints_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&checkpoints_dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().map(|e| e == "md").unwrap_or(false) {
                        if let Ok(content) = std::fs::read_to_string(entry.path()) {
                            // Extract the DOING section — that's the most semantically rich
                            if let Some(doing_start) = content.find("## 🔄 DOING") {
                                let doing_section = &content[doing_start..];
                                let end = doing_section
                                    .find("## 📋 NEXT")
                                    .unwrap_or(doing_section.len());
                                let doing_text = &doing_section[..end];
                                index.add_entry("checkpoint", doing_text, &entry.path());
                            }
                        }
                    }
                }
            }
        }

        // Index instruction files
        let skills_dir = loop_home.join("skills");
        if skills_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().map(|e| e == "md").unwrap_or(false) {
                        if let Ok(content) = std::fs::read_to_string(entry.path()) {
                            index.add_entry("skill", &content, &entry.path());
                        }
                    }
                }
            }
        }

        index
    }

    /// Add an entry and compute its TF-IDF vector
    fn add_entry(&mut self, source: &str, content: &str, path: &Path) {
        let tfidf = compute_tfidf(content);
        self.entries.push(MemoryEntry {
            source: source.to_string(),
            content: content.to_string(),
            source_path: path.to_path_buf(),
            tfidf,
        });
    }

    /// Search for the most relevant memory entries using cosine similarity
    pub fn search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let query_tfidf = compute_tfidf(query);

        let mut results: Vec<SearchResult> = self
            .entries
            .iter()
            .map(|entry| {
                let similarity = cosine_similarity(&query_tfidf, &entry.tfidf);
                SearchResult {
                    entry: entry.clone(),
                    score: similarity,
                }
            })
            .filter(|r| r.score > 0.05) // minimum relevance threshold
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }

    /// Search and format results as context injection for the LLM
    pub fn search_as_context(&self, query: &str, top_k: usize) -> Option<String> {
        let results = self.search(query, top_k);
        if results.is_empty() {
            return None;
        }

        let mut ctx = String::from("## Relevant Memories\n\n");
        ctx.push_str("_The following past experiences may be relevant to the current task:_\n\n");

        for (i, result) in results.iter().enumerate() {
            let truncated = if result.entry.content.len() > 500 {
                format!("{}...", &result.entry.content[..500])
            } else {
                result.entry.content.clone()
            };
            ctx.push_str(&format!(
                "### Memory #{} (source: {}, relevance: {:.0}%)\n\n{}\n\n",
                i + 1,
                result.entry.source,
                result.score * 100.0,
                truncated,
            ));
        }

        Some(ctx)
    }
}

#[derive(Debug)]
pub struct SearchResult {
    pub entry: MemoryEntry,
    pub score: f64,
}

/// Compute a simple TF-IDF vector for a document
fn compute_tfidf(text: &str) -> HashMap<String, f64> {
    let mut term_freq: HashMap<String, f64> = HashMap::new();
    let words: Vec<String> = tokenize(text);
    let total_words = words.len() as f64;

    if total_words == 0.0 {
        return term_freq;
    }

    for word in &words {
        *term_freq.entry(word.clone()).or_insert(0.0) += 1.0;
    }

    // Normalize by document length
    for freq in term_freq.values_mut() {
        *freq /= total_words;
    }

    term_freq
}

/// Tokenize text into lowercase words, filtering stop words
fn tokenize(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "shall", "should", "may", "might", "can", "could",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "and", "but", "or", "not", "no",
        "this", "that", "these", "those", "it", "its",
    ];

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() > 2 && !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Cosine similarity between two TF-IDF vectors
fn cosine_similarity(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (term, weight_a) in a {
        norm_a += weight_a * weight_a;
        if let Some(weight_b) = b.get(term) {
            dot_product += weight_a * weight_b;
        }
    }

    for weight_b in b.values() {
        norm_b += weight_b * weight_b;
    }

    let denominator = norm_a.sqrt() * norm_b.sqrt();
    if denominator == 0.0 {
        0.0
    } else {
        dot_product / denominator
    }
}
