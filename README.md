# Helmsman

Adaptive instruction server for AI coding agents.

## The Problem

Static `AGENTS.md` files create **instruction entropy collapse**.

The same instructions for Opus, Sonnet, and Haiku is fantasy. They have different capabilities, different costs, and different failure modes. Static instructions:

- **Rot silently** - written once, never updated, drift from reality
- **Waste tokens** - Opus doesn't need step-by-step guidance
- **Cause failures** - Haiku needs guardrails it doesn't get
- **Ignore context** - "use brew" when you're on Arch with mise

**The real cost isn't just tokens - it's instruction determinism.** You can't control what you can't adapt.

## The Solution

Helmsman serves **dynamic, context-aware instructions** via MCP (and CLI):

- **Model-aware**: Opus gets minimal guidance, Haiku gets verbose hand-holding
- **Environment-aware**: Detects OS, shell, available tools
- **Template-based**: Jinja2 templates adapt to context

```jinja
{% if model.tier == "agi" %}
Verify packages exist. You know what to do.
{% else %}
1. Read the file first with Read tool
2. Check for existing patterns
3. Verify packages exist - do NOT invent them
{% endif %}

{% if env.has_mise %}
Use mise for runtime management.
{% elif env.has_brew %}
Use brew for packages.
{% endif %}
```

## Install

```bash
cargo install helmsman
```

## Quick Start

1. Create `AGENTS.md.j2` in your project root

2. Add to `.mcp.json`:
```json
{
  "mcpServers": {
    "helmsman": {
      "type": "stdio",
      "command": "helmsman"
    }
  }
}
```

3. Call the prompt:
```
/helmsman:instructions claude-opus-4-5-20251101
```

## Tiers

Three capability tiers, parallel to Anthropic's model siblings. Could expand to 4-5, but we're not going the OpenAI route of 40 model names (nano, mini, micro, medium, large...).

### `monkey`
Follows instructions. Useful but needs guardrails. Tell it exactly what to do, what NOT to do, and keep it inside the perimeter. Without guidance, it will hallucinate packages and invent APIs.

*Examples: Haiku, GPT-5.2 mini, Gemini Flash*

### `engineer`
Knows the basics. Competent but lacks judgment. Will delete your 300GB cache to fix a bug. Bug fixed, but now you wait 4 hours for it to rebuild. Needs boundaries, not hand-holding.

*Examples: Sonnet, GPT-5.2 medium, Gemini Pro*

### `agi`
The architect. Don't explain how to use cargo or how to publish. It knows. Give it constraints and goals, not procedures. Wasting tokens on step-by-step instructions is burning money.

*Examples: Opus, GPT-5.2 high/xhigh, DeepSeek R3*

**Shortcuts:** `a`/`architect` (agi), `e`/`eng`/`standard` (engineer), `m`/`basic`/`simple` (monkey)

## CLI

```bash
helmsman                              # MCP server mode
helmsman -i                           # print instructions (default tier)
helmsman -i m                         # monkey tier
helmsman -i basic                     # monkey tier (neutral alias)
helmsman -i a                         # agi tier
helmsman -i architect                 # agi tier (neutral alias)
helmsman -i claude-opus-4-5-20251101  # resolves to agi tier
helmsman -i gpt-4o-mini               # resolves to monkey tier

# Override tier mapping for new/unknown models
helmsman -i unknown-model --tier engineer

# Show diff between tiers
helmsman -i a --diff e                # show AGI vs Engineer differences

helmsman -s commit                    # render .skills/commit.j2
helmsman -l                           # list available skills
helmsman --validate                   # check skill syntax
helmsman -t                           # show token count
```

## Template Context

```jinja
{# Model #}
{{ model.id }}        {# "claude-opus-4-5-20251101" #}
{{ model.tier }}      {# "agi", "engineer", "monkey" #}

{# Environment #}
{{ env.os }}          {# "macos", "arch", "debian", "alpine" #}
{{ env.shell }}       {# "zsh", "bash", "fish", "sh" #}
{{ env.in_docker }}   {# true/false #}
{{ env.in_ssh }}      {# true/false #}

{# Tools #}
{{ env.has_mise }}
{{ env.has_brew }}
{{ env.has_apt }}
{{ env.has_gh }}
{{ env.has_git }}
```

## Configuration

Create optional `helmsman.toml` in:
1. Current directory (project-local)
2. `~/.config/helmsman/` (user global)
3. Set `$HELMSMAN_CONFIG` env var to override

```toml
[defaults]
tier = "engineer"

[server]
templates_dir = "~/my-templates"
```

Model → tier mappings are pre-configured for Anthropic, OpenAI, Google, and other major models. Unknown models default to `engineer` tier.

## Skills

Project skills live in `.skills/` and are discovered automatically. Files prefixed with `_` are partials.

## Environment Detection

Helmsman detects OS, shell, and available tools automatically. **Best effort only, never authoritative.**

Known edge cases:
- SSH + Docker may report wrong shell (`$SHELL` lies)
- Alpine/busybox may lack expected binaries
- Container detection uses heuristics (cgroup parsing)

Use these values for optimization hints, not hard requirements.

## Non-Goals

Things Helmsman deliberately doesn't do:

- **Prompt engineering framework** - not here to optimize your prompts
- **Model memory/learning** - stateless, no persistence between calls
- **Teaching tool** - assumes you know what you're doing
- **Configuration management** - use real tools for that

Helmsman is infrastructure, not a product.

## License

BSD-3-Clause
