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
use skrills_metrics::{MetricEvent, MetricsCollector, SkillStats};

use crate::events::{Event, EventHandler};
use crate::ui;

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

/// Skill display info.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// Skill name.
    pub name: String,
    /// Skill URI.
    pub uri: String,
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
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Home => self.skill_index = 0,
            KeyCode::End => {
                if !self.skills.is_empty() {
                    self.skill_index = self.skills.len() - 1;
                }
            }
            _ => {}
        }
    }

    fn select_next(&mut self) {
        if !self.skills.is_empty() {
            self.skill_index = (self.skill_index + 1) % self.skills.len();
        }
    }

    fn select_prev(&mut self) {
        if !self.skills.is_empty() {
            if self.skill_index == 0 {
                self.skill_index = self.skills.len() - 1;
            } else {
                self.skill_index -= 1;
            }
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
            terminal.draw(|f| ui::draw(f, &app))?;

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

    fn refresh_skills(&self, app: &mut App) {
        use skrills_discovery::{discover_skills, skill_roots_or_default};

        let roots = skill_roots_or_default(&self.skill_dirs);

        let discovered = discover_skills(&roots, None).unwrap_or_default();
        app.total_skills = discovered.len();
        app.skills.clear();

        for skill in discovered {
            let name = skill
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Get stats if available
            let stats = self.collector.get_skill_stats(&name).ok();
            let invocations = stats.as_ref().map(|s| s.total_invocations).unwrap_or(0);

            // Build URI from path
            let uri = format!("skill://{}", skill.path.display());

            app.skills.push(SkillInfo {
                name,
                uri,
                valid: None, // Will be set by validation
                last_used: None,
                invocations,
            });
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
                name: "a".into(),
                uri: "a".into(),
                valid: None,
                last_used: None,
                invocations: 0,
            },
            SkillInfo {
                name: "b".into(),
                uri: "b".into(),
                valid: None,
                last_used: None,
                invocations: 0,
            },
        ];

        assert_eq!(app.skill_index, 0);
        app.on_key(KeyCode::Down);
        assert_eq!(app.skill_index, 1);
        app.on_key(KeyCode::Down);
        assert_eq!(app.skill_index, 0); // Wraps
        app.on_key(KeyCode::Up);
        assert_eq!(app.skill_index, 1); // Wraps back
    }

    #[test]
    fn test_activity_limit() {
        let mut app = App::new();
        for i in 0..150 {
            app.add_activity(format!("msg {}", i));
        }
        assert_eq!(app.activity.len(), 100);
    }
}
