//! Hint pane (TASK-016).
//!
//! Renders the snapshot's `ScoredHint` list with category filter
//! and per-hint pin toggle. Pin state persists to
//! `~/.skrills/cold-window-pins.json` so user pins survive across
//! daemon restarts.
//!
//! Keystrokes:
//!
//! - `1` filter to Token, `2` Validation, `3` Redundancy,
//!   `4` SyncDrift, `5` Quality, `0` clear filter.
//! - `P` toggle pin on the highest-priority visible hint (by URI).
//!
//! Pinned hints sort to the top of the visible list regardless of
//! their score (matching the `MultiSignalScorer` behavior in T010).

use std::collections::HashSet;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use serde::{Deserialize, Serialize};
use skrills_snapshot::{HintCategory, ScoredHint};

use super::state::ColdWindowState;

/// File name used by [`HintPaneState::with_default_persistence`].
pub const PIN_FILE_NAME: &str = "cold-window-pins.json";

/// Mutable state owned by the hint pane: filter + pin set + path
/// for persistence.
#[derive(Debug, Clone, Default)]
pub struct HintPaneState {
    /// Active category filter; `None` = show all.
    pub filter: Option<HintCategory>,
    /// URIs of pinned hints (sticky across snapshots and restarts).
    pub pinned: HashSet<String>,
    /// Persistence path for pins; `None` = in-memory only.
    pub persistence_path: Option<PathBuf>,
}

impl HintPaneState {
    /// Construct an in-memory state (no persistence).
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with the default persistence path
    /// (`~/.skrills/cold-window-pins.json`). Returns the state with
    /// any existing pin set already loaded.
    pub fn with_default_persistence() -> Self {
        let path = dirs::home_dir().map(|home| home.join(".skrills").join(PIN_FILE_NAME));
        match path {
            Some(p) => Self::load_from_path(p).unwrap_or_else(|_| Self::default()),
            None => Self::default(),
        }
    }

    /// Construct with a specific persistence path; loads the file if
    /// it exists.
    pub fn load_from_path(path: PathBuf) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self {
                filter: None,
                pinned: HashSet::new(),
                persistence_path: Some(path),
            });
        }
        let bytes = std::fs::read(&path)?;
        let pinned: HashSet<String> = serde_json::from_slice::<PinnedFile>(&bytes)
            .map(|f| f.pinned)
            .unwrap_or_default();
        Ok(Self {
            filter: None,
            pinned,
            persistence_path: Some(path),
        })
    }

    /// Save the pin set if a persistence path is configured.
    pub fn save(&self) -> std::io::Result<()> {
        let path = match &self.persistence_path {
            Some(p) => p,
            None => return Ok(()),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = PinnedFile {
            pinned: self.pinned.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Toggle the pin state of a URI. Returns `true` if newly pinned,
    /// `false` if newly unpinned. Best-effort persistence.
    pub fn toggle_pin(&mut self, uri: &str) -> bool {
        let pinned = if self.pinned.contains(uri) {
            self.pinned.remove(uri);
            false
        } else {
            self.pinned.insert(uri.to_string());
            true
        };
        let _ = self.save();
        pinned
    }

    /// Set the category filter (use `None` to clear).
    pub fn set_filter(&mut self, filter: Option<HintCategory>) {
        self.filter = filter;
    }

    /// Filter + sort the snapshot's hints. Pinned hints sort to the
    /// top regardless of score; within each group, descending score.
    pub fn visible_hints<'a>(&self, snap_state: &'a ColdWindowState) -> Vec<&'a ScoredHint> {
        let snap = match snap_state.current.as_deref() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let mut visible: Vec<&ScoredHint> = snap
            .hints
            .iter()
            .filter(|h| match self.filter {
                None => true,
                Some(c) => h.hint.category == c,
            })
            .collect();
        let pinned = &self.pinned;
        visible.sort_by(|a, b| {
            let a_pinned = pinned.contains(&a.hint.uri);
            let b_pinned = pinned.contains(&b.hint.uri);
            b_pinned.cmp(&a_pinned).then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        visible
    }
}

/// On-disk format for the pin file.
#[derive(Debug, Serialize, Deserialize)]
struct PinnedFile {
    pinned: HashSet<String>,
}

/// Action returned by the pane after handling a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintAction {
    /// No state change.
    NoOp,
    /// Filter changed (or cleared with `None`).
    FilterChanged(Option<HintCategory>),
    /// Pin toggled on a URI; `pinned = true` means newly pinned.
    PinToggled {
        /// URI whose pin status flipped.
        uri: String,
        /// New pin status (true = pinned).
        pinned: bool,
    },
}

/// Stateless renderer + key handler for the hint pane.
pub struct HintPane;

impl HintPane {
    /// Render the visible hints into `area`.
    pub fn render(
        snap_state: &ColdWindowState,
        pane_state: &HintPaneState,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let title = match pane_state.filter {
            None => " Hints  (1=tok 2=val 3=red 4=sync 5=qual 0=all  P=pin) ".to_string(),
            Some(c) => format!(" Hints  filter:{}  (0=clear) ", c.label()),
        };
        let visible = pane_state.visible_hints(snap_state);
        let items: Vec<ListItem> = visible
            .iter()
            .map(|h| Self::render_row(h, pane_state))
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default());
        frame.render_widget(list, area);
    }

    fn render_row<'a>(hint: &'a ScoredHint, pane_state: &HintPaneState) -> ListItem<'a> {
        let pinned = pane_state.pinned.contains(&hint.hint.uri);
        let pin_marker = if pinned { "[*] " } else { "[ ] " };
        let score_str = format!("{:>5.1}", hint.score);
        let category = hint.hint.category.label();
        let line = Line::from(vec![
            Span::styled(
                pin_marker.to_string(),
                Style::default()
                    .fg(if pinned {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                score_str,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format!("[{category}]"), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::raw(hint.hint.uri.clone()),
            Span::raw("  —  "),
            Span::raw(hint.hint.message.clone()),
        ]);
        ListItem::new(line)
    }

    /// Handle a keystroke; mutate state if relevant, return action.
    pub fn handle_key(
        snap_state: &ColdWindowState,
        pane_state: &mut HintPaneState,
        key: KeyCode,
    ) -> HintAction {
        match key {
            KeyCode::Char('0') => {
                pane_state.set_filter(None);
                HintAction::FilterChanged(None)
            }
            KeyCode::Char('1') => {
                pane_state.set_filter(Some(HintCategory::Token));
                HintAction::FilterChanged(Some(HintCategory::Token))
            }
            KeyCode::Char('2') => {
                pane_state.set_filter(Some(HintCategory::Validation));
                HintAction::FilterChanged(Some(HintCategory::Validation))
            }
            KeyCode::Char('3') => {
                pane_state.set_filter(Some(HintCategory::Redundancy));
                HintAction::FilterChanged(Some(HintCategory::Redundancy))
            }
            KeyCode::Char('4') => {
                pane_state.set_filter(Some(HintCategory::SyncDrift));
                HintAction::FilterChanged(Some(HintCategory::SyncDrift))
            }
            KeyCode::Char('5') => {
                pane_state.set_filter(Some(HintCategory::Quality));
                HintAction::FilterChanged(Some(HintCategory::Quality))
            }
            KeyCode::Char('P') => {
                if let Some(top) = pane_state.visible_hints(snap_state).first() {
                    let uri = top.hint.uri.clone();
                    let pinned = pane_state.toggle_pin(&uri);
                    HintAction::PinToggled { uri, pinned }
                } else {
                    HintAction::NoOp
                }
            }
            _ => HintAction::NoOp,
        }
    }
}

/// Public so the cli can derive labels for help text without going
/// through the trait. Thin shim over [`HintCategory::label`] (S1).
pub fn category_label_pub(c: HintCategory) -> &'static str {
    c.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ratatui::backend::TestBackend;
    use skrills_snapshot::{Hint, LoadSample, TokenLedger, WindowSnapshot};
    use tempfile::TempDir;

    fn hint(uri: &str, category: HintCategory, score: f64) -> ScoredHint {
        ScoredHint {
            hint: Hint {
                uri: uri.to_string(),
                category,
                message: format!("hint for {uri}"),
                frequency: 1,
                impact: 1.0,
                ease_score: 1.0,
                age_days: 0.0,
            },
            score,
            pinned: false,
        }
    }

    fn snap(hints: Vec<ScoredHint>) -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![],
            hints,
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        })
    }

    #[test]
    fn empty_state_yields_no_visible_hints() {
        let snap_state = ColdWindowState::new();
        let pane_state = HintPaneState::new();
        assert!(pane_state.visible_hints(&snap_state).is_empty());
    }

    #[test]
    fn filter_keys_set_filter() {
        let snap_state = ColdWindowState::new();
        let mut pane_state = HintPaneState::new();
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('1'));
        assert_eq!(action, HintAction::FilterChanged(Some(HintCategory::Token)));
        assert_eq!(pane_state.filter, Some(HintCategory::Token));

        HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('0'));
        assert_eq!(pane_state.filter, None);
    }

    #[test]
    fn filter_excludes_other_categories() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            hint("a", HintCategory::Token, 5.0),
            hint("b", HintCategory::Quality, 9.0),
            hint("c", HintCategory::Validation, 3.0),
        ]));
        let mut pane_state = HintPaneState::new();
        pane_state.set_filter(Some(HintCategory::Token));
        let visible = pane_state.visible_hints(&snap_state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].hint.uri, "a");
    }

    #[test]
    fn pinned_hint_floats_to_top_regardless_of_score() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            hint("low", HintCategory::Token, 0.1),
            hint("high", HintCategory::Token, 99.0),
        ]));
        let mut pane_state = HintPaneState::new();
        pane_state.toggle_pin("low");
        let visible = pane_state.visible_hints(&snap_state);
        assert_eq!(visible[0].hint.uri, "low", "pinned hint must be first");
        assert_eq!(visible[1].hint.uri, "high");
    }

    #[test]
    fn pin_keystroke_toggles_top_visible_hint() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            hint("a", HintCategory::Token, 1.0),
            hint("b", HintCategory::Token, 2.0),
        ]));
        let mut pane_state = HintPaneState::new();
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('P'));
        match action {
            HintAction::PinToggled { uri, pinned } => {
                assert_eq!(uri, "b");
                assert!(pinned);
            }
            other => panic!("expected PinToggled, got {other:?}"),
        }
    }

    #[test]
    fn pin_keystroke_with_no_hints_is_noop() {
        let snap_state = ColdWindowState::new();
        let mut pane_state = HintPaneState::new();
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('P'));
        assert_eq!(action, HintAction::NoOp);
    }

    #[test]
    fn pin_persistence_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pins.json");
        let mut state = HintPaneState::load_from_path(path.clone()).unwrap();
        state.toggle_pin("skill://demo");
        state.toggle_pin("plugin://alpha");

        let reloaded = HintPaneState::load_from_path(path).unwrap();
        assert!(reloaded.pinned.contains("skill://demo"));
        assert!(reloaded.pinned.contains("plugin://alpha"));
        assert_eq!(reloaded.pinned.len(), 2);
    }

    #[test]
    fn pin_persistence_handles_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = HintPaneState::load_from_path(path).unwrap();
        assert!(state.pinned.is_empty());
    }

    #[test]
    fn unrelated_keystroke_is_noop() {
        let snap_state = ColdWindowState::new();
        let mut pane_state = HintPaneState::new();
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('q'));
        assert_eq!(action, HintAction::NoOp);
    }

    #[test]
    fn category_labels_are_kebab_case_for_two_word_variants() {
        assert_eq!(category_label_pub(HintCategory::SyncDrift), "sync-drift");
        assert_eq!(category_label_pub(HintCategory::Token), "token");
    }

    #[test]
    fn render_does_not_panic_at_various_sizes() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            hint("a", HintCategory::Token, 5.0),
            hint("b", HintCategory::Quality, 9.0),
        ]));
        let pane_state = HintPaneState::new();
        for size in [(40, 10), (120, 30), (20, 5)] {
            let backend = TestBackend::new(size.0, size.1);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area()))
                .unwrap();
        }
    }
}
