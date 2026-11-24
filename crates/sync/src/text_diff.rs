//! Line-based unified diff shared by `skill-diff` and the interactive sync
//! prompts, so both show changes the same way without a diff dependency.

use crate::decision::ConflictSides;

/// One line of an edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op<'a> {
    Keep(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

/// Above this many table cells (lines of `a` times lines of `b`, once their
/// common first and last lines are set aside), the changed middle is
/// "delete all, insert all" rather than an LCS table that size.
const MAX_LCS_CELLS: usize = 16_000_000;

/// An edit script from `a` to `b`. The lines both start and end with are
/// kept, and the middle comes from a longest common subsequence, so the
/// script is the shortest unless that middle is over [`MAX_LCS_CELLS`].
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mut ops: Vec<Op> = a[..prefix].iter().map(|l| Op::Keep(l)).collect();
    ops.extend(middle_ops(
        &a[prefix..a.len() - suffix],
        &b[prefix..b.len() - suffix],
    ));
    ops.extend(a[a.len() - suffix..].iter().map(|l| Op::Keep(l)));
    ops
}

/// The edit script between the differing middles of two texts.
fn middle_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
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

/// Unchanged rows kept on each side of a change in [`side_by_side`]; a longer
/// unchanged run folds into one `...` row.
const SIDE_CONTEXT: usize = 3;

/// Characters between the two columns of [`side_by_side`]: space, mark, space.
const GUTTER: usize = 3;

/// One row of a side-by-side view. `mark` is ` ` for an unchanged line, `|`
/// for a line changed in place, `<` for a line only the target has and `>`
/// for one only the source has.
struct Row<'a> {
    left: &'a str,
    right: &'a str,
    mark: char,
}

/// Two columns, the target's text on the left and the source's on the
/// right, in `width` characters. Each column gets half of what the gutter
/// leaves, and longer lines are cut with `…`. Below three characters of
/// width the gutter alone overflows; callers pass a usable width.
pub fn side_by_side(sides: &ConflictSides, width: usize) -> String {
    let column = width.saturating_sub(GUTTER) / 2;
    let target_lines: Vec<&str> = sides.target.lines().collect();
    let source_lines: Vec<&str> = sides.source.lines().collect();

    let mut rows: Vec<Row> = Vec::new();
    let (mut removed, mut added): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for op in diff_ops(&target_lines, &source_lines) {
        match op {
            Op::Delete(line) => removed.push(line),
            Op::Insert(line) => added.push(line),
            Op::Keep(line) => {
                pair_changes(&mut rows, &mut removed, &mut added);
                rows.push(Row {
                    left: line,
                    right: line,
                    mark: ' ',
                });
            }
        }
    }
    pair_changes(&mut rows, &mut removed, &mut added);

    let near_change: Vec<bool> = (0..rows.len())
        .map(|at| {
            let from = at.saturating_sub(SIDE_CONTEXT);
            let to = at.saturating_add(SIDE_CONTEXT).min(rows.len() - 1);
            rows[from..=to].iter().any(|row| row.mark != ' ')
        })
        .collect();

    let mut lines = vec![render_row("target", "source", ' ', column)];
    let mut folding = false;
    for (row, near) in rows.iter().zip(near_change) {
        if near {
            folding = false;
            lines.push(render_row(row.left, row.right, row.mark, column));
        } else if !folding {
            folding = true;
            lines.push(render_row("...", "...", ' ', column));
        }
    }
    let mut shown = lines.join("\n");
    shown.push('\n');
    shown
}

/// Turns a run of removed and added lines into rows, pairing them in order so
/// a line changed in place shows on one row.
fn pair_changes<'a>(rows: &mut Vec<Row<'a>>, removed: &mut Vec<&'a str>, added: &mut Vec<&'a str>) {
    for at in 0..removed.len().max(added.len()) {
        let (left, right) = (removed.get(at).copied(), added.get(at).copied());
        rows.push(Row {
            left: left.unwrap_or(""),
            right: right.unwrap_or(""),
            mark: match (left, right) {
                (Some(_), Some(_)) => '|',
                (Some(_), None) => '<',
                _ => '>',
            },
        });
    }
    removed.clear();
    added.clear();
}

fn render_row(left: &str, right: &str, mark: char, column: usize) -> String {
    let left = fit_column(left, column);
    let right = fit_column(right, column);
    format!("{left:<column$} {mark} {right}")
        .trim_end()
        .to_string()
}

/// `text` as at most `column` characters, tabs widened so columns stay
/// aligned, with `…` where it was cut.
fn fit_column(text: &str, column: usize) -> String {
    let text = text.replace('\t', "    ");
    if text.chars().count() <= column {
        return text;
    }
    let kept: String = text.chars().take(column.saturating_sub(1)).collect();
    if column == 0 {
        kept
    } else {
        format!("{kept}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "keep\nmodel = old\nend\n";
    const NEW: &str = "keep\nmodel = new\nend\n";

    fn side_by_side(target: &str, source: &str, width: usize) -> String {
        super::side_by_side(
            &ConflictSides {
                target: target.to_string(),
                source: source.to_string(),
            },
            width,
        )
    }

    #[test]
    fn side_by_side_labels_target_left_and_source_right() {
        let shown = side_by_side(OLD, NEW, 60);
        let header = shown.lines().next().unwrap();

        assert!(header.starts_with("target"), "{header}");
        assert!(header.ends_with("source"), "{header}");
    }

    #[test]
    fn side_by_side_pairs_a_changed_line_on_one_row() {
        let shown = side_by_side(OLD, NEW, 60);

        let row = shown.lines().find(|l| l.contains("model = old")).unwrap();
        assert!(row.contains("| model = new"), "{row}");
    }

    #[test]
    fn side_by_side_puts_a_target_only_line_left_and_a_source_only_line_right() {
        let shown = side_by_side("a\nonly-target\nb\n", "a\nb\nonly-source\n", 60);

        let left = shown.lines().find(|l| l.contains("only-target")).unwrap();
        let right = shown.lines().find(|l| l.contains("only-source")).unwrap();
        assert!(
            left.contains("<") && !left.contains("only-source"),
            "{left}"
        );
        assert!(
            right.contains(">") && !right.contains("only-target"),
            "{right}"
        );
    }

    #[test]
    fn side_by_side_fits_every_line_to_a_narrow_width() {
        let long = "x".repeat(200);
        let shown = side_by_side(&format!("{long}\n"), &format!("y{long}\n"), 30);

        assert!(shown.lines().all(|l| l.chars().count() <= 30), "{shown}");
        assert!(shown.contains('…'), "{shown}");
    }

    #[test]
    fn side_by_side_uses_the_extra_room_of_a_wide_width() {
        let line = "z".repeat(70);
        let shown = side_by_side(&format!("{line}\n"), &format!("{line}!\n"), 200);

        assert!(shown.contains(&format!("{line}!")), "{shown}");
        assert!(!shown.contains('…'), "{shown}");
    }

    #[test]
    fn side_by_side_survives_a_width_too_small_for_two_columns() {
        for width in 0..8 {
            let shown = side_by_side(OLD, NEW, width);
            assert!(
                shown.lines().all(|l| l.chars().count() <= 8),
                "{width}: {shown}"
            );
        }
    }

    #[test]
    fn side_by_side_folds_a_long_unchanged_run() {
        let body: String = (0..40).map(|n| format!("line {n}\n")).collect();
        let shown = side_by_side(&format!("{body}old\n"), &format!("{body}new\n"), 60);

        assert!(shown.contains("..."), "{shown}");
        assert!(!shown.contains("line 0"), "{shown}");
        assert!(shown.contains("line 39"), "{shown}");
    }

    #[test]
    fn side_by_side_expands_tabs_so_columns_stay_aligned() {
        let shown = side_by_side("a\tb\n", "a\tc\n", 40);

        assert!(!shown.contains('\t'), "{shown:?}");
    }

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
        // No common first or last line, so nothing is set aside.
        let a: Vec<String> = (0..4000).map(|i| format!("a{i}")).collect();
        let b: Vec<String> = (0..4001).map(|i| format!("b{i}")).collect();
        let (a, b): (Vec<&str>, Vec<&str>) = (
            a.iter().map(String::as_str).collect(),
            b.iter().map(String::as_str).collect(),
        );

        let ops = diff_ops(&a, &b);

        assert_eq!(ops.len(), 8001);
        assert!(ops[..4000].iter().all(|op| matches!(op, Op::Delete(_))));
        assert!(ops[4000..].iter().all(|op| matches!(op, Op::Insert(_))));
    }

    #[test]
    fn a_one_line_edit_to_a_file_past_the_cell_limit_shows_only_that_line() {
        let lines: Vec<String> = (0..5000).map(|i| format!("line {i}")).collect();
        let before = lines.join("\n");
        let after = before.replace("line 2500\n", "line 2500 edited\n");

        let diff = unified_diff(&before, &after, 1);

        assert_eq!(
            diff,
            "@@ -2500,3 +2500,3 @@\n line 2499\n-line 2500\n+line 2500 edited\n line 2501\n"
        );
    }

    #[test]
    fn diff_ops_uses_the_lcs_at_the_cell_limit() {
        let lines = vec!["x"; 4000];

        let ops = diff_ops(&lines, &lines);

        assert_eq!(ops.len(), 4000);
        assert!(ops.iter().all(|op| *op == Op::Keep("x")));
    }
}
