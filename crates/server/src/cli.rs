use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Validation target for skills.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum ValidationTarget {
    /// Validate for Claude Code (permissive).
    Claude,
    /// Validate for Codex CLI (strict).
    Codex,
    /// Validate for both targets.
    #[default]
    Both,
}

/// Command-line interface for the `skrills` application.
#[derive(Debug, Parser)]
#[command(
    name = "skrills",
    about = "Skills support engine for Claude Code and Codex CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available `skrills` commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Runs as an MCP server over stdio.
    Serve {
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Cache TTL for skill discovery in milliseconds (overrides `SKRILLS_CACHE_TTL_MS`).
        #[arg(long, value_name = "MILLIS")]
        cache_ttl_ms: Option<u64>,
        /// Dumps raw MCP initialize traffic (stdin/stdout) as hex+UTF8 for debugging.
        #[arg(long, env = "SKRILLS_TRACE_WIRE", default_value_t = false)]
        trace_wire: bool,
        #[cfg(feature = "watch")]
        /// Watches filesystem for changes and invalidates caches immediately.
        #[arg(long, default_value_t = false)]
        watch: bool,
    },
    /// Lists discovered skills (debug).
    #[command(alias = "list-skills")]
    List,
    /// Lists pinned skills.
    ListPinned,
    /// Mirrors Claude assets (skills, agents, commands, MCP prefs) into Codex defaults and refreshes AGENTS.md.
    Mirror {
        /// Perform dry run (no file writes for commands/prefs; skills still hashed but not copied).
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Do not overwrite existing prompts under ~/.codex/prompts (only add new ones).
        #[arg(long, default_value_t = false)]
        skip_existing_commands: bool,
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
    },
    /// Launches a discovered agent by name using the stored run template.
    Agent {
        /// Agent name or unique substring to launch.
        #[arg(required = true)]
        agent: String,
        /// Additional agent directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Only print the resolved command without executing it.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Pins one or more skills by name (substring match allowed if unique).
    Pin {
        /// Skill names or unique substrings to pin.
        #[arg(required = true)]
        skills: Vec<String>,
    },
    /// Unpin specific skills or everything with `--all`.
    Unpin {
        /// Skill names or unique substrings to unpin (ignored when `--all` is set).
        skills: Vec<String>,
        /// Removes every pinned skill.
        #[arg(long)]
        all: bool,
    },
    /// Enables or disables heuristic auto-pinning.
    AutoPin {
        /// Sets to `true` to enable, `false` to disable.
        #[arg(long)]
        enable: bool,
    },
    /// Shows recent autoload match history.
    History {
        /// Limits number of entries shown (most recent first).
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Generates `<available_skills>` section in AGENTS.md for non-MCP agents.
    SyncAgents {
        /// Optional path to AGENTS.md (default: `./AGENTS.md`).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Emits hook JSON for autoload.
    EmitAutoload {
        /// Includes `~/.claude` skills in autoload output.
        #[arg(long, default_value_t = crate::runtime::env_include_claude_default())]
        include_claude: bool,
        /// Maximum bytes of `additionalContext` payload.
        #[arg(long)]
        max_bytes: Option<usize>,
        /// Prompt text to filter relevant skills (optional; uses env `SKRILLS_PROMPT` if not provided).
        #[arg(long)]
        prompt: Option<String>,
        /// Embedding similarity threshold (0-1) for fuzzy prompt matching.
        #[arg(long)]
        embed_threshold: Option<f32>,
        /// Enables heuristic auto-pinning based on recent prompt matches.
        #[arg(long, default_value_t = crate::runtime::env_auto_pin_default())]
        auto_pin: bool,
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Emits diagnostics (included skills + skips).
        #[arg(long, default_value_t = crate::runtime::env_diag_default())]
        diagnose: bool,
    },
    /// Copies skills from `~/.claude` into `~/.codex/skills-mirror`.
    Sync {
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
    },
    /// Syncs slash commands between Claude Code and Codex.
    SyncCommands {
        /// Source agent: "claude" or "codex".
        #[arg(long, default_value = "claude")]
        from: String,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
        /// Do not overwrite existing commands on target side.
        #[arg(long, default_value_t = false)]
        skip_existing_commands: bool,
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
    },
    /// Syncs MCP server configurations between Claude Code and Codex.
    SyncMcpServers {
        /// Source agent: "claude" or "codex".
        #[arg(long, default_value = "claude")]
        from: String,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Syncs preferences between Claude Code and Codex.
    SyncPreferences {
        /// Source agent: "claude" or "codex".
        #[arg(long, default_value = "claude")]
        from: String,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Syncs all configurations (commands, MCP servers, preferences, skills).
    SyncAll {
        /// Source agent: "claude" or "codex".
        #[arg(long, default_value = "claude")]
        from: String,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
        /// Do not overwrite existing commands on target side.
        #[arg(long, default_value_t = false)]
        skip_existing_commands: bool,
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
        /// Validate skills before syncing.
        #[arg(long)]
        validate: bool,
        /// Automatically fix validation issues (add frontmatter).
        #[arg(long)]
        autofix: bool,
    },
    /// Shows sync status and configuration differences.
    SyncStatus {
        /// Source agent: "claude" or "codex".
        #[arg(long, default_value = "claude")]
        from: String,
    },
    /// Diagnoses Codex MCP configuration for this server.
    Doctor,
    /// Validates skills for Claude Code and/or Codex CLI compatibility.
    Validate {
        /// Skills directory to validate (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Validation target: claude, codex, or both.
        #[arg(long, value_enum, default_value = "both")]
        target: ValidationTarget,
        /// Automatically fix validation issues (add frontmatter).
        #[arg(long)]
        autofix: bool,
        /// Create backup files before autofix.
        #[arg(long)]
        backup: bool,
        /// Output format: text or json.
        #[arg(long, default_value = "text")]
        format: String,
        /// Only show skills with errors.
        #[arg(long)]
        errors_only: bool,
    },
    /// Analyzes skills for token usage, dependencies, and optimization suggestions.
    Analyze {
        /// Skills directory to analyze (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, default_value = "text")]
        format: String,
        /// Only show skills exceeding this token count.
        #[arg(long)]
        min_tokens: Option<usize>,
        /// Include optimization suggestions.
        #[arg(long, default_value_t = true)]
        suggestions: bool,
    },
    /// Interactive TUI for sync and pin management.
    Tui {
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Sets up skrills for Claude Code or Codex (hooks, MCP, directories).
    Setup {
        /// Client to set up for (claude, codex, or both). If not specified, prompts interactively.
        #[arg(long)]
        client: Option<String>,
        /// Binary installation directory. If not specified, uses default or prompts.
        #[arg(long)]
        bin_dir: Option<PathBuf>,
        /// Reinstall/reconfigure existing setup.
        #[arg(long)]
        reinstall: bool,
        /// Uninstall skrills configuration (removes hooks, MCP registration).
        #[arg(long)]
        uninstall: bool,
        /// Add installation for an additional client (preserves existing setup).
        #[arg(long)]
        add: bool,
        /// Skip confirmation prompts (non-interactive mode).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Also sync skills to universal ~/.agent/skills directory for cross-agent reuse.
        #[arg(long)]
        universal: bool,
        /// Source directory for mirroring skills (default: ~/.claude).
        #[arg(long)]
        mirror_source: Option<PathBuf>,
    },
}
