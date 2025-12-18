//! Token estimation for skill files.
//!
//! Uses a character-based heuristic (~4 characters per token)
//! which is a reasonable approximation for GPT-style tokenizers.

use serde::{Deserialize, Serialize};

/// Approximate characters per token.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Token count breakdown by section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenBreakdown {
    /// Tokens in frontmatter.
    pub frontmatter: usize,
    /// Tokens in markdown prose.
    pub prose: usize,
    /// Tokens in code blocks.
    pub code: usize,
    /// Total estimated tokens.
    pub total: usize,
}

/// Estimate token count from character count.
pub fn estimate_tokens(chars: usize) -> usize {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as usize
}

/// Count tokens in a skill file with section breakdown.
pub fn count_tokens(content: &str) -> TokenBreakdown {
    let mut breakdown = TokenBreakdown::default();

    let mut in_frontmatter = false;
    let mut in_code_block = false;
    let mut frontmatter_chars = 0usize;
    let mut code_chars = 0usize;
    let mut prose_chars = 0usize;

    let mut frontmatter_started = false;

    for line in content.lines() {
        let line_len = line.len() + 1; // +1 for newline

        // Handle frontmatter delimiters
        if line.trim() == "---" {
            if !frontmatter_started {
                frontmatter_started = true;
                in_frontmatter = true;
                frontmatter_chars += line_len;
                continue;
            } else if in_frontmatter {
                in_frontmatter = false;
                frontmatter_chars += line_len;
                continue;
            }
        }

        if in_frontmatter {
            frontmatter_chars += line_len;
            continue;
        }

        // Handle code blocks
        if line.trim().starts_with("```") {
            in_code_block = !in_code_block;
            code_chars += line_len;
            continue;
        }

        if in_code_block {
            code_chars += line_len;
        } else {
            prose_chars += line_len;
        }
    }

    breakdown.frontmatter = estimate_tokens(frontmatter_chars);
    breakdown.code = estimate_tokens(code_chars);
    breakdown.prose = estimate_tokens(prose_chars);
    breakdown.total = breakdown.frontmatter + breakdown.code + breakdown.prose;

    breakdown
}

/// Token budget categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenCategory {
    /// Small skill, minimal context impact.
    Small,
    /// Medium skill, moderate context usage.
    Medium,
    /// Large skill, significant context usage.
    Large,
    /// Very large, may cause context pressure.
    VeryLarge,
}

impl TokenCategory {
    /// Categorize based on token count.
    pub fn from_count(tokens: usize) -> Self {
        match tokens {
            0..=500 => TokenCategory::Small,
            501..=2000 => TokenCategory::Medium,
            2001..=8000 => TokenCategory::Large,
            _ => TokenCategory::VeryLarge,
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            TokenCategory::Small => "small",
            TokenCategory::Medium => "medium",
            TokenCategory::Large => "large",
            TokenCategory::VeryLarge => "very-large",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(4), 1);
        assert_eq!(estimate_tokens(8), 2);
        assert_eq!(estimate_tokens(100), 25);
    }

    #[test]
    fn test_count_tokens_simple() {
        let content = "# Hello\nThis is content.";
        let breakdown = count_tokens(content);

        assert!(breakdown.total > 0);
        assert_eq!(breakdown.frontmatter, 0);
        assert_eq!(breakdown.code, 0);
    }

    #[test]
    fn test_count_tokens_with_frontmatter() {
        let content = "---\nname: test\ndescription: A test\n---\n# Content\nBody here.";
        let breakdown = count_tokens(content);

        assert!(breakdown.frontmatter > 0);
        assert!(breakdown.prose > 0);
    }

    #[test]
    fn test_count_tokens_with_code() {
        let content = "# Skill\n\n```rust\nfn main() {}\n```\n\nMore text.";
        let breakdown = count_tokens(content);

        assert!(breakdown.code > 0);
        assert!(breakdown.prose > 0);
    }

    #[test]
    fn test_token_category() {
        assert_eq!(TokenCategory::from_count(100), TokenCategory::Small);
        assert_eq!(TokenCategory::from_count(1000), TokenCategory::Medium);
        assert_eq!(TokenCategory::from_count(5000), TokenCategory::Large);
        assert_eq!(TokenCategory::from_count(10000), TokenCategory::VeryLarge);
    }
}
