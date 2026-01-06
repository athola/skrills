//! YAML frontmatter parsing for SKILL.md files.
//!
//! Codex CLI requires YAML frontmatter with:
//! - `name`: max 100 characters
//! - `description`: max 500 characters
//!
//! Claude Code is more permissive and doesn't require frontmatter.
//!
//! ## Dependency Declaration
//!
//! Skills can declare dependencies via the `depends` field:
//!
//! ```yaml
//! depends:
//!   - base-skill                    # Simple: any version, any source
//!   - name: utility-skill           # Structured form
//!     version: "^2.0"               # Semver constraint
//!     source: codex                 # Source pinning
//!     optional: true                # Optional dependency
//!   - codex:auth-helpers@^1.0       # Compact: source:name@version
//! ```

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// A declared skill dependency in frontmatter.
///
/// Supports three formats:
/// - Simple: `"skill-name"` (any version, any source)
/// - Compact: `"source:skill-name@version"` (with optional source and version)
/// - Structured: `{ name, version, source, optional }` (explicit fields)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeclaredDependency {
    /// Simple string form: "skill-name" or "source:skill-name@version"
    Simple(String),
    /// Structured form with explicit fields.
    Structured {
        /// The skill name.
        name: String,
        /// Semver version constraint (e.g., "^1.0", ">=2.0.0").
        #[serde(default)]
        version: Option<String>,
        /// Source to look up the skill from (e.g., "codex", "claude").
        #[serde(default)]
        source: Option<String>,
        /// Whether this dependency is optional.
        #[serde(default)]
        optional: bool,
    },
}

/// Normalized dependency after parsing all formats into a common structure.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedDependency {
    /// The skill name (without source prefix or version suffix).
    pub name: String,
    /// Semver version requirement, if specified.
    pub version_req: Option<semver::VersionReq>,
    /// Source to look up the skill from.
    pub source: Option<String>,
    /// Whether this dependency is optional.
    pub optional: bool,
}

// Regex for parsing compact dependency syntax: [source:]name[@version]
static COMPACT_DEP_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:([a-z]+):)?([^@:\s]+)(?:@(.+))?$").expect("valid regex"));

impl DeclaredDependency {
    /// Normalize the dependency into a common structure.
    ///
    /// Parses compact syntax and validates version constraints.
    pub fn normalize(&self) -> Result<NormalizedDependency, String> {
        match self {
            DeclaredDependency::Simple(s) => parse_compact_dependency(s),
            DeclaredDependency::Structured {
                name,
                version,
                source,
                optional,
            } => {
                let version_req = match version {
                    Some(v) => Some(
                        semver::VersionReq::parse(v)
                            .map_err(|e| format!("Invalid version constraint '{}': {}", v, e))?,
                    ),
                    None => None,
                };
                Ok(NormalizedDependency {
                    name: name.clone(),
                    version_req,
                    source: source.clone(),
                    optional: *optional,
                })
            }
        }
    }
}

/// Parse compact dependency syntax: `[source:]name[@version]`
///
/// Examples:
/// - `"base-skill"` -> name=base-skill, source=None, version=None
/// - `"codex:base-skill"` -> name=base-skill, source=Some("codex")
/// - `"base-skill@^1.0"` -> name=base-skill, version=Some(^1.0)
/// - `"codex:base-skill@^1.0"` -> all three
fn parse_compact_dependency(s: &str) -> Result<NormalizedDependency, String> {
    let caps = COMPACT_DEP_REGEX
        .captures(s.trim())
        .ok_or_else(|| format!("Invalid dependency format: '{}'", s))?;

    let source = caps.get(1).map(|m| m.as_str().to_string());
    let name = caps
        .get(2)
        .map(|m| m.as_str().trim().to_string())
        .ok_or_else(|| format!("Missing skill name in dependency: '{}'", s))?;
    let version_str = caps.get(3).map(|m| m.as_str());

    let version_req = match version_str {
        Some(v) => Some(
            semver::VersionReq::parse(v)
                .map_err(|e| format!("Invalid version constraint '{}': {}", v, e))?,
        ),
        None => None,
    };

    Ok(NormalizedDependency {
        name,
        version_req,
        source,
        optional: false, // Compact syntax doesn't support optional
    })
}

/// Parsed frontmatter from a SKILL.md file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill name (max 100 chars for Codex).
    pub name: Option<String>,
    /// Skill description (max 500 chars for Codex).
    pub description: Option<String>,
    /// Skill version (semver).
    #[serde(default)]
    pub version: Option<String>,
    /// Declared dependencies on other skills.
    #[serde(default)]
    pub depends: Vec<DeclaredDependency>,
    /// Additional fields that may be present.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_yaml::Value>,
}

impl SkillFrontmatter {
    /// Get all dependencies normalized into a common structure.
    ///
    /// Returns errors for any dependencies that fail to parse.
    pub fn normalized_dependencies(&self) -> Result<Vec<NormalizedDependency>, String> {
        self.depends
            .iter()
            .map(|d| d.normalize())
            .collect::<Result<Vec<_>, _>>()
    }

    /// Parse the skill's own version as semver.
    pub fn parsed_version(&self) -> Option<Result<semver::Version, semver::Error>> {
        self.version.as_ref().map(|v| semver::Version::parse(v))
    }
}

/// Result of parsing frontmatter.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// Parsed frontmatter, if present.
    pub frontmatter: Option<SkillFrontmatter>,
    /// The markdown content after frontmatter.
    pub content: String,
    /// Raw frontmatter YAML string, if present.
    pub raw_frontmatter: Option<String>,
    /// Line number where content starts (1-indexed).
    pub content_start_line: usize,
}

/// Check if a skill file has frontmatter.
pub fn has_frontmatter(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("---")
}

/// Split content into frontmatter and body sections.
///
/// Returns (frontmatter_yaml, body_content, content_start_line).
pub fn split_frontmatter(content: &str) -> (Option<String>, String, usize) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return (None, content.to_string(), 1);
    }

    // Count leading whitespace lines
    let leading_lines = content
        .lines()
        .take_while(|line| line.trim().is_empty())
        .count();

    // Find the closing ---
    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_open.find("\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 4..];
        let rest = rest.trim_start_matches(['\r', '\n']);

        // Calculate content start line
        let frontmatter_lines = yaml.lines().count() + 2; // +2 for opening and closing ---
        let content_start = leading_lines + frontmatter_lines + 1;

        (Some(yaml.to_string()), rest.to_string(), content_start)
    } else if let Some(end_pos) = after_open.find("\r\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 5..];
        let rest = rest.trim_start_matches(['\r', '\n']);

        let frontmatter_lines = yaml.lines().count() + 2;
        let content_start = leading_lines + frontmatter_lines + 1;

        (Some(yaml.to_string()), rest.to_string(), content_start)
    } else {
        // No closing ---, treat entire content as body
        (None, content.to_string(), 1)
    }
}

/// Parse YAML frontmatter from a skill file.
pub fn parse_frontmatter(content: &str) -> Result<ParsedSkill, String> {
    let (raw_yaml, body, content_start) = split_frontmatter(content);

    let frontmatter = if let Some(ref yaml) = raw_yaml {
        match serde_yaml::from_str::<SkillFrontmatter>(yaml) {
            Ok(fm) => Some(fm),
            Err(e) => return Err(format!("Invalid YAML frontmatter: {e}")),
        }
    } else {
        None
    };

    Ok(ParsedSkill {
        frontmatter,
        content: body,
        raw_frontmatter: raw_yaml,
        content_start_line: content_start,
    })
}

/// Generate frontmatter YAML from a name and description.
pub fn generate_frontmatter(name: &str, description: &str) -> String {
    // Escape special characters in YAML strings
    let name_escaped = if name.contains(':') || name.contains('#') {
        format!("\"{}\"", name.replace('"', "\\\""))
    } else {
        name.to_string()
    };

    let desc_escaped = if description.contains('\n') || description.len() > 80 {
        // Use YAML literal block scalar for multi-line or long descriptions
        let line_count = description.lines().count();
        let mut indented = String::with_capacity(description.len() + (2 * line_count) + line_count);
        for (idx, line) in description.lines().enumerate() {
            if idx > 0 {
                indented.push('\n');
            }
            indented.push_str("  ");
            indented.push_str(line);
        }
        format!("|\n{indented}")
    } else if description.contains(':') || description.contains('#') {
        format!("\"{}\"", description.replace('"', "\\\""))
    } else {
        description.to_string()
    };

    format!("---\nname: {name_escaped}\ndescription: {desc_escaped}\n---\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_frontmatter() {
        assert!(has_frontmatter("---\nname: test\n---\n# Content"));
        assert!(has_frontmatter("  ---\nname: test\n---\n# Content"));
        assert!(!has_frontmatter("# No frontmatter"));
        assert!(!has_frontmatter("-- not quite"));
    }

    #[test]
    fn test_split_frontmatter() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Heading\nBody";
        let (yaml, body, line) = split_frontmatter(content);

        assert!(yaml.is_some());
        assert!(yaml.unwrap().contains("name: test"));
        assert!(body.starts_with("# Heading"));
        assert_eq!(line, 5);
    }

    #[test]
    fn test_split_no_frontmatter() {
        let content = "# Just markdown\nNo frontmatter here.";
        let (yaml, body, line) = split_frontmatter(content);

        assert!(yaml.is_none());
        assert_eq!(body, content);
        assert_eq!(line, 1);
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nname: my-skill\ndescription: Does something useful\n---\n# My Skill";
        let parsed = parse_frontmatter(content).unwrap();

        assert!(parsed.frontmatter.is_some());
        let fm = parsed.frontmatter.unwrap();
        assert_eq!(fm.name, Some("my-skill".to_string()));
        assert_eq!(fm.description, Some("Does something useful".to_string()));
        assert!(parsed.content.starts_with("# My Skill"));
    }

    #[test]
    fn test_generate_frontmatter() {
        let fm = generate_frontmatter("test-skill", "A simple test skill");
        assert!(fm.contains("name: test-skill"));
        assert!(fm.contains("description: A simple test skill"));
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n"));
    }

    #[test]
    fn test_generate_frontmatter_special_chars() {
        let fm = generate_frontmatter("skill:name", "Contains: colon");
        assert!(fm.contains("name: \"skill:name\""));
        assert!(fm.contains("description: \"Contains: colon\""));
    }

    // ============================================================
    // Dependency parsing tests
    // ============================================================

    #[test]
    fn test_parse_simple_dependency() {
        let dep = DeclaredDependency::Simple("base-skill".to_string());
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "base-skill");
        assert!(normalized.version_req.is_none());
        assert!(normalized.source.is_none());
        assert!(!normalized.optional);
    }

    #[test]
    fn test_parse_compact_with_source() {
        let dep = DeclaredDependency::Simple("codex:base-skill".to_string());
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "base-skill");
        assert!(normalized.version_req.is_none());
        assert_eq!(normalized.source, Some("codex".to_string()));
        assert!(!normalized.optional);
    }

    #[test]
    fn test_parse_compact_with_version() {
        let dep = DeclaredDependency::Simple("base-skill@^1.0".to_string());
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "base-skill");
        assert!(normalized.version_req.is_some());
        let req = normalized.version_req.unwrap();
        assert!(req.matches(&semver::Version::new(1, 5, 0)));
        assert!(!req.matches(&semver::Version::new(2, 0, 0)));
        assert!(normalized.source.is_none());
    }

    #[test]
    fn test_parse_compact_full() {
        let dep = DeclaredDependency::Simple("codex:auth-helpers@^2.0.0".to_string());
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "auth-helpers");
        assert_eq!(normalized.source, Some("codex".to_string()));
        let req = normalized.version_req.unwrap();
        assert!(req.matches(&semver::Version::new(2, 1, 0)));
        assert!(!req.matches(&semver::Version::new(3, 0, 0)));
    }

    #[test]
    fn test_parse_structured_dependency() {
        let dep = DeclaredDependency::Structured {
            name: "utility-skill".to_string(),
            version: Some(">=1.0, <2.0".to_string()),
            source: Some("claude".to_string()),
            optional: true,
        };
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "utility-skill");
        assert_eq!(normalized.source, Some("claude".to_string()));
        assert!(normalized.optional);
        let req = normalized.version_req.unwrap();
        assert!(req.matches(&semver::Version::new(1, 5, 0)));
        assert!(!req.matches(&semver::Version::new(2, 0, 0)));
    }

    #[test]
    fn test_parse_structured_minimal() {
        let dep = DeclaredDependency::Structured {
            name: "minimal".to_string(),
            version: None,
            source: None,
            optional: false,
        };
        let normalized = dep.normalize().unwrap();

        assert_eq!(normalized.name, "minimal");
        assert!(normalized.version_req.is_none());
        assert!(normalized.source.is_none());
        assert!(!normalized.optional);
    }

    #[test]
    fn test_invalid_version_constraint() {
        let dep = DeclaredDependency::Simple("skill@not-a-version".to_string());
        let result = dep.normalize();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid version constraint"));
    }

    #[test]
    fn test_frontmatter_with_dependencies() {
        let content = r#"---
name: my-skill
description: A skill with dependencies
version: 1.0.0
depends:
  - base-skill
  - codex:utility@^2.0
  - name: optional-helper
    optional: true
---
# My Skill"#;

        let parsed = parse_frontmatter(content).unwrap();
        let fm = parsed.frontmatter.unwrap();

        assert_eq!(fm.name, Some("my-skill".to_string()));
        assert_eq!(fm.version, Some("1.0.0".to_string()));
        assert_eq!(fm.depends.len(), 3);

        let deps = fm.normalized_dependencies().unwrap();
        assert_eq!(deps.len(), 3);

        assert_eq!(deps[0].name, "base-skill");
        assert!(deps[0].source.is_none());

        assert_eq!(deps[1].name, "utility");
        assert_eq!(deps[1].source, Some("codex".to_string()));
        assert!(deps[1].version_req.is_some());

        assert_eq!(deps[2].name, "optional-helper");
        assert!(deps[2].optional);
    }

    #[test]
    fn test_frontmatter_without_dependencies_backward_compat() {
        let content = "---\nname: old-skill\ndescription: No deps\n---\n# Old";
        let parsed = parse_frontmatter(content).unwrap();
        let fm = parsed.frontmatter.unwrap();

        assert!(fm.depends.is_empty());
        assert!(fm.normalized_dependencies().unwrap().is_empty());
    }

    #[test]
    fn test_parsed_version() {
        let fm = SkillFrontmatter {
            version: Some("1.2.3".to_string()),
            ..Default::default()
        };

        let version = fm.parsed_version().unwrap().unwrap();
        assert_eq!(version, semver::Version::new(1, 2, 3));
    }

    #[test]
    fn test_parsed_version_invalid() {
        let fm = SkillFrontmatter {
            version: Some("not-a-version".to_string()),
            ..Default::default()
        };

        assert!(fm.parsed_version().unwrap().is_err());
    }

    #[test]
    fn test_various_semver_constraints() {
        // Test different semver constraint formats
        let test_cases = [
            ("skill@^1.0", "1.9.9", true),
            ("skill@^1.0", "2.0.0", false),
            ("skill@~1.2.3", "1.2.5", true),
            ("skill@~1.2.3", "1.3.0", false),
            ("skill@>=1.0.0", "2.0.0", true),
            ("skill@<2.0.0", "1.9.9", true),
            ("skill@<2.0.0", "2.0.0", false),
            ("skill@=1.2.3", "1.2.3", true),
            ("skill@=1.2.3", "1.2.4", false),
        ];

        for (dep_str, version_str, should_match) in test_cases {
            let dep = DeclaredDependency::Simple(dep_str.to_string());
            let normalized = dep.normalize().unwrap();
            let req = normalized.version_req.unwrap();
            let version = semver::Version::parse(version_str).unwrap();

            assert_eq!(
                req.matches(&version),
                should_match,
                "{}@{} should{} match",
                dep_str,
                version_str,
                if should_match { "" } else { " not" }
            );
        }
    }

    // ============================================================
    // Edge case tests
    // ============================================================

    #[test]
    fn test_dependency_with_whitespace() {
        // Given a dependency string with leading/trailing whitespace
        let dep = DeclaredDependency::Simple("  base-skill  ".to_string());

        // When normalized
        let normalized = dep.normalize().unwrap();

        // Then whitespace should be trimmed from name
        assert_eq!(normalized.name, "base-skill");
    }

    #[test]
    fn test_dependency_version_only_fails() {
        // Given a dependency with only version (missing name)
        let dep = DeclaredDependency::Simple("@^1.0".to_string());

        // When normalized
        let result = dep.normalize();

        // Then it should fail with missing name error
        assert!(result.is_err());
    }

    #[test]
    fn test_dependency_source_without_name_fails() {
        // Given a dependency with only source prefix (no name after colon)
        let dep = DeclaredDependency::Simple("codex:".to_string());

        // When normalized
        let result = dep.normalize();

        // Then it should fail due to missing name
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid dependency format"));
    }

    #[test]
    fn test_prerelease_version_constraint() {
        // Given a dependency with prerelease version
        let dep = DeclaredDependency::Simple("skill@^1.0.0-beta.1".to_string());

        // When normalized
        let normalized = dep.normalize().unwrap();

        // Then it should parse the prerelease constraint
        let req = normalized.version_req.unwrap();
        // Prerelease versions only match prerelease constraints
        assert!(req.matches(&semver::Version::parse("1.0.0-beta.2").unwrap()));
    }

    #[test]
    fn test_structured_dependency_with_empty_version() {
        // Given a structured dependency with empty version string
        let dep = DeclaredDependency::Structured {
            name: "skill".to_string(),
            version: Some("".to_string()),
            source: None,
            optional: false,
        };

        // When normalized
        let result = dep.normalize();

        // Then it should fail due to invalid version
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid version constraint"));
    }

    #[test]
    fn test_wildcard_version_constraint() {
        // Given a dependency with wildcard version
        let dep = DeclaredDependency::Simple("skill@*".to_string());

        // When normalized
        let normalized = dep.normalize().unwrap();

        // Then wildcard should match any version
        let req = normalized.version_req.unwrap();
        assert!(req.matches(&semver::Version::new(0, 0, 1)));
        assert!(req.matches(&semver::Version::new(99, 99, 99)));
    }

    #[test]
    fn test_complex_version_range() {
        // Given a dependency with complex range (semver crate format)
        // Note: semver crate uses comma-separated constraints but doesn't support !=
        let dep = DeclaredDependency::Structured {
            name: "skill".to_string(),
            version: Some(">=1.0.0, <2.0.0".to_string()),
            source: None,
            optional: false,
        };

        // When normalized
        let normalized = dep.normalize().unwrap();

        // Then it should match versions in range
        let req = normalized.version_req.unwrap();
        assert!(req.matches(&semver::Version::new(1, 0, 0)));
        assert!(req.matches(&semver::Version::new(1, 9, 9)));
        assert!(!req.matches(&semver::Version::new(2, 0, 0)));
        assert!(!req.matches(&semver::Version::new(0, 9, 9)));
    }

    #[test]
    fn test_invalid_negation_constraint() {
        // Given a dependency with != constraint (not supported by semver crate)
        let dep = DeclaredDependency::Structured {
            name: "skill".to_string(),
            version: Some("!=1.5.0".to_string()),
            source: None,
            optional: false,
        };

        // When normalized
        let result = dep.normalize();

        // Then it should fail - semver crate doesn't support != syntax
        assert!(result.is_err());
    }

    #[test]
    fn test_dependency_name_with_numbers() {
        // Given a dependency with numbers in name
        let dep = DeclaredDependency::Simple("skill-v2@^1.0".to_string());

        // When normalized
        let normalized = dep.normalize().unwrap();

        // Then name should preserve numbers
        assert_eq!(normalized.name, "skill-v2");
        assert!(normalized.version_req.is_some());
    }

    #[test]
    fn test_frontmatter_empty_depends_array() {
        // Given frontmatter with explicit empty depends
        let content = r#"---
name: my-skill
description: Test
depends: []
---
# Content"#;

        // When parsed
        let parsed = parse_frontmatter(content).unwrap();
        let fm = parsed.frontmatter.unwrap();

        // Then depends should be empty
        assert!(fm.depends.is_empty());
        assert!(fm.normalized_dependencies().unwrap().is_empty());
    }

    #[test]
    fn test_frontmatter_null_depends() {
        // Given frontmatter with null depends (YAML null)
        let content = r#"---
name: my-skill
description: Test
depends: ~
---
# Content"#;

        // When parsed - null depends should be rejected
        let parsed = parse_frontmatter(content);
        assert!(parsed.is_err());
    }
}
