//! Terminal prompts for `sync-all --interactive`.

use anyhow::{bail, Result};
use inquire::InquireError;
use skrills_sync::text_diff::side_by_side;
use skrills_sync::{ChangeKind, Decision, DecisionScope, ItemChange, ItemDecider, SyncError};
use std::fmt;

/// Width of the side-by-side view when the terminal's size is unknown, such
/// as output captured by a pty that reports none.
const FALLBACK_WIDTH: usize = 100;

/// Answers each change by showing its diff and asking Accept or Reject, or
/// for a conflict, which side to keep.
pub(crate) struct PromptDecider;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Accept,
    Reject,
    KeepSource,
    KeepTarget,
    Abort,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Accept => "Accept: write it",
            Self::Reject => "Reject: keep the target as it is",
            Self::KeepSource => "Keep source: overwrite the target's copy",
            Self::KeepTarget => "Keep target: leave the target's copy as it is",
            Self::Abort => "Abort: stop with nothing written",
        })
    }
}

impl ItemDecider for PromptDecider {
    fn decide(&self, change: &ItemChange) -> skrills_sync::Result<Decision> {
        let columns = crossterm::terminal::size().ok().map(|(columns, _)| columns);
        println!("\n{}", render(change, view_width(columns)));
        let options = if change.conflict.is_some() {
            vec![Choice::KeepSource, Choice::KeepTarget, Choice::Abort]
        } else {
            vec![Choice::Accept, Choice::Reject]
        };
        interpret(inquire::Select::new(&question(change), options).prompt())
    }
}

/// The side-by-side view's width: the terminal's, or [`FALLBACK_WIDTH`].
fn view_width(terminal_columns: Option<u16>) -> usize {
    terminal_columns.map_or(FALLBACK_WIDTH, usize::from)
}

/// The heading and diff shown before the question. A conflict shows both
/// copies side by side, since a unified diff cannot say which side is which
/// edit.
fn render(change: &ItemChange, width: usize) -> String {
    if let Some(sides) = &change.conflict {
        return format!(
            "{} {}: changed on both sides since the last sync\n{}",
            change.kind,
            change.name,
            side_by_side(&sides.target, &sides.source, width).trim_end()
        );
    }
    let heading = match (change.scope, change.change) {
        (DecisionScope::WholeCategory, _) => format!(
            "{}: the target rewrites these on write, so they are one decision.{} Changing:",
            change.name,
            match change.conflicting_items {
                0 => String::new(),
                count => format!(
                    " {count} of them changed on both sides since the last sync, so accepting overwrites the target's edits."
                ),
            }
        ),
        (DecisionScope::PerItem, ChangeKind::New) => {
            format!("{} {}: new on the target", change.kind, change.name)
        }
        (DecisionScope::PerItem, ChangeKind::Changed) => {
            format!("{} {}: differs from the target", change.kind, change.name)
        }
    };
    format!("{heading}\n{}", change.diff.trim_end())
}

fn question(change: &ItemChange) -> String {
    if change.conflict.is_some() {
        return format!("Resolve {} {}?", change.kind, change.name);
    }
    match change.scope {
        DecisionScope::WholeCategory => format!("Write all {}?", change.name),
        DecisionScope::PerItem => format!("Write {} {}?", change.kind, change.name),
    }
}

/// Esc and Ctrl-C cancel the whole run, which has written nothing yet; they
/// do not mean "reject this one".
fn interpret(answer: std::result::Result<Choice, InquireError>) -> skrills_sync::Result<Decision> {
    match answer {
        Ok(Choice::Accept | Choice::KeepSource) => Ok(Decision::Accept),
        Ok(Choice::Reject | Choice::KeepTarget) => Ok(Decision::Reject),
        Ok(Choice::Abort) => Err(SyncError::InteractiveAborted.into()),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Err(SyncError::InteractiveAborted.into())
        }
        Err(other) => Err(anyhow::Error::new(other).context("could not read an answer")),
    }
}

/// `sync-all --interactive` asks questions, so it needs a person at the
/// terminal. A dry run asks nothing and needs none.
pub(crate) fn require_terminal(will_prompt: bool, is_terminal: bool) -> Result<()> {
    if will_prompt && !is_terminal {
        bail!(
            "sync-all --interactive needs a terminal on stdin and stdout to ask about each \
             change; nothing was written. Run it from a shell, or drop --interactive \
             (--dry-run previews without asking)"
        );
    }
    Ok(())
}

/// Whether the error is the person cancelling the prompts.
pub(crate) fn is_cancelled(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<SyncError>(),
        Some(SyncError::InteractiveAborted)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_sync::{ConflictSides, ItemKind};

    fn change(scope: DecisionScope, kind: ChangeKind) -> ItemChange {
        let mut change = ItemChange::per_item(ItemKind::Command, "hello", kind, "-old\n+new\n");
        change.scope = scope;
        change
    }

    #[test]
    fn a_changed_item_is_shown_with_its_diff() {
        let shown = render(&change(DecisionScope::PerItem, ChangeKind::Changed), 80);

        assert!(shown.starts_with("command hello: differs from the target"));
        assert!(shown.ends_with("-old\n+new"));
    }

    #[test]
    fn a_new_item_is_labelled_new() {
        let shown = render(&change(DecisionScope::PerItem, ChangeKind::New), 80);

        assert!(shown.starts_with("command hello: new on the target"));
    }

    #[test]
    fn a_whole_category_says_why_it_is_one_decision() {
        let shown = render(
            &change(DecisionScope::WholeCategory, ChangeKind::Changed),
            80,
        );

        assert!(shown.contains("one decision"), "{shown}");
        assert_eq!(
            question(&change(DecisionScope::WholeCategory, ChangeKind::Changed)),
            "Write all hello?"
        );
    }

    fn conflict() -> ItemChange {
        let mut change = change(DecisionScope::PerItem, ChangeKind::Changed);
        change.conflict = Some(ConflictSides {
            target: "model = mine\n".to_string(),
            source: "model = theirs\n".to_string(),
        });
        change
    }

    #[test]
    fn a_conflict_is_shown_side_by_side_with_target_left_and_source_right() {
        let shown = render(&conflict(), 70);

        assert!(
            shown.starts_with("command hello: changed on both sides"),
            "{shown}"
        );
        let header = shown.lines().nth(1).unwrap();
        assert!(
            header.starts_with("target") && header.ends_with("source"),
            "{header}"
        );
        assert!(shown.contains("model = mine"), "{shown}");
        assert!(
            shown
                .lines()
                .any(|l| l.contains("model = mine") && l.contains("| model = theirs")),
            "{shown}"
        );
        assert!(
            !shown.contains("-old"),
            "no unified diff for a conflict: {shown}"
        );
    }

    #[test]
    fn a_conflict_asks_which_side_to_keep() {
        assert_eq!(question(&conflict()), "Resolve command hello?");
    }

    #[test]
    fn a_whole_category_with_conflicts_says_how_many() {
        let mut category = change(DecisionScope::WholeCategory, ChangeKind::Changed);
        category.conflicting_items = 2;

        let shown = render(&category, 80);

        assert!(shown.contains("2 of them changed on both sides"), "{shown}");
        assert!(!render(
            &change(DecisionScope::WholeCategory, ChangeKind::Changed),
            80
        )
        .contains("both sides"));
    }

    #[test]
    fn keep_source_accepts_keep_target_rejects_and_abort_cancels_the_run() {
        assert_eq!(interpret(Ok(Choice::KeepSource)).unwrap(), Decision::Accept);
        assert_eq!(interpret(Ok(Choice::KeepTarget)).unwrap(), Decision::Reject);
        let error = interpret(Ok(Choice::Abort)).unwrap_err();
        assert!(is_cancelled(&error), "{error:#}");
    }

    #[test]
    fn the_view_uses_the_terminal_width_or_a_fixed_fallback() {
        assert_eq!(view_width(Some(132)), 132);
        assert_eq!(view_width(Some(40)), 40);
        assert_eq!(view_width(None), FALLBACK_WIDTH);
    }

    #[test]
    fn a_narrow_terminal_keeps_every_conflict_line_inside_it() {
        let shown = render(&conflict(), 30);

        assert!(
            shown.lines().skip(1).all(|l| l.chars().count() <= 30),
            "{shown}"
        );
    }

    #[test]
    fn accept_and_reject_map_to_decisions() {
        assert_eq!(interpret(Ok(Choice::Accept)).unwrap(), Decision::Accept);
        assert_eq!(interpret(Ok(Choice::Reject)).unwrap(), Decision::Reject);
    }

    #[test]
    fn escape_and_ctrl_c_cancel_the_run_instead_of_rejecting() {
        for key in [
            InquireError::OperationCanceled,
            InquireError::OperationInterrupted,
        ] {
            let error = interpret(Err(key)).unwrap_err();
            assert!(is_cancelled(&error), "{error:#}");
        }
    }

    #[test]
    fn another_prompt_failure_is_an_error_not_a_cancellation() {
        let error = interpret(Err(InquireError::NotTTY)).unwrap_err();

        assert!(!is_cancelled(&error));
        assert!(format!("{error:#}").contains("could not read an answer"));
    }

    #[test]
    fn interactive_without_a_terminal_is_refused_with_a_reason() {
        let error = require_terminal(true, false).unwrap_err();

        assert!(format!("{error}").contains("needs a terminal"));
        assert!(format!("{error}").contains("nothing was written"));
    }

    #[test]
    fn a_terminal_or_a_run_without_prompts_needs_no_check() {
        assert!(require_terminal(true, true).is_ok());
        assert!(require_terminal(false, false).is_ok());
    }
}
