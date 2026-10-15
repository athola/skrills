//! Token estimation for skill files.
//!
//! Uses character-based heuristics for GPT-style tokenizers.
//! Different ratios are used for prose vs code content since
//! code tends to have shorter tokens due to symbols and keywords.

use serde::{Deserialize, Serialize};

use crate::fence::FenceTracker;

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

    let mut fence = FenceTracker::default();
    let mut frontmatter_chars = 0usize;
    let mut code_chars = 0usize;
    let mut prose_chars = 0usize;

    let frontmatter = frontmatter_lines(content);

    // `split_inclusive` keeps each line's own newline, so a last line
    // without one is not charged for a newline that is not there.
    for (index, segment) in content.split_inclusive('\n').enumerate() {
        // The ratios are characters per token, so count characters,
        // not UTF-8 bytes.
        let line_len = segment.chars().count();
        let line = segment.trim_end_matches(['\n', '\r']);

        if frontmatter.as_ref().is_some_and(|r| r.contains(&index)) {
            frontmatter_chars += line_len;
        } else if fence.is_code(line) {
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
    // Rounded once: each section rounded up and then summed overstated
    // the total by up to 2.
    breakdown.total = (frontmatter_chars as f64 / FRONTMATTER_CHARS_PER_TOKEN
        + code_chars as f64 / CODE_CHARS_PER_TOKEN
        + prose_chars as f64 / PROSE_CHARS_PER_TOKEN)
        .ceil() as usize;

    breakdown
}

/// Line indices of the frontmatter block, delimiters included.
///
/// Frontmatter opens only on the first non-empty line and counts only
/// once a closing `---` is found; an unclosed opener is body text, not a
/// file-long frontmatter block. After the opener's line, a `---` is a
/// horizontal rule unless it closes the block.
pub(crate) fn frontmatter_lines(content: &str) -> Option<std::ops::RangeInclusive<usize>> {
    let mut lines = content.lines().enumerate();
    let (open, _) = lines.find(|(_, l)| !l.trim().is_empty())?;
    if content.lines().nth(open)?.trim() != "---" {
        return None;
    }
    let (close, _) = lines.find(|(_, l)| l.trim() == "---")?;
    Some(open..=close)
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

/// Source category for per-source token attribution.
///
/// Splits system-prompt and conversation tokens into categories so the
/// cold-window's token ledger can attribute each contribution to the
/// right culprit. This is the difference between "your context is
/// 100K tokens" (uninformative) and "your three MCP servers are
/// dragging in 70K of tool descriptions" (actionable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenSource {
    /// Skill body (frontmatter, prose, and code blocks).
    #[default]
    Skill,
    /// Plugin (sum of skills + commands + agents).
    Plugin,
    /// MCP server (tool descriptions especially, see Willison's
    /// "Too many MCP servers" note: a single MCP can swallow 55K
    /// tokens of tool description overhead).
    Mcp,
    /// Active conversation (cache reads and writes).
    Conversation,
}

/// A token breakdown attributed to a specific source.
///
/// Returned by `count_tokens_attributed`; also stored in the
/// cold-window's `TokenLedger`. The `breakdown` is the same
/// section split returned by `count_tokens()`; the `source` and
/// `identifier` make the attribution explicit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttributedBreakdown {
    /// Which source this attribution belongs to.
    pub source: TokenSource,
    /// Stable identifier (skill URI, plugin name, MCP server name,
    /// conversation ID).
    pub identifier: String,
    /// Section-level breakdown of the underlying content.
    pub breakdown: TokenBreakdown,
}

/// Count tokens for `content` and attribute the result to a named source.
///
/// Existing `count_tokens()` API is unchanged; this is purely additive.
/// The same character-heuristic limits apply (see crate-level docs in
/// `tokens.rs`); accuracy on a known fixture is asserted at ≥95%
/// in the cold-window test plan.
///
/// # Example
///
/// ```rust
/// use skrills_analyze::tokens::{count_tokens_attributed, TokenSource};
///
/// let attributed = count_tokens_attributed(
///     "# Skill\nbody",
///     TokenSource::Skill,
///     "skill://demo",
/// );
/// assert_eq!(attributed.source, TokenSource::Skill);
/// assert_eq!(attributed.identifier, "skill://demo");
/// assert!(attributed.breakdown.total > 0);
/// ```
pub fn count_tokens_attributed(
    content: &str,
    source: TokenSource,
    identifier: impl Into<String>,
) -> AttributedBreakdown {
    AttributedBreakdown {
        source,
        identifier: identifier.into(),
        breakdown: count_tokens(content),
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
    fn horizontal_rule_after_content_is_not_a_frontmatter_opener() {
        // IN-17: a `---` rule in the body of a skill with no
        // frontmatter must stay prose, and code after it stays code.
        let content = "# Title\n\nIntro.\n\n---\n\n```rust\nfn main() {}\n```\n";
        let breakdown = count_tokens(content);
        assert_eq!(breakdown.frontmatter, 0);
        assert!(breakdown.code > 0);
    }

    /// M309: the same 70 characters cost more tokens as code than as
    /// prose, and more as frontmatter than as code, through count_tokens.
    #[test]
    fn same_length_content_costs_most_as_frontmatter_and_least_as_prose() {
        let body = "x".repeat(61);
        let prose = count_tokens(&format!("{}\n", "x".repeat(69)));
        let code = count_tokens(&format!("```\n{body}\n```\n"));
        let frontmatter = count_tokens(&format!("---\n{body}\n---\n"));
        assert_eq!((prose.prose, prose.total), (18, 18));
        assert_eq!((code.code, code.total), (20, 20));
        assert_eq!((frontmatter.frontmatter, frontmatter.total), (22, 22));
    }

    /// M304: no newline is counted past the end of the file.
    #[test]
    fn final_line_without_newline_counts_no_newline() {
        assert_eq!(count_tokens("abcd").total, 1);
        assert_eq!(count_tokens("abcd\n").total, 2);
    }

    /// M304: the total is rounded once, not once per section.
    #[test]
    fn total_is_rounded_once_over_all_sections() {
        // 10/3.2 + 2/4 + 10/3.5 = 6.48 tokens, so 7, not 4 + 1 + 3.
        let t = count_tokens("---\nx\n---\np\n```\nq\n```\n");
        assert_eq!((t.frontmatter, t.prose, t.code), (4, 1, 3));
        assert_eq!(t.total, 7);
    }

    /// M303: a `~~~` fence is code like a backtick fence.
    #[test]
    fn tilde_fence_counts_as_code() {
        let t = count_tokens("~~~\nfn a() {}\n~~~\n");
        assert_eq!(t.prose, 0, "{t:?}");
        assert!(t.code > 0);
    }

    /// M304: category boundaries.
    #[test]
    fn token_category_boundaries() {
        use TokenCategory::*;
        for (n, cat) in [
            (500, Small),
            (501, Medium),
            (2000, Medium),
            (2001, Large),
            (8000, Large),
            (8001, VeryLarge),
        ] {
            assert_eq!(TokenCategory::from_count(n), cat, "{n}");
        }
    }

    #[test]
    fn unterminated_frontmatter_counts_as_body() {
        let content = "---\n# Title\n\nSome prose here.\n\n```rust\nfn main() {}\n```\n";
        let tokens = count_tokens(content);
        assert_eq!(tokens.frontmatter, 0, "{tokens:?}");
        assert!(tokens.prose > 0, "{tokens:?}");
        assert!(tokens.code > 0, "{tokens:?}");
    }

    #[test]
    fn frontmatter_opener_may_follow_leading_blank_lines() {
        let breakdown = count_tokens("\n---\nname: x\n---\nBody.");
        assert!(breakdown.frontmatter > 0);
    }

    #[test]
    fn non_ascii_text_is_counted_in_characters_not_bytes() {
        // IN-59: the ratios are chars-per-token. 40 CJK characters are
        // 120 UTF-8 bytes; 40 chars (no trailing newline) gives 40 / 4.0.
        let line = "漢".repeat(40);
        let breakdown = count_tokens(&line);
        assert_eq!(breakdown.prose, 10);
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

        // Each part is rounded up on its own; the total is rounded once,
        // so it is at most their sum.
        assert!(
            breakdown.total <= breakdown.frontmatter + breakdown.code + breakdown.prose,
            "{breakdown:?}"
        );
    }
}
