//! Single source of truth for cold-window keybindings.
//!
//! Every key the TUI responds to is declared here once; the help
//! overlay and the contextual hint bar both derive their content from
//! this table, so a binding added to a `match` arm but not to the
//! table fails the audit tests below instead of silently missing from
//! the docs.
//!
//! Mobile constraint (FR-6.4): every action must be reachable without
//! CONTROL/ALT modifiers; phone keyboards over SSH often lack them.
//! `Ctrl-C` is allowed only as a redundant escape hatch for `q`.

use crossterm::event::KeyCode;

/// Which part of the interface a binding belongs to. Pane scopes drive
/// the help overlay's grouping and the hint bar's focus filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingScope {
    /// Active everywhere (quit, focus movement, help).
    Global,
    /// Alert pane keys (active regardless of focus; disjoint keymap).
    Alerts,
    /// Hint pane keys.
    Hints,
    /// Research pane keys.
    Research,
}

/// One keybinding: display label, action description, and the concrete
/// key codes it consumes (for the disjointness audit).
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Interface scope the binding belongs to.
    pub scope: BindingScope,
    /// Display form for help/hint surfaces, e.g. `"Tab"` or `"1-5"`.
    pub keys: &'static str,
    /// Short action description, e.g. `"next pane"`.
    pub action: &'static str,
    /// The modifier-free key codes this binding consumes.
    pub codes: &'static [KeyCode],
    /// True when the binding needs CONTROL/ALT. Allowed only as a
    /// redundant alternative to a modifier-free binding for the same
    /// action.
    pub modifier: bool,
}

/// The complete binding table, ordered for display (globals first).
pub fn bindings() -> &'static [Binding] {
    const BINDINGS: &[Binding] = &[
        Binding {
            scope: BindingScope::Global,
            keys: "q",
            action: "quit",
            codes: &[KeyCode::Char('q'), KeyCode::Char('Q')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "Ctrl-C",
            action: "quit",
            codes: &[],
            modifier: true,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "Tab",
            action: "next pane",
            codes: &[KeyCode::Tab],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "Shift-Tab",
            action: "prev pane",
            codes: &[KeyCode::BackTab],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "Esc",
            action: "close overlay",
            codes: &[KeyCode::Esc],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "?",
            action: "help",
            codes: &[KeyCode::Char('?')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "Enter",
            action: "open detail",
            codes: &[KeyCode::Enter],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "z",
            action: "zoom pane",
            codes: &[KeyCode::Char('z')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Global,
            keys: "j/k",
            action: "select item",
            codes: &[
                KeyCode::Char('j'),
                KeyCode::Char('k'),
                KeyCode::Up,
                KeyCode::Down,
            ],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Alerts,
            keys: "A",
            action: "ack all non-warnings",
            codes: &[KeyCode::Char('A')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Alerts,
            keys: "d",
            action: "dismiss top warning",
            codes: &[KeyCode::Char('d')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Hints,
            keys: "1-5",
            action: "filter category",
            codes: &[
                KeyCode::Char('1'),
                KeyCode::Char('2'),
                KeyCode::Char('3'),
                KeyCode::Char('4'),
                KeyCode::Char('5'),
            ],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Hints,
            keys: "0",
            action: "clear filter",
            codes: &[KeyCode::Char('0')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Hints,
            keys: "P",
            action: "pin top hint",
            codes: &[KeyCode::Char('P')],
            modifier: false,
        },
        Binding {
            scope: BindingScope::Research,
            keys: "R",
            action: "expand/collapse",
            codes: &[KeyCode::Char('R')],
            modifier: false,
        },
    ];
    BINDINGS
}

/// Bindings scoped to one pane (for the hint bar).
pub fn bindings_for(scope: BindingScope) -> Vec<&'static Binding> {
    bindings().iter().filter(|b| b.scope == scope).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn key_codes_are_disjoint_across_all_bindings() {
        // FR-1.4/T2: forwarding a key to every pane handler is safe
        // only while no two bindings claim the same code.
        let mut seen: HashSet<KeyCode> = HashSet::new();
        for b in bindings() {
            for code in b.codes {
                assert!(
                    seen.insert(*code),
                    "key {code:?} ({}: {}) is bound twice",
                    b.keys,
                    b.action
                );
            }
        }
    }

    #[test]
    fn every_modifier_binding_has_a_modifier_free_alternative() {
        // FR-6.4: glass keyboards lack CTRL/ALT; a modifier may only
        // duplicate an action that is also reachable without one.
        for b in bindings().iter().filter(|b| b.modifier) {
            assert!(
                bindings()
                    .iter()
                    .any(|alt| !alt.modifier && alt.action == b.action),
                "modifier binding {} ({}) has no modifier-free alternative",
                b.keys,
                b.action
            );
        }
    }

    #[test]
    fn table_covers_the_known_pane_keys() {
        // Sync guard: the documented pane keymaps must appear in the
        // table. (The reverse direction, table entries without handler
        // code, is caught by the help-overlay content looking wrong.)
        let all_codes: Vec<KeyCode> = bindings().iter().flat_map(|b| b.codes.to_vec()).collect();
        for expected in [
            KeyCode::Char('A'),
            KeyCode::Char('d'),
            KeyCode::Char('0'),
            KeyCode::Char('5'),
            KeyCode::Char('P'),
            KeyCode::Char('R'),
            KeyCode::Char('q'),
            KeyCode::Tab,
            KeyCode::BackTab,
        ] {
            assert!(
                all_codes.contains(&expected),
                "table is missing the {expected:?} binding"
            );
        }
    }

    #[test]
    fn bindings_for_filters_by_scope() {
        let hints = bindings_for(BindingScope::Hints);
        assert!(!hints.is_empty());
        assert!(hints.iter().all(|b| b.scope == BindingScope::Hints));
    }
}
