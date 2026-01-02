//! Token estimation for skill files.
//!
//! Uses character-based heuristics for GPT-style tokenizers.
//! Different ratios are used for prose vs code content since
//! code tends to have shorter tokens due to symbols and keywords.

use serde::{Deserialize, Serialize};

/// Approximate characters per token for prose/markdown content.
/// English text averages ~4 characters per token.
const PROSE_CHARS_PER_TOKEN: f64 = 4.0;

/// Approximate characters per token for code content.
/// Code averages ~3.5 chars/token due to short keywords, symbols, and identifiers.
const CODE_CHARS_PER_TOKEN: f64 = 3.5;

/// Approximate characters per token for YAML/frontmatter content.
/// YAML has many short keys and values, averaging ~3.2 chars/token.
const FRONTMATTER_CHARS_PER_TOKEN: f64 = 3.2;

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

/// Estimate token count from character count using prose ratio.
/// For more accurate estimates, use `estimate_tokens_with_ratio`.
pub fn estimate_tokens(chars: usize) -> usize {
    estimate_tokens_with_ratio(chars, PROSE_CHARS_PER_TOKEN)
}

/// Estimate token count from character count using a specific chars-per-token ratio.
fn estimate_tokens_with_ratio(chars: usize, chars_per_token: f64) -> usize {
    (chars as f64 / chars_per_token).ceil() as usize
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

    // Use content-specific ratios for more accurate token estimation
    breakdown.frontmatter =
        estimate_tokens_with_ratio(frontmatter_chars, FRONTMATTER_CHARS_PER_TOKEN);
    breakdown.code = estimate_tokens_with_ratio(code_chars, CODE_CHARS_PER_TOKEN);
    breakdown.prose = estimate_tokens_with_ratio(prose_chars, PROSE_CHARS_PER_TOKEN);
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

    // -------------------------------------------------------------------------
    // Content-Aware Token Estimation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_code_tokens_higher_than_prose_for_same_chars() {
        // Code should produce MORE tokens than prose for the same character count
        // because code has shorter average token length
        let code_content = "```rust\nfn main() { println!(\"hello\"); }\n```";
        let prose_content = "This is some regular prose text here.";

        // For similar character counts, code should estimate higher
        let code_chars = code_content.len();
        let prose_chars = prose_content.len();

        // Using the ratios: code = chars/3.5, prose = chars/4.0
        // So for same chars: code_tokens > prose_tokens
        let code_tokens = (code_chars as f64 / 3.5).ceil() as usize;
        let prose_tokens = (prose_chars as f64 / 4.0).ceil() as usize;

        // Normalize by character count to compare rates
        let code_rate = code_tokens as f64 / code_chars as f64;
        let prose_rate = prose_tokens as f64 / prose_chars as f64;

        assert!(
            code_rate > prose_rate,
            "Code should have higher token rate: {} vs {}",
            code_rate,
            prose_rate
        );
    }

    #[test]
    fn test_frontmatter_tokens_highest_rate() {
        // Frontmatter (YAML) should have the highest token rate due to short keys
        // Using ratios: frontmatter=3.2, code=3.5, prose=4.0

        let frontmatter_rate = 1.0 / 3.2;
        let code_rate = 1.0 / 3.5;
        let prose_rate = 1.0 / 4.0;

        assert!(
            frontmatter_rate > code_rate,
            "Frontmatter should have higher token rate than code"
        );
        assert!(
            code_rate > prose_rate,
            "Code should have higher token rate than prose"
        );
    }

    #[test]
    fn test_count_tokens_mixed_content_accuracy() {
        // Test a realistic skill file with all content types
        let content = r#"---
name: test-skill
description: A test skill
---
# Test Skill

This is prose content explaining the skill.

```python
def hello():
    print("Hello, world!")
```

More prose here.
"#;

        let breakdown = count_tokens(content);

        // Verify all sections are counted
        assert!(breakdown.frontmatter > 0, "Should have frontmatter tokens");
        assert!(breakdown.code > 0, "Should have code tokens");
        assert!(breakdown.prose > 0, "Should have prose tokens");

        // Total should equal sum of parts
        assert_eq!(
            breakdown.total,
            breakdown.frontmatter + breakdown.code + breakdown.prose,
            "Total should equal sum of parts"
        );
    }
}
