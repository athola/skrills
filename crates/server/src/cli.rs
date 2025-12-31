use clap::{ArgAction, Parser, Subcommand, ValueEnum};
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

/// Dependency traversal direction.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DependencyDirection {
    /// Resolve dependencies (what this skill needs).
    Dependencies,
    /// Resolve dependents (what uses this skill).
    Dependents,
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
}

impl CreateSkillMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Llm => "llm",
            Self::Both => "both",
        }
    }
}

impl std::fmt::Display for CreateSkillMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
    about = "Skills support engine for Claude Code and Codex CLI"
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
        #[cfg(feature = "http-transport")]
        /// Bind address for HTTP transport (e.g., "0.0.0.0:3000" or "127.0.0.1:8080").
        /// When specified, serves MCP over HTTP instead of stdio.
        #[arg(long, value_name = "BIND_ADDR")]
        http: Option<String>,
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
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
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
    /// Create a new skill via GitHub search, LLM generation, or both.
    CreateSkill {
        /// Name or topic for the skill.
        #[arg(required = true)]
        name: String,
        /// Detailed description of what the skill should do.
        #[arg(long)]
        description: String,
        /// Creation method: github, llm, or both.
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::env_guard()
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                std::env::set_var(self.key, v);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = std::env::var(key).ok();
        if let Some(v) = value {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    #[test]
    fn parse_defaults_to_serve_when_no_subcommand() {
        let cli = Cli::try_parse_from(["skrills"]).expect("default parse should succeed");
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_serve_arguments() {
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
                #[cfg(feature = "http-transport")]
                http,
            }) => {
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
                assert_eq!(cache_ttl_ms, Some(1500));
                assert!(trace_wire);
                #[cfg(feature = "watch")]
                assert!(!watch);
                #[cfg(feature = "http-transport")]
                assert!(http.is_none());
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
                format,
                skill_dirs,
            }) => {
                assert_eq!(uri.as_deref(), Some("skill://skrills/codex/test-skill"));
                assert_eq!(prompt.as_deref(), Some("testing workflow"));
                assert_eq!(project_dir, Some(PathBuf::from("/tmp/project")));
                assert_eq!(limit, 5);
                assert!(!include_usage);
                assert!(include_context);
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(skill_dirs, vec![PathBuf::from("/tmp/skills")]);
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
}
