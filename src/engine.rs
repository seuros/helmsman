//! Template engine wrapper for minijinja.

use crate::environment::Environment;
use crate::models::ModelContext;
use crate::skills::{
    is_partial_skill, parse_frontmatter, skill_name_from_path, SkillMeta, SKILL_EXTENSION,
    SKILLS_DIR_NAME,
};
use minijinja::Environment as JinjaEnv;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Template not found: {0}")]
    TemplateNotFound(String),
    #[error("Render error: {0}")]
    RenderError(#[from] minijinja::Error),
    #[error("Skill not available for tier: {0}")]
    TierNotAllowed(String),
}

/// Skill with metadata and content.
#[derive(Debug, Clone)]
pub struct Skill {
    pub id: String,
    pub meta: SkillMeta,
    pub content: String,
}

impl Skill {
    /// Check if skill is available for a given tier.
    pub fn is_available_for_tier(&self, tier: &str) -> bool {
        match &self.meta.tiers {
            Some(tiers) => tiers.iter().any(|t| t == tier),
            None => true, // No restriction = available to all
        }
    }
}

/// Project context passed to templates.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProjectContext {
    pub cwd: Option<String>,
    pub stack: Option<Vec<String>>,
    pub vars: Option<HashMap<String, String>>,
}

/// Combined context for template rendering.
#[derive(Debug, Serialize)]
struct TemplateContext {
    model: ModelContext,
    project: ProjectContext,
    env: Environment,
}

/// Template engine for rendering instructions.
pub struct TemplateEngine {
    jinja: JinjaEnv<'static>,
    templates_dir: std::path::PathBuf,
}

impl TemplateEngine {
    fn build_context(
        &self,
        model_ctx: ModelContext,
        project_ctx: Option<ProjectContext>,
    ) -> TemplateContext {
        TemplateContext {
            model: model_ctx,
            project: project_ctx.unwrap_or_default(),
            env: Environment::detect(),
        }
    }

    /// Compute skill search paths from a given directory.
    fn compute_skill_paths(templates_dir: &Path) -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        let home = dirs::home_dir();

        let mut current = Some(templates_dir);
        while let Some(dir) = current {
            paths.push(dir.join(SKILLS_DIR_NAME));

            if home.as_ref().map(|h| h.as_path() == dir).unwrap_or(false) {
                break;
            }
            current = dir.parent();
        }

        // Platform config dir (~/Library/Application Support on macOS)
        if let Some(config_dir) = dirs::config_dir() {
            paths.push(config_dir.join("helmsman").join("skills"));
        }

        // XDG fallback (~/.config) - for cross-platform consistency
        if let Some(home) = &home {
            let xdg_config = home.join(".config").join("helmsman").join("skills");
            if !paths.contains(&xdg_config) {
                paths.push(xdg_config);
            }
        }

        paths
    }

    /// Create a new template engine loading from the given directory.
    pub fn new(templates_dir: &Path) -> Result<Self, EngineError> {
        // Canonicalize to enable walking up the tree
        let canonical_dir = templates_dir
            .canonicalize()
            .unwrap_or_else(|_| templates_dir.to_path_buf());

        // Compute skill paths for the loader
        let skill_paths = Self::compute_skill_paths(&canonical_dir);

        let mut jinja = JinjaEnv::new();
        let base_path = canonical_dir.clone();

        // Custom loader: first check base templates, then skill paths for includes
        jinja.set_loader(move |name| {
            // First check base templates dir (for AGENTS.md.j2 etc)
            let path = base_path.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Ok(Some(content));
            }

            // Then search skill paths (for partials/includes)
            for dir in &skill_paths {
                let path = dir.join(name);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    return Ok(Some(content));
                }
            }

            Ok(None)
        });

        Ok(Self {
            jinja,
            templates_dir: canonical_dir,
        })
    }

    /// Render AGENTS.md.j2 with model and environment context.
    pub fn render(
        &self,
        model_ctx: ModelContext,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        self.render_template("AGENTS.md.j2", model_ctx, project_ctx)
    }

    /// Render a specific template.
    pub fn render_template(
        &self,
        template_name: &str,
        model_ctx: ModelContext,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        let template = self
            .jinja
            .get_template(template_name)
            .map_err(|_| EngineError::TemplateNotFound(template_name.to_string()))?;

        let ctx = self.build_context(model_ctx, project_ctx);

        let rendered = template.render(&ctx)?;
        Ok(rendered)
    }

    /// Get skill search paths (walk up from templates_dir to home, then user config).
    fn skill_paths(&self) -> Vec<std::path::PathBuf> {
        Self::compute_skill_paths(&self.templates_dir)
    }

    /// List available skills (project overrides user, excludes partials).
    pub fn list_skills(&self) -> Vec<String> {
        self.list_skills_for_tier(None)
    }

    /// List skills available for a specific tier.
    pub fn list_skills_for_tier(&self, tier: Option<&str>) -> Vec<String> {
        self.get_skills(tier).into_iter().map(|s| s.id).collect()
    }

    /// Get skills with metadata, optionally filtered by tier.
    pub fn get_skills(&self, tier: Option<&str>) -> Vec<Skill> {
        let mut seen = std::collections::HashSet::new();
        let mut skills = Vec::new();

        for dir in self.skill_paths() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();

                let Some(name) = skill_name_from_path(&path) else {
                    continue;
                };
                if is_partial_skill(&name) {
                    continue;
                }

                // Skip if already seen (higher priority dir wins)
                if !seen.insert(name.to_string()) {
                    continue;
                }

                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };

                let (meta, body) = parse_frontmatter(&content);
                let skill = Skill {
                    id: name.to_string(),
                    meta,
                    content: body,
                };

                // Filter by tier if specified
                if let Some(t) = tier
                    && !skill.is_available_for_tier(t)
                {
                    continue;
                }

                skills.push(skill);
            }
        }

        skills.sort_by(|a, b| a.id.cmp(&b.id));
        skills
    }

    /// Get a single skill by name with metadata.
    pub fn get_skill(&self, name: &str) -> Result<Skill, EngineError> {
        let content = self.read_skill_raw(name)?;
        let (meta, body) = parse_frontmatter(&content);
        Ok(Skill {
            id: name.to_string(),
            meta,
            content: body,
        })
    }

    /// Read raw skill template content (project overrides user).
    pub fn read_skill_raw(&self, name: &str) -> Result<String, EngineError> {
        let filename = format!("{}.{}", name, SKILL_EXTENSION);

        for dir in self.skill_paths() {
            let path = dir.join(&filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Ok(content);
            }
        }

        Err(EngineError::TemplateNotFound(format!(
            "{}/{}.{}",
            SKILLS_DIR_NAME, name, SKILL_EXTENSION
        )))
    }

    /// Render a skill with hierarchical search (project overrides parents).
    /// Checks tier availability and strips frontmatter.
    pub fn render_skill(
        &self,
        name: &str,
        model_ctx: ModelContext,
        project_ctx: Option<ProjectContext>,
    ) -> Result<String, EngineError> {
        let skill = self.get_skill(name)?;

        // Check tier availability
        if !skill.is_available_for_tier(&model_ctx.tier) {
            return Err(EngineError::TierNotAllowed(format!(
                "Skill '{}' is not available for tier '{}'",
                name, model_ctx.tier
            )));
        }

        let ctx = self.build_context(model_ctx, project_ctx);
        let rendered = self.jinja.render_str(&skill.content, &ctx)?;
        Ok(rendered)
    }
}
