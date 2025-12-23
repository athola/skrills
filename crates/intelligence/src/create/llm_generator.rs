//! Generate skills using LLM via CLI.

use super::{cli_detector, CreateSkillRequest, CreateSkillResult, CreationMethod};
use crate::context::ProjectProfile;
use anyhow::Result;
use std::process::Command;
use tracing::{debug, info};

/// Prompt template for skill generation.
const SKILL_GENERATION_PROMPT: &str = r#"Create a Claude Code / Codex CLI skill file (SKILL.md) for the following:

Name: {name}
Description: {description}
{context_section}

Requirements:
1. Start with YAML frontmatter containing:
   - name: (max 100 characters, the skill identifier)
   - description: (max 500 characters, what the skill does)
2. Write clear, actionable instructions that guide the AI assistant
3. Include practical examples where helpful
4. Structure with markdown headers for organization
5. Keep the skill focused on a single, well-defined purpose
6. Follow Claude Code skill best practices

Output ONLY the complete SKILL.md content, starting with the --- YAML frontmatter delimiter.
Do not include any explanation before or after the skill content."#;

/// Validate CLI binary name to prevent command injection.
/// Rejects paths, special characters, and suspiciously long names.
fn validate_cli_binary(name: &str) -> Result<&str, &'static str> {
    // Reject empty names
    if name.is_empty() {
        return Err("CLI binary name cannot be empty");
    }

    // Reject path separators (prevent path traversal)
    if name.contains('/') || name.contains('\\') {
        return Err("CLI binary name cannot contain path separators");
    }

    // Reject shell metacharacters
    let forbidden_chars = ['|', ';', '&', '$', '`', '(', ')', '{', '}', '<', '>', '!', '*', '?'];
    if name.chars().any(|c| forbidden_chars.contains(&c)) {
        return Err("CLI binary name contains forbidden shell characters");
    }

    // Reject suspiciously long names (typical binary names are short)
    if name.len() > 64 {
        return Err("CLI binary name is too long");
    }

    // Reject names that don't look like valid identifiers
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("CLI binary name contains invalid characters");
    }

    Ok(name)
}

fn cli_binary_override() -> Option<String> {
    let raw = std::env::var("SKRILLS_CLI_BINARY").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        None
    } else {
        match validate_cli_binary(trimmed) {
            Ok(valid) => Some(valid.to_string()),
            Err(reason) => {
                tracing::warn!(
                    binary = trimmed,
                    reason,
                    "Invalid SKRILLS_CLI_BINARY value, using auto-detection"
                );
                None
            }
        }
    }
}

/// Generate a skill using the CLI.
pub async fn generate_skill_with_llm(request: &CreateSkillRequest) -> Result<CreateSkillResult> {
    let env = cli_detector::detect_cli_environment();
    let binary = cli_binary_override().unwrap_or_else(|| cli_detector::get_cli_binary(env).into());

    info!("Generating skill using {} CLI", binary);

    // Build the prompt with context
    let context_section = if let Some(ref ctx) = request.project_context {
        build_context_section(ctx)
    } else {
        String::new()
    };

    let prompt = SKILL_GENERATION_PROMPT
        .replace("{name}", &request.name)
        .replace("{description}", &request.description)
        .replace("{context_section}", &context_section);

    debug!("Generation prompt:\n{}", prompt);

    // Shell out to CLI
    let output = Command::new(&binary)
        .args(["--print", "-p", &prompt])
        .output();

    match output {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Ok(CreateSkillResult::failure(
                    CreationMethod::LLMGenerate,
                    format!("CLI failed: {}", stderr),
                ));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let content = extract_skill_content(&stdout);

            if content.is_empty() || !content.contains("---") {
                return Ok(CreateSkillResult::failure(
                    CreationMethod::LLMGenerate,
                    "Generated content does not appear to be a valid SKILL.md",
                ));
            }

            Ok(CreateSkillResult::success(
                CreationMethod::LLMGenerate,
                content,
                None, // Path is set when writing
            ))
        }
        Err(e) => Ok(CreateSkillResult::failure(
            CreationMethod::LLMGenerate,
            format!("Failed to run CLI: {}", e),
        )),
    }
}

/// Build context section from project profile.
fn build_context_section(ctx: &ProjectProfile) -> String {
    let mut parts = Vec::new();

    // Languages
    let languages: Vec<_> = ctx.languages.keys().collect();
    if !languages.is_empty() {
        parts.push(format!(
            "- Languages: {}",
            languages
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Frameworks
    if !ctx.frameworks.is_empty() {
        parts.push(format!("- Frameworks: {}", ctx.frameworks.join(", ")));
    }

    // Project type
    parts.push(format!("- Project type: {:?}", ctx.project_type));

    // Keywords from README
    if !ctx.keywords.is_empty() {
        let keywords: Vec<_> = ctx.keywords.iter().take(10).map(|s| s.as_str()).collect();
        parts.push(format!("- Keywords: {}", keywords.join(", ")));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("\nProject Context:\n{}", parts.join("\n"))
    }
}

/// Extract skill content from CLI output.
fn extract_skill_content(response: &str) -> String {
    let response = response.trim();

    // Look for code blocks
    if let Some(start) = response.find("```") {
        // Find end of code block
        if let Some(end_offset) = response[start + 3..].find("```") {
            let content = &response[start + 3..start + 3 + end_offset];
            // Skip language identifier if present (e.g., ```markdown)
            if let Some(newline) = content.find('\n') {
                let after_lang = &content[newline + 1..];
                if after_lang.contains("---") {
                    return after_lang.trim().to_string();
                }
            }
        }
    }

    // Look for frontmatter start directly
    if let Some(start) = response.find("---") {
        return response[start..].trim().to_string();
    }

    // Return as-is if nothing found
    response.to_string()
}

/// Generate a skill synchronously (blocking).
pub fn generate_skill_sync(request: &CreateSkillRequest) -> Result<CreateSkillResult> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(generate_skill_with_llm(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_skill_content_code_block() {
        let response = r#"Here's the skill:

```markdown
---
name: test-skill
description: A test skill
---

# Test Skill

This is a test.
```

That's the skill!
"#;

        let content = extract_skill_content(response);
        assert!(content.starts_with("---"));
        assert!(content.contains("name: test-skill"));
    }

    #[test]
    fn test_extract_skill_content_direct() {
        let response = r#"---
name: test-skill
description: A test skill
---

# Test Skill

This is a test."#;

        let content = extract_skill_content(response);
        assert!(content.starts_with("---"));
        assert!(content.contains("name: test-skill"));
    }

    #[test]
    fn test_build_context_section() {
        let mut ctx = ProjectProfile::default();
        ctx.languages.insert(
            "Rust".to_string(),
            crate::context::LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );
        ctx.frameworks.push("Tokio".to_string());

        let section = build_context_section(&ctx);
        assert!(section.contains("Rust"));
        assert!(section.contains("Tokio"));
    }

    #[test]
    fn test_cli_binary_override() {
        let previous = std::env::var("SKRILLS_CLI_BINARY").ok();

        std::env::set_var("SKRILLS_CLI_BINARY", "codex");
        assert_eq!(cli_binary_override().as_deref(), Some("codex"));

        std::env::set_var("SKRILLS_CLI_BINARY", "auto");
        assert!(cli_binary_override().is_none());

        if let Some(value) = previous {
            std::env::set_var("SKRILLS_CLI_BINARY", value);
        } else {
            std::env::remove_var("SKRILLS_CLI_BINARY");
        }
    }

    #[test]
    fn test_validate_cli_binary_accepts_valid_names() {
        // Valid binary names
        assert!(validate_cli_binary("claude").is_ok());
        assert!(validate_cli_binary("codex").is_ok());
        assert!(validate_cli_binary("my-cli").is_ok());
        assert!(validate_cli_binary("my_cli").is_ok());
        assert!(validate_cli_binary("cli.exe").is_ok());
    }

    #[test]
    fn test_validate_cli_binary_rejects_path_traversal() {
        // Path separators should be rejected
        assert!(validate_cli_binary("/usr/bin/evil").is_err());
        assert!(validate_cli_binary("../../../bin/sh").is_err());
        assert!(validate_cli_binary("C:\\Windows\\cmd.exe").is_err());
    }

    #[test]
    fn test_validate_cli_binary_rejects_shell_metacharacters() {
        // Shell injection characters should be rejected
        assert!(validate_cli_binary("cmd; rm -rf /").is_err());
        assert!(validate_cli_binary("cmd | cat /etc/passwd").is_err());
        assert!(validate_cli_binary("$(whoami)").is_err());
        assert!(validate_cli_binary("`id`").is_err());
        assert!(validate_cli_binary("cmd & background").is_err());
    }

    #[test]
    fn test_validate_cli_binary_rejects_empty_and_long() {
        // Empty and excessively long names should be rejected
        assert!(validate_cli_binary("").is_err());
        assert!(validate_cli_binary(&"a".repeat(100)).is_err());
    }
}
