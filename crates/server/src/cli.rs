use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use indexmap::IndexMap;
use std::path::PathBuf;

/// Validation target for skills.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum ValidationTarget {
    /// Validate for Claude Code (permissive).
    Claude,
    /// Validate for Codex CLI (strict).
    Codex,
    /// Validate for GitHub Copilot CLI (strict).
    Copilot,
    /// Validate for all targets (Claude, Codex, and Copilot).
    #[default]
    All,
    /// Validate for Claude and Codex (legacy, use 'all' for new code).
    Both,
}

/// Source/target for sync operations.
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
pub enum SyncSource {
    /// Claude Code CLI.
    #[default]
    Claude,
    /// OpenAI Codex CLI.
    Codex,
    /// GitHub Copilot CLI.
    Copilot,
    /// Cursor IDE.
    Cursor,
}

impl SyncSource {
    /// Returns the default target for a given source.
    /// Claude → Codex, Codex/Copilot/Cursor → Claude.
    pub fn default_target(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex | Self::Copilot | Self::Cursor => Self::Claude,
        }
    }

    /// Returns true if this source is Claude.
    pub fn is_claude(self) -> bool {
        matches!(self, Self::Claude)
    }

    /// Returns true if this source is Codex.
    pub fn is_codex(self) -> bool {
        matches!(self, Self::Codex)
    }

    /// Returns true if this source is Copilot.
    pub fn is_copilot(self) -> bool {
        matches!(self, Self::Copilot)
    }

    /// Returns true if this source is Cursor.
    pub fn is_cursor(self) -> bool {
        matches!(self, Self::Cursor)
    }

    /// Returns the string name for this source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
        }
    }

    /// Returns all other targets (CLIs other than this one).
    /// Used when `--to` is not specified to sync to all other CLIs.
    pub fn other_targets(self) -> Vec<Self> {
        match self {
            Self::Claude => vec![Self::Codex, Self::Copilot, Self::Cursor],
            Self::Codex => vec![Self::Claude, Self::Copilot, Self::Cursor],
            Self::Copilot => vec![Self::Claude, Self::Codex, Self::Cursor],
            Self::Cursor => vec![Self::Claude, Self::Codex, Self::Copilot],
        }
    }
}

/// Dependency traversal direction.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DependencyDirection {
    /// Resolve dependencies (what this skill needs).
    Dependencies,
    /// Resolve dependents (what uses this skill).
    Dependents,
}

/// Backend for multi-CLI agent routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Default)]
pub enum AgentBackend {
    /// Auto-detect the best available backend.
    #[default]
    Auto,
    /// Use Claude Code CLI.
    Claude,
    /// Use Codex CLI.
    Codex,
}

/// CLI binary names to probe for availability.
const CLAUDE_BINS: &[&str] = &["claude"];
const CODEX_BINS: &[&str] = &["codex"];

impl AgentBackend {
    /// Return a human-readable name for the backend.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentBackend::Auto => "auto",
            AgentBackend::Claude => "claude",
            AgentBackend::Codex => "codex",
        }
    }

    /// Return an ordered map of backends to try, with this preference first.
    ///
    /// Iteration order encodes fallback priority.
    pub fn backends(self) -> IndexMap<AgentBackend, &'static [&'static str]> {
        match self {
            AgentBackend::Claude | AgentBackend::Auto => IndexMap::from([
                (AgentBackend::Claude, CLAUDE_BINS),
                (AgentBackend::Codex, CODEX_BINS),
            ]),
            AgentBackend::Codex => IndexMap::from([
                (AgentBackend::Codex, CODEX_BINS),
                (AgentBackend::Claude, CLAUDE_BINS),
            ]),
        }
    }
}

/// Creation method for new skills.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CreateSkillMethod {
    /// Search GitHub for existing skills.
    Github,
    /// Generate skill content via LLM.
    Llm,
    /// Use both GitHub search and LLM generation.
    Both,
    /// Generate from empirical session patterns.
    Empirical,
}

impl CreateSkillMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Llm => "llm",
            Self::Both => "both",
            Self::Empirical => "empirical",
        }
    }
}

impl std::fmt::Display for CreateSkillMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Certificate management action.
#[derive(Debug, Clone, Subcommand)]
pub enum CertAction {
    /// Show certificate status and expiry information.
    Status {
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Renew or regenerate self-signed certificate.
    Renew {
        /// Force renewal even if not expiring.
        #[arg(long)]
        force: bool,
    },
    /// Install a certificate from external source.
    Install {
        /// Path to certificate file (PEM format).
        cert: PathBuf,
        /// Path to private key file (PEM format).
        #[arg(long)]
        key: Option<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

/// Output format for command results.
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for machine parsing.
    Json,
}

impl OutputFormat {
    /// Check if this format is JSON.
    pub fn is_json(&self) -> bool {
        matches!(self, Self::Json)
    }
}

/// Command-line interface for the `skrills` application.
#[derive(Debug, Parser)]
#[command(
    name = "skrills",
    about = "Skills support engine for Claude Code and Codex CLI",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available `skrills` commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Runs as an MCP server over stdio (default) or HTTP.
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
        /// Bind address for HTTP transport (e.g., "0.0.0.0:3000" or "127.0.0.1:8080").
        /// When specified, serves MCP over HTTP instead of stdio.
        /// Requires the `http-transport` feature (enabled by default).
        #[arg(long, value_name = "BIND_ADDR")]
        http: Option<String>,

        // --- Phase 2 Security Options ---
        /// Bearer token for HTTP authentication. Validates `Authorization: Bearer <token>`.
        /// If not specified, auth is disabled (Phase 1 behavior).
        #[arg(long, value_name = "TOKEN", env = "SKRILLS_AUTH_TOKEN")]
        auth_token: Option<String>,
        /// Path to TLS certificate file (PEM format). Requires --tls-key.
        #[arg(
            long,
            value_name = "PATH",
            env = "SKRILLS_TLS_CERT",
            requires = "tls_key"
        )]
        tls_cert: Option<std::path::PathBuf>,
        /// Path to TLS private key file (PEM format). Requires --tls-cert.
        #[arg(
            long,
            value_name = "PATH",
            env = "SKRILLS_TLS_KEY",
            requires = "tls_cert"
        )]
        tls_key: Option<std::path::PathBuf>,
        /// Comma-separated list of allowed CORS origins (e.g., `http://localhost:3000,https://app.example.com`).
        /// Default: no CORS (server-to-server only).
        #[arg(
            long,
            value_name = "ORIGINS",
            env = "SKRILLS_CORS_ORIGINS",
            value_delimiter = ','
        )]
        cors_origins: Vec<String>,
        /// Auto-generate self-signed TLS certificate for development.
        /// Stores certificate in ~/.skrills/tls/. Overrides --tls-cert and --tls-key.
        #[arg(long, env = "SKRILLS_TLS_AUTO")]
        tls_auto: bool,

        /// List all available MCP tools and exit.
        #[arg(long, default_value_t = false)]
        list_tools: bool,

        /// Automatically open the dashboard in the default browser after starting
        /// the HTTP server. Requires `--http` because there is no browser endpoint
        /// to open in stdio-only mode.
        #[arg(long, default_value_t = false, requires = "http")]
        open: bool,
    },
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
    /// Launches an agent with multi-CLI backend routing.
    ///
    /// Tries the primary backend first and falls back to alternatives if
    /// the primary is unavailable. Supports claude, codex, and auto-detect.
    MultiCliAgent {
        /// Agent name or unique substring to launch.
        #[arg(required = true)]
        agent: String,
        ///// Primary backend: auto, claude, or codex (default: auto).
        #[arg(long, value_enum, default_value_t = AgentBackend::Auto)]
        backend: AgentBackend,
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Only print the resolved command without executing it.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
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
    /// Copies skills from `~/.claude` into `~/.codex/skills` (Codex discovery root).
    #[command(alias = "sync-from-claude")]
    Sync {
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
    },
    /// Syncs slash commands between AI CLI tools (Claude, Codex, Copilot, Cursor).
    SyncCommands {
        /// Source CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        from: SyncSource,
        /// Target CLI: claude, codex, copilot, or cursor. If not specified, defaults to codex (for claude source) or claude (for others).
        #[arg(long, value_enum)]
        to: Option<SyncSource>,
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
    /// Syncs MCP server configurations between AI CLI tools.
    SyncMcpServers {
        /// Source CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        from: SyncSource,
        /// Target CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum)]
        to: Option<SyncSource>,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Syncs preferences between AI CLI tools.
    SyncPreferences {
        /// Source CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        from: SyncSource,
        /// Target CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum)]
        to: Option<SyncSource>,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Syncs all configurations (commands, MCP servers, preferences, skills).
    SyncAll {
        /// Source CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        from: SyncSource,
        /// Target CLI: claude, codex, copilot, or cursor. If omitted, syncs to ALL other CLIs.
        #[arg(long, value_enum)]
        to: Option<SyncSource>,
        /// Preview changes without writing.
        #[arg(long)]
        dry_run: bool,
        /// Do not overwrite existing commands on target side.
        #[arg(long, default_value_t = false)]
        skip_existing_commands: bool,
        /// Include marketplace content (uninstalled plugins).
        #[arg(long, env = "SKRILLS_INCLUDE_MARKETPLACE", default_value_t = false)]
        include_marketplace: bool,
        /// Exclude plugins by name (comma-separated). Skills and assets from
        /// these plugins will not be synced to the target.
        #[arg(long, value_delimiter = ',', env = "SKRILLS_EXCLUDE_PLUGINS")]
        exclude_plugins: Vec<String>,
        /// Validate skills before syncing.
        #[arg(long)]
        validate: bool,
        /// Automatically fix validation issues (add frontmatter).
        #[arg(long)]
        autofix: bool,
    },
    /// Shows sync status and configuration differences.
    SyncStatus {
        /// Source CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        from: SyncSource,
        /// Target CLI: claude, codex, copilot, or cursor.
        #[arg(long, value_enum)]
        to: Option<SyncSource>,
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
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Only show skills with errors.
        #[arg(long)]
        errors_only: bool,
        #[cfg(feature = "watch")]
        /// Watch skill directories for changes and auto-revalidate.
        #[arg(long, default_value_t = false)]
        watch: bool,
        #[cfg(feature = "watch")]
        /// Debounce interval in milliseconds for watch mode.
        #[arg(long, default_value_t = 300)]
        debounce_ms: u64,
    },
    /// Analyzes skills for token usage, dependencies, and optimization suggestions.
    Analyze {
        /// Skills directory to analyze (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Only show skills exceeding this token count.
        #[arg(long)]
        min_tokens: Option<usize>,
        /// Include optimization suggestions.
        #[arg(long, default_value_t = true)]
        suggestions: bool,
    },
    /// Shows aggregate statistics about discovered skills.
    Metrics {
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Include validation summary (slower).
        #[arg(long)]
        include_validation: bool,
    },
    /// Recommends related skills based on dependency relationships.
    Recommend {
        /// Skill URI to get recommendations for.
        #[arg(required = true)]
        uri: String,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Maximum number of recommendations.
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Include quality scores in recommendations.
        #[arg(long, default_value_t = true)]
        include_quality: bool,
    },
    /// Resolve dependencies or dependents for a skill URI.
    ResolveDependencies {
        /// Skill URI to resolve.
        #[arg(required = true)]
        uri: String,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Direction to traverse: dependencies or dependents.
        #[arg(long, value_enum, default_value = "dependencies")]
        direction: DependencyDirection,
        /// Include transitive relationships.
        #[arg(long, default_value_t = true, value_parser = clap::builder::BoolishValueParser::new(), action = ArgAction::Set)]
        transitive: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Smart skill recommendations using dependencies, usage patterns, and project context.
    RecommendSkillsSmart {
        /// Optional skill URI for relationship-based recommendations.
        #[arg(long)]
        uri: Option<String>,
        /// Optional prompt text for semantic matching.
        #[arg(long)]
        prompt: Option<String>,
        /// Project directory for context analysis.
        #[arg(long)]
        project_dir: Option<PathBuf>,
        /// Maximum recommendations to return.
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Include usage pattern analysis.
        #[arg(long, default_value_t = true, value_parser = clap::builder::BoolishValueParser::new(), action = ArgAction::Set)]
        include_usage: bool,
        /// Include project context analysis.
        #[arg(long, default_value_t = true, value_parser = clap::builder::BoolishValueParser::new(), action = ArgAction::Set)]
        include_context: bool,
        /// Auto-persist analytics to cache after building.
        /// Can also be enabled via SKRILLS_AUTO_PERSIST=1 environment variable.
        #[arg(long)]
        auto_persist: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Analyze project context for recommendations.
    AnalyzeProjectContext {
        /// Project directory to analyze (defaults to cwd).
        #[arg(long)]
        project_dir: Option<PathBuf>,
        /// Include git commit keyword analysis.
        #[arg(long, default_value_t = true, value_parser = clap::builder::BoolishValueParser::new(), action = ArgAction::Set)]
        include_git: bool,
        /// Number of recent commits to analyze.
        #[arg(long, default_value = "50")]
        commit_limit: usize,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Suggest new skills to create based on project context.
    #[command(alias = "suggest-skills")]
    SuggestNewSkills {
        /// Project directory for context analysis.
        #[arg(long)]
        project_dir: Option<PathBuf>,
        /// Specific areas to focus on (repeatable).
        #[arg(long = "focus-area")]
        focus_areas: Vec<String>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Create a new skill via GitHub search, LLM generation, empirical patterns, or both.
    CreateSkill {
        /// Name or topic for the skill.
        #[arg(required = true)]
        name: String,
        /// Detailed description of what the skill should do.
        #[arg(long)]
        description: String,
        /// Creation method: github, llm, both, or empirical.
        #[arg(long, default_value = "both", value_enum)]
        method: CreateSkillMethod,
        /// Directory to create skill in (defaults to installed client, Claude preferred).
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// Project directory for context analysis.
        #[arg(long)]
        project_dir: Option<PathBuf>,
        /// Preview without creating files.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Search GitHub for existing SKILL.md files.
    SearchSkillsGithub {
        /// Search query for skills.
        #[arg(required = true)]
        query: String,
        /// Maximum results to return.
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Fuzzy search installed skills by name or description.
    SearchSkills {
        /// Search query for skills.
        #[arg(required = true)]
        query: String,
        /// Similarity threshold (0.0-1.0, lower = more results).
        #[arg(long, short = 't', default_value = "0.3")]
        threshold: f64,
        /// Maximum results to return.
        #[arg(long, short = 'l', default_value = "10")]
        limit: usize,
        /// Also search descriptions, not just names.
        #[arg(long, default_value = "true")]
        include_description: bool,
        /// Skills directory to search (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Export usage analytics to a file for persistence or backup.
    ExportAnalytics {
        /// Output file path (defaults to ~/.skrills/analytics_cache.json).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Rebuild analytics from session data even if cache exists.
        #[arg(long)]
        force_rebuild: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Import usage analytics from a previously exported file.
    ImportAnalytics {
        /// Input file path (required).
        #[arg(required = true)]
        input: PathBuf,
        /// Overwrite existing cache file.
        #[arg(long)]
        overwrite: bool,
    },
    /// Interactive TUI for sync and pin management.
    Tui {
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Real-time terminal dashboard for skills monitoring.
    Dashboard {
        /// Additional skill directories (repeatable).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Compare a skill across Claude, Codex, Copilot, and Cursor to show differences.
    SkillDiff {
        /// Skill name to compare (e.g., "commit", "review-pr").
        #[arg(required = true)]
        name: String,
        /// Output format: text (unified diff), json, or summary.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Show context lines around differences.
        #[arg(long, short = 'C', default_value = "3")]
        context: usize,
    },
    /// Sets up skrills for Claude Code, Codex, Copilot, or Cursor (hooks, MCP, directories).
    Setup {
        /// Client to set up for (claude, codex, copilot, cursor, or all). If not specified, prompts interactively.
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
    /// Mark a skill as deprecated with optional migration guidance.
    SkillDeprecate {
        /// Skill name to mark as deprecated.
        #[arg(required = true)]
        name: String,
        /// Deprecation message explaining why.
        #[arg(long)]
        message: Option<String>,
        /// Suggested replacement skill.
        #[arg(long)]
        replacement: Option<String>,
        /// Skills directory to search (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Revert a skill to a previous version.
    SkillRollback {
        /// Skill name to rollback.
        #[arg(required = true)]
        name: String,
        /// Specific version hash to rollback to (if not specified, shows available versions).
        #[arg(long)]
        version: Option<String>,
        /// Skills directory to search (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Pull skill updates from remote sources.
    SyncPull {
        /// Remote source URL (git repo, HTTP endpoint, or registry).
        #[arg(long)]
        source: Option<String>,
        /// Specific skill name to pull (if omitted, pulls all from source).
        #[arg(long)]
        skill: Option<String>,
        /// Target CLI to pull skills into.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        target: SyncSource,
        /// Preview changes without applying.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// View skill execution statistics and performance metrics.
    SkillProfile {
        /// Skill name to profile (if omitted, shows overall stats).
        name: Option<String>,
        /// Time period in days to analyze.
        #[arg(long, default_value = "30")]
        period: u32,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Browse and search all available skills across configured sources.
    SkillCatalog {
        /// Search query to filter skills.
        #[arg(long)]
        search: Option<String>,
        /// Filter by source CLI.
        #[arg(long, value_enum)]
        source: Option<SyncSource>,
        /// Filter by category tag.
        #[arg(long)]
        category: Option<String>,
        /// Maximum results to return.
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Validate skill files for git pre-commit hook integration.
    PreCommitValidate {
        /// Only validate staged files (auto-detects via git status).
        #[arg(long)]
        staged: bool,
        /// Validation target: claude, codex, copilot, or all.
        #[arg(long, value_enum, default_value = "all")]
        target: ValidationTarget,
        /// Skills directory to validate (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Import skills from external sources (URLs, git repos, local paths).
    SkillImport {
        /// Source to import from (URL, git URL, or local path).
        #[arg(required = true)]
        source: String,
        /// Target CLI to import skill into.
        #[arg(long, value_enum, default_value_t = SyncSource::Claude)]
        target: SyncSource,
        /// Force overwrite if skill already exists.
        #[arg(long)]
        force: bool,
        /// Preview import without writing files.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Generate detailed usage reports for skills.
    SkillUsageReport {
        /// Time period in days to analyze.
        #[arg(long, default_value = "30")]
        period: u32,
        /// Output format: text, json, html, or markdown.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Output file path (if not specified, prints to stdout).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Calculate quality scores for skills based on validation, completeness, and metrics.
    SkillScore {
        /// Skill name to score (if omitted, scores all discovered skills).
        name: Option<String>,
        /// Skills directory to include (default: all discovered skills).
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Only show skills below this score threshold.
        #[arg(long)]
        below_threshold: Option<u8>,
    },
    /// Manage TLS certificates for HTTPS transport.
    #[command(subcommand)]
    Cert(CertAction),
    /// Run the cold-window real-time analysis surface (TUI library + browser).
    ///
    /// Continuously re-reads authoritative state on each tick, runs a
    /// 4-tier alert policy, ranks hints, and (with `--browser`)
    /// serves a Server-Sent Events dashboard at
    /// `http://127.0.0.1:<port>/dashboard`. Press Ctrl-C for graceful
    /// shutdown within the spec § 3 / TASK-031 2-second budget.
    #[cfg(feature = "http-transport")]
    ColdWindow(crate::cold_window_cli::ColdWindowArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    use crate::test_support::{env_guard, set_env_var};

    #[test]
    fn parse_defaults_to_serve_when_no_subcommand() {
        let cli = Cli::try_parse_from(["skrills"]).expect("default parse should succeed");
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_serve_arguments() {
        let _guard = env_guard();
        // Ensure env var is cleared so default (None) is used
        let _env = set_env_var("SKRILLS_AUTH_TOKEN", None);

        let cli = Cli::try_parse_from([
            "skrills",
            "serve",
            "--skill-dir",
            "/tmp/skills",
            "--cache-ttl-ms",
            "1500",
            "--trace-wire",
        ])
        .expect("serve args should parse");

        match cli.command {
            Some(Commands::Serve {
                skill_dirs,
                cache_ttl_ms,
                trace_wire,
                #[cfg(feature = "watch")]
                watch,
                http,
                list_tools,
                auth_token,
                tls_cert,
                tls_key,
                cors_origins,
                tls_auto,
                open,
            }) => {
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
                assert_eq!(cache_ttl_ms, Some(1500));
                assert!(trace_wire);
                #[cfg(feature = "watch")]
                assert!(!watch);
                assert!(http.is_none());
                assert!(!list_tools);
                assert!(auth_token.is_none());
                assert!(tls_cert.is_none());
                assert!(tls_key.is_none());
                assert!(cors_origins.is_empty());
                assert!(!tls_auto);
                assert!(!open);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_with_security_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "serve",
            "--http",
            "127.0.0.1:3000",
            "--auth-token",
            "secret-token",
            "--tls-cert",
            "/path/to/cert.pem",
            "--tls-key",
            "/path/to/key.pem",
            "--cors-origins",
            "http://localhost:3000,https://app.example.com",
        ])
        .expect("serve with security args should parse");

        match cli.command {
            Some(Commands::Serve {
                http,
                auth_token,
                tls_cert,
                tls_key,
                cors_origins,
                ..
            }) => {
                assert_eq!(http, Some("127.0.0.1:3000".to_string()));
                assert_eq!(auth_token, Some("secret-token".to_string()));
                assert_eq!(tls_cert, Some(PathBuf::from("/path/to/cert.pem")));
                assert_eq!(tls_key, Some(PathBuf::from("/path/to/key.pem")));
                assert_eq!(
                    cors_origins,
                    vec![
                        "http://localhost:3000".to_string(),
                        "https://app.example.com".to_string()
                    ]
                );
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_auth_token_from_env() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_AUTH_TOKEN", Some("env-secret-token"));

        let cli = Cli::try_parse_from(["skrills", "serve", "--http", "0.0.0.0:8080"])
            .expect("serve with env auth token should parse");

        match cli.command {
            Some(Commands::Serve { auth_token, .. }) => {
                assert_eq!(auth_token, Some("env-secret-token".to_string()));
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_with_tls_auto() {
        let cli =
            Cli::try_parse_from(["skrills", "serve", "--http", "127.0.0.1:3000", "--tls-auto"])
                .expect("serve with tls-auto should parse");

        match cli.command {
            Some(Commands::Serve { tls_auto, .. }) => {
                assert!(tls_auto);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_tls_paths_from_env() {
        let _guard = env_guard();
        let _cert_env = set_env_var("SKRILLS_TLS_CERT", Some("/env/cert.pem"));
        let _key_env = set_env_var("SKRILLS_TLS_KEY", Some("/env/key.pem"));

        let cli = Cli::try_parse_from(["skrills", "serve", "--http", "0.0.0.0:8080"])
            .expect("serve with env TLS paths should parse");

        match cli.command {
            Some(Commands::Serve {
                tls_cert, tls_key, ..
            }) => {
                assert_eq!(tls_cert, Some(PathBuf::from("/env/cert.pem")));
                assert_eq!(tls_key, Some(PathBuf::from("/env/key.pem")));
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_cors_from_env() {
        let _guard = env_guard();
        let _cors_env = set_env_var(
            "SKRILLS_CORS_ORIGINS",
            Some("http://localhost:3000,https://example.com"),
        );

        let cli = Cli::try_parse_from(["skrills", "serve", "--http", "0.0.0.0:8080"])
            .expect("serve with env CORS should parse");

        match cli.command {
            Some(Commands::Serve { cors_origins, .. }) => {
                assert_eq!(
                    cors_origins,
                    vec![
                        "http://localhost:3000".to_string(),
                        "https://example.com".to_string()
                    ]
                );
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn sync_uses_env_include_marketplace_default() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", Some("true"));

        let cli = Cli::try_parse_from(["skrills", "sync"]).expect("sync should parse with env");

        match cli.command {
            Some(Commands::Sync {
                include_marketplace,
            }) => assert!(include_marketplace),
            _ => panic!("expected Sync command"),
        }
    }

    #[test]
    fn sync_from_claude_alias_parses() {
        let _guard = env_guard();
        // Ensure env var is cleared so default is used
        let _env = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", None);

        let cli = Cli::try_parse_from(["skrills", "sync-from-claude"]).expect("alias should parse");

        match cli.command {
            Some(Commands::Sync {
                include_marketplace,
            }) => assert!(!include_marketplace),
            _ => panic!("expected Sync command"),
        }
    }

    #[test]
    #[should_panic]
    fn validate_rejects_invalid_target() {
        let _guard = env_guard();
        let _ = Cli::try_parse_from(["skrills", "validate", "--target", "invalid"]).unwrap();
    }

    #[test]
    fn parse_resolve_dependencies_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "resolve-dependencies",
            "skill://skrills/codex/test-skill",
            "--direction",
            "dependents",
            "--transitive",
            "false",
            "--format",
            "json",
            "--skill-dir",
            "/tmp/skills",
        ])
        .expect("resolve-dependencies args should parse");

        match cli.command {
            Some(Commands::ResolveDependencies {
                uri,
                direction,
                transitive,
                format,
                skill_dirs,
            }) => {
                assert_eq!(uri, "skill://skrills/codex/test-skill");
                assert!(matches!(direction, DependencyDirection::Dependents));
                assert!(!transitive);
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
            }
            _ => panic!("expected ResolveDependencies command"),
        }
    }

    #[test]
    fn parse_recommend_skills_smart_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "recommend-skills-smart",
            "--uri",
            "skill://skrills/codex/test-skill",
            "--prompt",
            "testing workflow",
            "--project-dir",
            "/tmp/project",
            "--limit",
            "5",
            "--include-usage",
            "false",
            "--include-context",
            "true",
            "--format",
            "json",
            "--skill-dir",
            "/tmp/skills",
        ])
        .expect("recommend-skills-smart args should parse");

        match cli.command {
            Some(Commands::RecommendSkillsSmart {
                uri,
                prompt,
                project_dir,
                limit,
                include_usage,
                include_context,
                auto_persist,
                format,
                skill_dirs,
            }) => {
                assert_eq!(uri.as_deref(), Some("skill://skrills/codex/test-skill"));
                assert_eq!(prompt.as_deref(), Some("testing workflow"));
                assert_eq!(project_dir, Some(PathBuf::from("/tmp/project")));
                assert_eq!(limit, 5);
                assert!(!include_usage);
                assert!(include_context);
                assert!(!auto_persist); // Default is false
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
            }
            _ => panic!("expected RecommendSkillsSmart command"),
        }
    }

    #[test]
    fn parse_recommend_skills_smart_with_auto_persist() {
        let cli = Cli::try_parse_from([
            "skrills",
            "recommend-skills-smart",
            "--auto-persist",
            "--prompt",
            "test query",
        ])
        .expect("recommend-skills-smart with --auto-persist should parse");

        match cli.command {
            Some(Commands::RecommendSkillsSmart { auto_persist, .. }) => {
                assert!(auto_persist, "--auto-persist flag should be true");
            }
            _ => panic!("expected RecommendSkillsSmart command"),
        }
    }

    #[test]
    fn parse_analyze_project_context_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "analyze-project-context",
            "--project-dir",
            "/tmp/project",
            "--include-git",
            "false",
            "--commit-limit",
            "25",
            "--format",
            "json",
        ])
        .expect("analyze-project-context args should parse");

        match cli.command {
            Some(Commands::AnalyzeProjectContext {
                project_dir,
                include_git,
                commit_limit,
                format,
            }) => {
                assert_eq!(project_dir, Some(PathBuf::from("/tmp/project")));
                assert!(!include_git);
                assert_eq!(commit_limit, 25);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected AnalyzeProjectContext command"),
        }
    }

    #[test]
    fn parse_suggest_new_skills_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "suggest-new-skills",
            "--project-dir",
            "/tmp/project",
            "--focus-area",
            "testing",
            "--focus-area",
            "deployment",
            "--format",
            "json",
            "--skill-dir",
            "/tmp/skills",
        ])
        .expect("suggest-new-skills args should parse");

        match cli.command {
            Some(Commands::SuggestNewSkills {
                project_dir,
                focus_areas,
                format,
                skill_dirs,
            }) => {
                assert_eq!(project_dir, Some(PathBuf::from("/tmp/project")));
                assert_eq!(
                    focus_areas,
                    vec!["testing".to_string(), "deployment".to_string()]
                );
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
            }
            _ => panic!("expected SuggestNewSkills command"),
        }
    }

    #[test]
    fn parse_create_skill_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "create-skill",
            "audit-skill",
            "--description",
            "Audit build outputs",
            "--method",
            "github",
            "--target-dir",
            "/tmp/skills",
            "--project-dir",
            "/tmp/project",
            "--dry-run",
            "--format",
            "json",
        ])
        .expect("create-skill args should parse");

        match cli.command {
            Some(Commands::CreateSkill {
                name,
                description,
                method,
                target_dir,
                project_dir,
                dry_run,
                format,
            }) => {
                assert_eq!(name, "audit-skill");
                assert_eq!(description, "Audit build outputs");
                assert!(matches!(method, CreateSkillMethod::Github));
                assert_eq!(target_dir, Some(PathBuf::from("/tmp/skills")));
                assert_eq!(project_dir, Some(PathBuf::from("/tmp/project")));
                assert!(dry_run);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected CreateSkill command"),
        }
    }

    #[test]
    fn create_skill_rejects_invalid_method() {
        let result = Cli::try_parse_from([
            "skrills",
            "create-skill",
            "audit-skill",
            "--description",
            "Audit build outputs",
            "--method",
            "invalid",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn parse_search_skills_github_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "search-skills-github",
            "testing skills",
            "--limit",
            "5",
            "--format",
            "json",
        ])
        .expect("search-skills-github args should parse");

        match cli.command {
            Some(Commands::SearchSkillsGithub {
                query,
                limit,
                format,
            }) => {
                assert_eq!(query, "testing skills");
                assert_eq!(limit, 5);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected SearchSkillsGithub command"),
        }
    }

    #[test]
    fn unknown_flag_is_rejected() {
        let result = Cli::try_parse_from(["skrills", "--nonexistent"]);
        assert!(result.is_err(), "unknown global flag should be rejected");
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        let result = Cli::try_parse_from(["skrills", "frobnicate"]);
        assert!(result.is_err(), "unknown subcommand should be rejected");
    }

    #[test]
    fn parse_mirror_arguments() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", None);

        let cli = Cli::try_parse_from([
            "skrills",
            "mirror",
            "--dry-run",
            "--skip-existing-commands",
            "--include-marketplace",
        ])
        .expect("mirror args should parse");

        match cli.command {
            Some(Commands::Mirror {
                dry_run,
                skip_existing_commands,
                include_marketplace,
            }) => {
                assert!(dry_run);
                assert!(skip_existing_commands);
                assert!(include_marketplace);
            }
            _ => panic!("expected Mirror command"),
        }
    }

    #[test]
    fn parse_agent_requires_name() {
        let result = Cli::try_parse_from(["skrills", "agent"]);
        assert!(
            result.is_err(),
            "agent subcommand requires a positional name argument"
        );
    }

    #[test]
    fn parse_agent_with_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "agent",
            "my-agent",
            "--skill-dir",
            "/tmp/agent-skills",
            "--dry-run",
        ])
        .expect("agent args should parse");

        match cli.command {
            Some(Commands::Agent {
                agent,
                skill_dirs,
                dry_run,
            }) => {
                assert_eq!(agent, "my-agent");
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/agent-skills")]);
                assert!(dry_run);
            }
            _ => panic!("expected Agent command"),
        }
    }

    #[test]
    fn parse_sync_all_arguments() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", None);

        let cli = Cli::try_parse_from([
            "skrills",
            "sync-all",
            "--from",
            "codex",
            "--to",
            "copilot",
            "--dry-run",
            "--skip-existing-commands",
            "--validate",
            "--autofix",
        ])
        .expect("sync-all args should parse");

        match cli.command {
            Some(Commands::SyncAll {
                from,
                to,
                dry_run,
                skip_existing_commands,
                include_marketplace,
                exclude_plugins,
                validate,
                autofix,
            }) => {
                assert!(matches!(from, SyncSource::Codex));
                assert_eq!(to, Some(SyncSource::Copilot));
                assert!(dry_run);
                assert!(skip_existing_commands);
                assert!(!include_marketplace);
                assert!(exclude_plugins.is_empty());
                assert!(validate);
                assert!(autofix);
            }
            _ => panic!("expected SyncAll command"),
        }
    }

    #[test]
    fn parse_sync_commands_defaults() {
        let cli = Cli::try_parse_from(["skrills", "sync-commands"])
            .expect("sync-commands with defaults should parse");

        match cli.command {
            Some(Commands::SyncCommands {
                from, to, dry_run, ..
            }) => {
                assert!(matches!(from, SyncSource::Claude));
                assert!(to.is_none());
                assert!(!dry_run);
            }
            _ => panic!("expected SyncCommands command"),
        }
    }

    #[test]
    fn parse_doctor_subcommand() {
        let cli = Cli::try_parse_from(["skrills", "doctor"]).expect("doctor should parse");

        assert!(
            matches!(cli.command, Some(Commands::Doctor)),
            "expected Doctor command"
        );
    }

    #[test]
    fn parse_validate_all_flags() {
        let cli = Cli::try_parse_from([
            "skrills",
            "validate",
            "--skill-dir",
            "/tmp/s",
            "--target",
            "codex",
            "--autofix",
            "--backup",
            "--format",
            "json",
            "--errors-only",
        ])
        .expect("validate with all flags should parse");

        match cli.command {
            Some(Commands::Validate {
                skill_dirs,
                target,
                autofix,
                backup,
                format,
                errors_only,
                ..
            }) => {
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/s")]);
                assert!(matches!(target, ValidationTarget::Codex));
                assert!(autofix);
                assert!(backup);
                assert_eq!(format, OutputFormat::Json);
                assert!(errors_only);
            }
            _ => panic!("expected Validate command"),
        }
    }

    #[test]
    fn parse_search_skills_with_short_flags() {
        let cli = Cli::try_parse_from([
            "skrills",
            "search-skills",
            "rust testing",
            "-t",
            "0.5",
            "-l",
            "3",
            "--format",
            "json",
        ])
        .expect("search-skills with short flags should parse");

        match cli.command {
            Some(Commands::SearchSkills {
                query,
                threshold,
                limit,
                format,
                ..
            }) => {
                assert_eq!(query, "rust testing");
                assert!((threshold - 0.5).abs() < f64::EPSILON);
                assert_eq!(limit, 3);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected SearchSkills command"),
        }
    }

    #[test]
    fn parse_recommend_requires_uri() {
        let result = Cli::try_parse_from(["skrills", "recommend"]);
        assert!(
            result.is_err(),
            "recommend subcommand requires a positional URI"
        );
    }

    #[test]
    fn tls_cert_requires_tls_key() {
        let _guard = env_guard();
        let _cert_env = set_env_var("SKRILLS_TLS_CERT", None);
        let _key_env = set_env_var("SKRILLS_TLS_KEY", None);

        let result = Cli::try_parse_from(["skrills", "serve", "--tls-cert", "/path/cert.pem"]);
        assert!(
            result.is_err(),
            "providing --tls-cert without --tls-key should fail"
        );
    }

    #[test]
    fn tls_key_requires_tls_cert() {
        let _guard = env_guard();
        let _cert_env = set_env_var("SKRILLS_TLS_CERT", None);
        let _key_env = set_env_var("SKRILLS_TLS_KEY", None);

        let result = Cli::try_parse_from(["skrills", "serve", "--tls-key", "/path/key.pem"]);
        assert!(
            result.is_err(),
            "providing --tls-key without --tls-cert should fail"
        );
    }

    #[test]
    fn parse_cert_status_subcommand() {
        let cli = Cli::try_parse_from(["skrills", "cert", "status", "--format", "json"])
            .expect("cert status should parse");

        match cli.command {
            Some(Commands::Cert(CertAction::Status { format })) => {
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected Cert Status command"),
        }
    }

    #[test]
    fn parse_cert_renew_force() {
        let cli = Cli::try_parse_from(["skrills", "cert", "renew", "--force"])
            .expect("cert renew --force should parse");

        match cli.command {
            Some(Commands::Cert(CertAction::Renew { force })) => {
                assert!(force);
            }
            _ => panic!("expected Cert Renew command"),
        }
    }

    #[test]
    fn parse_setup_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "setup",
            "--client",
            "claude",
            "--bin-dir",
            "/usr/local/bin",
            "--reinstall",
            "--yes",
            "--universal",
        ])
        .expect("setup args should parse");

        match cli.command {
            Some(Commands::Setup {
                client,
                bin_dir,
                reinstall,
                uninstall,
                add,
                yes,
                universal,
                mirror_source,
            }) => {
                assert_eq!(client.as_deref(), Some("claude"));
                assert_eq!(bin_dir, Some(PathBuf::from("/usr/local/bin")));
                assert!(reinstall);
                assert!(!uninstall);
                assert!(!add);
                assert!(yes);
                assert!(universal);
                assert!(mirror_source.is_none());
            }
            _ => panic!("expected Setup command"),
        }
    }

    #[test]
    fn parse_skill_import_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "skill-import",
            "https://github.com/example/skill.git",
            "--target",
            "codex",
            "--force",
            "--dry-run",
            "--format",
            "json",
        ])
        .expect("skill-import args should parse");

        match cli.command {
            Some(Commands::SkillImport {
                source,
                target,
                force,
                dry_run,
                format,
            }) => {
                assert_eq!(source, "https://github.com/example/skill.git");
                assert!(matches!(target, SyncSource::Codex));
                assert!(force);
                assert!(dry_run);
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected SkillImport command"),
        }
    }

    #[test]
    fn parse_skill_diff_arguments() {
        let cli = Cli::try_parse_from([
            "skrills",
            "skill-diff",
            "commit",
            "--format",
            "json",
            "-C",
            "5",
        ])
        .expect("skill-diff args should parse");

        match cli.command {
            Some(Commands::SkillDiff {
                name,
                format,
                context,
            }) => {
                assert_eq!(name, "commit");
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(context, 5);
            }
            _ => panic!("expected SkillDiff command"),
        }
    }

    #[test]
    fn parse_export_analytics_defaults() {
        let cli = Cli::try_parse_from(["skrills", "export-analytics"])
            .expect("export-analytics with defaults should parse");

        match cli.command {
            Some(Commands::ExportAnalytics {
                output,
                force_rebuild,
                format,
            }) => {
                assert!(output.is_none());
                assert!(!force_rebuild);
                assert_eq!(format, OutputFormat::Text);
            }
            _ => panic!("expected ExportAnalytics command"),
        }
    }

    #[test]
    fn import_analytics_requires_input_path() {
        let result = Cli::try_parse_from(["skrills", "import-analytics"]);
        assert!(
            result.is_err(),
            "import-analytics requires a positional input path"
        );
    }

    #[test]
    fn parse_serve_list_tools_flag() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_AUTH_TOKEN", None);

        let cli = Cli::try_parse_from(["skrills", "serve", "--list-tools"])
            .expect("serve --list-tools should parse");

        match cli.command {
            Some(Commands::Serve { list_tools, .. }) => {
                assert!(list_tools);
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_with_open_flag() {
        let cli = Cli::try_parse_from(["skrills", "serve", "--http", "127.0.0.1:3000", "--open"])
            .expect("serve with --open should parse");

        match cli.command {
            Some(Commands::Serve { open, http, .. }) => {
                assert!(open);
                assert_eq!(http, Some("127.0.0.1:3000".to_string()));
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn parse_serve_open_without_http_is_error() {
        let result = Cli::try_parse_from(["skrills", "serve", "--open"]);
        assert!(
            result.is_err(),
            "--open without --http should be a parse error"
        );
    }

    #[test]
    fn parse_multi_cli_agent_defaults() {
        let cli = Cli::try_parse_from(["skrills", "multi-cli-agent", "my-agent"])
            .expect("multi-cli-agent should parse");

        match cli.command {
            Some(Commands::MultiCliAgent {
                agent,
                backend,
                skill_dirs,
                dry_run,
            }) => {
                assert_eq!(agent, "my-agent");
                assert!(matches!(backend, AgentBackend::Auto));
                assert!(skill_dirs.is_empty());
                assert!(!dry_run);
            }
            _ => panic!("expected MultiCliAgent command"),
        }
    }

    #[test]
    fn parse_multi_cli_agent_full_args() {
        let cli = Cli::try_parse_from([
            "skrills",
            "multi-cli-agent",
            "test-agent",
            "--backend",
            "codex",
            "--skill-dir",
            "/tmp/skills",
            "--dry-run",
        ])
        .expect("multi-cli-agent with full args should parse");

        match cli.command {
            Some(Commands::MultiCliAgent {
                agent,
                backend,
                skill_dirs,
                dry_run,
            }) => {
                assert_eq!(agent, "test-agent");
                assert!(matches!(backend, AgentBackend::Codex));
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
                assert!(dry_run);
            }
            _ => panic!("expected MultiCliAgent command"),
        }
    }

    #[test]
    fn parse_multi_cli_agent_requires_agent_name() {
        let result = Cli::try_parse_from(["skrills", "multi-cli-agent"]);
        assert!(result.is_err(), "multi-cli-agent requires agent argument");
    }

    #[test]
    fn parse_multi_cli_agent_backend_claude() {
        let cli = Cli::try_parse_from([
            "skrills",
            "multi-cli-agent",
            "my-agent",
            "--backend",
            "claude",
        ])
        .expect("multi-cli-agent with claude backend should parse");

        match cli.command {
            Some(Commands::MultiCliAgent { backend, .. }) => {
                assert!(matches!(backend, AgentBackend::Claude));
            }
            _ => panic!("expected MultiCliAgent command"),
        }
    }

    #[test]
    fn parse_sync_all_claude_to_cursor_dry_run() {
        let _guard = env_guard();
        let _env = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", None);

        let cli = Cli::try_parse_from([
            "skrills",
            "sync-all",
            "--from",
            "claude",
            "--to",
            "cursor",
            "--dry-run",
        ])
        .expect("sync-all --from claude --to cursor --dry-run should parse");

        match cli.command {
            Some(Commands::SyncAll {
                from, to, dry_run, ..
            }) => {
                assert!(matches!(from, SyncSource::Claude));
                assert_eq!(to, Some(SyncSource::Cursor));
                assert!(dry_run);
            }
            _ => panic!("expected SyncAll command"),
        }
    }

    // --- SyncSource enum method tests ---

    #[test]
    fn sync_source_as_str_returns_expected_names() {
        assert_eq!(SyncSource::Claude.as_str(), "claude");
        assert_eq!(SyncSource::Codex.as_str(), "codex");
        assert_eq!(SyncSource::Copilot.as_str(), "copilot");
        assert_eq!(SyncSource::Cursor.as_str(), "cursor");
    }

    #[test]
    fn sync_source_is_predicates() {
        assert!(SyncSource::Claude.is_claude());
        assert!(!SyncSource::Claude.is_codex());

        assert!(SyncSource::Codex.is_codex());
        assert!(!SyncSource::Codex.is_claude());

        assert!(SyncSource::Copilot.is_copilot());
        assert!(!SyncSource::Copilot.is_cursor());

        assert!(SyncSource::Cursor.is_cursor());
        assert!(!SyncSource::Cursor.is_copilot());
    }

    #[test]
    fn sync_source_default_target() {
        assert!(matches!(
            SyncSource::Claude.default_target(),
            SyncSource::Codex
        ));
        assert!(matches!(
            SyncSource::Codex.default_target(),
            SyncSource::Claude
        ));
        assert!(matches!(
            SyncSource::Copilot.default_target(),
            SyncSource::Claude
        ));
        assert!(matches!(
            SyncSource::Cursor.default_target(),
            SyncSource::Claude
        ));
    }

    #[test]
    fn sync_source_other_targets_excludes_self() {
        for source in [
            SyncSource::Claude,
            SyncSource::Codex,
            SyncSource::Copilot,
            SyncSource::Cursor,
        ] {
            let targets = source.other_targets();
            assert_eq!(targets.len(), 3, "{:?} should have 3 other targets", source);
            assert!(
                !targets.iter().any(|t| t.as_str() == source.as_str()),
                "{:?} should not include itself in other_targets",
                source,
            );
        }
    }

    // --- AgentBackend enum method tests ---

    #[test]
    fn agent_backend_as_str() {
        assert_eq!(AgentBackend::Auto.as_str(), "auto");
        assert_eq!(AgentBackend::Claude.as_str(), "claude");
        assert_eq!(AgentBackend::Codex.as_str(), "codex");
    }

    #[test]
    fn agent_backend_default_is_auto() {
        assert_eq!(AgentBackend::default(), AgentBackend::Auto);
    }
}
