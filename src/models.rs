//! Model identification and tier resolution.

use crate::config::Config;
use glob::Pattern;
use std::cmp::Ordering;

pub const DEFAULT_MODEL_ID: &str = "claude-4-5-sonnet";

/// Resolves model IDs to capability tiers.
pub struct ModelResolver {
    /// Compiled patterns: (pattern, tier_name)
    patterns: Vec<PatternEntry>,
    default_tier: String,
}

struct PatternEntry {
    pattern: Pattern,
    tier: String,
    raw: String,
}

impl ModelResolver {
    /// Create a new resolver from config.
    pub fn new(config: &Config) -> Self {
        let mut patterns: Vec<PatternEntry> = config
            .models
            .iter()
            .filter_map(|(pattern, tier)| {
                Pattern::new(pattern).ok().map(|p| PatternEntry {
                    pattern: p,
                    tier: tier.clone(),
                    raw: pattern.clone(),
                })
            })
            .collect();

        // Ensure deterministic precedence: more specific patterns match first.
        // This prevents broad fallbacks (e.g., "gpt-5.4*") from overriding
        // more specific entries like "gpt-5.4-xhigh*".
        patterns.sort_by(|a, b| pattern_precedence(&a.raw, &b.raw));

        Self {
            patterns,
            default_tier: config.defaults.tier.clone(),
        }
    }

    /// Resolve a model ID to its tier name.
    ///
    /// Supports canonical tier names plus neutral aliases:
    /// - AGI: `a`, `agi`, `architect`
    /// - Engineer: `e`, `eng`, `engineer`, `standard`
    /// - Monkey: `m`, `monkey`, `basic`, `simple`
    ///
    /// Aliases are hardcoded for ecosystem consistency - shared skills
    /// always use canonical names (agi/engineer/monkey).
    pub fn resolve(&self, model_id: &str) -> &str {
        // Built-in tier aliases for quick access
        // Canonical names + neutral aliases (hardcoded for skill portability)
        match model_id {
            "a" | "agi" | "architect" => return "agi",
            "e" | "eng" | "engineer" | "standard" => return "engineer",
            "m" | "monkey" | "basic" | "simple" => return "monkey",
            _ => {}
        }

        for entry in &self.patterns {
            if entry.pattern.matches(model_id) {
                return &entry.tier;
            }
        }
        &self.default_tier
    }
}

fn pattern_precedence(a: &str, b: &str) -> Ordering {
    let a_wc = wildcard_count(a);
    let b_wc = wildcard_count(b);

    a_wc.cmp(&b_wc) // fewer wildcards first
        .then_with(|| b.len().cmp(&a.len())) // longer patterns first
        .then_with(|| a.cmp(b)) // stable tie-breaker
}

fn wildcard_count(pattern: &str) -> usize {
    pattern.chars().filter(|ch| matches!(ch, '*' | '?')).count()
}

/// Model context passed to templates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelContext {
    pub id: String,
    pub tier: String,
}

impl ModelContext {
    /// Create model context from ID and tier name.
    pub fn new(model_id: &str, tier_name: &str) -> Self {
        Self {
            id: model_id.to_string(),
            tier: tier_name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
