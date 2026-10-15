//! Hint pane.
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
//! - `P` toggle pin on the selected hint (by URI).
//!
//! Pinned hints sort to the top of the visible list regardless of
//! their score (matching the `MultiSignalScorer` ranking behavior).

use std::collections::HashSet;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{List, ListItem};
use serde::{Deserialize, Serialize};
use skrills_snapshot::{HintCategory, ScoredHint};

use super::focus::{
    clamped_selection, pane_block, render_list, select_row, truncate_with_ellipsis,
};
use super::state::ColdWindowState;

/// File name used by [`HintPaneState::with_default_persistence`].
pub const PIN_FILE_NAME: &str = "cold-window-pins.json";

/// Mutable state owned by the hint pane: filter, pin set, and path
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
            Some(p) => Self::load_from_path(p.clone()).unwrap_or_else(|err| {
                // Keep the path so pins made this session are still saved.
                tracing::warn!(error = %err, path = %p.display(), "cannot read hint pin file");
                Self {
                    persistence_path: Some(p),
                    ..Self::default()
                }
            }),
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
        let pinned = match serde_json::from_slice::<PinnedFile>(&bytes) {
            Ok(file) => file.pinned,
            Err(err) => {
                // The next toggle rewrites the file, so set the unreadable
                // one aside rather than lose the pins it may still hold.
                let mut backup = path.clone().into_os_string();
                backup.push(".corrupt");
                tracing::warn!(
                    error = %err,
                    path = %path.display(),
                    backup = ?backup,
                    "hint pin file is not valid JSON; moved aside, starting with no pins"
                );
                std::fs::rename(&path, &backup)?;
                HashSet::new()
            }
        };
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
        // Write a sibling and rename it over the file, so an interrupted
        // save cannot leave a truncated pin file behind.
        let mut tmp = path.clone().into_os_string();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
        if written.is_err() {
            // Best effort: the write error below is what the caller needs.
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// Toggle the pin state of a URI. Returns `true` if newly pinned,
    /// `false` if newly unpinned. Best-effort persistence; persistence
    /// failures (`fs::create_dir_all`, `serde_json`, `fs::write`) are
    /// surfaced via `tracing::warn!` so the operator can investigate
    /// why pins disappear next launch.
    pub fn toggle_pin(&mut self, uri: &str) -> bool {
        let pinned = if self.pinned.contains(uri) {
            self.pinned.remove(uri);
            false
        } else {
            self.pinned.insert(uri.to_string());
            true
        };
        if let Err(err) = self.save() {
            tracing::warn!(
                error = %err,
                uri,
                "hint-pane pin persistence failed; pin may not survive restart"
            );
        }
        pinned
    }

    /// Set the category filter (use `None` to clear).
    pub fn set_filter(&mut self, filter: Option<HintCategory>) {
        self.filter = filter;
    }

    /// Filter and sort the snapshot's hints. Pinned hints sort to the
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
            // `total_cmp` is a total order even with NaN scores; a
            // `partial_cmp` fallback to Equal lets `sort_by` panic.
            b_pinned
                .cmp(&a_pinned)
                .then_with(|| b.score.total_cmp(&a.score))
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

/// Stateless renderer and key handler for the hint pane.
pub struct HintPane;

impl HintPane {
    /// Render the visible hints into `area`. `focused` emphasizes the
    /// border (see [`pane_block`]); `selected` highlights one row with
    /// the `> ` cursor (pass `None` when the pane is unfocused).
    pub fn render(
        snap_state: &ColdWindowState,
        pane_state: &HintPaneState,
        frame: &mut Frame<'_>,
        area: Rect,
        focused: bool,
        selected: Option<usize>,
    ) {
        let title = match pane_state.filter {
            None => " Hints  (1=tok 2=val 3=red 4=sync 5=qual 0=all  P=pin) ".to_string(),
            Some(c) => format!(" Hints  filter:{}  (0=clear) ", c.label()),
        };
        let visible = pane_state.visible_hints(snap_state);
        let cursor = selected.and_then(|s| clamped_selection(s, visible.len()));
        let inner_width = (area.width as usize).saturating_sub(4);
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, h)| {
                select_row(
                    Self::render_row(h, pane_state, inner_width),
                    cursor == Some(i),
                )
            })
            .collect();

        let list = List::new(items)
            .block(pane_block(title, focused))
            .style(Style::default());
        render_list(frame, list, area, cursor);
    }

    /// Build one ratatui list row for a hint, truncating with `"..."`
    /// when the URI and message together exceed the available width.
    /// Fixed chars: pin marker (4) + score (5) + `"  "` (2) +
    /// `"[{category}]"` (2 + cat_len) + `"  "` (2) = 15 + cat_len.
    fn render_row<'a>(
        hint: &'a ScoredHint,
        pane_state: &HintPaneState,
        inner_width: usize,
    ) -> Line<'a> {
        let pinned = pane_state.pinned.contains(&hint.hint.uri);
        let pin_marker = if pinned { "[*] " } else { "[ ] " };
        let score_str = format!("{:>5.1}", hint.score);
        let category = hint.hint.category.label();
        // 4 + 5 + 2 + 2 + cat_len + 2 = 15 + cat_len
        let available = inner_width.saturating_sub(15 + category.chars().count());
        let uri_chars = hint.hint.uri.chars().count();
        let msg_chars = hint.hint.message.chars().count();

        let mut spans = vec![
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
        ];

        if uri_chars + 2 + msg_chars <= available {
            spans.push(Span::raw(hint.hint.uri.clone()));
            spans.push(Span::raw(": "));
            spans.push(Span::raw(hint.hint.message.clone()));
        } else if uri_chars + 2 < available {
            let msg_space = available - uri_chars - 2;
            spans.push(Span::raw(hint.hint.uri.clone()));
            spans.push(Span::raw(": "));
            spans.push(Span::raw(truncate_with_ellipsis(
                &hint.hint.message,
                msg_space,
            )));
        } else {
            spans.push(Span::raw(truncate_with_ellipsis(&hint.hint.uri, available)));
        }

        Line::from(spans)
    }

    /// Handle a keystroke; mutate state if relevant, return action.
    /// `selected` is the pane's cursor; it is clamped to the visible list.
    pub fn handle_key(
        snap_state: &ColdWindowState,
        pane_state: &mut HintPaneState,
        key: KeyCode,
        selected: usize,
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
                let visible = pane_state.visible_hints(snap_state);
                let target = clamped_selection(selected, visible.len()).map(|idx| visible[idx]);
                if let Some(target) = target {
                    let uri = target.hint.uri.clone();
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
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('1'), 0);
        assert_eq!(action, HintAction::FilterChanged(Some(HintCategory::Token)));
        assert_eq!(pane_state.filter, Some(HintCategory::Token));

        HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('0'), 0);
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
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('P'), 0);
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
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('P'), 0);
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
        let action = HintPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('q'), 0);
        assert_eq!(action, HintAction::NoOp);
    }

    #[test]
    fn category_labels_are_kebab_case_for_two_word_variants() {
        assert_eq!(category_label_pub(HintCategory::SyncDrift), "sync-drift");
        assert_eq!(category_label_pub(HintCategory::Token), "token");
    }

    #[test]
    fn long_hint_row_truncates_at_narrow_width() {
        // inner_width = 30 - 4 = 26
        // category "token" (5) → fixed = 15+5 = 20, available = 6
        // uri "skill://example-really-long" > 6 → must truncate with "..."
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![hint(
            "skill://example-really-long",
            HintCategory::Token,
            5.0,
        )]));
        let pane_state = HintPaneState::new();
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("..."),
            "long URI must truncate with '...', got: {text}"
        );
    }

    #[test]
    fn hint_row_truncates_message_when_uri_fits_but_message_is_long() {
        // inner_width = 60 - 4 = 56, category "token"(5), fixed=20, available=36
        // uri "sk://x" (6) + ": " (2) = 8 < 36 → message path
        // message 40 chars > 36 - 8 = 28 → truncated
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![ScoredHint {
            hint: Hint {
                uri: "sk://x".into(),
                category: HintCategory::Token,
                message: "this message is quite long and will overflow the remaining space here"
                    .into(),
                frequency: 1,
                impact: 1.0,
                ease_score: 1.0,
                age_days: 0.0,
            },
            score: 1.0,
            pinned: false,
        }]));
        let pane_state = HintPaneState::new();
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("sk://x"),
            "URI must survive when message is truncated, got: {text}"
        );
        assert!(
            text.contains("..."),
            "truncated message must end with '...', got: {text}"
        );
    }

    #[test]
    fn short_hint_row_needs_no_truncation() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![hint("sk://x", HintCategory::Token, 1.0)]));
        let pane_state = HintPaneState::new();
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !text.contains("..."),
            "short URI must not be truncated, got: {text}"
        );
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
                .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area(), false, None))
                .unwrap();
        }
    }

    #[test]
    fn corrupt_pin_file_is_backed_up_before_it_can_be_overwritten() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(PIN_FILE_NAME);
        std::fs::write(&path, b"{not json").unwrap();

        let mut state = HintPaneState::load_from_path(path.clone()).unwrap();
        assert!(state.pinned.is_empty());
        state.toggle_pin("skill://new");

        let backup = dir.path().join(format!("{PIN_FILE_NAME}.corrupt"));
        assert_eq!(std::fs::read(&backup).unwrap(), b"{not json");
    }

    #[cfg(unix)]
    #[test]
    fn pin_save_replaces_the_file_by_rename() {
        use std::os::unix::fs::MetadataExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join(PIN_FILE_NAME);
        let mut state = HintPaneState::load_from_path(path.clone()).unwrap();
        state.toggle_pin("skill://a");
        let inode_before = std::fs::metadata(&path).unwrap().ino();
        state.toggle_pin("skill://b");
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), inode_before);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(PIN_FILE_NAME)]);
    }

    #[test]
    fn default_persistence_keeps_its_path_when_the_pin_file_is_unreadable() {
        let _env = skrills_test_utils::env_guard();
        let fixture = skrills_test_utils::TestFixture::new().unwrap();
        let _home = fixture.home_guard();
        let home = dirs::home_dir().unwrap();
        // A directory where the file should be: reading it fails.
        std::fs::create_dir_all(home.join(".skrills").join(PIN_FILE_NAME)).unwrap();

        let state = HintPaneState::with_default_persistence();
        assert!(state.persistence_path.is_some());
    }

    #[test]
    fn selected_hint_below_the_fold_is_scrolled_into_view() {
        let mut terminal = Terminal::new(TestBackend::new(80, 5)).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(
            (0..10)
                .map(|i| {
                    hint(
                        &format!("skill://h{i}"),
                        HintCategory::Token,
                        10.0 - f64::from(i),
                    )
                })
                .collect(),
        ));
        let pane_state = HintPaneState::new();
        terminal
            .draw(|f| HintPane::render(&snap_state, &pane_state, f, f.area(), true, Some(9)))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("skill://h9"), "{text}");
    }

    #[test]
    fn nan_scores_sort_without_panicking() {
        let mut snap_state = ColdWindowState::new();
        let mut hints: Vec<_> = (0..2000)
            .map(|i| {
                let score = f64::from((i * 7919) % 2003);
                hint(&format!("skill://h{i}"), HintCategory::Token, score)
            })
            .collect();
        for h in hints.iter_mut().step_by(3) {
            h.score = f64::NAN;
        }
        snap_state.ingest(snap(hints));
        let visible = HintPaneState::new().visible_hints(&snap_state);
        assert_eq!(visible.len(), 2000);
        // A total order puts every NaN together instead of scattering them.
        let finite: Vec<f64> = visible
            .iter()
            .map(|h| h.score)
            .filter(|s| !s.is_nan())
            .collect();
        assert!(
            finite.windows(2).all(|w| w[0] >= w[1]),
            "finite scores out of order"
        );
    }
}
