//! Helmsman - Adaptive instruction server.

mod anthropic;
mod config;
mod engine;
mod environment;
mod models;
mod registry;
mod remote;
mod server;
mod skills;
mod tokenizer;

use clap::{Parser, Subcommand};
use config::{Config, MCP_NAME, MCP_VERSION};
use dialoguer::{theme::ColorfulTheme, Select};
use engine::TemplateEngine;
use mcp_host::prelude::*;
use models::DEFAULT_MODEL_ID;
use registry::{Registry, SkillEntry};
use remote::{ParsedSource, RemoteFetcher};
use server::HelmsmanServer;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "helmsman")]
#[command(version)]
#[command(about = "Adaptive instruction server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Print instructions for a model and exit
    #[arg(short, long)]
    instructions: Option<Option<String>>,

    /// Render a skill from .skills/{name}.j2
    #[arg(short, long, value_name = "NAME")]
    skill: Option<String>,

    /// Model ID (used with --skill)
    #[arg(short, long, default_value_t = DEFAULT_MODEL_ID.to_string())]
    model: String,

    /// Override tier regardless of model mapping (agi, engineer, monkey)
    #[arg(long, value_name = "TIER")]
    tier: Option<String>,

    /// Show diff against another tier (e.g., --diff engineer)
    #[arg(long, value_name = "TIER")]
    diff: Option<String>,

    /// Show token count of rendered output
    #[arg(short, long)]
    tokens: bool,

    /// List available skills
    #[arg(short, long)]
    list: bool,

    /// Validate all skills (check for broken includes)
    #[arg(long)]
    validate: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Add skills from a remote repository
    Add {
        /// Source (owner/repo or GitHub URL)
        source: String,

        /// List available skills without installing
        #[arg(long)]
        list: bool,

        /// Install to global directory (~/.config/helmsman/skills)
        #[arg(short, long)]
        global: bool,

        /// Install to project directory (.skills)
        #[arg(long)]
        local: bool,

        /// Specific skill(s) to install
        #[arg(short, long)]
        skill: Vec<String>,
    },

    /// Remove an installed skill
    Remove {
        /// Skill name to remove
        name: String,
    },

    /// Update installed skills
    Update {
        /// Specific skill to update (updates all if omitted)
        name: Option<String>,
    },

    /// Migrate static markdown to templates
    Migrate {
        /// Overwrite existing .j2 files
        #[arg(short, long)]
        force: bool,
    },

    /// Count tokens in files (or stdin)
    Tokens {
        /// Files to count (reads stdin if omitted)
        files: Vec<PathBuf>,

        /// Print total only (no per-file breakdown)
        #[arg(short, long)]
        total: bool,
    },

    /// Check if helmsman can see you (and whether you're Claude)
    Ping,

    /// Handle Claude Code lifecycle hook events
    Hook {
        /// Override hook event (for testing without piping JSON)
        #[arg(long, value_name = "EVENT")]
        event: Option<String>,

        /// Override model ID
        #[arg(short, long, value_name = "MODEL")]
        model: Option<String>,

        /// Dump raw hook JSON to file (for debugging payloads)
        #[arg(long, value_name = "FILE")]
        dump: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Handle subcommands first
    if let Some(command) = cli.command {
        return handle_command(command).await;
    }

    // Load configuration
    let config = Config::load()?;

    // Get templates directory (current dir by default for CLI)
    let templates_dir = config.templates_dir().unwrap_or_else(|_| ".".into());

    // Initialize template engine
    let engine = TemplateEngine::new(&templates_dir)?;

    // Create helmsman server
    let helmsman = HelmsmanServer::new(&config, engine);

    // CLI mode: list skills
    if cli.list {
        let skills = helmsman.skills();
        if skills.is_empty() {
            eprintln!("No skills found");
        } else {
            for skill in skills {
                println!("{}", skill);
            }
        }
        return Ok(());
    }

    // CLI mode: validate skills
    if cli.validate {
        let errors = helmsman.validate_skills();
        if errors.is_empty() {
            println!("✓ All skills valid");
        } else {
            eprintln!("Found {} invalid skill(s):", errors.len());
            for (skill, error) in errors {
                eprintln!("  ✗ {}: {}", skill, error);
            }
            std::process::exit(1);
        }
        return Ok(());
    }

    // CLI mode: skill
    if let Some(skill_name) = cli.skill {
        match helmsman.render_skill(&skill_name, &cli.model, None) {
            Ok(output) => {
                println!("{}", output);
                print_tokens_if_requested(cli.tokens, &output);
                return Ok(());
            }
            Err(e) => exit_error(e),
        }
    }

    // CLI mode: instructions
    if let Some(model_arg) = cli.instructions {
        let model_id = model_arg.unwrap_or(cli.model);

        // Handle --diff flag
        if let Some(ref other_tier) = cli.diff {
            // Get the tier for comparison (either from --tier override or resolved from model)
            let base_tier = cli.tier.as_deref().unwrap_or(&model_id);
            match helmsman.render_diff(&model_id, base_tier, other_tier) {
                Ok(diff) => {
                    println!("{}", diff);
                    return Ok(());
                }
                Err(e) => exit_error(e),
            }
        }

        // Render instructions (with optional --tier override)
        let result = if let Some(ref tier) = cli.tier {
            helmsman.render_instructions_with_tier(&model_id, tier, None)
        } else {
            helmsman.render_instructions(&model_id, None)
        };

        match result {
            Ok(instructions) => {
                println!("{}", instructions);
                print_tokens_if_requested(cli.tokens, &instructions);
                return Ok(());
            }
            Err(e) => exit_error(e),
        }
    }

    // MCP server mode
    let project_ctx_handle = helmsman.project_ctx_handle();
    let helmsman = Arc::new(helmsman);
    let server = mcp_host::server::builder::server(MCP_NAME, MCP_VERSION)
        .with_instructions("Resources: skill:/// (list), skill:///{name} (render, default model).")
        .with_prompts(true)
        .with_resources(true, false)
        .on_initialized(move |_session_id, requester| {
            let ctx_handle = project_ctx_handle.clone();
            async move {
                if let Some(req) = requester
                    && let Ok(roots) = req.request_roots(None).await
                {
                    let cwd = roots.first().and_then(|r| {
                        r.uri.strip_prefix("file://").map(String::from)
                    });
                    if let Ok(mut ctx) = ctx_handle.write() {
                        ctx.cwd = cwd;
                    }
                }
            }
        })
        .build();

    HelmsmanServer::router().register_all(
        server.tool_registry(),
        server.prompt_manager(),
        server.resource_manager(),
        helmsman,
    );
    server.run(StdioTransport::new()).await?;

    Ok(())
}

async fn handle_command(command: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Commands::Add {
            source,
            list,
            global,
            local,
            skill,
        } => {
            cmd_add(&source, list, global, local, &skill).await?;
        }
        Commands::Remove { name } => {
            cmd_remove(&name)?;
        }
        Commands::Update { name } => {
            cmd_update(name.as_deref())?;
        }
        Commands::Migrate { force } => {
            cmd_migrate(force)?;
        }
        Commands::Tokens { files, total } => {
            cmd_tokens(&files, total)?;
        }
        Commands::Ping => {
            anthropic::cmd_ping();
        }
        Commands::Hook { event, model, dump } => {
            anthropic::cmd_hook(event.as_deref(), model.as_deref(), dump.as_deref())?;
        }
    }
    Ok(())
}

/// Add skills from a remote repository.
async fn cmd_add(
    source: &str,
    list_only: bool,
    global: bool,
    local: bool,
    specific_skills: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse the source
    let parsed = ParsedSource::parse(source)?;
    println!("📦 Fetching from {}...", parsed.display_name());

    // Clone the repository
    let fetcher = RemoteFetcher::clone(&parsed)?;

    // Discover skills
    let skills = match fetcher.discover_skills(parsed.subpath.as_deref()) {
        Ok(skills) => skills,
        Err(remote::RemoteError::NoSkillsFound) => {
            eprintln!(
                "No skills found in repository (scanned {})",
                fetcher.temp_path().display()
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if skills.is_empty() {
        eprintln!("No skills found in repository");
        return Ok(());
    }

    // List mode: just show skills and exit
    if list_only {
        println!("\nAvailable skills:");
        for skill in &skills {
            if let Some(desc) = &skill.description {
                println!("  {} - {}", skill.name, desc);
            } else {
                println!("  {}", skill.name);
            }
        }
        return Ok(());
    }

    // Filter skills if specific ones requested
    let skills_to_install: Vec<_> = if specific_skills.is_empty() {
        skills
    } else {
        skills
            .into_iter()
            .filter(|s| specific_skills.contains(&s.name))
            .collect()
    };

    if skills_to_install.is_empty() {
        eprintln!("No matching skills found");
        return Ok(());
    }

    // Determine installation scope
    let install_global = if global {
        true
    } else if local {
        false
    } else {
        // Ask the user
        let options = &["Project (.skills/)", "Global (~/.config/helmsman/skills/)"];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Install to")
            .items(options)
            .default(0)
            .interact()?;
        selection == 1
    };

    // Get destination directory
    let mut registry = Registry::new()?;
    let dest_dir = skills_dest_dir(&registry, install_global);

    // Install skills
    println!(
        "\nInstalling {} skill(s) to {}...",
        skills_to_install.len(),
        dest_dir.display()
    );

    for skill in &skills_to_install {
        if registry.is_installed(&skill.name) {
            println!("  ↷ {} (already installed)", skill.name);
            continue;
        }

        let installed_path = fetcher.install_skill(skill, &dest_dir)?;
        registry.record_install(&skill.name, &parsed.display_name(), installed_path, install_global)?;
        println!("  ✓ {}", skill.name);
    }

    println!("\n✨ Done! Run 'helmsman -l' to see installed skills.");
    Ok(())
}

/// Remove an installed skill.
fn cmd_remove(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = Registry::new()?;

    // Check if skill is registered
    let entry = registry_entry_or_exit(&registry, name);

    // Delete the file
    if entry.path.exists() {
        std::fs::remove_file(&entry.path)?;
        println!("Removed {}", entry.path.display());
    }

    // Update registry
    registry.record_remove(name)?;
    println!("✓ Skill '{}' removed", name);

    Ok(())
}

/// Update installed skills.
fn cmd_update(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let registry = Registry::new()?;

    if let Some(skill_name) = name {
        let entry = registry_entry_or_exit(&registry, skill_name);
        println!("📦 Updating from {}...", entry.source);

        let parsed = ParsedSource::parse(&entry.source)?;
        let fetcher = RemoteFetcher::clone(&parsed)?;
        let dest_dir = skills_dest_dir(&registry, entry.global);

        match fetcher.get_skill(skill_name, parsed.subpath.as_deref()) {
            Ok(skill) => {
                fetcher.install_skill(&skill, &dest_dir)?;
                println!("  ✓ {}", skill_name);
            }
            Err(e) => {
                eprintln!("  ✗ {}: {}", skill_name, e);
            }
        }
    } else {
        let mut sources = std::collections::HashSet::new();
        for (_name, entry) in registry.list() {
            sources.insert(entry.source.clone());
        }

        if sources.is_empty() {
            println!("No skills to update");
            return Ok(());
        }

        for source in sources {
            let entries = registry.list_from_source(&source);
            if entries.is_empty() {
                continue;
            }

            println!("📦 Updating from {}...", source);

            let parsed = ParsedSource::parse(&source)?;
            let fetcher = RemoteFetcher::clone(&parsed)?;

            for (skill_name, entry) in entries {
                let dest_dir = skills_dest_dir(&registry, entry.global);

                match fetcher.get_skill(skill_name, parsed.subpath.as_deref()) {
                    Ok(skill) => {
                        fetcher.install_skill(&skill, &dest_dir)?;
                        println!("  ✓ {}", skill_name);
                    }
                    Err(e) => {
                        eprintln!("  ✗ {}: {}", skill_name, e);
                    }
                }
            }
        }
    }

    println!("\n✨ Update complete!");
    Ok(())
}

/// Migrate AGENTS.md, CLAUDE.md, and .skills/*.md to .j2 templates.
fn cmd_migrate(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Migrate root markdown files
    let root_files = ["AGENTS.md", "CLAUDE.md"];

    for src_name in root_files {
        let dst_name = format!("{}.j2", src_name);
        migrate_file(std::path::Path::new(src_name), std::path::Path::new(&dst_name), force)?;
    }

    // Migrate skills in .skills/ directory
    let skills_dir = std::path::Path::new(skills::SKILLS_DIR_NAME);
    if skills_dir.is_dir() {
        for entry in std::fs::read_dir(skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only process .md files
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let dst_path = path.with_extension("md.j2");
            migrate_file(&path, &dst_path, force)?;
        }
    }

    println!("\n✨ Migration complete!");
    Ok(())
}

/// Migrate a single file to .j2 template.
fn migrate_file(src: &std::path::Path, dst: &std::path::Path, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Skip if source doesn't exist
    if !src.exists() {
        return Ok(());
    }

    let src_display = src.display();
    let dst_display = dst.display();

    // Skip if source is a symlink
    if src.symlink_metadata()?.file_type().is_symlink() {
        eprintln!("  ↷ {} (symlink, skipped)", src_display);
        return Ok(());
    }

    // Skip if destination exists (unless force)
    if dst.exists() && !force {
        eprintln!("  ↷ {} exists (use -f to overwrite)", dst_display);
        return Ok(());
    }

    // Copy content
    let content = std::fs::read_to_string(src)?;
    std::fs::write(dst, content)?;
    println!("  ✓ {} → {}", src_display, dst_display);

    Ok(())
}

/// Count tokens in files or stdin.
fn cmd_tokens(files: &[PathBuf], total_only: bool) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    if files.is_empty() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        println!("{}", tokenizer::count_tokens(&buf));
        return Ok(());
    }

    let mut grand_total = 0usize;
    for path in files {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: {}", path.display(), e))?;
        let count = tokenizer::count_tokens(&content);
        grand_total += count;
        if !total_only {
            println!("{}\t{}", count, path.display());
        }
    }

    if total_only || files.len() > 1 {
        if total_only {
            println!("{}", grand_total);
        } else {
            println!("{}\ttotal", grand_total);
        }
    }

    Ok(())
}


fn print_tokens_if_requested(enabled: bool, text: &str) {
    if enabled {
        eprintln!("{} tokens", tokenizer::count_tokens(text));
    }
}

fn exit_error(error: impl std::fmt::Display) -> ! {
    eprintln!("Error: {}", error);
    std::process::exit(1);
}

fn exit_with_message(message: impl std::fmt::Display) -> ! {
    eprintln!("{}", message);
    std::process::exit(1);
}

fn registry_entry_or_exit(registry: &Registry, name: &str) -> SkillEntry {
    registry.get(name).cloned().unwrap_or_else(|| {
        exit_with_message(format!("Skill '{}' not found in registry", name))
    })
}

fn skills_dest_dir(registry: &Registry, global: bool) -> PathBuf {
    if global {
        registry.global_skills_dir().to_path_buf()
    } else {
        Registry::project_skills_dir()
    }
}
