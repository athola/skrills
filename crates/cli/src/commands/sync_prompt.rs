//! Terminal prompts for `sync-all --interactive`.

use anyhow::{bail, Result};
use inquire::InquireError;
use skrills_sync::{ChangeKind, Decision, DecisionScope, ItemChange, ItemDecider, SyncError};
use std::fmt;

/// Answers each change by showing its diff and asking Accept or Reject.
pub(crate) struct PromptDecider;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Accept,
    Reject,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Accept => "Accept: write it",
            Self::Reject => "Reject: keep the target as it is",
        })
    }
}

impl ItemDecider for PromptDecider {
    fn decide(&self, change: &ItemChange) -> skrills_sync::Result<Decision> {
        println!("\n{}", render(change));
        let answer =
            inquire::Select::new(&question(change), vec![Choice::Accept, Choice::Reject]).prompt();
        interpret(answer)
    }
}

/// The heading and diff shown before the question.
fn render(change: &ItemChange) -> String {
    let heading = match (change.scope, change.change) {
        (DecisionScope::WholeCategory, _) => format!(
            "{}: the target rewrites these on write, so they are one decision. Changing:",
            change.name
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
    match change.scope {
        DecisionScope::WholeCategory => format!("Write all {}?", change.name),
        DecisionScope::PerItem => format!("Write {} {}?", change.kind, change.name),
    }
}

/// Esc and Ctrl-C cancel the whole run, which has written nothing yet; they
/// do not mean "reject this one".
fn interpret(answer: std::result::Result<Choice, InquireError>) -> skrills_sync::Result<Decision> {
    match answer {
        Ok(Choice::Accept) => Ok(Decision::Accept),
        Ok(Choice::Reject) => Ok(Decision::Reject),
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
    use skrills_sync::ItemKind;

    fn change(scope: DecisionScope, kind: ChangeKind) -> ItemChange {
        let mut change = ItemChange::per_item(ItemKind::Command, "hello", kind, "-old\n+new\n");
        change.scope = scope;
        change
    }

    #[test]
    fn a_changed_item_is_shown_with_its_diff() {
        let shown = render(&change(DecisionScope::PerItem, ChangeKind::Changed));

        assert!(shown.starts_with("command hello: differs from the target"));
        assert!(shown.ends_with("-old\n+new"));
    }

    #[test]
    fn a_new_item_is_labelled_new() {
        let shown = render(&change(DecisionScope::PerItem, ChangeKind::New));

        assert!(shown.starts_with("command hello: new on the target"));
    }

    #[test]
    fn a_whole_category_says_why_it_is_one_decision() {
        let shown = render(&change(DecisionScope::WholeCategory, ChangeKind::Changed));

        assert!(shown.contains("one decision"), "{shown}");
        assert_eq!(
            question(&change(DecisionScope::WholeCategory, ChangeKind::Changed)),
            "Write all hello?"
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
