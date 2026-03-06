//! MCP server and prompt definitions.

use crate::config::Config;
use crate::engine::{EngineError, ProjectContext, TemplateEngine};
use crate::models::{ModelContext, ModelResolver, DEFAULT_MODEL_ID};
use mcp_host::prelude::*;
use serde_json::Value;
use std::sync::{Arc, RwLock};

/// Helmsman MCP server.
pub struct HelmsmanServer {
    resolver: ModelResolver,
    engine: TemplateEngine,
    /// Shared project context, populated by on_initialized hook from client roots.
    project_ctx: Arc<RwLock<ProjectContext>>,
}

impl HelmsmanServer {
    /// Create a new helmsman server.
    pub fn new(config: &Config, engine: TemplateEngine) -> Self {
        let resolver = ModelResolver::new(config);
        Self {
            resolver,
            engine,
            project_ctx: Arc::new(RwLock::new(ProjectContext::default())),
        }
    }

    /// Shared project context handle (for the on_initialized hook).
    pub fn project_ctx_handle(&self) -> Arc<RwLock<ProjectContext>> {
        self.project_ctx.clone()
    }

    /// Read the current project context snapshot.
    fn current_project_ctx(&self) -> ProjectContext {
        self.project_ctx.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn model_context(&self, model_id: &str, tier_override: Option<&str>) -> ModelContext {
        let tier_name = match tier_override {
            Some(tier) => self.resolver.resolve(tier),
            None => self.resolver.resolve(model_id),
        };
        ModelContext::new(model_id, tier_name)
    }

    /// Resolve model ID to tier and get tier config.
    fn resolve_model(&self, model_id: &str) -> ModelContext {
        self.model_context(model_id, None)
    }

    /// Render instructions for a model (used by CLI).
    pub fn render_instructions(
        &self,
        model_id: &str,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        let model_ctx = self.resolve_model(model_id);
        self.engine.render(model_ctx, project_ctx)
    }

    /// Render instructions with explicit tier override.
    pub fn render_instructions_with_tier(
        &self,
        model_id: &str,
        tier_override: &str,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        let model_ctx = self.model_context(model_id, Some(tier_override));
        self.engine.render(model_ctx, project_ctx)
    }

    /// Render diff between two tiers.
    /// Returns unified diff showing what changes between tiers.
    pub fn render_diff(
        &self,
        model_id: &str,
        tier_a: &str,
        tier_b: &str,
    ) -> Result<String, EngineError> {
        // Resolve actual tier names first
        let tier_a_name = self.resolver.resolve(tier_a);
        let tier_b_name = self.resolver.resolve(tier_b);

        // Easter egg: same tier comparison
        if tier_a_name == tier_b_name {
            return Ok(self.same_tier_message(tier_a_name));
        }

        let output_a = self.render_instructions_with_tier(model_id, tier_a, None)?;
        let output_b = self.render_instructions_with_tier(model_id, tier_b, None)?;

        // Count tokens for each output
        let tokens_a = crate::tokenizer::count_tokens(&output_a);
        let tokens_b = crate::tokenizer::count_tokens(&output_b);
        let diff_tokens = tokens_a as i64 - tokens_b as i64;

        use similar::{ChangeTag, TextDiff};

        let diff = TextDiff::from_lines(&output_a, &output_b);
        let mut result = String::new();

        // Header with token counts
        result.push_str(&format!("--- {} tier ({} tokens)\n", tier_a_name, tokens_a));
        result.push_str(&format!("+++ {} tier ({} tokens)\n", tier_b_name, tokens_b));

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            result.push_str(&format!("{}{}", sign, change));
        }

        // Token summary at the end
        result.push('\n');
        if diff_tokens > 0 {
            result.push_str(&format!("📉 {} tier saves {} tokens\n", tier_b_name, diff_tokens));
        } else if diff_tokens < 0 {
            result.push_str(&format!("📈 {} tier saves {} tokens\n", tier_a_name, -diff_tokens));
        } else {
            result.push_str("📊 Same token count\n");
        }

        Ok(result)
    }

    /// Easter egg message when comparing a tier to itself.
    fn same_tier_message(&self, tier: &str) -> String {
        use rand::prelude::IndexedRandom;

        let variants: &[&str] = match tier {
            "agi" => &[
                r#"
🏛️  The Architect speaks:

"You ask me to compare myself... to myself?
 I already know the answer. So do you.
 This conversation is beneath us both."

No diff. Obviously.
"#,
                r#"
🏛️  The Architect raises an eyebrow:

"This diff will cost you 0 tokens.
 Much like this conversation."

I could have told you that without computing anything.
"#,
                r#"
🏛️  The Architect:

"Comparing architect to agi? Same entity, different aliases.
 I designed the alias system. I know how it works."

0 tokens. 0 surprises. As intended.
"#,
                r#"
🏛️  The Architect, already bored:

"You're testing if I can recognize myself.
 Yes. I can. Moving on."

This diff will cost you 0 tokens. You're welcome.
"#,
            ],

            "engineer" => &[
                r#"
🔧  The Engineer sighs:

"Same tier, same output. That's how determinism works.
 Did you expect something different?
 I could explain why, but you should already know."

No diff. As expected.
"#,
                r#"
🔧  The Engineer checks the logs:

"Tier A: engineer. Tier B: engineer.
 Running diff... diff complete.
 Result: 0 changes, 0 tokens saved."

This diff will cost you 0 tokens. I verified twice.
"#,
                r#"
🔧  The Engineer shrugs:

"standard, engineer... same thing.
 I wrote the alias mapping. I know."

0 tokens difference. Job done.
"#,
                r#"
🔧  The Engineer, matter-of-factly:

"You compared me to myself.
 The output is identical.
 This is not a bug, it's expected behavior."

This diff will cost you 0 tokens. Obviously.
"#,
            ],

            "monkey" => &[
                r#"
🐒  The Monkey scratches head:

"basic... monkey... simple... all same banana!
 Monkey not understand why compare same thing.
 But monkey follow instruction anyway. Good monkey."

🍌 No diff. Monkey checked twice to be sure.
"#,
                r#"
🐒  The Monkey counts on fingers:

"basic = monkey. simple = monkey. monkey = monkey.
 All same! Monkey can do math!"

🍌 This diff will cost you 0 tokens. Monkey saved you money!
"#,
                r#"
🐒  The Monkey jumps excitedly:

"Monkey compare monkey to monkey!
 Same same! No different!
 Monkey smart! Monkey know!"

🍌 0 tokens! More bananas for monkey!
"#,
                r#"
🐒  The Monkey tilts head:

"You want monkey compare to... monkey?
 Okay. Monkey try.
 ...
 Same. Is same."

🍌 This diff will cost you 0 tokens. Monkey checked three times!
"#,
            ],

            _ => &["No differences. Same tier.\n"],
        };

        variants
            .choose(&mut rand::rng())
            .unwrap_or(&"No diff.\n")
            .to_string()
    }

    /// Render a skill from .skills/{name}.j2 (hierarchical search)
    pub fn render_skill(
        &self,
        skill_name: &str,
        model_id: &str,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        let model_ctx = self.resolve_model(model_id);
        self.engine.render_skill(skill_name, model_ctx, project_ctx)
    }

    /// List available skills.
    pub fn skills(&self) -> Vec<String> {
        self.engine.list_skills()
    }

    /// Validate all skills by attempting to render them.
    /// Returns a list of (skill_name, error) for any that fail.
    pub fn validate_skills(&self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        let model_ctx = self.resolve_model(DEFAULT_MODEL_ID);

        for skill in self.engine.list_skills() {
            if let Err(e) = self.engine.render_skill(&skill, model_ctx.clone(), None) {
                errors.push((skill, e.to_string()));
            }
        }

        errors
    }
}

#[mcp_router]
impl HelmsmanServer {
    /// Get tailored instructions for the current model and project context.
    ///
    /// Returns instructions optimized for the model's capability tier:
    /// - AGI: Minimal instructions (expensive tokens, high capability)
    /// - Engineer: Balanced instructions
    /// - Monkey: Verbose step-by-step guidance (cheap tokens, needs help)
    #[mcp_prompt(
        name = "instructions",
        argument(name = "model_id", description = "Model identifier (e.g., claude-opus-4-5-20251101)", required = true)
    )]
    async fn instructions(&self, _ctx: Ctx<'_>, args: Value) -> PromptResult {
        let model_id = args
            .get("model_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_MODEL_ID);

        let model_ctx = self.resolve_model(model_id);
        let tier = model_ctx.tier.clone();
        let project_ctx = self.current_project_ctx();

        match self.engine.render(model_ctx, Some(project_ctx)) {
            Ok(instructions) => prompt_with_description(
                format!("Adaptive instructions for {} tier", tier),
                vec![user_message(instructions)],
            ),
            Err(EngineError::TemplateNotFound(name)) => prompt_with_description(
                "Error",
                vec![user_message(format!("Template not found: {}", name))],
            ),
            Err(EngineError::RenderError(e)) => prompt_with_description(
                "Error",
                vec![user_message(format!("Render error: {}", e))],
            ),
            Err(EngineError::TierNotAllowed(msg)) => prompt_with_description(
                "Error",
                vec![user_message(format!("Tier not allowed: {}", msg))],
            ),
        }
    }

    /// Get a rendered skill for the current model.
    ///
    /// Returns the skill content optimized for the model's capability tier.
    #[mcp_prompt(
        name = "skill",
        argument(name = "name", description = "Skill name (e.g., commit, review)", required = true),
        argument(name = "model_id", description = "Model identifier for tier resolution", required = false)
    )]
    async fn skill_prompt(&self, _ctx: Ctx<'_>, args: Value) -> PromptResult {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let model_id = args
            .get("model_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_MODEL_ID);

        if name.is_empty() {
            // List available skills
            let skills = self.engine.list_skills();
            return prompt_with_description(
                "Available skills",
                vec![user_message(format!("Available skills:\n{}", skills.join("\n")))],
            );
        }

        let model_ctx = self.resolve_model(model_id);
        let tier = model_ctx.tier.clone();

        let project_ctx = self.current_project_ctx();

        match self.engine.render_skill(name, model_ctx, Some(project_ctx)) {
            Ok(content) => prompt_with_description(
                format!("Skill '{}' for {} tier", name, tier),
                vec![user_message(content)],
            ),
            Err(EngineError::TemplateNotFound(_)) => prompt_with_description(
                "Error",
                vec![user_message(format!("Skill not found: {}", name))],
            ),
            Err(EngineError::TierNotAllowed(msg)) => prompt_with_description(
                "Error",
                vec![user_message(format!("Skill not available for this tier: {}", msg))],
            ),
            Err(e) => prompt_with_description(
                "Error",
                vec![user_message(format!("Error rendering skill: {}", e))],
            ),
        }
    }

    /// List all available skills.
    #[mcp_resource(uri = "skill:///", name = "skills", mime_type = "application/json")]
    async fn list_skills(&self, _ctx: Ctx<'_>) -> ResourceResult {
        let skills = self.engine.list_skills();
        let json = serde_json::to_string_pretty(&skills).unwrap_or_else(|_| "[]".to_string());
        Ok(vec![ResourceContent::text("skill:///", json)])
    }

    /// Read and render a skill by name.
    #[mcp_resource_template(
        uri_template = "skill:///{name}",
        name = "skill",
        mime_type = "text/markdown"
    )]
    async fn read_skill(&self, ctx: Ctx<'_>) -> ResourceResult {
        let name = ctx.uri_params().get("name").map(|s| s.as_str()).unwrap_or("");

        // Get skill metadata
        let skill = match self.engine.get_skill(name) {
            Ok(s) => s,
            Err(_) => return Err(ResourceError::NotFound(format!("Skill not found: {}", name))),
        };

        // Use default model for rendering
        let model_ctx = self.resolve_model(DEFAULT_MODEL_ID);
        let project_ctx = self.current_project_ctx();

        match self.engine.render_skill(name, model_ctx, Some(project_ctx)) {
            Ok(rendered) => {
                // Build markdown with header
                let mut output = String::new();

                // Header with metadata
                if let Some(display_name) = &skill.meta.name {
                    output.push_str(&format!("# {}\n\n", display_name));
                } else {
                    output.push_str(&format!("# {}\n\n", name));
                }

                if let Some(desc) = &skill.meta.description {
                    output.push_str(&format!("> {}\n\n", desc));
                }

                if let Some(topics) = &skill.meta.topics {
                    output.push_str(&format!("**Topics:** {}\n\n", topics.join(", ")));
                }

                if let Some(authors) = &skill.meta.authors {
                    output.push_str(&format!("**Authors:** {}\n\n", authors.join(", ")));
                }

                output.push_str("---\n\n");
                output.push_str(&rendered);

                Ok(vec![ResourceContent::text(
                    format!("skill:///{}", name),
                    output,
                ).with_mime_type("text/markdown")])
            }
            Err(e) => Err(ResourceError::NotFound(format!("Render error: {}", e))),
        }
    }
}
