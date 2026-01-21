//! CLI handler for the skill-diff command.

use crate::cli::OutputFormat;
use anyhow::{anyhow, Result};
use serde_json::json;
use skrills_discovery::{default_roots, discover_skills, SkillSource};
use skrills_state::home_dir;
use std::collections::HashMap;
use std::path::PathBuf;

/// Handle the `skill-diff` command.
pub(crate) fn handle_skill_diff_command(
    name: String,
    format: OutputFormat,
    context_lines: usize,
) -> Result<()> {
    // Discover skills from each source separately to avoid deduplication.
    // This ensures we find ALL versions of a skill across different CLIs.
    // Use default_roots which includes all CLIs (Codex, Claude, Copilot).
    let home = home_dir()?;
    let roots = default_roots(&home);
    let search_name = normalize_skill_name(&name);
    let mut versions: HashMap<SkillSource, (PathBuf, String)> = HashMap::new();

    // Process each root individually to capture all versions
    for root in &roots {
        let skills = discover_skills(std::slice::from_ref(root), None)?;
        for meta in skills.iter() {
            let normalized_meta_name = normalize_skill_name(&meta.name);
            if normalized_meta_name == search_name || meta.name == name {
                let content = match std::fs::read_to_string(&meta.path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                versions.insert(meta.source.clone(), (meta.path.clone(), content));
            }
        }
    }

    if versions.is_empty() {
        return Err(anyhow!("Skill '{}' not found in any CLI", name));
    }

    if versions.len() == 1 {
        let (source, (path, _)) = versions.iter().next().unwrap();
        if format.is_json() {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "skill": name,
                    "found_in": [format!("{:?}", source)],
                    "identical": true,
                    "message": "Skill only exists in one CLI"
                }))?
            );
        } else {
            println!(
                "Skill '{}' only found in {:?} at {}",
                name,
                source,
                path.display()
            );
            println!("No diff available - skill exists in only one location.");
        }
        return Ok(());
    }

    // Compare versions
    let sources: Vec<_> = versions.keys().cloned().collect();
    let mut comparisons = Vec::new();
    let mut all_identical = true;

    for i in 0..sources.len() {
        for j in (i + 1)..sources.len() {
            let source_a = &sources[i];
            let source_b = &sources[j];
            let (path_a, content_a) = &versions[source_a];
            let (path_b, content_b) = &versions[source_b];

            let diff = unified_diff(content_a, content_b, context_lines);
            let is_identical = content_a == content_b;

            if !is_identical {
                all_identical = false;
            }

            comparisons.push(json!({
                "source_a": format!("{:?}", source_a),
                "source_b": format!("{:?}", source_b),
                "path_a": path_a.to_string_lossy(),
                "path_b": path_b.to_string_lossy(),
                "identical": is_identical,
                "diff": if is_identical { None } else { Some(&diff) },
                "token_diff": estimate_token_diff(content_a, content_b)
            }));

            if !format.is_json() && !is_identical {
                println!("\n=== {:?} vs {:?} ===", source_a, source_b);
                println!("--- {:?}: {}", source_a, path_a.display());
                println!("+++ {:?}: {}", source_b, path_b.display());
                println!("{}", diff);
            }
        }
    }

    if format.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": name,
                "found_in": sources.iter().map(|s| format!("{:?}", s)).collect::<Vec<_>>(),
                "identical": all_identical,
                "comparisons": comparisons
            }))?
        );
    } else if all_identical {
        println!(
            "Skill '{}' is identical across {} CLIs: {:?}",
            name,
            versions.len(),
            sources
        );
    } else {
        println!(
            "\nSummary: Skill '{}' found in {} CLIs with differences",
            name,
            versions.len()
        );
    }

    Ok(())
}

/// Generate a unified diff between two strings.
fn unified_diff(a: &str, b: &str, context: usize) -> String {
    use std::fmt::Write;

    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();

    // Simple line-by-line diff (not optimal but functional)
    let mut output = String::new();
    let max_len = lines_a.len().max(lines_b.len());

    let mut i = 0;
    while i < max_len {
        let line_a = lines_a.get(i);
        let line_b = lines_b.get(i);

        match (line_a, line_b) {
            (Some(a), Some(b)) if a == b => {
                // Context line - only show if near a change
                let has_nearby_change = (i.saturating_sub(context)
                    ..=(i + context).min(max_len - 1))
                    .any(|j| lines_a.get(j) != lines_b.get(j));
                if has_nearby_change {
                    let _ = writeln!(output, " {}", a);
                }
            }
            (Some(a), Some(b)) => {
                let _ = writeln!(output, "-{}", a);
                let _ = writeln!(output, "+{}", b);
            }
            (Some(a), None) => {
                let _ = writeln!(output, "-{}", a);
            }
            (None, Some(b)) => {
                let _ = writeln!(output, "+{}", b);
            }
            (None, None) => {}
        }
        i += 1;
    }

    output
}

/// Estimate token count difference between two contents.
fn estimate_token_diff(a: &str, b: &str) -> i64 {
    let tokens_a = estimate_tokens(a);
    let tokens_b = estimate_tokens(b);
    tokens_b as i64 - tokens_a as i64
}

/// Simple token estimation (words + punctuation).
fn estimate_tokens(content: &str) -> usize {
    // Rough estimate: ~4 chars per token for English text
    content.len() / 4
}

/// Normalize a skill name by extracting the base name from the path.
///
/// Converts names like "test-skill/SKILL.md" or "plugins/cache/.../skills/my-skill/SKILL.md"
/// to just "test-skill" or "my-skill".
fn normalize_skill_name(name: &str) -> String {
    // Remove trailing /SKILL.md or SKILL.md
    let name = name
        .trim_end_matches("/SKILL.md")
        .trim_end_matches("SKILL.md");

    // Extract the last component (skill directory name)
    if let Some(last_slash) = name.rfind('/') {
        name[last_slash + 1..].to_string()
    } else {
        name.to_string()
    }
}
