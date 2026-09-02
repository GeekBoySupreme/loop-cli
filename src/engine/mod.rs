//! Core engine: the double-loop architecture.
//!
//! The outer loop handles orchestration, tool execution, memory,
//! checkpointing, directive recording, git auto-commits, and
//! parallel task decomposition. The inner loop drives LLM inference.

pub mod parallel;

use crate::checkpoint::{CheckpointManager, FileOperations, WorkflowCheckpoint};
use crate::cli::charm;
use crate::directives::classifier::{self, DirectiveCategory};
use crate::directives::embeddings::MemoryIndex;
use crate::directives::{self, Directive, DirectiveOutcome, DirectiveStore};
use crate::git;
use crate::memory::MemoryManager;
use crate::provider::{
    CompletionResponse, LlmProvider, Message, MessageContent, Role, StopReason, ToolCall,
};
use crate::router::{Router, RoutingDecision};
use crate::tools::ToolRegistry;

use console::style;
use std::path::PathBuf;
use std::sync::Arc;

/// The outer orchestration loop
pub struct OuterLoop {
    /// The LLM provider
    provider: Arc<dyn LlmProvider>,
    /// Available tools (Arc for sharing with parallel tasks)
    tools: Arc<ToolRegistry>,
    /// Conversation memory
    memory: MemoryManager,
    /// Checkpoint manager
    checkpoint_mgr: CheckpointManager,
    /// Intent router
    router: Router,
    /// Directive store
    directive_store: DirectiveStore,
    /// Semantic memory index
    memory_index: MemoryIndex,
    /// Paths to instruction .md files
    instruction_files: Vec<PathBuf>,
    /// Max inner loop iterations before forced checkpoint
    max_iterations: usize,
    /// Whether to require user approval for mutating tools
    require_approval: bool,
    /// Whether to auto-commit accepted changes via git
    git_auto_commit: bool,
    /// Working directory for git operations
    working_dir: PathBuf,
    /// Current session ID
    session_id: String,
    /// Running total of tokens used
    total_input_tokens: usize,
    total_output_tokens: usize,
    /// Track completed actions for checkpointing
    completed_actions: Vec<String>,
    /// Active directive being tracked (if classifier triggered)
    active_directive: Option<ActiveDirective>,
}

/// State for a directive being tracked through the current execution
struct ActiveDirective {
    fingerprint: String,
    user_input: String,
    category: DirectiveCategory,
    files: Vec<String>,
    tags: Vec<String>,
}

impl OuterLoop {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: ToolRegistry,
        memory: MemoryManager,
        checkpoint_mgr: CheckpointManager,
        router: Router,
        directive_store: DirectiveStore,
        memory_index: MemoryIndex,
        instruction_files: Vec<PathBuf>,
        max_iterations: usize,
        require_approval: bool,
        git_auto_commit: bool,
    ) -> Self {
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            provider,
            tools: Arc::new(tools),
            memory,
            checkpoint_mgr,
            router,
            directive_store,
            memory_index,
            instruction_files,
            max_iterations,
            require_approval,
            git_auto_commit,
            working_dir,
            session_id: uuid::Uuid::new_v4().to_string(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            completed_actions: Vec::new(),
            active_directive: None,
        }
    }

    pub fn checkpoint_manager(&self) -> &CheckpointManager {
        &self.checkpoint_mgr
    }

    pub fn active_model(&self) -> &str {
        self.provider.model_id()
    }

    pub fn git_auto_commit_enabled(&self) -> bool {
        self.git_auto_commit && git::is_git_repo(&self.working_dir)
    }

    /// Resume from a previously saved checkpoint
    pub fn resume_from_checkpoint(
        &mut self,
        checkpoint: &WorkflowCheckpoint,
    ) -> anyhow::Result<()> {
        self.session_id = checkpoint.session_id.clone();
        self.completed_actions = checkpoint.completed_actions.clone();
        self.memory.read_files = checkpoint.file_operations.read_files.clone();
        self.memory.modified_files = checkpoint.file_operations.modified_files.clone();

        // Inject checkpoint context into the system prompt
        let resume_prompt = checkpoint.to_resume_prompt();
        let current_prompt = self.memory.system_prompt().to_string();
        self.memory
            .set_system_prompt(format!("{}\n\n{}", current_prompt, resume_prompt));

        Ok(())
    }

    /// Execute a user request through the full agent loop
    pub async fn execute(&mut self, user_input: &str) -> anyhow::Result<()> {
        let (user_input, thinking_loops) = parse_thinking_request(user_input)?;
        let user_input = user_input.as_str();
        self.tools.refresh_plugins();

        // ── Step 0: Classify the input for directive recording ────────
        let classification = classifier::classify(user_input);
        if classification.is_directive {
            println!(
                "  {} Directive detected ({}, {:.0}% confidence)",
                style("📌").magenta(),
                style(&classification.category).bold(),
                classification.confidence * 100.0,
            );
            self.active_directive = Some(ActiveDirective {
                fingerprint: String::new(), // Will be filled by LLM
                user_input: user_input.to_string(),
                category: classification.category,
                files: Vec::new(),
                tags: classification.tags,
            });
        }

        // ── Step 1: Route to the best skill ──────────────────────────
        let routing = self.router.route(user_input);
        tracing::debug!(
            "Routed to skill '{}' (confidence: {:.1})",
            routing.skill.name,
            routing.confidence
        );

        // ── Step 2: Search memories for relevant context ─────────────
        let mut extra_context = String::new();

        // Search directive store
        let has_relevant_directives =
            if let Some(directive_ctx) = self.directive_store.relevant_context(user_input) {
                println!(
                    "  {} Found relevant prior directives",
                    style("🧠").color256(69)
                );
                extra_context.push_str(&directive_ctx);
                true
            } else {
                false
            };

        // Search semantic memory index
        if let Some(memory_ctx) = self.memory_index.search_as_context(user_input, 3) {
            println!("  {} Found relevant memories", style("🧠").color256(69));
            extra_context.push_str(&memory_ctx);
        }

        // Build system prompt from skill + instruction files + memories
        let system_prompt = self.build_system_prompt(&routing, &extra_context);
        self.memory.set_system_prompt(system_prompt);

        // Add user message
        self.memory.add_user_message(user_input);

        // ── Step 2.5: Plan for parallelism ───────────────────────────
        let plan = if thinking_loops == 0 {
            let plan_spinner = crate::cli::animation::ThinkingSpinner::start();
            let plan = parallel::plan_tasks(
                &self.provider,
                user_input,
                &self.build_system_prompt(&routing, &extra_context),
            )
            .await;
            plan_spinner.stop();
            Some(plan)
        } else {
            None
        };

        if let Some(Ok(plan)) = plan {
            if plan.should_parallelize && plan.tasks.len() > 1 {
                // Count independent tasks
                let independent: Vec<_> = plan
                    .tasks
                    .iter()
                    .filter(|t| t.depends_on.is_none())
                    .collect();

                if independent.len() > 1 {
                    println!(
                        "  {} Plan: {} ({})",
                        style("📋").color256(69),
                        style(&plan.reasoning).dim().italic(),
                        style(format!("{} parallel tasks", independent.len())).bold(),
                    );

                    // Execute in parallel
                    let system_prompt = self.build_system_prompt(&routing, &extra_context);
                    let tool_defs = self.tools.definitions();
                    let subtasks: Vec<_> = plan
                        .tasks
                        .iter()
                        .filter(|t| t.depends_on.is_none())
                        .cloned()
                        .collect();

                    let results = parallel::execute_parallel(
                        &self.provider,
                        &self.tools,
                        &subtasks,
                        &system_prompt,
                        &tool_defs,
                        self.require_approval,
                    )
                    .await;

                    // Merge results into memory
                    let mut summary = String::from("## Parallel Execution Results\n\n");
                    for result in &results {
                        self.total_input_tokens += result.tokens_in;
                        self.total_output_tokens += result.tokens_out;
                        summary.push_str(&format!(
                            "### {} {}\n{}\n\n",
                            if result.success { "✓" } else { "✗" },
                            result.label,
                            result.output
                        ));
                        for tc in &result.tool_calls {
                            self.completed_actions.push(tc.clone());
                        }
                    }

                    self.memory.add_assistant_message(&summary);

                    // Print the merged output
                    for result in &results {
                        if !result.output.is_empty() {
                            println!(
                                "\n  {} {}\n",
                                style(&result.label).bold(),
                                style("─".repeat(30)).dim()
                            );
                            // Parse out visible text (strip thinking tags)
                            let (_, visible) = parse_thinking(&result.output);
                            if !visible.is_empty() {
                                println!("{}", visible);
                            }
                        }
                    }

                    // Finalize directive if tracked
                    if self.active_directive.is_some() {
                        self.finalize_directive().await?;
                    }

                    return Ok(());
                }
            }
        }

        // ── Step 3: Inner loop + optional self-review loops ──────────
        self.run_inference_cycle(thinking_loops == 0).await?;

        for level in 1..=thinking_loops {
            println!(
                "  {} Thinking pass {}/{}: reviewing the current solution",
                style("↻").magenta(),
                level,
                thinking_loops
            );
            let guidance = self
                .generate_thinking_guidance(
                    user_input,
                    level,
                    thinking_loops,
                    has_relevant_directives,
                )
                .await?;
            self.memory.add_user_message(&format!(
                "[Thinking pass {}/{}]\n\nUse this self-review guidance to improve the solution:\n\n{}\n\nRe-read relevant code where needed, use tools to apply and validate any necessary patches, and produce one consolidated answer reflecting the improved result.",
                level, thinking_loops, guidance
            ));
            self.run_inference_cycle(level == thinking_loops).await?;
        }

        // ── Step 4: Finalize directive if one was being tracked ───────
        if self.active_directive.is_some() {
            self.finalize_directive().await?;
        }

        Ok(())
    }

    async fn run_inference_cycle(&mut self, display_text: bool) -> anyhow::Result<()> {
        let mut iteration = 0;
        loop {
            self.tools.refresh_plugins();

            if self.memory.needs_compaction() {
                self.run_compaction().await?;
            }

            let spinner = crate::cli::animation::ThinkingSpinner::start();
            spinner.update_tokens(self.total_input_tokens, self.total_output_tokens);
            let response = self
                .provider
                .complete(
                    self.memory.messages(),
                    &self.tools.definitions(),
                    self.memory.system_prompt(),
                )
                .await;

            let response = match response {
                Ok(response) => {
                    spinner.stop();
                    response
                }
                Err(error) => {
                    spinner.stop();
                    println!("\n  {} {}", style("API Error:").red().bold(), error);
                    self.memory.add_error_observation(&error.to_string());
                    iteration += 1;
                    if iteration >= 3 {
                        break;
                    }
                    continue;
                }
            };

            self.total_input_tokens += response.usage.input_tokens;
            self.total_output_tokens += response.usage.output_tokens;

            match self.process_response(response, display_text).await? {
                LoopAction::Continue => {
                    iteration += 1;
                    if iteration >= self.max_iterations {
                        println!(
                            "\n  {} Max iterations ({}) reached. Saving checkpoint...",
                            style("⚠").yellow(),
                            self.max_iterations
                        );
                        self.force_checkpoint().await?;
                        break;
                    }
                }
                LoopAction::Done => break,
            }
        }
        Ok(())
    }

    async fn generate_thinking_guidance(
        &mut self,
        user_input: &str,
        level: u8,
        total_levels: u8,
        has_relevant_directives: bool,
    ) -> anyhow::Result<String> {
        let directive_instruction = if has_relevant_directives {
            "Relevant prior directives are present in the system context. Treat successful directives as constraints and explicitly avoid previously failed approaches."
        } else {
            "No relevant prior directive was found. Use the default review dimensions below."
        };
        let review_prompt = format!(
            "{}\n\n## Self-review pass {}/{}\nYou are reviewing the latest answer to this coding task: {}\n{}\nGenerate concise, concrete questions and guidance for the next implementation pass. Check how the code works, correctness, edge cases, security, maintainability, consistency with the repository, completeness of patches, and validation evidence. Do not provide the final answer and do not call tools.",
            self.memory.system_prompt(), level, total_levels, user_input, directive_instruction
        );

        let spinner = crate::cli::animation::ThinkingSpinner::start();
        let response = self
            .provider
            .complete(self.memory.messages(), &[], &review_prompt)
            .await;
        spinner.stop();
        let response = response?;
        self.total_input_tokens += response.usage.input_tokens;
        self.total_output_tokens += response.usage.output_tokens;
        response
            .text
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("Thinking pass {} returned no review guidance", level))
    }

    /// Process a single LLM response
    async fn process_response(
        &mut self,
        response: CompletionResponse,
        display_text: bool,
    ) -> anyhow::Result<LoopAction> {
        // Handle text output
        if let Some(text) = &response.text {
            if display_text && !text.is_empty() {
                let (thinking, visible) = parse_thinking(text);
                if let Some(thinking) = thinking {
                    println!(
                        "\n  {} {}\n",
                        style("💭").dim(),
                        style(&thinking).dim().italic()
                    );
                }
                if !visible.is_empty() {
                    if !crate::cli::charm::render_markdown(&visible) {
                        println!("\n{}\n", visible);
                    }
                }
            }
        }

        // Handle tool calls
        if !response.tool_calls.is_empty() {
            for tool_call in &response.tool_calls {
                self.handle_tool_call(tool_call).await?;
            }
            let summary = response
                .tool_calls
                .iter()
                .map(|tc| format!("Called tool '{}' with args: {}", tc.name, tc.arguments))
                .collect::<Vec<_>>()
                .join("\n");
            self.memory.add_assistant_tool_use(&summary);

            return Ok(LoopAction::Continue);
        }

        if let Some(text) = &response.text {
            self.memory.add_assistant_message(text);
        }

        match response.stop_reason {
            StopReason::EndTurn => Ok(LoopAction::Done),
            StopReason::ToolUse => Ok(LoopAction::Continue),
            StopReason::MaxTokens => {
                println!(
                    "\n  {} Response truncated (max tokens). Continuing...",
                    style("⚠").yellow()
                );
                Ok(LoopAction::Continue)
            }
            StopReason::Error(e) => {
                println!("\n  {} {}", style("Error:").red(), e);
                Ok(LoopAction::Done)
            }
        }
    }

    /// Handle a single tool call with approval flow and git checkpoint
    async fn handle_tool_call(&mut self, tool_call: &ToolCall) -> anyhow::Result<()> {
        let is_mutating = self.tools.is_mutating(&tool_call.name);

        println!(
            "  {} {} {}",
            if is_mutating {
                style("⚡").yellow()
            } else {
                style("🔍").blue()
            },
            style(&tool_call.name).bold(),
            style(format_args_short(&tool_call.arguments)).dim()
        );

        // Approval check for mutating tools
        if is_mutating && self.require_approval {
            let approved = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "  {} Execute {}?",
                    style("▸").yellow(),
                    style(&tool_call.name).bold()
                ))
                .default(true)
                .interact()
                .unwrap_or(false);

            if !approved {
                self.memory
                    .add_tool_result(&tool_call.id, "Tool execution was denied by the user.");
                println!("  {} Skipped\n", style("✗").red());
                return Ok(());
            }
        }

        // Execute the tool
        let result = self
            .tools
            .execute(&tool_call.name, tool_call.arguments.clone())
            .await;

        match result {
            Ok(tool_result) => {
                // Track file operations
                if tool_call.name == "read" {
                    if let Some(path) = tool_call.arguments["path"].as_str() {
                        self.memory.track_read(PathBuf::from(path));
                    }
                }
                if tool_result.is_mutation {
                    // Track modified files — handle both single-file and multi_edit
                    if tool_call.name == "multi_edit" {
                        if let Some(edits) = tool_call.arguments["edits"].as_array() {
                            for edit in edits {
                                if let Some(path) = edit["path"].as_str() {
                                    self.memory.track_modification(PathBuf::from(path));
                                    if let Some(ref mut ad) = self.active_directive {
                                        ad.files.push(path.to_string());
                                    }
                                }
                            }
                        }
                    } else if let Some(path) = tool_call.arguments["path"].as_str() {
                        self.memory.track_modification(PathBuf::from(path));
                        // Track for active directive
                        if let Some(ref mut ad) = self.active_directive {
                            ad.files.push(path.to_string());
                        }
                    }
                    self.completed_actions.push(format!(
                        "{}({})",
                        tool_call.name,
                        format_args_short(&tool_call.arguments)
                    ));

                    // ── Git auto-commit after accepted mutation ──────
                    if self.git_auto_commit
                        && git::is_git_repo(&self.working_dir)
                        && git::has_changes(&self.working_dir)
                    {
                        self.git_checkpoint(&tool_call.name, &tool_call.arguments)
                            .await;
                    }
                }

                let display = if tool_result.output.chars().count() > 500 {
                    format!(
                        "{}...",
                        tool_result.output.chars().take(500).collect::<String>()
                    )
                } else {
                    tool_result.output.clone()
                };

                if tool_result.success {
                    println!("  {} {}\n", style("✓").color256(111), style(display).dim());
                } else {
                    println!("  {} {}\n", style("✗").red(), style(display).dim());
                }

                self.memory
                    .add_tool_result(&tool_call.id, &tool_result.output);
            }
            Err(e) => {
                let error_msg = format!("Tool execution failed: {}", e);
                println!("  {} {}\n", style("✗").red(), style(&error_msg).dim());
                self.memory.add_tool_result(&tool_call.id, &error_msg);
            }
        }

        Ok(())
    }

    /// Create a git commit with an LLM-generated semantic message
    async fn git_checkpoint(&self, tool_name: &str, args: &serde_json::Value) {
        let diff_summary = git::diff_summary(&self.working_dir);
        let diff_content = git::diff_content_short(&self.working_dir);
        let prompt = git::commit_message_prompt(&diff_summary, &diff_content);

        // Ask LLM for a commit message
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Text(prompt),
        }];

        let commit_msg = match self
            .provider
            .complete(
                &messages,
                &[],
                "You are a git commit message generator. Output ONLY the commit message.",
            )
            .await
        {
            Ok(r) => r
                .text
                .unwrap_or_else(|| format!("loop: {} via {}", tool_name, self.provider.model_id())),
            Err(_) => format!("loop({}): auto-checkpoint", tool_name),
        };

        // Clean up the message — strip quotes, trim
        let commit_msg = commit_msg
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();

        match git::commit_changes(&self.working_dir, &commit_msg) {
            Ok(()) => {
                println!(
                    "  {} {}",
                    style("📝").color256(111),
                    style(format!("git: {}", commit_msg)).dim()
                );
            }
            Err(e) => {
                tracing::warn!("Git commit failed: {}", e);
            }
        }
    }

    /// Finalize and save the tracked directive
    async fn finalize_directive(&mut self) -> anyhow::Result<()> {
        let ad = match self.active_directive.take() {
            Some(ad) => ad,
            None => return Ok(()),
        };

        // Ask the LLM to generate a fingerprint
        let fp_prompt = classifier::fingerprint_extraction_prompt(&ad.user_input, &ad.category);
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Text(fp_prompt),
        }];

        let (fingerprint, action_summary, extra_files, extra_tags) = match self
            .provider
            .complete(
                &messages,
                &[],
                "You extract structured data. Output ONLY valid JSON.",
            )
            .await
        {
            Ok(r) if r.text.is_some() => {
                let text = r.text.unwrap();
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(json) => (
                        json["fingerprint"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string(),
                        json["action_summary"]
                            .as_str()
                            .unwrap_or(&ad.user_input)
                            .to_string(),
                        json["files_involved"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                        json["tags"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                    ),
                    Err(_) => (
                        format!("{}-{}", ad.category, &self.session_id[..8]),
                        ad.user_input.clone(),
                        Vec::new(),
                        Vec::new(),
                    ),
                }
            }
            _ => (
                format!("{}-{}", ad.category, &self.session_id[..8]),
                ad.user_input.clone(),
                Vec::new(),
                Vec::new(),
            ),
        };

        // Merge files and tags
        let mut all_files = ad.files;
        all_files.extend(extra_files);
        all_files.sort();
        all_files.dedup();

        let mut all_tags = ad.tags;
        all_tags.extend(extra_tags);
        all_tags.sort();
        all_tags.dedup();

        // Ask user for outcome
        let outcome_idx = dialoguer::Select::new()
            .with_prompt(format!(
                "  {} Directive outcome for '{}'",
                style("📌").magenta(),
                style(&fingerprint).bold()
            ))
            .items(&[
                "✅ Worked",
                "❌ Did not work",
                "⚠️  Partial",
                "❓ Skip recording",
            ])
            .default(0)
            .interact()
            .unwrap_or(3);

        if outcome_idx == 3 {
            println!("  {} Directive not recorded\n", style("↩").dim());
            return Ok(());
        }

        let outcome = match outcome_idx {
            0 => DirectiveOutcome::Worked,
            1 => DirectiveOutcome::DidNotWork,
            2 => DirectiveOutcome::Partial,
            _ => DirectiveOutcome::Unknown,
        };

        let directive = directives::create_directive_from_llm_response(
            &fingerprint,
            &ad.user_input,
            &action_summary,
            all_files,
            all_tags,
        );

        // Update outcome and save
        let mut directive = directive;
        directive.outcome = outcome;
        self.directive_store.add(directive)?;

        println!(
            "  {} Directive '{}' saved to directives.md\n",
            style("✓").color256(111),
            style(&fingerprint).bold()
        );

        Ok(())
    }

    /// Run context compaction via LLM summarization
    async fn run_compaction(&mut self) -> anyhow::Result<()> {
        println!("  {} Compacting context...", style("📦").yellow());

        if let Some((summary_request, split_point)) = self.memory.prepare_compaction() {
            let summary_messages = vec![Message {
                role: Role::User,
                content: MessageContent::Text(summary_request),
            }];

            let response = self.provider.complete(
                &summary_messages,
                &[],
                "You are a precise summarizer. Produce a concise summary preserving all key technical details.",
            ).await?;

            if let Some(summary) = response.text {
                self.memory.apply_compaction(&summary, split_point);
                println!("  {} Context compacted ✓\n", style("✓").color256(111));
            }
        }

        Ok(())
    }

    /// Build the system prompt from skill, instructions, memories, and directives
    fn build_system_prompt(&self, routing: &RoutingDecision, extra_context: &str) -> String {
        let mut prompt = String::new();

        // Core identity
        prompt.push_str("You are Loop, a minimalist coding agent. You have direct access to the user's filesystem and shell.\n\n");

        // Skill-specific instructions
        if !routing.skill.prompt.is_empty() {
            prompt.push_str("## Active Skill\n");
            prompt.push_str(&routing.skill.prompt);
            prompt.push_str("\n\n");
        }

        // Thinking protocol
        prompt.push_str("## Thinking Protocol\n");
        prompt.push_str("Before taking any action, wrap your reasoning in <thinking> tags.\n");
        prompt.push_str("Plan your approach, then execute tools step by step.\n");
        prompt.push_str("After each tool result, evaluate if you need to adjust your plan.\n\n");

        // Tool usage guidelines
        prompt.push_str("## Tool Guidelines\n");
        prompt.push_str("- Use `read` to understand code before modifying it\n");
        prompt.push_str(
            "- Use `edit` for surgical changes (preferred over `write` for existing files)\n",
        );
        prompt.push_str("- Use `multi_edit` when making coordinated changes across multiple files in one atomic operation\n");
        prompt.push_str("- Use `bash` for running commands, tests, and verification\n");
        prompt.push_str("- Use `list_dir` to explore project structure\n");
        prompt.push_str("- Always verify changes after making them\n\n");

        // Load instruction files
        for path in &self.instruction_files {
            if let Ok(content) = std::fs::read_to_string(path) {
                prompt.push_str(&format!(
                    "## Instructions (from {})\n{}\n\n",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    content.trim()
                ));
            }
        }

        // Inject memory/directive context
        if !extra_context.is_empty() {
            prompt.push_str(extra_context);
            prompt.push('\n');
        }

        prompt
    }

    /// Force a checkpoint save
    pub async fn force_checkpoint(&mut self) -> anyhow::Result<()> {
        let checkpoint_prompt = CheckpointManager::checkpoint_extraction_prompt();
        let messages = vec![Message {
            role: Role::User,
            content: MessageContent::Text(format!(
                "Based on our conversation, {}\n\nConversation summary so far:\n- Completed: {:?}\n- Files read: {:?}\n- Files modified: {:?}",
                checkpoint_prompt,
                self.completed_actions,
                self.memory.read_files,
                self.memory.modified_files,
            )),
        }];

        let response = self
            .provider
            .complete(
                &messages,
                &[],
                "You are a checkpoint generator. Output only valid JSON.",
            )
            .await;

        let checkpoint = match response {
            Ok(r) if r.text.is_some() => {
                let text = r.text.unwrap();
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(json) => WorkflowCheckpoint {
                        session_id: self.session_id.clone(),
                        timestamp: chrono::Utc::now(),
                        active_skill: "general".into(),
                        active_model: self.provider.model_id().to_string(),
                        completed_actions: json["completed_actions"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_else(|| self.completed_actions.clone()),
                        current_context: json["current_context"]
                            .as_str()
                            .unwrap_or("Session ended by user")
                            .to_string(),
                        pending_tasks: json["pending_tasks"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        file_operations: FileOperations {
                            read_files: self.memory.read_files.clone(),
                            modified_files: self.memory.modified_files.clone(),
                        },
                    },
                    Err(_) => self.fallback_checkpoint(),
                }
            }
            _ => self.fallback_checkpoint(),
        };

        self.checkpoint_mgr.save(&checkpoint)?;
        Ok(())
    }

    fn fallback_checkpoint(&self) -> WorkflowCheckpoint {
        WorkflowCheckpoint {
            session_id: self.session_id.clone(),
            timestamp: chrono::Utc::now(),
            active_skill: "general".into(),
            active_model: self.provider.model_id().to_string(),
            completed_actions: self.completed_actions.clone(),
            current_context: "Session ended (fallback checkpoint)".into(),
            pending_tasks: vec![],
            file_operations: FileOperations {
                read_files: self.memory.read_files.clone(),
                modified_files: self.memory.modified_files.clone(),
            },
        }
    }

    pub fn model_and_session(&self) -> (&str, &str) {
        (self.provider.model_id(), &self.session_id[..8])
    }

    pub fn file_stats(&self) -> (Vec<String>, Vec<String>) {
        let reads = self
            .memory
            .read_files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let writes = self
            .memory
            .modified_files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        (reads, writes)
    }

    pub fn directive_count(&self) -> usize {
        self.directive_store.all().len()
    }

    pub fn memory_stats(&self) -> (usize, usize) {
        (self.memory.messages().len(), self.memory.estimated_tokens())
    }

    pub fn token_stats(&self) -> (usize, usize) {
        (self.total_input_tokens, self.total_output_tokens)
    }

    /// Print agent status (legacy CLI output, mostly superseded by dashboard but kept for non-interactive)
    pub fn print_status(&self) {
        charm::section("Loop status", "current session at a glance");
        let mut lines = vec![
            format!(
                "{}  {}",
                charm::key("MODEL"),
                charm::value(self.provider.model_id())
            ),
            format!(
                "{}  {}",
                charm::key("SESSION"),
                charm::value(&self.session_id[..8])
            ),
            format!(
                "{}  {}",
                charm::key("TOKENS"),
                charm::value(&format!(
                    "{} in / {} out",
                    self.total_input_tokens, self.total_output_tokens
                ))
            ),
            format!(
                "{}  {}",
                charm::key("CONTEXT"),
                charm::value(&format!(
                    "~{} tokens · {} messages",
                    self.memory.estimated_tokens(),
                    self.memory.messages().len()
                ))
            ),
            format!(
                "{}  {}",
                charm::key("FILES"),
                charm::value(&format!(
                    "{} read · {} modified",
                    self.memory.read_files.len(),
                    self.memory.modified_files.len()
                ))
            ),
            format!(
                "{}  {}",
                charm::key("MEMORY"),
                charm::value(&format!("{} directives", self.directive_store.all().len()))
            ),
        ];
        if self.git_auto_commit {
            let git_status = if git::is_git_repo(&self.working_dir) {
                charm::badge("ACTIVE", charm::BLUE_SOFT)
            } else {
                charm::badge("NO REPO", charm::YELLOW)
            };
            lines.push(format!("{}  {}", charm::key("GIT"), git_status));
        }
        println!("{}\n", charm::panel(&lines.join("\n"), charm::BLUE));
    }

    /// Print available tools
    pub fn print_tools(&self) {
        charm::section(
            "Available tools",
            "live registry · plugins hot-load each turn",
        );
        let mut rows = Vec::new();
        for def in self.tools.definitions() {
            let kind = if self.tools.is_mutating(&def.name) {
                charm::badge("WRITE", charm::YELLOW)
            } else {
                charm::badge("READ", charm::BLUE)
            };
            rows.push(format!(
                "{}  {}  {}",
                kind,
                charm::command(&format!("{:<22}", def.name)),
                charm::muted(&def.description)
            ));
        }
        println!("{}\n", charm::panel(&rows.join("\n"), charm::PINK));
    }

    /// Clear conversation context
    pub fn clear_context(&mut self) {
        self.memory.clear();
        self.completed_actions.clear();
    }
}

enum LoopAction {
    Continue,
    Done,
}

fn parse_thinking_request(input: &str) -> anyhow::Result<(String, u8)> {
    let trimmed = input.trim_end();
    let Some((task, suffix)) = trimmed.rsplit_once(char::is_whitespace) else {
        return Ok((trimmed.to_string(), 0));
    };

    let levels = match suffix {
        "-t" | "-t1" => 1,
        "-t2" => 2,
        "-t3" => 3,
        value
            if value.starts_with("-t")
                && value[2..]
                    .chars()
                    .all(|character| character.is_ascii_digit()) =>
        {
            anyhow::bail!("Thinking mode supports -t, -t1, -t2, or -t3")
        }
        _ => return Ok((trimmed.to_string(), 0)),
    };

    let task = task.trim_end();
    if task.is_empty() {
        anyhow::bail!("Add a query before the thinking suffix")
    }
    Ok((task.to_string(), levels))
}

#[cfg(test)]
mod tests {
    use super::parse_thinking_request;

    #[test]
    fn parses_bounded_thinking_suffixes() {
        assert_eq!(
            parse_thinking_request("fix it -t").unwrap(),
            ("fix it".to_string(), 1)
        );
        assert_eq!(
            parse_thinking_request("fix it -t2").unwrap(),
            ("fix it".to_string(), 2)
        );
        assert_eq!(
            parse_thinking_request("fix it -t3").unwrap(),
            ("fix it".to_string(), 3)
        );
        assert_eq!(
            parse_thinking_request("fix -tests").unwrap(),
            ("fix -tests".to_string(), 0)
        );
        assert!(parse_thinking_request("fix it -t4").is_err());
    }
}

/// Parse <thinking> tags from model output
fn parse_thinking(text: &str) -> (Option<String>, String) {
    if let Some(start) = text.find("<thinking>") {
        if let Some(end) = text.find("</thinking>") {
            let thinking = text[start + 10..end].trim().to_string();
            let visible = format!("{}{}", text[..start].trim(), text[end + 11..].trim());
            return (Some(thinking), visible.trim().to_string());
        }
    }
    (None, text.to_string())
}

/// Format tool arguments for short display
fn format_args_short(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        let parts: Vec<String> = obj
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => {
                        if s.chars().count() > 60 {
                            format!("\"{}...\"", s.chars().take(60).collect::<String>())
                        } else {
                            format!("\"{}\"", s)
                        }
                    }
                    other => other.to_string(),
                };
                format!("{}={}", k, val)
            })
            .collect();
        parts.join(", ")
    } else {
        args.to_string()
    }
}
