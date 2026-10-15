//! Model mapping between agent platforms.
//!
//! When syncing configurations between agents, a model name is carried over
//! only where the target is known to accept it; otherwise the target keeps
//! its own model.

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

// Public API

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
        // No current, tested table maps between Claude and OpenAI models:
        // Codex has moved past the `gpt-4o` family the old table produced, and
        // Copilot names Claude models with its own ids. `None` leaves the
        // target's model as it is.
        ("claude", "codex") | ("claude", "copilot") => None,
        ("codex", "claude") | ("copilot", "claude") => None,
        // Both take OpenAI model ids.
        ("codex", "copilot") | ("copilot", "codex") => Some(model.to_string()),
        // Cursor accepts Claude model IDs directly, pass through
        ("claude", "cursor") => Some(model.to_string()),
        // Cursor special values: "inherit" = use parent model, "fast" = lightweight model
        ("cursor", "claude") => match model {
            "inherit" => None, // No model preference, let target decide
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
        // Cursor uses Claude ids and its own `fast`/`inherit`; none of them
        // has a known Codex or Copilot id.
        ("cursor", "codex") | ("cursor", "copilot") => None,
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

    /// The old table wrote `gpt-4o`/`gpt-4o-mini` into Codex and Copilot:
    /// neither is a current Codex model (learn.chatgpt.com/docs/models), and
    /// Copilot runs Claude models under its own ids (`claude-haiku-4.5`), so
    /// a Claude shorthand or dashed id is not one either. With no tested
    /// cross-vendor table, the target keeps its own model.
    #[test]
    fn claude_models_have_no_codex_or_copilot_equivalent() {
        for model in ["opus", "sonnet", "haiku", "claude-sonnet-4-6"] {
            assert_eq!(transform_model(model, "claude", "codex"), None, "{model}");
            assert_eq!(transform_model(model, "claude", "copilot"), None, "{model}");
        }
    }

    #[test]
    fn openai_models_have_no_claude_equivalent() {
        for model in ["gpt-4o", "gpt-4o-mini", "o1", "o1-mini", "gpt-6.1-sol"] {
            assert_eq!(transform_model(model, "codex", "claude"), None, "{model}");
            assert_eq!(transform_model(model, "copilot", "claude"), None, "{model}");
        }
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
    fn cursor_models_have_no_codex_or_copilot_equivalent() {
        for model in ["opus", "fast", "inherit"] {
            assert_eq!(transform_model(model, "cursor", "codex"), None, "{model}");
            assert_eq!(transform_model(model, "cursor", "copilot"), None, "{model}");
        }
    }

    /// Codex and Copilot both take OpenAI ids; only the orchestrator knew
    /// that, so other callers got `None`.
    #[test]
    fn codex_and_copilot_pass_models_through_to_each_other() {
        assert_eq!(
            transform_model("gpt-5.3-codex", "codex", "copilot"),
            Some("gpt-5.3-codex".to_string())
        );
        assert_eq!(
            transform_model("gpt-5.3-codex", "copilot", "codex"),
            Some("gpt-5.3-codex".to_string())
        );
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
