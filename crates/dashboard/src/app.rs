//! Application state and main dashboard runner.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use ratatui::widgets::ListState;
use skrills_metrics::{MetricEvent, MetricsCollector, ValidationDetail};

use crate::events::{Event, EventHandler};
use crate::ui;

/// Sort order for the skills panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// Display skills in discovery order (default).
    #[default]
    Discovery,
    /// Display skills sorted alphabetically by name.
    Alphabetical,
}

/// Panel focus for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusPanel {
    /// Skills list panel.
    #[default]
    Skills,
    /// Activity feed panel.
    Activity,
    /// Metrics panel.
    Metrics,
}

impl FocusPanel {
    /// Move to next panel.
    pub fn next(self) -> Self {
        match self {
            Self::Skills => Self::Activity,
            Self::Activity => Self::Metrics,
            Self::Metrics => Self::Skills,
        }
    }

    /// Move to previous panel.
    pub fn prev(self) -> Self {
        match self {
            Self::Skills => Self::Metrics,
            Self::Activity => Self::Skills,
            Self::Metrics => Self::Activity,
        }
    }
}

/// A single discovered location for a skill (source + path).
#[derive(Debug, Clone)]
pub struct SkillLocation {
    /// Discovery source label (e.g. "claude", "marketplace", "cache").
    pub source: String,
    /// File path to the skill.
    pub path: String,
}

/// Skill display info (deduplicated by name, with all locations).
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// Index in the original discovery order.
    pub discovery_index: usize,
    /// Skill name.
    pub name: String,
    /// Primary discovery source (first seen).
    pub source: String,
    /// Primary skill URI.
    pub uri: String,
    /// All locations where this skill was discovered.
    pub locations: Vec<SkillLocation>,
    /// Validation status (None = not validated, Some(true) = valid, Some(false) = invalid).
    pub valid: Option<bool>,
    /// Total invocations.
    pub invocations: u64,
}

/// A single activity feed entry with dedup support.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    /// The activity message.
    pub message: String,
    /// Shortened timestamp (HH:MM:SS).
    pub timestamp: String,
    /// Number of identical events (1 = first occurrence).
    pub count: u32,
    /// Optional dedup key. When present, used instead of `message` for matching.
    /// This allows events with varying message text (e.g. different skill counts)
    /// to still be consolidated into a single line.
    pub key: Option<String>,
}

impl ActivityEntry {
    fn new(message: String) -> Self {
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let timestamp = format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());
        Self {
            message,
            timestamp,
            count: 1,
            key: None,
        }
    }

    fn new_keyed(key: String, message: String) -> Self {
        let mut entry = Self::new(message);
        entry.key = Some(key);
        entry
    }

    /// Format the entry for display, truncating to fit `max_width`.
    pub fn format(&self, max_width: usize) -> String {
        let count_suffix = if self.count > 1 {
            format!(" (x{})", self.count)
        } else {
            String::new()
        };
        let prefix = format!("{} ", self.timestamp);
        let overhead = prefix.len() + count_suffix.len();

        if max_width <= overhead + 3 {
            // Not enough room even for "..."
            return format!("{}{}", &self.timestamp, count_suffix);
        }

        let msg_budget = max_width - overhead;
        let msg = if self.message.len() > msg_budget {
            format!("{}(...)", &self.message[..msg_budget.saturating_sub(5)])
        } else {
            self.message.clone()
        };

        format!("{}{}{}", prefix, msg, count_suffix)
    }
}

/// Number of skills to show per page for lazy loading.
pub const PAGE_SIZE: usize = 50;

/// Application state.
#[derive(Debug)]
pub struct App {
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Currently focused panel.
    pub focus: FocusPanel,
    /// Selected skill index.
    pub skill_index: usize,
    /// List state for scrollable skills panel.
    pub skill_list_state: ListState,
    /// List of discovered skills.
    pub skills: Vec<SkillInfo>,
    /// How many skills are currently visible (lazy-loaded in pages).
    pub visible_count: usize,
    /// Recent activity events with dedup and timestamps.
    pub activity: Vec<ActivityEntry>,
    /// Total skills count.
    pub total_skills: usize,
    /// Valid skills count.
    pub valid_skills: usize,
    /// Invalid skills count.
    pub invalid_skills: usize,
    /// Last refresh time.
    pub last_refresh: String,
    /// Show help overlay.
    pub show_help: bool,
    /// Current sort order for skills panel.
    pub sort_order: SortOrder,
    /// Total invocations across all skills (from analytics summary).
    pub total_invocations: u64,
    /// Overall success rate percentage (from analytics summary).
    pub overall_success_rate: f64,
    /// Latest validation detail for the currently selected skill.
    pub selected_validation: Option<ValidationDetail>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            focus: FocusPanel::default(),
            skill_index: 0,
            skill_list_state: ListState::default(),
            skills: Vec::new(),
            visible_count: PAGE_SIZE,
            activity: Vec::new(),

            total_skills: 0,
            valid_skills: 0,
            invalid_skills: 0,
            last_refresh: String::new(),
            show_help: false,
            sort_order: SortOrder::default(),
            total_invocations: 0,
            overall_success_rate: 0.0,
            selected_validation: None,
        }
    }
}

impl App {
    /// Create new app state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of skills currently visible (capped to total).
    pub fn visible_skill_count(&self) -> usize {
        self.visible_count.min(self.skills.len())
    }

    /// Handle keyboard input.
    pub fn on_key(&mut self, key: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;

        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            KeyCode::Char('s') => self.toggle_sort(),
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Home => {
                self.skill_index = 0;
                self.skill_list_state.select(Some(0));
            }
            KeyCode::End => {
                if !self.skills.is_empty() {
                    // End loads all skills and jumps to the last one
                    self.visible_count = self.skills.len();
                    self.skill_index = self.skills.len() - 1;
                    self.skill_list_state.select(Some(self.skill_index));
                }
            }
            _ => {}
        }
    }

    fn toggle_sort(&mut self) {
        self.sort_order = match self.sort_order {
            SortOrder::Discovery => SortOrder::Alphabetical,
            SortOrder::Alphabetical => SortOrder::Discovery,
        };
        match self.sort_order {
            SortOrder::Alphabetical => self.skills.sort_by(|a, b| a.name.cmp(&b.name)),
            SortOrder::Discovery => self.skills.sort_by_key(|s| s.discovery_index),
        }
        // Reset selection and visible window after re-sort
        self.skill_index = 0;
        self.visible_count = PAGE_SIZE;
        if !self.skills.is_empty() {
            self.skill_list_state.select(Some(0));
        }
    }

    fn select_next(&mut self) {
        let visible = self.visible_skill_count();
        if visible == 0 {
            return;
        }
        if self.skill_index + 1 >= visible {
            if self.visible_count < self.skills.len() {
                // At the bottom of the visible window with more to load
                self.visible_count = (self.visible_count + PAGE_SIZE).min(self.skills.len());
                self.skill_index += 1;
            } else {
                // All skills visible — wrap to top
                self.skill_index = 0;
            }
        } else {
            self.skill_index += 1;
        }
        self.skill_list_state.select(Some(self.skill_index));
    }

    fn select_prev(&mut self) {
        let visible = self.visible_skill_count();
        if visible > 0 {
            if self.skill_index == 0 {
                self.skill_index = visible - 1;
            } else {
                self.skill_index -= 1;
            }
            self.skill_list_state.select(Some(self.skill_index));
        }
    }

    /// Maximum activity entries to keep.
    const MAX_ACTIVITY_ENTRIES: usize = 100;

    /// Add activity message, deduplicating identical events anywhere in the list.
    ///
    /// Matching uses the entry's `key` field when present, otherwise falls back
    /// to exact `message` comparison.
    pub fn add_activity(&mut self, msg: String) {
        if let Some(idx) = self.find_matching_activity(None, &msg) {
            self.bump_activity(idx, msg);
            return;
        }
        self.activity.insert(0, ActivityEntry::new(msg));
        if self.activity.len() > Self::MAX_ACTIVITY_ENTRIES {
            self.activity.pop();
        }
    }

    /// Add activity with a stable dedup key, allowing the display message to
    /// vary (e.g. "Refreshed: 161 skills" vs "Refreshed: 162 skills") while
    /// still consolidating into one line with an incrementing count.
    pub fn add_activity_keyed(&mut self, key: String, msg: String) {
        if let Some(idx) = self.find_matching_activity(Some(&key), &msg) {
            self.bump_activity(idx, msg);
            return;
        }
        self.activity.insert(0, ActivityEntry::new_keyed(key, msg));
        if self.activity.len() > Self::MAX_ACTIVITY_ENTRIES {
            self.activity.pop();
        }
    }

    /// Find an existing activity entry that matches `key` (if given) or `msg`.
    fn find_matching_activity(&self, key: Option<&str>, msg: &str) -> Option<usize> {
        for i in 0..self.activity.len() {
            let entry = &self.activity[i];
            let matches = match (&entry.key, key) {
                // Both have keys — compare keys
                (Some(existing_key), Some(k)) => existing_key == k,
                // Neither has a key — compare messages
                (None, None) => entry.message == msg,
                // One has a key, the other doesn't — no match
                _ => false,
            };
            if matches {
                return Some(i);
            }
        }
        None
    }

    /// Bump an existing activity entry: update count, timestamp, message, and
    /// move it to the front of the list.
    fn bump_activity(&mut self, idx: usize, msg: String) {
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        self.activity[idx].timestamp =
            format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());
        self.activity[idx].count += 1;
        self.activity[idx].message = msg;
        if idx > 0 {
            let entry = self.activity.remove(idx);
            self.activity.insert(0, entry);
        }
    }

    /// Update from metric event.
    pub fn on_metric_event(&mut self, event: MetricEvent) {
        let msg = match event {
            MetricEvent::SkillInvocation {
                skill_name,
                success,
                ..
            } => {
                let status = if success { "OK" } else { "FAIL" };
                format!("[INV] {} - {}", skill_name, status)
            }
            MetricEvent::Validation {
                skill_name,
                checks_failed,
                ..
            } => {
                let status = if checks_failed.is_empty() {
                    "PASS"
                } else {
                    "FAIL"
                };
                format!("[VAL] {} - {}", skill_name, status)
            }
            MetricEvent::Sync {
                ref operation,
                files_count,
                ref status,
                ..
            } => {
                let status_tag = match status {
                    skrills_metrics::SyncStatus::Success => "OK",
                    skrills_metrics::SyncStatus::Failed => "FAIL",
                    skrills_metrics::SyncStatus::InProgress => "...",
                };
                format!(
                    "[SYNC] {} {} files - {}",
                    operation, files_count, status_tag
                )
            }
            MetricEvent::RuleTrigger {
                rule_name,
                ref outcome,
                ..
            } => {
                let tag = match outcome {
                    skrills_metrics::RuleOutcome::Pass => "OK",
                    skrills_metrics::RuleOutcome::Fail => "FAIL",
                    skrills_metrics::RuleOutcome::Skip => "SKIP",
                    skrills_metrics::RuleOutcome::Error => "ERR",
                };
                format!("[RULE] {} - {}", rule_name, tag)
            }
        };
        self.add_activity(msg);
    }
}

/// Dashboard runner.
pub struct Dashboard {
    skill_dirs: Vec<PathBuf>,
    collector: Arc<MetricsCollector>,
    /// Refresh interval in ticks (each tick is 250ms).
    refresh_ticks: u32,
}

impl Dashboard {
    /// Create new dashboard.
    pub fn new(skill_dirs: Vec<PathBuf>) -> Result<Self> {
        let collector = Arc::new(MetricsCollector::new()?);
        Ok(Self {
            skill_dirs,
            collector,
            refresh_ticks: Self::REFRESH_INTERVAL_TICKS,
        })
    }

    /// Create dashboard with existing collector.
    pub fn with_collector(skill_dirs: Vec<PathBuf>, collector: Arc<MetricsCollector>) -> Self {
        Self {
            skill_dirs,
            collector,
            refresh_ticks: Self::REFRESH_INTERVAL_TICKS,
        }
    }

    /// Set the refresh interval in seconds.
    pub fn with_refresh_secs(mut self, secs: u32) -> Self {
        // Each tick is 250ms, so multiply seconds by 4
        self.refresh_ticks = secs.max(1) * 4;
        self
    }

    /// Restore terminal to normal state.
    fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        if let Err(e) = disable_raw_mode() {
            eprintln!("Warning: failed to disable raw mode: {e}");
        }
        if let Err(e) = execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ) {
            eprintln!("Warning: failed to leave alternate screen: {e}");
        }
        if let Err(e) = terminal.show_cursor() {
            eprintln!("Warning: failed to show cursor: {e}");
        }
    }

    /// Run the dashboard.
    pub async fn run(self) -> Result<()> {
        // Install panic hook to restore terminal on panic
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                LeaveAlternateScreen,
                DisableMouseCapture,
                crossterm::cursor::Show
            );
            original_hook(info);
        }));

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.run_inner(&mut terminal).await;

        // Always restore terminal, even on error
        Self::restore_terminal(&mut terminal);

        result
    }

    /// How many ticks between automatic skill refreshes.
    /// At 250 ms per tick this gives a ~5 s refresh cadence.
    const REFRESH_INTERVAL_TICKS: u32 = 20;

    async fn run_inner(self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        // Create app state
        let mut app = App::new();

        // Load initial skills
        self.refresh_skills(&mut app);

        // Subscribe to metrics
        let mut rx = self.collector.subscribe();

        // Event handler
        let mut events = EventHandler::new(Duration::from_millis(250));

        // Ctrl+C signal handler for graceful shutdown
        let sigint = tokio::signal::ctrl_c();
        tokio::pin!(sigint);

        let mut tick_count: u32 = 0;

        // Main loop
        loop {
            // Draw UI
            terminal.draw(|f| ui::draw(f, &mut app))?;

            // Handle events
            tokio::select! {
                event = events.next() => {
                    match event {
                        Some(Event::Key(key)) => {
                            app.on_key(key.code);
                            if app.should_quit {
                                break;
                            }
                        }
                        Some(Event::Tick) => {
                            tick_count += 1;
                            if tick_count >= self.refresh_ticks {
                                tick_count = 0;
                                self.refresh_skills(&mut app);
                            }
                        }
                        Some(Event::Resize(w, h)) => {
                            terminal.resize(Rect::new(0, 0, w, h))?;
                        }
                        None => break, // Event channel closed
                    }
                }
                Ok(metric) = rx.recv() => {
                    app.on_metric_event(metric);
                }
                _ = &mut sigint => {
                    // Graceful shutdown on Ctrl+C
                    break;
                }
            }
        }

        Ok(())
    }

    /// Version-like suffix pattern: `-v1.2.3`, `-1.2.3`, etc.
    fn strip_version_suffix(s: &str) -> &str {
        // Find last '-' or '_' followed by a version-like pattern
        if let Some(pos) = s.rfind(['-', '_']) {
            let after = &s[pos + 1..];
            // Check if remainder starts with optional 'v' then digit
            let rest = after.strip_prefix('v').unwrap_or(after);
            if rest.starts_with(|c: char| c.is_ascii_digit()) {
                return &s[..pos];
            }
        }
        s
    }

    /// Extract a display name from a discovery path.
    ///
    /// Discovery names are relative paths like
    /// `leyline-v1.3.0/skills/plugin-validation/SKILL.md`.
    /// We produce `leyline:plugin-validation` — preserving the plugin
    /// namespace so distinct plugins with the same skill name stay separate,
    /// while version-only differences are consolidated.
    fn base_skill_name(raw: &str) -> String {
        let p = std::path::Path::new(raw);

        // Skill dir = parent of SKILL.md
        let skill_dir = p
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(raw);

        // Plugin dir = first path component (may contain version suffix)
        let components: Vec<_> = p.components().collect();
        if components.len() >= 3 {
            if let Some(plugin_raw) = components[0].as_os_str().to_str() {
                let plugin = Self::strip_version_suffix(plugin_raw);
                if plugin != skill_dir {
                    return format!("{}:{}", plugin, skill_dir);
                }
            }
        }

        skill_dir.to_string()
    }

    fn refresh_skills(&self, app: &mut App) {
        use skrills_discovery::{discover_skills, skill_roots_or_default};
        use std::collections::HashMap;

        let roots = skill_roots_or_default(&self.skill_dirs);

        let discovered = discover_skills(&roots, None).unwrap_or_default();
        app.total_skills = discovered.len();
        app.skills.clear();

        // Group all occurrences by base skill name, preserving insertion order.
        // This consolidates the same skill found across cache versions
        // (e.g. leyline-v1.3.0/plugin-validation and leyline-v1.4.2/plugin-validation).
        let mut seen_order: Vec<String> = Vec::new();
        let mut grouped: HashMap<String, Vec<_>> = HashMap::new();
        for skill in discovered {
            let name = Self::base_skill_name(&skill.name);
            if !grouped.contains_key(&name) {
                seen_order.push(name.clone());
            }
            grouped.entry(name).or_default().push(skill);
        }

        for (idx, base_name) in seen_order.into_iter().enumerate() {
            let entries = grouped
                .remove(&base_name)
                .expect("base_name present in grouped by seen_order construction");

            // Primary entry is the first (highest-priority) occurrence
            let primary = &entries[0];
            let source = primary.source.label();
            let uri = format!("skill://{}", primary.path.display());

            let locations: Vec<SkillLocation> = entries
                .iter()
                .map(|s| SkillLocation {
                    source: s.source.label(),
                    path: s.path.display().to_string(),
                })
                .collect();

            // Get stats if available
            let stats = self.collector.get_skill_stats(&base_name).ok();
            let invocations = stats.as_ref().map(|s| s.total_invocations()).unwrap_or(0);

            app.skills.push(SkillInfo {
                discovery_index: idx,
                name: base_name,
                source,
                uri,
                locations,
                valid: None,

                invocations,
            });
        }

        // Re-apply current sort order after rebuilding the list
        match app.sort_order {
            SortOrder::Alphabetical => app.skills.sort_by(|a, b| a.name.cmp(&b.name)),
            SortOrder::Discovery => {} // already in discovery order
        }

        // Reset visible window on refresh (keep existing visible_count if user has scrolled)
        // but cap it to the new skills length
        if app.visible_count > app.skills.len() {
            app.visible_count = app.skills.len().max(PAGE_SIZE);
        }

        // Sync list state selection
        if !app.skills.is_empty() {
            let visible = app.visible_skill_count();
            app.skill_index = app.skill_index.min(visible.saturating_sub(1));
            app.skill_list_state.select(Some(app.skill_index));
        }

        // Update analytics summary
        if let Ok(summary) = self.collector.get_analytics_summary() {
            app.total_invocations = summary.total_invocations;
            app.overall_success_rate = summary.success_rate;
        }

        // Update validation summary counts
        if let Ok(val_summary) = self.collector.get_validation_summary() {
            app.valid_skills = val_summary.valid as usize;
            app.invalid_skills = (val_summary.error + val_summary.warning) as usize;
        }

        // Load validation detail for the currently selected skill
        app.selected_validation = None;
        if let Some(skill) = app.skills.get(app.skill_index) {
            if let Ok(history) = self.collector.get_validation_history(&skill.name, 1) {
                app.selected_validation = history.into_iter().next();
            }
        }

        // Update timestamp
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        app.last_refresh = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "now".to_string());

        app.add_activity_keyed(
            "refresh".into(),
            format!("Refreshed: {} skills discovered", app.total_skills),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_focus_panel_next() {
        assert_eq!(FocusPanel::Skills.next(), FocusPanel::Activity);
        assert_eq!(FocusPanel::Activity.next(), FocusPanel::Metrics);
        assert_eq!(FocusPanel::Metrics.next(), FocusPanel::Skills);
    }

    #[test]
    fn test_focus_panel_prev() {
        assert_eq!(FocusPanel::Skills.prev(), FocusPanel::Metrics);
        assert_eq!(FocusPanel::Activity.prev(), FocusPanel::Skills);
        assert_eq!(FocusPanel::Metrics.prev(), FocusPanel::Activity);
    }

    #[test]
    fn test_app_quit() {
        let mut app = App::new();
        assert!(!app.should_quit);
        app.on_key(KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn test_app_navigation() {
        let mut app = App::new();
        app.skills = vec![
            SkillInfo {
                discovery_index: 0,
                name: "a".into(),
                source: "claude".into(),
                uri: "a".into(),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: "/a".into(),
                }],
                valid: None,

                invocations: 0,
            },
            SkillInfo {
                discovery_index: 1,
                name: "b".into(),
                source: "claude".into(),
                uri: "b".into(),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: "/b".into(),
                }],
                valid: None,

                invocations: 0,
            },
        ];
        app.skill_list_state.select(Some(0));

        assert_eq!(app.skill_index, 0);
        app.on_key(KeyCode::Down);
        assert_eq!(app.skill_index, 1);
        assert_eq!(app.skill_list_state.selected(), Some(1));
        app.on_key(KeyCode::Down);
        assert_eq!(app.skill_index, 0); // Wraps
        assert_eq!(app.skill_list_state.selected(), Some(0));
        app.on_key(KeyCode::Up);
        assert_eq!(app.skill_index, 1); // Wraps back
        assert_eq!(app.skill_list_state.selected(), Some(1));

        // Home/End
        app.on_key(KeyCode::Home);
        assert_eq!(app.skill_index, 0);
        assert_eq!(app.skill_list_state.selected(), Some(0));
        app.on_key(KeyCode::End);
        assert_eq!(app.skill_index, 1);
        assert_eq!(app.skill_list_state.selected(), Some(1));
    }

    #[test]
    fn test_activity_limit() {
        let mut app = App::new();
        for i in 0..150 {
            app.add_activity(format!("msg {}", i));
        }
        assert_eq!(app.activity.len(), 100);
    }

    #[test]
    fn test_activity_dedup_consecutive() {
        let mut app = App::new();
        app.add_activity("Refreshed: 161 skills discovered".into());
        app.add_activity("Refreshed: 161 skills discovered".into());
        app.add_activity("Refreshed: 161 skills discovered".into());
        assert_eq!(app.activity.len(), 1);
        assert_eq!(app.activity[0].count, 3);
    }

    #[test]
    fn test_activity_dedup_interleaved() {
        let mut app = App::new();
        app.add_activity("Refreshed: 161 skills discovered".into());
        app.add_activity("[INV] some-skill - OK".into());
        app.add_activity("[RULE] some-rule - PASS".into());
        // Same refresh message should still dedup within the scan window
        app.add_activity("Refreshed: 161 skills discovered".into());
        assert_eq!(app.activity.len(), 3); // not 4
                                           // The deduped entry should be moved to the front
        assert_eq!(app.activity[0].message, "Refreshed: 161 skills discovered");
        assert_eq!(app.activity[0].count, 2);
    }

    #[test]
    fn test_activity_dedup_across_many_events() {
        let mut app = App::new();
        app.add_activity("Refreshed: 161 skills discovered".into());
        // Push many unique events between the two identical messages
        for i in 0..20 {
            app.add_activity(format!("unique-event-{}", i));
        }
        // Same message should still dedup regardless of distance
        app.add_activity("Refreshed: 161 skills discovered".into());
        let refresh_count: usize = app
            .activity
            .iter()
            .filter(|e| e.message == "Refreshed: 161 skills discovered")
            .count();
        assert_eq!(refresh_count, 1);
        // Should be moved to the front with count bumped
        assert_eq!(app.activity[0].message, "Refreshed: 161 skills discovered");
        assert_eq!(app.activity[0].count, 2);
    }

    #[test]
    fn test_activity_keyed_dedup_varying_message() {
        let mut app = App::new();
        // Simulate refresh cycles where skill count changes
        app.add_activity_keyed("refresh".into(), "Refreshed: 161 skills discovered".into());
        app.add_activity("[INV] some-skill - OK".into());
        app.add_activity_keyed("refresh".into(), "Refreshed: 162 skills discovered".into());
        app.add_activity_keyed("refresh".into(), "Refreshed: 160 skills discovered".into());

        // Should be 2 entries: one refresh (consolidated) + one invocation
        assert_eq!(app.activity.len(), 2);
        // Refresh entry should be at front with count 3 and the latest message
        assert_eq!(app.activity[0].message, "Refreshed: 160 skills discovered");
        assert_eq!(app.activity[0].count, 3);
        assert!(app.activity[0].key.as_deref() == Some("refresh"));
    }

    #[test]
    fn test_activity_keyed_does_not_match_unkeyted() {
        let mut app = App::new();
        // A keyed entry should NOT match an unkeyed entry with the same message
        app.add_activity("refresh".into());
        app.add_activity_keyed("refresh".into(), "Refreshed: 161 skills discovered".into());
        assert_eq!(app.activity.len(), 2);
    }

    #[test]
    fn test_activity_format_includes_timestamp_and_count() {
        let mut app = App::new();
        app.add_activity("test event".into());
        app.add_activity("test event".into());
        let formatted = app.activity[0].format(80);
        // Should contain timestamp (HH:MM:SS pattern) and count suffix
        assert!(formatted.contains("(x2)"), "missing count: {}", formatted);
        // Timestamp is 8 chars (HH:MM:SS) at the start
        assert!(
            formatted.len() >= 8,
            "too short for timestamp: {}",
            formatted
        );
        assert_eq!(&formatted[2..3], ":", "no timestamp colon: {}", formatted);
    }

    #[test]
    fn test_base_skill_name() {
        // Versioned cache path → plugin:skill
        assert_eq!(
            Dashboard::base_skill_name("leyline-v1.3.0/skills/plugin-validation/SKILL.md"),
            "leyline:plugin-validation"
        );
        // Different version of same plugin → same key
        assert_eq!(
            Dashboard::base_skill_name("leyline-v1.4.2/skills/plugin-validation/SKILL.md"),
            "leyline:plugin-validation"
        );
        // Different plugin, same skill name → different key
        assert_eq!(
            Dashboard::base_skill_name("abstract-v2.0.0/skills/plugin-validation/SKILL.md"),
            "abstract:plugin-validation"
        );
        // Simple path without plugin prefix
        assert_eq!(Dashboard::base_skill_name("my-skill/SKILL.md"), "my-skill");
        // Bare SKILL.md
        assert_eq!(Dashboard::base_skill_name("SKILL.md"), "SKILL.md");
    }

    #[test]
    fn test_strip_version_suffix() {
        assert_eq!(Dashboard::strip_version_suffix("leyline-v1.3.0"), "leyline");
        assert_eq!(
            Dashboard::strip_version_suffix("abstract-2.0.0"),
            "abstract"
        );
        assert_eq!(Dashboard::strip_version_suffix("my-plugin"), "my-plugin");
        assert_eq!(Dashboard::strip_version_suffix("foo_v3.1"), "foo");
    }

    #[test]
    fn test_toggle_sort() {
        let mut app = App::new();
        app.skills = vec![
            SkillInfo {
                discovery_index: 0,
                name: "c".into(),
                source: "claude".into(),
                uri: "c".into(),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: "/c".into(),
                }],
                valid: None,

                invocations: 0,
            },
            SkillInfo {
                discovery_index: 1,
                name: "a".into(),
                source: "claude".into(),
                uri: "a".into(),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: "/a".into(),
                }],
                valid: None,

                invocations: 0,
            },
            SkillInfo {
                discovery_index: 2,
                name: "b".into(),
                source: "claude".into(),
                uri: "b".into(),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: "/b".into(),
                }],
                valid: None,

                invocations: 0,
            },
        ];
        app.skill_list_state.select(Some(0));

        // Toggle to Alphabetical
        app.on_key(KeyCode::Char('s'));
        assert_eq!(app.sort_order, SortOrder::Alphabetical);
        let names: Vec<&str> = app.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert_eq!(app.skill_index, 0);

        // Toggle back to Discovery
        app.on_key(KeyCode::Char('s'));
        assert_eq!(app.sort_order, SortOrder::Discovery);
        let names: Vec<&str> = app.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["c", "a", "b"]);
        assert_eq!(app.skill_index, 0);
    }

    #[test]
    fn test_sort_order_default() {
        assert_eq!(SortOrder::default(), SortOrder::Discovery);
    }

    /// Helper to create N dummy skills.
    fn make_skills(n: usize) -> Vec<SkillInfo> {
        (0..n)
            .map(|i| SkillInfo {
                discovery_index: i,
                name: format!("skill-{}", i),
                source: "claude".into(),
                uri: format!("skill://{}", i),
                locations: vec![SkillLocation {
                    source: "claude".into(),
                    path: format!("/skill-{}", i),
                }],
                valid: None,

                invocations: 0,
            })
            .collect()
    }

    #[test]
    fn test_lazy_loading_initial_visible_count() {
        let mut app = App::new();
        app.skills = make_skills(120);
        // Default visible count is PAGE_SIZE
        assert_eq!(app.visible_skill_count(), PAGE_SIZE);
        assert_eq!(app.visible_count, PAGE_SIZE);
    }

    #[test]
    fn test_lazy_loading_extends_at_bottom() {
        let mut app = App::new();
        app.skills = make_skills(120);
        app.skill_list_state.select(Some(0));

        // Navigate to the last visible item (index 49)
        for _ in 0..PAGE_SIZE - 1 {
            app.on_key(KeyCode::Down);
        }
        assert_eq!(app.skill_index, PAGE_SIZE - 1);
        // Still on the first page
        assert_eq!(app.visible_count, PAGE_SIZE);

        // One more Down at the bottom loads the next page
        app.on_key(KeyCode::Down);
        assert_eq!(app.visible_count, PAGE_SIZE * 2);
        assert_eq!(app.skill_index, PAGE_SIZE); // moved to first item of new page
    }

    #[test]
    fn test_lazy_loading_wraps_only_when_all_loaded() {
        let mut app = App::new();
        app.skills = make_skills(60);
        app.skill_list_state.select(Some(0));

        // Navigate to the last visible item (index 49)
        for _ in 0..PAGE_SIZE - 1 {
            app.on_key(KeyCode::Down);
        }
        // Down at bottom extends to show remaining 10 skills
        app.on_key(KeyCode::Down);
        assert_eq!(app.visible_count, 60);
        assert_eq!(app.skill_index, PAGE_SIZE);

        // Navigate to the real end
        for _ in 0..9 {
            app.on_key(KeyCode::Down);
        }
        assert_eq!(app.skill_index, 59);

        // Now Down should wrap since all skills are visible
        app.on_key(KeyCode::Down);
        assert_eq!(app.skill_index, 0);
    }

    #[test]
    fn test_lazy_loading_end_key_loads_all() {
        let mut app = App::new();
        app.skills = make_skills(120);
        app.skill_list_state.select(Some(0));

        assert_eq!(app.visible_count, PAGE_SIZE);
        app.on_key(KeyCode::End);
        // End should reveal all skills and jump to last
        assert_eq!(app.visible_count, 120);
        assert_eq!(app.skill_index, 119);
    }

    #[test]
    fn test_lazy_loading_sort_resets_visible() {
        let mut app = App::new();
        app.skills = make_skills(120);
        app.skill_list_state.select(Some(0));

        // Extend visible
        app.on_key(KeyCode::End);
        assert_eq!(app.visible_count, 120);

        // Sort resets to PAGE_SIZE
        app.on_key(KeyCode::Char('s'));
        assert_eq!(app.visible_count, PAGE_SIZE);
    }

    #[test]
    fn test_lazy_loading_small_set_shows_all() {
        let mut app = App::new();
        app.skills = make_skills(10);
        app.skill_list_state.select(Some(0));

        // With fewer skills than PAGE_SIZE, visible_skill_count is just the total
        assert_eq!(app.visible_skill_count(), 10);
    }
}
