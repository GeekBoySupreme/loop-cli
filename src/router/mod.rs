//! Intent router and skill system.
//!
//! Routes user requests to the appropriate skill profile based on
//! keyword matching. Skills are loaded from `~/.loop/skills/*.md`.

use crate::config;
use std::collections::HashMap;

/// A skill profile loaded from a .md file
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub triggers: Vec<String>,
    pub tools: Vec<String>,
    pub prompt: String,
}

/// Routing decision from the classifier
pub struct RoutingDecision {
    pub skill: Skill,
    pub confidence: f32,
}

/// Keyword-based intent router
pub struct Router {
    skills: HashMap<String, Skill>,
    default_skill: Skill,
}

impl Router {
    /// Load skills from `~/.loop/skills/` directory
    pub fn load_skills() -> anyhow::Result<Self> {
        let skills_dir = config::skills_dir();
        let mut skills = HashMap::new();
        let mut default_skill = None;

        if skills_dir.exists() {
            for entry in std::fs::read_dir(&skills_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(skill) = parse_skill_file(&path) {
                        if skill.name == "general" {
                            default_skill = Some(skill.clone());
                        }
                        skills.insert(skill.name.clone(), skill);
                    }
                }
            }
        }

        let default_skill = default_skill.unwrap_or_else(|| Skill {
            name: "general".into(),
            triggers: vec!["*".into()],
            tools: vec![
                "read".into(),
                "write".into(),
                "edit".into(),
                "bash".into(),
                "list_dir".into(),
            ],
            prompt: "You are a helpful coding assistant with access to the filesystem and shell."
                .into(),
        });

        Ok(Self {
            skills,
            default_skill,
        })
    }

    /// Route a user message to the best matching skill
    pub fn route(&self, input: &str) -> RoutingDecision {
        let input_lower = input.to_lowercase();
        let mut best_match: Option<(&Skill, f32)> = None;

        for skill in self.skills.values() {
            if skill.name == "general" {
                continue;
            }
            let score = skill
                .triggers
                .iter()
                .filter(|t| input_lower.contains(&t.to_lowercase()))
                .count() as f32;

            if score > 0.0 {
                if best_match.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                    best_match = Some((skill, score));
                }
            }
        }

        match best_match {
            Some((skill, confidence)) => RoutingDecision {
                skill: skill.clone(),
                confidence,
            },
            None => RoutingDecision {
                skill: self.default_skill.clone(),
                confidence: 1.0,
            },
        }
    }
}

/// Parse a skill .md file with header metadata
fn parse_skill_file(path: &std::path::Path) -> anyhow::Result<Skill> {
    let content = std::fs::read_to_string(path)?;
    let mut name = String::new();
    let mut triggers = Vec::new();
    let mut tools = Vec::new();
    let mut prompt_lines = Vec::new();
    let mut in_header = true;

    for line in content.lines() {
        if in_header {
            if let Some(n) = line.strip_prefix("# skill:") {
                name = n.trim().to_string();
                continue;
            }
            if let Some(t) = line.strip_prefix("## triggers:") {
                triggers = t.split(',').map(|s| s.trim().to_string()).collect();
                continue;
            }
            if let Some(t) = line.strip_prefix("## tools:") {
                tools = t.split(',').map(|s| s.trim().to_string()).collect();
                continue;
            }
            if line.is_empty() && !name.is_empty() {
                in_header = false;
                continue;
            }
        }
        prompt_lines.push(line);
    }

    if name.is_empty() {
        name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }

    Ok(Skill {
        name,
        triggers,
        tools,
        prompt: prompt_lines.join("\n").trim().to_string(),
    })
}
