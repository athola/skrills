//! Fenced code block tracking shared by the token, dependency and
//! optimization scanners, so all three agree on which lines are code.
//!
//! Follows CommonMark: a fence is three or more backticks or tildes
//! indented at most three spaces; a backtick fence's info string may not
//! contain a backtick; the block closes on a line of the same character,
//! at least as long, with nothing after it but whitespace.

/// Fence state carried across the lines of one document.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FenceTracker {
    /// The open fence's character and length.
    open: Option<(char, usize)>,
}

impl FenceTracker {
    /// Whether a code block is open after the lines fed so far.
    pub(crate) fn in_block(&self) -> bool {
        self.open.is_some()
    }

    /// Feeds the next line and reports whether it belongs to a code block,
    /// counting the opening and closing fence lines themselves.
    pub(crate) fn is_code(&mut self, line: &str) -> bool {
        match self.open {
            Some((ch, len)) => {
                if let Some((c, n, rest)) = fence_run(line) {
                    if c == ch && n >= len && rest.trim().is_empty() {
                        self.open = None;
                    }
                }
                true
            }
            None => match fence_run(line) {
                Some(('`', _, rest)) if rest.contains('`') => false,
                Some((c, n, _)) => {
                    self.open = Some((c, n));
                    true
                }
                None => false,
            },
        }
    }
}

/// The fence character, run length and remainder of `line`, when it
/// starts (after at most three spaces) with three or more backticks or
/// tildes.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let ch = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = body.len() - body.trim_start_matches(ch).len();
    (run >= 3).then(|| (ch, run, &body[run..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_lines(text: &str) -> Vec<bool> {
        let mut fence = FenceTracker::default();
        text.lines().map(|l| fence.is_code(l)).collect()
    }

    #[test]
    fn tilde_fence_is_not_closed_by_backticks() {
        assert_eq!(
            code_lines("~~~\na\n```\nb\n~~~\nc"),
            [true, true, true, true, true, false]
        );
    }

    #[test]
    fn longer_fence_needs_an_equally_long_close() {
        assert_eq!(
            code_lines("````md\n```\nx\n````\ny"),
            [true, true, true, true, false]
        );
    }

    #[test]
    fn info_string_after_a_close_length_run_does_not_close() {
        assert_eq!(
            code_lines("```\n```rust\n```\nz"),
            [true, true, true, false]
        );
    }

    #[test]
    fn backtick_info_string_with_a_backtick_is_not_a_fence() {
        assert_eq!(code_lines("``` a`b\ntext"), [false, false]);
    }

    #[test]
    fn four_space_indent_is_not_a_fence() {
        assert_eq!(code_lines("    ```\ntext"), [false, false]);
    }
}
