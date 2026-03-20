//! Model mapping between Claude and Codex/OpenAI platforms.
//!
//! When syncing configurations between agents, model names need to be
//! transformed to their equivalents on the target platform.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Claude model shorthand names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaudeModel {
    Opus,
    Sonnet,
    Haiku,
}

/// OpenAI/Codex model shorthand names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpenAiModel {
    Gpt4o,
    Gpt4oMini,
    O1,
    O1Mini,
    O3Mini,
}

impl ClaudeModel {
    /// Parse a Claude model string into the enum variant.
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        // Match both shorthand and full model IDs
        if lower.contains("opus") {
            Some(Self::Opus)
        } else if lower.contains("sonnet") {
            Some(Self::Sonnet)
        } else if lower.contains("haiku") {
            Some(Self::Haiku)
        } else {
            None
        }
    }

    /// Get the canonical shorthand name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
        }
    }
}

impl OpenAiModel {
    /// Parse an OpenAI model string into the enum variant.
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        // Match model patterns - order matters for specificity
        if lower.contains("o3-mini") || lower.contains("o3_mini") {
            Some(Self::O3Mini)
        } else if lower.contains("o1-mini") || lower.contains("o1_mini") {
            Some(Self::O1Mini)
        } else if lower.contains("o1") && !lower.contains("mini") {
            Some(Self::O1)
        } else if lower.contains("gpt-4o-mini") || lower.contains("gpt4o-mini") {
            Some(Self::Gpt4oMini)
        } else if lower.contains("gpt-4o") || lower.contains("gpt4o") {
            Some(Self::Gpt4o)
        } else {
            None
        }
    }

    /// Get the canonical model name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gpt4o => "gpt-4o",
            Self::Gpt4oMini => "gpt-4o-mini",
            Self::O1 => "o1",
            Self::O1Mini => "o1-mini",
            Self::O3Mini => "o3-mini",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Model Mapping Tables
// ─────────────────────────────────────────────────────────────────────────────

/// Claude → OpenAI mapping table.
static CLAUDE_TO_OPENAI: LazyLock<HashMap<ClaudeModel, OpenAiModel>> = LazyLock::new(|| {
    HashMap::from([
        (ClaudeModel::Opus, OpenAiModel::Gpt4o),
        (ClaudeModel::Sonnet, OpenAiModel::Gpt4oMini),
        (ClaudeModel::Haiku, OpenAiModel::Gpt4oMini),
    ])
});

/// OpenAI → Claude mapping table.
static OPENAI_TO_CLAUDE: LazyLock<HashMap<OpenAiModel, ClaudeModel>> = LazyLock::new(|| {
    HashMap::from([
        (OpenAiModel::Gpt4o, ClaudeModel::Opus),
        (OpenAiModel::Gpt4oMini, ClaudeModel::Sonnet),
        (OpenAiModel::O1, ClaudeModel::Opus),
        (OpenAiModel::O1Mini, ClaudeModel::Haiku),
        (OpenAiModel::O3Mini, ClaudeModel::Haiku),
    ])
});

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Transform a model name from source platform to target platform.
///
/// Returns `None` if the model is unrecognized (passthrough recommended).
///
/// # Arguments
/// * `model` - The model name to transform
/// * `source` - Source adapter name ("claude", "codex", "copilot", or "cursor")
/// * `target` - Target adapter name ("claude", "codex", "copilot", or "cursor")
pub fn transform_model(model: &str, source: &str, target: &str) -> Option<String> {
    // Same platform = no transformation needed
    if source == target {
        return Some(model.to_string());
    }

    match (source, target) {
        ("claude", "codex") | ("claude", "copilot") => {
            let claude_model = ClaudeModel::parse(model)?;
            let openai_model = CLAUDE_TO_OPENAI.get(&claude_model)?;
            Some(openai_model.as_str().to_string())
        }
        ("codex", "claude") | ("copilot", "claude") => {
            let openai_model = OpenAiModel::parse(model)?;
            let claude_model = OPENAI_TO_CLAUDE.get(&openai_model)?;
            Some(claude_model.as_str().to_string())
        }
        // Cursor accepts Claude model IDs directly — pass through
        ("claude", "cursor") => Some(model.to_string()),
        // Cursor special values: "inherit" = use parent model, "fast" = lightweight model
        ("cursor", "claude") => match model {
            "inherit" => None, // No model preference — let target decide
            "fast" => Some("haiku".to_string()),
            _ => {
                // Cursor may use Claude model IDs directly
                if ClaudeModel::parse(model).is_some() {
                    Some(model.to_string())
                } else {
                    None
                }
            }
        },
        // Cursor ↔ Codex: route through Claude as intermediate
        ("cursor", "codex") | ("cursor", "copilot") => {
            // First try to parse as Claude model (Cursor often uses Claude IDs)
            if let Some(claude_model) = ClaudeModel::parse(model) {
                let openai_model = CLAUDE_TO_OPENAI.get(&claude_model)?;
                Some(openai_model.as_str().to_string())
            } else {
                match model {
                    "fast" => Some("gpt-4o-mini".to_string()),
                    _ => None,
                }
            }
        }
        ("codex", "cursor") | ("copilot", "cursor") => {
            // OpenAI model → pass through (Cursor accepts various model IDs)
            Some(model.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_to_openai_mappings() {
        assert_eq!(
            transform_model("opus", "claude", "codex"),
            Some("gpt-4o".to_string())
        );
        assert_eq!(
            transform_model("sonnet", "claude", "codex"),
            Some("gpt-4o-mini".to_string())
        );
        assert_eq!(
            transform_model("haiku", "claude", "codex"),
            Some("gpt-4o-mini".to_string())
        );
    }

    #[test]
    fn openai_to_claude_mappings() {
        assert_eq!(
            transform_model("gpt-4o", "codex", "claude"),
            Some("opus".to_string())
        );
        assert_eq!(
            transform_model("gpt-4o-mini", "codex", "claude"),
            Some("sonnet".to_string())
        );
        assert_eq!(
            transform_model("o1", "codex", "claude"),
            Some("opus".to_string())
        );
        assert_eq!(
            transform_model("o1-mini", "codex", "claude"),
            Some("haiku".to_string())
        );
    }

    #[test]
    fn full_model_ids_are_recognized() {
        // Claude full IDs
        assert_eq!(
            transform_model("claude-3-5-sonnet-20240620", "claude", "codex"),
            Some("gpt-4o-mini".to_string())
        );
        assert_eq!(
            transform_model("claude-3-opus-20240229", "claude", "codex"),
            Some("gpt-4o".to_string())
        );
    }

    #[test]
    fn same_platform_passthrough() {
        assert_eq!(
            transform_model("sonnet", "claude", "claude"),
            Some("sonnet".to_string())
        );
        assert_eq!(
            transform_model("gpt-4o", "codex", "codex"),
            Some("gpt-4o".to_string())
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(transform_model("unknown-model", "claude", "codex"), None);
        assert_eq!(transform_model("davinci", "codex", "claude"), None);
    }

    #[test]
    fn claude_to_cursor_passthrough() {
        // Cursor accepts Claude model IDs directly
        assert_eq!(
            transform_model("sonnet", "claude", "cursor"),
            Some("sonnet".to_string())
        );
        assert_eq!(
            transform_model("claude-opus-4-6", "claude", "cursor"),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn cursor_to_claude_special_values() {
        assert_eq!(transform_model("inherit", "cursor", "claude"), None);
        assert_eq!(
            transform_model("fast", "cursor", "claude"),
            Some("haiku".to_string())
        );
        // Cursor using Claude model IDs passes through
        assert_eq!(
            transform_model("sonnet", "cursor", "claude"),
            Some("sonnet".to_string())
        );
        // Unknown Cursor model returns None
        assert_eq!(transform_model("unknown", "cursor", "claude"), None);
    }

    #[test]
    fn cursor_to_codex_routes_through_claude() {
        // Cursor with Claude model → OpenAI equivalent
        assert_eq!(
            transform_model("opus", "cursor", "codex"),
            Some("gpt-4o".to_string())
        );
        assert_eq!(
            transform_model("fast", "cursor", "codex"),
            Some("gpt-4o-mini".to_string())
        );
        assert_eq!(transform_model("inherit", "cursor", "codex"), None);
    }

    #[test]
    fn codex_to_cursor_passthrough() {
        assert_eq!(
            transform_model("gpt-4o", "codex", "cursor"),
            Some("gpt-4o".to_string())
        );
    }

    #[test]
    fn cursor_same_platform_passthrough() {
        assert_eq!(
            transform_model("sonnet", "cursor", "cursor"),
            Some("sonnet".to_string())
        );
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn claude_parse_never_panics(s in "\\PC*") {
                // Should return Some or None, never panic
                let _ = ClaudeModel::parse(&s);
            }

            #[test]
            fn openai_parse_never_panics(s in "\\PC*") {
                let _ = OpenAiModel::parse(&s);
            }

            #[test]
            fn transform_model_never_panics(
                model in "\\PC{0,100}",
                source in prop::sample::select(vec!["claude", "codex", "copilot", "cursor", "unknown"]),
                target in prop::sample::select(vec!["claude", "codex", "copilot", "cursor", "unknown"]),
            ) {
                // Should return Some or None, never panic
                let _ = transform_model(&model, source, target);
            }
        }
    }
}
