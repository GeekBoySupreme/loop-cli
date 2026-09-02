//! Directive classifier: scans user input to detect when something
//! should be saved as a directive (bug fix, specific instruction, etc.)
//!
//! Uses a combination of keyword signals and structural patterns to
//! determine if the user is giving a specific, recordable directive.

/// Classification result for user input
#[derive(Debug)]
pub struct ClassificationResult {
    /// Whether this input should trigger directive recording
    pub is_directive: bool,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Detected category
    pub category: DirectiveCategory,
    /// Extracted tags
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveCategory {
    /// User is telling the agent to fix a specific bug in a specific way
    BugFix,
    /// User is giving a specific implementation instruction
    Implementation,
    /// User is correcting a previous attempt
    Correction,
    /// User is specifying a workaround
    Workaround,
    /// Not a directive — just a general request
    General,
}

impl std::fmt::Display for DirectiveCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectiveCategory::BugFix => write!(f, "bug-fix"),
            DirectiveCategory::Implementation => write!(f, "implementation"),
            DirectiveCategory::Correction => write!(f, "correction"),
            DirectiveCategory::Workaround => write!(f, "workaround"),
            DirectiveCategory::General => write!(f, "general"),
        }
    }
}

/// Signal keywords and their weights
const BUG_SIGNALS: &[(&str, f32)] = &[
    ("fix", 0.3),
    ("bug", 0.4),
    ("error", 0.3),
    ("crash", 0.4),
    ("broken", 0.3),
    ("failing", 0.3),
    ("doesn't work", 0.4),
    ("not working", 0.4),
    ("wrong", 0.2),
    ("issue", 0.2),
];

const DIRECTIVE_SIGNALS: &[(&str, f32)] = &[
    ("instead", 0.4),
    ("use this", 0.5),
    ("try this", 0.5),
    ("do this", 0.5),
    ("change it to", 0.5),
    ("replace", 0.3),
    ("don't", 0.3),
    ("should be", 0.3),
    ("must be", 0.4),
    ("make sure", 0.3),
    ("specifically", 0.4),
    ("exactly", 0.3),
    ("the correct way", 0.5),
    ("the right way", 0.5),
];

const CORRECTION_SIGNALS: &[(&str, f32)] = &[
    ("no,", 0.5),
    ("wrong", 0.3),
    ("that's not", 0.5),
    ("not what i", 0.5),
    ("i said", 0.4),
    ("i meant", 0.5),
    ("go back", 0.3),
    ("undo", 0.4),
    ("revert", 0.4),
    ("actually", 0.2),
    ("previous approach", 0.4),
];

const WORKAROUND_SIGNALS: &[(&str, f32)] = &[
    ("workaround", 0.6),
    ("hack", 0.3),
    ("temporary", 0.3),
    ("for now", 0.4),
    ("until", 0.2),
    ("bypass", 0.4),
    ("skip", 0.2),
];

/// Classify user input to determine if it's a recordable directive
pub fn classify(input: &str) -> ClassificationResult {
    let input_lower = input.to_lowercase();
    let mut tags = Vec::new();

    // Score each category
    let bug_score = score_signals(&input_lower, BUG_SIGNALS);
    let directive_score = score_signals(&input_lower, DIRECTIVE_SIGNALS);
    let correction_score = score_signals(&input_lower, CORRECTION_SIGNALS);
    let workaround_score = score_signals(&input_lower, WORKAROUND_SIGNALS);

    // Determine category and confidence
    let mut scores = vec![
        (DirectiveCategory::BugFix, bug_score + directive_score * 0.5),
        (DirectiveCategory::Correction, correction_score),
        (DirectiveCategory::Workaround, workaround_score),
        (DirectiveCategory::Implementation, directive_score),
    ];
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (category, raw_confidence) = scores.into_iter().next().unwrap();
    let confidence = raw_confidence.min(1.0);

    // Structural signals boost confidence:
    // - Input mentions specific file paths
    // - Input contains code snippets (backticks)
    // - Input has imperative verbs at the start
    let has_file_paths = input.contains('/')
        || input.contains(".rs")
        || input.contains(".ts")
        || input.contains(".py")
        || input.contains(".js")
        || input.contains(".go");
    let has_code = input.contains('`');
    let starts_imperative = input_lower.starts_with("fix")
        || input_lower.starts_with("change")
        || input_lower.starts_with("use")
        || input_lower.starts_with("replace")
        || input_lower.starts_with("add")
        || input_lower.starts_with("remove")
        || input_lower.starts_with("update");

    let structural_boost = if has_file_paths { 0.15 } else { 0.0 }
        + if has_code { 0.1 } else { 0.0 }
        + if starts_imperative { 0.1 } else { 0.0 };

    let final_confidence = (confidence + structural_boost).min(1.0);

    // Extract tags
    if has_file_paths {
        tags.push("file-specific".to_string());
    }
    if has_code {
        tags.push("code-specific".to_string());
    }
    tags.push(category.to_string());

    // Threshold: 0.4 confidence to be considered a directive
    let is_directive = final_confidence >= 0.4 && category != DirectiveCategory::General;

    ClassificationResult {
        is_directive,
        confidence: final_confidence,
        category: if is_directive {
            category
        } else {
            DirectiveCategory::General
        },
        tags,
    }
}

/// Generate a fingerprint extraction prompt for the LLM
pub fn fingerprint_extraction_prompt(user_input: &str, category: &DirectiveCategory) -> String {
    format!(
        r#"Analyze this user input and extract a structured directive record.
The user appears to be giving a {} directive.

User input: "{}"

Output ONLY valid JSON:
{{
    "fingerprint": "short unique identifier for this bug/issue (e.g., 'off-by-one-loop-counter', 'missing-null-check-auth')",
    "action_summary": "what the user wants done, in one sentence",
    "files_involved": ["list of file paths mentioned or implied"],
    "tags": ["relevant tags like 'rust', 'async', 'memory-leak', etc."]
}}"#,
        category, user_input
    )
}

fn score_signals(input: &str, signals: &[(&str, f32)]) -> f32 {
    signals
        .iter()
        .filter(|(keyword, _)| input.contains(keyword))
        .map(|(_, weight)| weight)
        .sum()
}
