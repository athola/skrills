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
use skrills_metrics::{MetricEvent, MetricsCollector, SkillStats};

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
    /// Last invocation time.
    pub last_used: Option<String>,
    /// Total invocations.
    pub invocations: u64,
}

/// Application state.
#[derive(Debug, Default)]
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
    /// Recent activity events.
    pub activity: Vec<String>,
    /// Skill stats for selected skill.
    pub selected_stats: Option<SkillStats>,
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
}

impl App {
    /// Create new app state.
    pub fn new() -> Self {
        Self::default()
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
        // Reset selection to top after re-sort
        self.skill_index = 0;
        if !self.skills.is_empty() {
            self.skill_list_state.select(Some(0));
        }
    }

    fn select_next(&mut self) {
        if !self.skills.is_empty() {
            self.skill_index = (self.skill_index + 1) % self.skills.len();
            self.skill_list_state.select(Some(self.skill_index));
        }
    }

    fn select_prev(&mut self) {
        if !self.skills.is_empty() {
            if self.skill_index == 0 {
                self.skill_index = self.skills.len() - 1;
            } else {
                self.skill_index -= 1;
            }
            self.skill_list_state.select(Some(self.skill_index));
        }
    }

    /// Maximum activity entries to keep.
    const MAX_ACTIVITY_ENTRIES: usize = 100;

    /// Add activity message.
    pub fn add_activity(&mut self, msg: String) {
        self.activity.insert(0, msg);
        if self.activity.len() > Self::MAX_ACTIVITY_ENTRIES {
            self.activity.pop();
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
                operation, status, ..
            } => {
                format!("[SYNC] {} - {}", operation, status)
            }
        };
        self.add_activity(msg);
    }
}

/// Dashboard runner.
pub struct Dashboard {
    skill_dirs: Vec<PathBuf>,
    collector: Arc<MetricsCollector>,
}

impl Dashboard {
    /// Create new dashboard.
    pub fn new(skill_dirs: Vec<PathBuf>) -> Result<Self> {
        let collector = Arc::new(MetricsCollector::new()?);
        Ok(Self {
            skill_dirs,
            collector,
        })
    }

    /// Create dashboard with existing collector.
    pub fn with_collector(skill_dirs: Vec<PathBuf>, collector: Arc<MetricsCollector>) -> Self {
        Self {
            skill_dirs,
            collector,
        }
    }

    /// Restore terminal to normal state.
    fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
        let _ = disable_raw_mode();
        let _ = execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = terminal.show_cursor();
    }

    /// Run the dashboard.
    pub async fn run(self) -> Result<()> {
        // Install panic hook to restore terminal on panic
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
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

    async fn run_inner(self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        // Create app state
        let mut app = App::new();

        // Load initial skills
        self.refresh_skills(&mut app);

        // Subscribe to metrics
        let mut rx = self.collector.subscribe();

        // Event handler
        let mut events = EventHandler::new(Duration::from_millis(250));

        // Main loop
        loop {
            // Draw UI
            terminal.draw(|f| ui::draw(f, &mut app))?;

            // Handle events
            tokio::select! {
                event = events.next() => {
                    match event {
                        Event::Key(key) => {
                            app.on_key(key.code);
                            if app.should_quit {
                                break;
                            }
                        }
                        Event::Tick => {
                            // Periodic refresh
                        }
                        Event::Resize(_, _) => {
                            // Terminal handles resize
                        }
                    }
                }
                Ok(metric) = rx.recv() => {
                    app.on_metric_event(metric);
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
            let entries = grouped.remove(&base_name).unwrap();

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
            let invocations = stats.as_ref().map(|s| s.total_invocations).unwrap_or(0);

            app.skills.push(SkillInfo {
                discovery_index: idx,
                name: base_name,
                source,
                uri,
                locations,
                valid: None,
                last_used: None,
                invocations,
            });
        }

        // Sync list state selection
        if !app.skills.is_empty() {
            app.skill_index = app.skill_index.min(app.skills.len() - 1);
            app.skill_list_state.select(Some(app.skill_index));
        }

        // Update timestamp
        let now = time::OffsetDateTime::now_utc();
        app.last_refresh = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "now".to_string());

        app.add_activity(format!("Refreshed: {} skills discovered", app.total_skills));
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
                last_used: None,
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
                last_used: None,
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
                last_used: None,
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
                last_used: None,
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
                last_used: None,
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
}
