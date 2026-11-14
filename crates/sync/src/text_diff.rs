//! Line-based unified diff shared by `skill-diff` and the interactive sync
//! prompts, so both show changes the same way without a diff dependency.

/// One line of an edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op<'a> {
    Keep(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

/// Above this many table cells (lines of `a` times lines of `b`), the diff
/// is "delete all, insert all" rather than an LCS table that size.
const MAX_LCS_CELLS: usize = 16_000_000;

/// The shortest edit script from `a` to `b`, from a longest common
/// subsequence of their lines.
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    if a.len().saturating_mul(b.len()) > MAX_LCS_CELLS {
        let mut ops: Vec<Op> = a.iter().map(|l| Op::Delete(l)).collect();
        ops.extend(b.iter().map(|l| Op::Insert(l)));
        return ops;
    }
    // lcs[i][j]: length of the LCS of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::with_capacity(a.len() + b.len());
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            ops.push(Op::Keep(a[i]));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
            ops.push(Op::Delete(a[i]));
            i += 1;
        } else {
            ops.push(Op::Insert(b[j]));
            j += 1;
        }
    }
    ops.extend(a[i..].iter().map(|l| Op::Delete(l)));
    ops.extend(b[j..].iter().map(|l| Op::Insert(l)));
    ops
}

/// A unified diff of `a` and `b` with `context` lines around each change and
/// an `@@ -start,len +start,len @@` header per hunk. Empty when they match.
pub fn unified_diff(a: &str, b: &str, context: usize) -> String {
    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();
    let ops = diff_ops(&lines_a, &lines_b);
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Keep(_)))
        .map(|(i, _)| i)
        .collect();

    let mut output = String::new();
    let last = ops.len().saturating_sub(1);
    let mut k = 0;
    while k < changes.len() {
        let start = changes[k].saturating_sub(context);
        let mut end = changes[k].saturating_add(context).min(last);
        k += 1;
        // Merge the next change into this hunk when their context touches.
        while k < changes.len() && changes[k].saturating_sub(context) <= end.saturating_add(1) {
            end = changes[k].saturating_add(context).min(last);
            k += 1;
        }

        let before = &ops[..start];
        let hunk = &ops[start..=end];
        let a_before = before
            .iter()
            .filter(|o| !matches!(o, Op::Insert(_)))
            .count();
        let b_before = before
            .iter()
            .filter(|o| !matches!(o, Op::Delete(_)))
            .count();
        let a_len = hunk.iter().filter(|o| !matches!(o, Op::Insert(_))).count();
        let b_len = hunk.iter().filter(|o| !matches!(o, Op::Delete(_))).count();
        // An empty side names the line before the hunk, as `diff -u` does.
        let a_start = if a_len == 0 { a_before } else { a_before + 1 };
        let b_start = if b_len == 0 { b_before } else { b_before + 1 };
        output.push_str(&format!("@@ -{a_start},{a_len} +{b_start},{b_len} @@\n"));
        for op in hunk {
            let (mark, line) = match op {
                Op::Keep(l) => (' ', l),
                Op::Delete(l) => ('-', l),
                Op::Insert(l) => ('+', l),
            };
            output.push(mark);
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One inserted line is one `+` line: the old diff compared line N to
    /// line N and marked every later line as changed.
    #[test]
    fn an_inserted_line_marks_only_that_line_changed() {
        let a = "1\n2\n3\n4\n5\n6\n";
        let b = "1\nnew\n2\n3\n4\n5\n6\n";

        assert_eq!(unified_diff(a, b, 1), "@@ -1,2 +1,3 @@\n 1\n+new\n 2\n");
    }

    #[test]
    fn distant_changes_get_separate_hunks() {
        let a = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let b = "A\nb\nc\nd\ne\nf\ng\nH\n";

        assert_eq!(
            unified_diff(a, b, 1),
            "@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -7,2 +7,2 @@\n g\n-h\n+H\n"
        );
    }

    #[test]
    fn identical_text_has_an_empty_diff() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", 3), "");
    }

    /// `skill-diff -C 18446744073709551615` overflowed `i + context`.
    #[test]
    fn a_huge_context_does_not_overflow() {
        let diff = unified_diff("a\nb\n", "a\nc\n", usize::MAX);

        assert_eq!(diff, "@@ -1,2 +1,2 @@\n a\n-b\n+c\n");
    }

    #[test]
    fn diff_ops_falls_back_to_delete_all_insert_all_above_the_cell_limit() {
        let a = vec!["x"; 4000];
        let b = vec!["x"; 4001];

        let ops = diff_ops(&a, &b);

        assert_eq!(ops.len(), 8001);
        assert!(ops[..4000].iter().all(|op| *op == Op::Delete("x")));
        assert!(ops[4000..].iter().all(|op| *op == Op::Insert("x")));
    }

    #[test]
    fn diff_ops_uses_the_lcs_at_the_cell_limit() {
        let lines = vec!["x"; 4000];

        let ops = diff_ops(&lines, &lines);

        assert_eq!(ops.len(), 4000);
        assert!(ops.iter().all(|op| *op == Op::Keep("x")));
    }
}
