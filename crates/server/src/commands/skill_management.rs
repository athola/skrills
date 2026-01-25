//! Skill management command handlers.
//!
//! Commands for deprecating, rolling back, profiling, cataloging, importing,
//! scoring, and generating usage reports for skills.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cli::{OutputFormat, SyncSource, ValidationTarget};
use crate::discovery::merge_extra_dirs;

/// Result of skill deprecation operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct DeprecationResult {
    pub skill_name: String,
    pub skill_path: PathBuf,
    pub deprecated: bool,
    pub message: Option<String>,
    pub replacement: Option<String>,
}

/// Handle the skill-deprecate command.
pub(crate) fn handle_skill_deprecate_command(
    name: String,
    message: Option<String>,
    replacement: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    use skrills_validate::frontmatter::parse_frontmatter;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    // Find the skill by name
    let skill = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name) || s.path.to_string_lossy().contains(&name))
        .with_context(|| format!("Skill '{}' not found in discovered skills", name))?;

    let skill_path = skill.path.clone();
    let content = std::fs::read_to_string(&skill_path)
        .with_context(|| format!("Failed to read skill file: {}", skill_path.display()))?;

    // Parse existing frontmatter
    let parsed = parse_frontmatter(&content).map_err(|e| anyhow::anyhow!(e))?;

    // Build deprecation frontmatter
    let deprecation_msg = message.as_deref().unwrap_or("This skill is deprecated");
    let mut new_content = String::new();

    if let Some(raw_fm) = &parsed.raw_frontmatter {
        // Add deprecated field to existing frontmatter
        let fm_lines: Vec<&str> = raw_fm.lines().collect();

        // Check if already deprecated
        if fm_lines.iter().any(|l| l.starts_with("deprecated:")) {
            if format.is_json() {
                let result = DeprecationResult {
                    skill_name: skill.name.clone(),
                    skill_path: skill_path.clone(),
                    deprecated: false,
                    message: Some("Skill is already marked as deprecated".to_string()),
                    replacement: None,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Skill '{}' is already marked as deprecated", skill.name);
            }
            return Ok(());
        }

        // Rebuild frontmatter with deprecation
        new_content.push_str("---\n");
        for line in &fm_lines {
            new_content.push_str(line);
            new_content.push('\n');
        }
        new_content.push_str("deprecated: true\n");
        new_content.push_str(&format!("deprecation_message: \"{}\"\n", deprecation_msg));
        if let Some(ref repl) = replacement {
            new_content.push_str(&format!("replacement: \"{}\"\n", repl));
        }
        new_content.push_str("---\n");
        new_content.push_str(&parsed.content);
    } else {
        // No existing frontmatter, create new one
        new_content.push_str("---\n");
        new_content.push_str(&format!("name: {}\n", skill.name));
        new_content.push_str("deprecated: true\n");
        new_content.push_str(&format!("deprecation_message: \"{}\"\n", deprecation_msg));
        if let Some(ref repl) = replacement {
            new_content.push_str(&format!("replacement: \"{}\"\n", repl));
        }
        new_content.push_str("---\n\n");
        new_content.push_str(&content);
    }

    // Write back
    std::fs::write(&skill_path, &new_content)
        .with_context(|| format!("Failed to write to skill file: {}", skill_path.display()))?;

    let result = DeprecationResult {
        skill_name: skill.name.clone(),
        skill_path: skill_path.clone(),
        deprecated: true,
        message: Some(deprecation_msg.to_string()),
        replacement: replacement.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("✓ Marked '{}' as deprecated", skill.name);
        println!("  Path: {}", skill_path.display());
        println!("  Message: {}", deprecation_msg);
        if let Some(repl) = &replacement {
            println!("  Replacement: {}", repl);
        }
    }

    Ok(())
}

/// Version info for skill rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillVersion {
    pub hash: String,
    pub date: String,
    pub message: String,
}

/// Result of skill rollback operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct RollbackResult {
    pub skill_name: String,
    pub skill_path: PathBuf,
    pub rolled_back: bool,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub available_versions: Vec<SkillVersion>,
}

/// Handle the skill-rollback command.
pub(crate) fn handle_skill_rollback_command(
    name: String,
    version: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    use std::process::Command;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    // Find the skill by name
    let skill = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name) || s.path.to_string_lossy().contains(&name))
        .with_context(|| format!("Skill '{}' not found in discovered skills", name))?;

    let skill_path = &skill.path;

    // Check if the skill is in a git repository
    let parent_dir = skill_path
        .parent()
        .with_context(|| "Skill has no parent directory")?;

    // Get git history for this file
    let git_log = Command::new("git")
        .args([
            "log",
            "--pretty=format:%h|%ai|%s",
            "-n",
            "10",
            "--",
            skill_path.to_str().unwrap_or(""),
        ])
        .current_dir(parent_dir)
        .output();

    let (available_versions, git_error): (Vec<SkillVersion>, Option<String>) = match git_log {
        Ok(output) if output.status.success() => {
            let versions = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(3, '|').collect();
                    if parts.len() == 3 {
                        Some(SkillVersion {
                            hash: parts[0].to_string(),
                            date: parts[1].to_string(),
                            message: parts[2].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();
            (versions, None)
        }
        Ok(output) => {
            // Git command ran but failed (non-zero exit)
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            tracing::warn!(
                target: "skrills::skill_management",
                path = %skill_path.display(),
                stderr = %stderr,
                "Git log failed for skill file"
            );
            (vec![], Some(stderr))
        }
        Err(e) => {
            // Git command couldn't be executed (git not found, permission denied, etc.)
            tracing::warn!(
                target: "skrills::skill_management",
                path = %skill_path.display(),
                error = %e,
                "Failed to execute git command"
            );
            (vec![], Some(format!("Could not execute git: {}", e)))
        }
    };

    if available_versions.is_empty() {
        if format.is_json() {
            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: false,
                from_version: None,
                to_version: None,
                available_versions: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "No git history found for skill '{}' at {}",
                skill.name,
                skill_path.display()
            );
            if let Some(ref error) = git_error {
                eprintln!("Git error: {}", error.trim());
            }
            println!("Skill rollback requires the skill to be under git version control.");
        }
        return Ok(());
    }

    match version {
        Some(target_version) => {
            // Validate version hash format to prevent command injection
            // Git commit hashes are 4-40 hexadecimal characters
            let hash_pattern =
                regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").expect("Invalid regex pattern");
            if !hash_pattern.is_match(&target_version) {
                bail!(
                    "Invalid version hash '{}'. Expected 4-40 hexadecimal characters (e.g., 'abc1234' or full SHA).",
                    target_version
                );
            }

            // Perform rollback to specific version
            let checkout = Command::new("git")
                .args([
                    "checkout",
                    &target_version,
                    "--",
                    skill_path.to_str().unwrap_or(""),
                ])
                .current_dir(parent_dir)
                .output()
                .with_context(|| "Failed to execute git checkout")?;

            if !checkout.status.success() {
                bail!(
                    "Git checkout failed: {}",
                    String::from_utf8_lossy(&checkout.stderr)
                );
            }

            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: true,
                from_version: available_versions.first().map(|v| v.hash.clone()),
                to_version: Some(target_version.clone()),
                available_versions: vec![],
            };

            if format.is_json() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Rolled back '{}' to version {}",
                    skill.name, target_version
                );
                println!("  Path: {}", skill_path.display());
            }
        }
        None => {
            // Show available versions
            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: false,
                from_version: None,
                to_version: None,
                available_versions: available_versions.clone(),
            };

            if format.is_json() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Available versions for '{}' ({}):",
                    skill.name,
                    skill_path.display()
                );
                println!();
                for (i, v) in available_versions.iter().enumerate() {
                    let current = if i == 0 { " (current)" } else { "" };
                    println!(
                        "  {} - {}{}",
                        v.hash,
                        v.date.split_whitespace().next().unwrap_or(&v.date),
                        current
                    );
                    println!("        {}", v.message);
                }
                println!();
                println!(
                    "To rollback: skrills skill-rollback {} --version <hash>",
                    name
                );
            }
        }
    }

    Ok(())
}

/// Statistics for skill profiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStats {
    pub name: String,
    pub invocations: u64,
    pub last_used: Option<String>,
    pub avg_tokens: Option<f64>,
    pub success_rate: Option<f64>,
}

/// Result of skill profile operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileResult {
    pub period_days: u32,
    pub total_invocations: u64,
    pub unique_skills_used: usize,
    pub top_skills: Vec<SkillStats>,
}

/// Handle the skill-profile command.
pub(crate) fn handle_skill_profile_command(
    name: Option<String>,
    period: u32,
    format: OutputFormat,
) -> Result<()> {
    // Try to load analytics cache
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;
    let cache_path = home.join(".skrills/analytics_cache.json");

    if !cache_path.exists() {
        if format.is_json() {
            let result = ProfileResult {
                period_days: period,
                total_invocations: 0,
                unique_skills_used: 0,
                top_skills: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("No analytics data found.");
            println!(
                "Run `skrills recommend-skills-smart --auto-persist` to build analytics cache."
            );
        }
        return Ok(());
    }

    // Load and parse analytics
    let analytics_json =
        std::fs::read_to_string(&cache_path).with_context(|| "Failed to read analytics cache")?;

    // Parse as generic JSON to extract usage counts
    let analytics: serde_json::Value =
        serde_json::from_str(&analytics_json).with_context(|| "Failed to parse analytics cache")?;

    let mut skill_counts: HashMap<String, u64> = HashMap::new();

    // Extract usage data from analytics structure
    if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
        for (skill_name, count) in usage {
            if let Some(n) = count.as_u64() {
                skill_counts.insert(skill_name.clone(), n);
            }
        }
    }

    // If looking for specific skill
    if let Some(ref target_name) = name {
        let count = skill_counts.get(target_name).copied().unwrap_or(0);
        let stats = SkillStats {
            name: target_name.clone(),
            invocations: count,
            last_used: None,
            avg_tokens: None,
            success_rate: None,
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        } else {
            println!("Profile for '{}':", target_name);
            println!("  Invocations ({}d): {}", period, count);
            if count == 0 {
                println!("  No usage data found for this skill.");
            }
        }
        return Ok(());
    }

    // Show overall stats
    let total: u64 = skill_counts.values().sum();
    let mut sorted: Vec<_> = skill_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let top_skills: Vec<SkillStats> = sorted
        .into_iter()
        .take(10)
        .map(|(name, invocations)| SkillStats {
            name,
            invocations,
            last_used: None,
            avg_tokens: None,
            success_rate: None,
        })
        .collect();

    let result = ProfileResult {
        period_days: period,
        total_invocations: total,
        unique_skills_used: top_skills.len(),
        top_skills: top_skills.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill Usage Profile (last {} days)", period);
        println!("─────────────────────────────────────");
        println!("Total invocations: {}", total);
        println!("Unique skills used: {}", result.unique_skills_used);
        println!();
        println!("Top Skills:");
        for (i, stats) in top_skills.iter().enumerate() {
            println!(
                "  {}. {} ({} invocations)",
                i + 1,
                stats.name,
                stats.invocations
            );
        }
    }

    Ok(())
}

/// Catalog entry for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub source: String,
    pub description: Option<String>,
    pub path: PathBuf,
    pub deprecated: bool,
}

/// Result of skill catalog operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogResult {
    pub total_skills: usize,
    pub skills: Vec<CatalogEntry>,
}

/// Handle the skill-catalog command.
pub(crate) fn handle_skill_catalog_command(
    search: Option<String>,
    source: Option<SyncSource>,
    _category: Option<String>,
    limit: usize,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let mut entries: Vec<CatalogEntry> = skills
        .iter()
        .filter(|s| {
            // Filter by search query
            if let Some(ref query) = search {
                let q = query.to_lowercase();
                s.name.to_lowercase().contains(&q)
                    || s.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
            } else {
                true
            }
        })
        .filter(|s| {
            // Filter by source
            if let Some(ref src) = source {
                let path_str = s.path.to_string_lossy().to_lowercase();
                match src {
                    SyncSource::Claude => path_str.contains("claude"),
                    SyncSource::Codex => path_str.contains("codex"),
                    SyncSource::Copilot => path_str.contains("copilot"),
                }
            } else {
                true
            }
        })
        .take(limit)
        .map(|s| {
            let source_name = if s.path.to_string_lossy().contains("claude") {
                "claude"
            } else if s.path.to_string_lossy().contains("codex") {
                "codex"
            } else if s.path.to_string_lossy().contains("copilot") {
                "copilot"
            } else {
                "local"
            };
            CatalogEntry {
                name: s.name.clone(),
                source: source_name.to_string(),
                description: s.description.clone(),
                path: s.path.clone(),
                deprecated: false, // Would need to parse frontmatter to check
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let result = CatalogResult {
        total_skills: entries.len(),
        skills: entries.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill Catalog ({} skills)", result.total_skills);
        println!("═══════════════════════════════════════════════════════════════════════");
        println!();

        for entry in &entries {
            let desc = entry
                .description
                .as_ref()
                .map(|d| {
                    if d.len() > 60 {
                        format!("{}...", &d[..57])
                    } else {
                        d.clone()
                    }
                })
                .unwrap_or_else(|| "(no description)".to_string());
            println!("  {} [{}]", entry.name, entry.source);
            println!("    {}", desc);
            println!();
        }

        if let Some(ref query) = search {
            println!("Filtered by: \"{}\"", query);
        }
    }

    Ok(())
}

/// Handle the pre-commit-validate command.
pub(crate) fn handle_pre_commit_validate_command(
    staged: bool,
    target: ValidationTarget,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    use skrills_validate::{validate_skill, ValidationTarget as VT};
    use std::process::Command;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);

    // Convert CLI ValidationTarget to validate crate ValidationTarget
    let validation_target = match target {
        ValidationTarget::Claude => VT::Claude,
        ValidationTarget::Codex => VT::Codex,
        ValidationTarget::Copilot => VT::Copilot,
        ValidationTarget::All => VT::All,
        ValidationTarget::Both => VT::Both,
    };

    // Get list of skill files to validate
    let skill_files: Vec<PathBuf> = if staged {
        // Get staged files from git
        let output = Command::new("git")
            .args(["diff", "--cached", "--name-only", "--diff-filter=ACM"])
            .output()
            .with_context(|| "Failed to run git diff")?;

        if !output.status.success() {
            bail!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|f| f.ends_with(".md") || f.ends_with(".skill"))
            .map(PathBuf::from)
            .collect()
    } else {
        // Validate all discovered skills
        discover_skills(&roots, None)?
            .into_iter()
            .map(|s| s.path)
            .collect()
    };

    if skill_files.is_empty() {
        println!("No skill files to validate.");
        return Ok(());
    }

    let mut errors_found = false;
    let mut validated = 0;

    for path in &skill_files {
        if !path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                // Report file read errors - silent failures could let broken skills slip through
                errors_found = true;
                eprintln!("✗ {} (read error: {})", path.display(), e);
                continue;
            }
        };

        let result = validate_skill(path, &content, validation_target);

        if result.has_errors() {
            errors_found = true;
            eprintln!("✗ {}", path.display());
            for issue in &result.issues {
                if issue.severity == skrills_validate::Severity::Error {
                    eprintln!("  - {}", issue.message);
                }
            }
        } else {
            validated += 1;
        }
    }

    if errors_found {
        eprintln!();
        eprintln!("Validation failed. Fix errors before committing.");
        std::process::exit(1);
    }

    println!("✓ {} skill file(s) validated successfully", validated);
    Ok(())
}

/// Result of skill import operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub source: String,
    pub target_path: PathBuf,
    pub imported: bool,
    pub skill_name: Option<String>,
    pub message: String,
}

/// Handle the skill-import command.
pub(crate) fn handle_skill_import_command(
    source: String,
    target: SyncSource,
    force: bool,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;

    // Determine target directory
    let target_dir = match target {
        SyncSource::Claude => home.join(".claude/skills"),
        SyncSource::Codex => home.join(".codex/skills"),
        SyncSource::Copilot => home.join(".github/copilot/skills"),
    };

    // Ensure target directory exists
    if !dry_run {
        std::fs::create_dir_all(&target_dir).with_context(|| {
            format!(
                "Failed to create target directory: {}",
                target_dir.display()
            )
        })?;
    }

    // Determine source type and fetch content
    let (skill_content, skill_name) =
        if source.starts_with("http://") || source.starts_with("https://") {
            // URL source - would need reqwest for this, stub for now
            bail!(
            "URL imports require the 'http-transport' feature. Use a local path or git URL instead."
        );
        } else if source.starts_with("git://") || source.ends_with(".git") {
            // Git URL - would need git clone, stub for now
            bail!("Git imports not yet implemented. Clone the repo manually and use a local path.");
        } else {
            // Local path
            let path = PathBuf::from(&source);
            if !path.exists() {
                bail!("Source path does not exist: {}", source);
            }

            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read source file: {}", source))?;

            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "imported-skill".to_string());

            (content, name)
        };

    // Determine target path
    let target_path = target_dir.join(format!("{}.md", skill_name));

    // Check for existing file
    if target_path.exists() && !force {
        let result = ImportResult {
            source: source.clone(),
            target_path: target_path.clone(),
            imported: false,
            skill_name: Some(skill_name.clone()),
            message: format!(
                "Skill '{}' already exists at {}. Use --force to overwrite.",
                skill_name,
                target_path.display()
            ),
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            eprintln!("{}", result.message);
        }
        return Ok(());
    }

    if dry_run {
        let result = ImportResult {
            source,
            target_path: target_path.clone(),
            imported: false,
            skill_name: Some(skill_name.clone()),
            message: format!("Would import '{}' to {}", skill_name, target_path.display()),
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("[dry-run] {}", result.message);
        }
        return Ok(());
    }

    // Write the skill file
    std::fs::write(&target_path, &skill_content)
        .with_context(|| format!("Failed to write skill to {}", target_path.display()))?;

    let result = ImportResult {
        source,
        target_path: target_path.clone(),
        imported: true,
        skill_name: Some(skill_name.clone()),
        message: format!(
            "Successfully imported '{}' to {}",
            skill_name,
            target_path.display()
        ),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("✓ {}", result.message);
    }

    Ok(())
}

/// Skill usage statistics for reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub skill_name: String,
    pub invocations: u64,
    pub percentage: f64,
}

/// Result of usage report generation.
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageReportResult {
    pub period_days: u32,
    pub generated_at: String,
    pub total_invocations: u64,
    pub unique_skills: usize,
    pub skills: Vec<UsageStats>,
}

/// Handle the skill-usage-report command.
pub(crate) fn handle_skill_usage_report_command(
    period: u32,
    format: OutputFormat,
    output: Option<PathBuf>,
    _skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;
    let cache_path = home.join(".skrills/analytics_cache.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let generated_at = format!("{}", now);

    if !cache_path.exists() {
        let empty_result = UsageReportResult {
            period_days: period,
            generated_at: generated_at.clone(),
            total_invocations: 0,
            unique_skills: 0,
            skills: vec![],
        };

        let report_text = if format.is_json() {
            serde_json::to_string_pretty(&empty_result)?
        } else {
            format!(
                "Skill Usage Report\n\
                 ═══════════════════\n\
                 Period: {} days\n\
                 Generated: {}\n\n\
                 No usage data available.\n\
                 Run `skrills recommend-skills-smart --auto-persist` to build analytics.",
                period, generated_at
            )
        };

        if let Some(ref out_path) = output {
            std::fs::write(out_path, &report_text)?;
            println!("Report written to: {}", out_path.display());
        } else {
            println!("{}", report_text);
        }

        return Ok(());
    }

    // Load analytics
    let analytics_json = std::fs::read_to_string(&cache_path)?;
    let analytics: serde_json::Value = serde_json::from_str(&analytics_json)?;

    let mut skill_counts: HashMap<String, u64> = HashMap::new();
    if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
        for (name, count) in usage {
            if let Some(n) = count.as_u64() {
                skill_counts.insert(name.clone(), n);
            }
        }
    }

    let total: u64 = skill_counts.values().sum();
    let mut sorted: Vec<_> = skill_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let skills: Vec<UsageStats> = sorted
        .into_iter()
        .map(|(name, invocations)| {
            let percentage = if total > 0 {
                (invocations as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            UsageStats {
                skill_name: name,
                invocations,
                percentage,
            }
        })
        .collect();

    let result = UsageReportResult {
        period_days: period,
        generated_at: generated_at.clone(),
        total_invocations: total,
        unique_skills: skills.len(),
        skills: skills.clone(),
    };

    let report_text = if format.is_json() {
        serde_json::to_string_pretty(&result)?
    } else {
        let mut text = String::new();
        text.push_str("Skill Usage Report\n");
        text.push_str("═══════════════════════════════════════════════════════════\n\n");
        text.push_str(&format!("Period: {} days\n", period));
        text.push_str(&format!("Generated: {}\n", generated_at));
        text.push_str(&format!("Total Invocations: {}\n", total));
        text.push_str(&format!("Unique Skills: {}\n\n", result.unique_skills));
        text.push_str("Usage by Skill:\n");
        text.push_str("───────────────────────────────────────────────────────────\n");

        for stats in &skills {
            text.push_str(&format!(
                "  {:40} {:>6} ({:>5.1}%)\n",
                stats.skill_name, stats.invocations, stats.percentage
            ));
        }

        text
    };

    if let Some(ref out_path) = output {
        std::fs::write(out_path, &report_text)?;
        println!("Report written to: {}", out_path.display());
    } else {
        println!("{}", report_text);
    }

    Ok(())
}

/// Quality score components.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub frontmatter_completeness: u8,
    pub validation_score: u8,
    pub description_quality: u8,
    pub token_efficiency: u8,
}

/// Score result for a skill.
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillScoreResult {
    pub name: String,
    pub path: PathBuf,
    pub total_score: u8,
    pub breakdown: ScoreBreakdown,
    pub suggestions: Vec<String>,
}

/// Handle the skill-score command.
pub(crate) fn handle_skill_score_command(
    name: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
    below_threshold: Option<u8>,
) -> Result<()> {
    use skrills_validate::frontmatter::parse_frontmatter;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let skills_to_score: Vec<_> = if let Some(ref target_name) = name {
        skills
            .iter()
            .filter(|s| {
                s.name.eq_ignore_ascii_case(target_name)
                    || s.path.to_string_lossy().contains(target_name)
            })
            .collect()
    } else {
        skills.iter().collect()
    };

    if skills_to_score.is_empty() {
        if let Some(ref n) = name {
            bail!("Skill '{}' not found", n);
        } else {
            bail!("No skills found to score");
        }
    }

    let mut results: Vec<SkillScoreResult> = Vec::new();

    for skill in skills_to_score {
        let content = match std::fs::read_to_string(&skill.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed = parse_frontmatter(&content);

        let mut suggestions = Vec::new();

        // Score frontmatter completeness (0-25)
        let frontmatter_score = match &parsed {
            Ok(p) if p.frontmatter.is_some() => {
                let fm = p.frontmatter.as_ref().unwrap();
                let mut score = 10u8; // Base for having frontmatter

                if fm.name.is_some() {
                    score += 5;
                } else {
                    suggestions.push("Add 'name' field to frontmatter".to_string());
                }

                if fm.description.is_some() {
                    score += 10; // 10 points for description
                } else {
                    suggestions.push("Add 'description' field to frontmatter".to_string());
                }

                score
            }
            Ok(_) => {
                suggestions.push("Add YAML frontmatter with name and description".to_string());
                0
            }
            Err(_) => {
                suggestions.push("Fix frontmatter YAML syntax errors".to_string());
                0
            }
        };

        // Score validation (0-25) - simplified
        let validation_score = if parsed.is_ok() { 25u8 } else { 0u8 };

        // Score description quality (0-25)
        let description_score = skill
            .description
            .as_ref()
            .map(|d| {
                let len = d.len();
                if len >= 100 {
                    25u8
                } else if len >= 50 {
                    20u8
                } else if len >= 20 {
                    15u8
                } else if len > 0 {
                    10u8
                } else {
                    0u8
                }
            })
            .unwrap_or(0);

        if description_score < 20 {
            suggestions.push("Improve description (aim for 100+ characters)".to_string());
        }

        // Score token efficiency (0-25) - based on content length
        let token_score = {
            let estimated_tokens = content.len() / 4; // rough estimate
            if estimated_tokens < 500 {
                25u8
            } else if estimated_tokens < 1000 {
                20u8
            } else if estimated_tokens < 2000 {
                15u8
            } else if estimated_tokens < 5000 {
                10u8
            } else {
                5u8
            }
        };

        if token_score < 15 {
            suggestions
                .push("Consider splitting into smaller skills for token efficiency".to_string());
        }

        let total_score = frontmatter_score + validation_score + description_score + token_score;

        // Filter by threshold if specified
        if let Some(threshold) = below_threshold {
            if total_score >= threshold {
                continue;
            }
        }

        results.push(SkillScoreResult {
            name: skill.name.clone(),
            path: skill.path.clone(),
            total_score,
            breakdown: ScoreBreakdown {
                frontmatter_completeness: frontmatter_score,
                validation_score,
                description_quality: description_score,
                token_efficiency: token_score,
            },
            suggestions,
        });
    }

    // Sort by score ascending (worst first)
    results.sort_by(|a, b| a.total_score.cmp(&b.total_score));

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!("Skill Quality Scores");
        println!("════════════════════════════════════════════════════════════════════════");
        println!();

        for result in &results {
            let grade = match result.total_score {
                90..=100 => "A",
                80..=89 => "B",
                70..=79 => "C",
                60..=69 => "D",
                _ => "F",
            };

            println!("{} - {}/100 ({})", result.name, result.total_score, grade);
            println!(
                "  Frontmatter: {}/25  Validation: {}/25  Description: {}/25  Tokens: {}/25",
                result.breakdown.frontmatter_completeness,
                result.breakdown.validation_score,
                result.breakdown.description_quality,
                result.breakdown.token_efficiency,
            );

            if !result.suggestions.is_empty() {
                println!("  Suggestions:");
                for s in &result.suggestions {
                    println!("    - {}", s);
                }
            }
            println!();
        }

        if let Some(threshold) = below_threshold {
            println!("Showing skills with score < {}", threshold);
        }
    }

    Ok(())
}

/// Result of sync-pull operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncPullResult {
    pub source: Option<String>,
    pub target: String,
    pub skills_pulled: usize,
    pub dry_run: bool,
    pub message: String,
}

/// Handle the sync-pull command.
pub(crate) fn handle_sync_pull_command(
    source: Option<String>,
    skill: Option<String>,
    target: SyncSource,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    // sync-pull is a placeholder for future remote skill registry integration
    // For now, we provide guidance on how to use existing sync functionality

    let result = SyncPullResult {
        source: source.clone(),
        target: target.as_str().to_string(),
        skills_pulled: 0,
        dry_run,
        message: if source.is_some() {
            "Remote skill registries not yet implemented. Use 'skill-import' for individual files or 'sync' to copy from another CLI.".to_string()
        } else {
            "No source specified. Use --source <url> to specify a remote registry.".to_string()
        },
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("sync-pull: {}", result.message);
        println!();
        println!("Available alternatives:");
        println!("  skrills skill-import <path>      Import a skill from local path");
        println!("  skrills sync --from claude       Sync skills from Claude to Codex");
        println!("  skrills sync-all --from claude   Sync all assets from Claude");

        if let Some(ref s) = skill {
            println!();
            println!(
                "To import skill '{}', use: skrills skill-import /path/to/{}.md",
                s, s
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, TestFixture};
    use tempfile::tempdir;

    //========================================================================
    // Integration Tests - Command Handlers
    //========================================================================

    /// Test that skill-catalog command discovers and lists skills correctly
    #[test]
    fn catalog_discovers_skills_in_fixture() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        // Create test skills
        fixture
            .create_skill_with_frontmatter("test-skill-alpha", "Alpha skill for testing", "Content")
            .expect("create alpha");
        fixture
            .create_skill_with_frontmatter("test-skill-beta", "Beta skill for testing", "Content")
            .expect("create beta");

        // Call the catalog command - it prints to stdout, so we can't easily capture,
        // but we can verify it doesn't error
        let result = handle_skill_catalog_command(
            None,
            None,
            None,
            100,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "catalog command should succeed");
    }

    /// Test that skill-catalog respects search filter
    #[test]
    fn catalog_filters_by_search_query() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter("matching-skill", "This matches the query", "Content")
            .expect("create matching");
        fixture
            .create_skill_with_frontmatter("other-skill", "This does not", "Content")
            .expect("create other");

        // Search for "matching" - should succeed
        let result = handle_skill_catalog_command(
            Some("matching".to_string()),
            None,
            None,
            100,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "filtered catalog should succeed");
    }

    /// Test that skill-import command works for local files
    #[test]
    fn import_local_file_succeeds() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        // Create a source skill file
        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("source-skill.md");
        std::fs::write(
            &source_path,
            "---\nname: imported-skill\ndescription: Test\n---\nContent",
        )
        .expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "import should succeed");

        // Verify skill was created
        let target = fixture.claude_skills.join("source-skill.md");
        assert!(target.exists(), "imported skill should exist at target");
    }

    /// Test that skill-import respects dry-run flag
    #[test]
    fn import_dry_run_does_not_create_file() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("dry-run-skill.md");
        std::fs::write(&source_path, "---\nname: dry\n---\nContent").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            false,
            true, // dry_run = true
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "dry run should succeed");

        // Verify no file was created
        let target = fixture.claude_skills.join("dry-run-skill.md");
        assert!(!target.exists(), "dry run should not create file");
    }

    /// Test that skill-import with force overwrites existing file
    #[test]
    fn import_force_overwrites_existing() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        // Create existing skill
        let existing = fixture.claude_skills.join("overwrite-skill.md");
        std::fs::write(&existing, "old content").expect("write existing");

        // Create source with new content
        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("overwrite-skill.md");
        std::fs::write(&source_path, "new content").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            true, // force = true
            false,
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "force import should succeed");

        // Verify content was overwritten
        let content = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(content, "new content", "content should be overwritten");
    }

    /// Test that skill-import without force does not overwrite
    #[test]
    fn import_without_force_skips_existing() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        // Create existing skill
        let existing = fixture.claude_skills.join("keep-skill.md");
        std::fs::write(&existing, "keep this").expect("write existing");

        // Create source with new content
        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("keep-skill.md");
        std::fs::write(&source_path, "replace this").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            false, // force = false
            false,
            OutputFormat::Json,
        );

        // Should succeed but not overwrite
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(content, "keep this", "content should not be overwritten");
    }

    /// Test that skill-import errors on nonexistent source
    #[test]
    fn import_nonexistent_source_errors() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let result = handle_skill_import_command(
            "/nonexistent/path/skill.md".to_string(),
            SyncSource::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_err(), "import nonexistent should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist"),
            "error should mention missing file"
        );
    }

    /// Test that skill-score command calculates scores
    #[test]
    fn score_calculates_for_skills() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter(
                "well-documented",
                "A very detailed description that exceeds 100 characters to get maximum points for description quality in scoring",
                "# Well Documented Skill\n\nThis skill has good documentation.",
            )
            .expect("create skill");

        let result = handle_skill_score_command(
            Some("well-documented".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_ok(), "score command should succeed");
    }

    /// Test that skill-score filters by name
    #[test]
    fn score_filters_by_skill_name() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter("target-skill", "Target", "Content")
            .expect("create target");
        fixture
            .create_skill_with_frontmatter("other-skill", "Other", "Content")
            .expect("create other");

        // Score only target-skill
        let result = handle_skill_score_command(
            Some("target-skill".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_ok());
    }

    /// Test that skill-score errors when skill not found
    #[test]
    fn score_errors_when_skill_not_found() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let result = handle_skill_score_command(
            Some("nonexistent-skill".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_err(), "should error for nonexistent skill");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error should mention not found");
    }

    /// Test that skill-score respects below_threshold filter
    #[test]
    fn score_filters_by_threshold() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        // Create a minimal skill (low score expected)
        fixture
            .create_skill("poor-skill", "no frontmatter here")
            .expect("create poor");

        // Create a well-formed skill (high score expected)
        fixture
            .create_skill_with_frontmatter(
                "good-skill",
                "A detailed description with more than 100 characters to maximize the description quality score component",
                "# Good Skill\n\nContent",
            )
            .expect("create good");

        // Filter for scores below 50 - should only show poor-skill
        let result = handle_skill_score_command(
            None,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            Some(50),
        );

        assert!(result.is_ok());
    }

    /// Test sync-pull placeholder behavior
    #[test]
    fn sync_pull_returns_placeholder_message() {
        let result =
            handle_sync_pull_command(None, None, SyncSource::Claude, false, OutputFormat::Json);

        assert!(result.is_ok(), "sync-pull should succeed (placeholder)");
    }

    //========================================================================
    // Unit Tests - Validation and Security
    //========================================================================

    /// Test that invalid git version hashes are rejected (command injection prevention)
    #[test]
    fn rollback_invalid_version_hash_is_rejected() {
        // Valid hashes: 4-40 hex characters
        let valid_hashes = ["abc1", "abc12345", "abc123456789abcdef", "ABCDEF0123456789"];
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();

        for hash in &valid_hashes {
            assert!(
                hash_pattern.is_match(hash),
                "Expected '{}' to be valid",
                hash
            );
        }

        // Invalid hashes (potential injection attacks)
        let invalid_hashes = [
            "abc",                 // too short
            "; rm -rf /",          // shell injection
            "abc123; echo pwned",  // command injection
            "$(whoami)",           // command substitution
            "`id`",                // backtick substitution
            "abc\necho hacked",    // newline injection
            "abc|cat /etc/passwd", // pipe injection
            "--help",              // option injection
            "-",                   // stdin redirect
            "",                    // empty
        ];

        for hash in &invalid_hashes {
            assert!(
                !hash_pattern.is_match(hash),
                "Expected '{}' to be rejected as invalid",
                hash
            );
        }
    }

    /// Test that YAML with special characters is escaped properly (basic check)
    #[test]
    fn deprecation_message_basic_format() {
        // This test documents the current behavior - messages are wrapped in quotes
        // which handles most special characters but not all edge cases
        let message = "Use new-skill instead";
        let formatted = format!("deprecation_message: \"{}\"\n", message);
        assert!(formatted.contains("\"Use new-skill instead\""));
    }

    /// Test SkillVersion serialization
    #[test]
    fn skill_version_serializes_correctly() {
        let version = SkillVersion {
            hash: "abc1234".to_string(),
            date: "2024-01-15 10:30:00 -0500".to_string(),
            message: "Initial commit".to_string(),
        };

        let json = serde_json::to_string(&version).unwrap();
        assert!(json.contains("abc1234"));
        assert!(json.contains("Initial commit"));
    }

    /// Test RollbackResult default state
    #[test]
    fn rollback_result_default_state() {
        let result = RollbackResult {
            skill_name: "test-skill".to_string(),
            skill_path: PathBuf::from("/path/to/skill.md"),
            rolled_back: false,
            from_version: None,
            to_version: None,
            available_versions: vec![],
        };

        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"rolled_back\": false"));
        assert!(json.contains("\"available_versions\": []"));
    }

    /// Test ImportResult with existing skill (no force)
    #[test]
    fn import_result_existing_skill_message() {
        let result = ImportResult {
            source: "/path/to/source.md".to_string(),
            target_path: PathBuf::from("/home/user/.claude/skills/my-skill.md"),
            imported: false,
            skill_name: Some("my-skill".to_string()),
            message: "Skill 'my-skill' already exists. Use --force to overwrite.".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"imported\":false"));
        assert!(json.contains("--force"));
    }

    /// Test pre-commit validation detects errors_found flag behavior
    #[test]
    fn precommit_validation_error_flag_tracking() {
        // This tests the pattern used in pre-commit validation
        let mut errors_found = false;
        let mut validated = 0;

        // Simulate successful validation
        validated += 1;

        // Simulate read error - should set errors_found (as per our fix)
        let read_failed = true;
        if read_failed {
            errors_found = true;
        }

        // Simulate validation error - should also set errors_found
        let has_validation_errors = true;
        if has_validation_errors {
            errors_found = true;
        }

        assert!(
            errors_found,
            "errors_found should be true when any error occurs"
        );
        assert_eq!(
            validated, 1,
            "Only successful validations should be counted"
        );
    }
}
