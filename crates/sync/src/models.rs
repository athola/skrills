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
/// * `source` - Source adapter name ("claude" or "codex")
/// * `target` - Target adapter name ("claude" or "codex")
pub fn transform_model(model: &str, source: &str, target: &str) -> Option<String> {
    // Same platform = no transformation needed
    if source == target {
        return Some(model.to_string());
    }

    match (source, target) {
        ("claude", "codex") => {
            let claude_model = ClaudeModel::parse(model)?;
            let openai_model = CLAUDE_TO_OPENAI.get(&claude_model)?;
            Some(openai_model.as_str().to_string())
        }
        ("codex", "claude") => {
            let openai_model = OpenAiModel::parse(model)?;
            let claude_model = OPENAI_TO_CLAUDE.get(&openai_model)?;
            Some(claude_model.as_str().to_string())
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
}
