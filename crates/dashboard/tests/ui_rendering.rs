//! UI rendering tests for the TUI dashboard.
//!
//! Uses Ratatui's TestBackend to verify that all panels render correctly
//! without needing a real terminal.

use crossterm::event::KeyCode;
use ratatui::{backend::TestBackend, Terminal};

use skrills_dashboard::app::{
    App, FocusPanel, McpServerInfo, SkillInfo, SkillLocation, SortOrder, PAGE_SIZE,
};
use skrills_dashboard::ui;
use skrills_metrics::{MetricEvent, RuleOutcome, SyncOperation, SyncStatus};

/// Helper to create a test terminal with a given width and height.
fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    Terminal::new(backend).expect("Failed to create test terminal")
}

/// Helper to create a sample SkillInfo.
fn sample_skill(name: &str, source: &str, valid: Option<bool>, invocations: u64) -> SkillInfo {
    SkillInfo {
        discovery_index: 0,
        name: name.to_string(),
        source: source.to_string(),
        uri: format!("skill://{}", name),
        locations: vec![SkillLocation {
            source: source.to_string(),
            path: format!("/test/{}/SKILL.md", name),
        }],
        valid,
        invocations,
    }
}

/// Helper to create an app with sample data.
fn app_with_skills() -> App {
    let mut app = App::new();
    app.skills = vec![
        sample_skill("commit", "claude", Some(true), 42),
        sample_skill("review", "codex", Some(false), 7),
        sample_skill("deploy", "copilot", None, 0),
    ];
    app.total_skills = 3;
    app.valid_skills = 1;
    app.invalid_skills = 1;
    app.last_refresh = "2025-01-15T10:30:00Z".to_string();
    app.skill_list_state.select(Some(0));
    app
}

/// Extract the rendered text from the test terminal buffer.
fn render_to_string(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer().clone();
    let mut output = String::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            line.push_str(cell.symbol());
        }
        output.push_str(line.trim_end());
        output.push('\n');
    }
    output
}

// Header Tests

#[test]
fn header_shows_skill_counts_and_timestamp() {
    let mut terminal = test_terminal(130, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("skrills dashboard"),
        "Header should contain title"
    );
    assert!(
        output.contains("Skills: 3"),
        "Header should show total skills count"
    );
    assert!(
        output.contains("Valid: 1"),
        "Header should show valid count"
    );
    assert!(
        output.contains("Invalid: 1"),
        "Header should show invalid count"
    );
    assert!(
        output.contains("Invocations: 0"),
        "Header should show total invocations"
    );
    assert!(
        output.contains("Success: 0.0%"),
        "Header should show overall success rate"
    );
    assert!(
        output.contains("2025-01-15T10:30:00"),
        "Header should show last refresh timestamp"
    );
}

#[test]
fn header_shows_dash_when_no_refresh() {
    let mut terminal = test_terminal(100, 20);
    let mut app = App::new();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Last: -"),
        "Header should show '-' when no refresh has occurred"
    );
}

// Skills Panel Tests

#[test]
fn skills_panel_renders_skill_names() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("commit"),
        "Skills panel should show skill name 'commit'"
    );
    assert!(
        output.contains("review"),
        "Skills panel should show skill name 'review'"
    );
    assert!(
        output.contains("deploy"),
        "Skills panel should show skill name 'deploy'"
    );
}

#[test]
fn skills_panel_shows_validation_status() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(output.contains("[OK]"), "Should show [OK] for valid skills");
    assert!(
        output.contains("[ERR]"),
        "Should show [ERR] for invalid skills"
    );
    assert!(
        output.contains("[--]"),
        "Should show [--] for unvalidated skills"
    );
}

#[test]
fn skills_panel_title_reflects_sort_order() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    // Default: Discovery order
    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");
    let output = render_to_string(&terminal);
    assert!(
        output.contains(" Skills "),
        "Default sort title should be ' Skills '"
    );

    // Toggle to Alphabetical
    app.on_key(KeyCode::Char('s'));
    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");
    let output = render_to_string(&terminal);
    assert!(
        output.contains("Skills [A-Z]"),
        "Alphabetical sort title should show [A-Z]"
    );
}

#[test]
fn skills_panel_title_shows_visible_total_when_partially_loaded() {
    let mut terminal = test_terminal(100, 40);
    let mut app = App::new();
    // Create more skills than PAGE_SIZE so only a subset is visible
    app.skills = (0..PAGE_SIZE + 20)
        .map(|i| sample_skill(&format!("s{}", i), "claude", Some(true), 0))
        .collect();
    app.skill_list_state.select(Some(0));

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    let expected = format!("({}/{})", PAGE_SIZE, PAGE_SIZE + 20);
    assert!(
        output.contains(&expected),
        "Title should show ({}/{}) when partially loaded, got:\n{}",
        PAGE_SIZE,
        PAGE_SIZE + 20,
        output
    );
}

#[test]
fn skills_panel_title_no_count_when_all_visible() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills(); // only 3 skills, all visible

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    // With all skills visible, title should NOT show (x/y) count
    assert!(
        !output.contains("(3/3)"),
        "Title should NOT show count when all skills are visible"
    );
    assert!(
        output.contains(" Skills "),
        "Title should just say 'Skills'"
    );
}

#[test]
fn skills_panel_empty_state() {
    let mut terminal = test_terminal(100, 20);
    let mut app = App::new();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Skills"),
        "Skills panel header should be visible even with no skills"
    );
}

// Activity Panel Tests

#[test]
fn activity_panel_renders_events() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();
    app.add_activity("[INV] commit - OK".to_string());
    app.add_activity("[VAL] review - FAIL".to_string());
    app.add_activity("[SYNC] push - complete".to_string());

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Activity"),
        "Activity panel header should be visible"
    );
    assert!(
        output.contains("[SYNC]"),
        "Activity should show sync events"
    );
}

#[test]
fn activity_panel_truncates_to_visible_area() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();
    for i in 0..50 {
        app.add_activity(format!("[INV] skill-{} - OK", i));
    }

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Activity"),
        "Activity panel should still render"
    );
}

// Metrics Panel Tests

#[test]
fn metrics_panel_shows_selected_skill_info() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Skill Info"),
        "Metrics panel header should say 'Skill Info'"
    );
    assert!(
        output.contains("Skill: commit"),
        "Should show selected skill name"
    );
    assert!(
        output.contains("Invocations: 42"),
        "Should show invocation count"
    );
}

#[test]
fn metrics_panel_shows_locations() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    app.skills[0].locations.push(SkillLocation {
        source: "cache".to_string(),
        path: "/cache/commit/SKILL.md".to_string(),
    });

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Locations (2)"),
        "Should show location count"
    );
}

#[test]
fn metrics_panel_no_skill_selected() {
    let mut terminal = test_terminal(100, 20);
    let mut app = App::new();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("No skill selected"),
        "Should show placeholder when no skill selected"
    );
}

// Footer Tests

#[test]
fn footer_shows_keyboard_shortcuts() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("q:Quit"),
        "Footer should show quit shortcut"
    );
    assert!(
        output.contains("Tab:Switch Panel"),
        "Footer should show tab shortcut"
    );
    assert!(
        output.contains("j/k:Navigate"),
        "Footer should show navigation shortcuts"
    );
    assert!(
        output.contains("s:Sort"),
        "Footer should show sort shortcut"
    );
    assert!(
        output.contains("?:Help"),
        "Footer should show help shortcut"
    );
}

// Help Overlay Tests

#[test]
fn help_overlay_toggles_on_question_mark() {
    let mut terminal = test_terminal(100, 30);
    let mut app = app_with_skills();

    assert!(!app.show_help);
    app.on_key(KeyCode::Char('?'));
    assert!(app.show_help);

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);
    assert!(
        output.contains("Keyboard Shortcuts"),
        "Help overlay should show keyboard shortcuts heading"
    );
    assert!(
        output.contains("Quit dashboard"),
        "Help overlay should describe quit action"
    );
    assert!(
        output.contains("Toggle sort"),
        "Help overlay should describe sort toggle"
    );

    app.on_key(KeyCode::Char('?'));
    assert!(!app.show_help);
}

#[test]
fn help_overlay_toggles_on_f1() {
    let mut app = App::new();
    app.on_key(KeyCode::F(1));
    assert!(app.show_help, "F1 should toggle help on");
    app.on_key(KeyCode::F(1));
    assert!(!app.show_help, "F1 again should toggle help off");
}

// Focus / Navigation Tests

#[test]
fn tab_cycles_panel_focus() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    assert_eq!(app.focus, FocusPanel::Skills);

    app.on_key(KeyCode::Tab);
    assert_eq!(app.focus, FocusPanel::Activity);

    app.on_key(KeyCode::Tab);
    assert_eq!(app.focus, FocusPanel::Metrics);

    app.on_key(KeyCode::Tab);
    assert_eq!(app.focus, FocusPanel::Skills);

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");
}

#[test]
fn backtab_cycles_panel_focus_reverse() {
    let mut app = App::new();

    assert_eq!(app.focus, FocusPanel::Skills);
    app.on_key(KeyCode::BackTab);
    assert_eq!(app.focus, FocusPanel::Metrics);
    app.on_key(KeyCode::BackTab);
    assert_eq!(app.focus, FocusPanel::Activity);
    app.on_key(KeyCode::BackTab);
    assert_eq!(app.focus, FocusPanel::Skills);
}

#[test]
fn j_k_navigation_moves_skill_selection() {
    let mut app = app_with_skills();
    app.skill_list_state.select(Some(0));

    app.on_key(KeyCode::Char('j'));
    assert_eq!(app.skill_index, 1);
    assert_eq!(app.skill_list_state.selected(), Some(1));

    app.on_key(KeyCode::Char('k'));
    assert_eq!(app.skill_index, 0);
    assert_eq!(app.skill_list_state.selected(), Some(0));
}

#[test]
fn navigation_wraps_around() {
    let mut app = app_with_skills();
    app.skill_list_state.select(Some(0));

    app.on_key(KeyCode::Up);
    assert_eq!(app.skill_index, 2);

    app.on_key(KeyCode::Down);
    assert_eq!(app.skill_index, 0);
}

#[test]
fn home_end_keys() {
    let mut app = app_with_skills();
    app.skill_list_state.select(Some(0));

    app.on_key(KeyCode::End);
    assert_eq!(app.skill_index, 2);
    assert_eq!(app.skill_list_state.selected(), Some(2));

    app.on_key(KeyCode::Home);
    assert_eq!(app.skill_index, 0);
    assert_eq!(app.skill_list_state.selected(), Some(0));
}

#[test]
fn navigation_on_empty_skills_does_not_panic() {
    let mut app = App::new();

    app.on_key(KeyCode::Down);
    app.on_key(KeyCode::Up);
    app.on_key(KeyCode::Home);
    app.on_key(KeyCode::End);
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Char('k'));

    assert_eq!(app.skill_index, 0);
}

// Sort Tests

#[test]
fn sort_toggle_reorders_skills() {
    let mut app = App::new();
    app.skills = vec![
        {
            let mut s = sample_skill("zebra", "claude", None, 0);
            s.discovery_index = 0;
            s
        },
        {
            let mut s = sample_skill("apple", "claude", None, 0);
            s.discovery_index = 1;
            s
        },
        {
            let mut s = sample_skill("mango", "claude", None, 0);
            s.discovery_index = 2;
            s
        },
    ];
    app.skill_list_state.select(Some(0));

    assert_eq!(app.sort_order, SortOrder::Discovery);
    let names: Vec<&str> = app.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["zebra", "apple", "mango"]);

    app.on_key(KeyCode::Char('s'));
    assert_eq!(app.sort_order, SortOrder::Alphabetical);
    let names: Vec<&str> = app.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["apple", "mango", "zebra"]);

    app.on_key(KeyCode::Char('s'));
    assert_eq!(app.sort_order, SortOrder::Discovery);
    let names: Vec<&str> = app.skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["zebra", "apple", "mango"]);
}

#[test]
fn sort_resets_selection_to_top() {
    let mut app = app_with_skills();
    app.skill_list_state.select(Some(0));

    app.on_key(KeyCode::Down);
    app.on_key(KeyCode::Down);
    assert_eq!(app.skill_index, 2);

    app.on_key(KeyCode::Char('s'));
    assert_eq!(app.skill_index, 0);
    assert_eq!(app.skill_list_state.selected(), Some(0));
}

// Quit Tests

#[test]
fn q_quits() {
    let mut app = App::new();
    assert!(!app.should_quit);
    app.on_key(KeyCode::Char('q'));
    assert!(app.should_quit);
}

#[test]
fn esc_quits() {
    let mut app = App::new();
    assert!(!app.should_quit);
    app.on_key(KeyCode::Esc);
    assert!(app.should_quit);
}

// Metric Event Processing Tests

#[test]
fn metric_event_skill_invocation_success() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::SkillInvocation {
        id: 1,
        skill_name: "commit".to_string(),
        plugin: None,
        duration_ms: 100,
        success: true,
        tokens_used: Some(50),
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("[INV]"));
    assert!(app.activity[0].message.contains("commit"));
    assert!(app.activity[0].message.contains("OK"));
}

#[test]
fn metric_event_skill_invocation_failure() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::SkillInvocation {
        id: 2,
        skill_name: "deploy".to_string(),
        plugin: None,
        duration_ms: 500,
        success: false,
        tokens_used: None,
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("FAIL"));
}

#[test]
fn metric_event_validation_pass() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::Validation {
        id: 3,
        skill_name: "commit".to_string(),
        checks_passed: vec!["name".to_string()],
        checks_failed: vec![],
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("[VAL]"));
    assert!(app.activity[0].message.contains("PASS"));
}

#[test]
fn metric_event_validation_fail() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::Validation {
        id: 4,
        skill_name: "bad-skill".to_string(),
        checks_passed: vec![],
        checks_failed: vec!["missing name".to_string()],
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("FAIL"));
}

#[test]
fn metric_event_sync() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::Sync {
        id: 5,
        operation: SyncOperation::Push,
        files_count: 5,
        status: SyncStatus::Success,
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("[SYNC]"));
    assert!(app.activity[0].message.contains("push"));
    assert!(app.activity[0].message.contains("OK"));
}

#[test]
fn metric_event_rule_trigger() {
    let mut app = App::new();
    app.on_metric_event(MetricEvent::RuleTrigger {
        id: 6,
        rule_name: "no-unsafe".to_string(),
        category: Some("safety".to_string()),
        outcome: RuleOutcome::Fail,
        duration_ms: Some(42),
        created_at: "2025-01-15T10:00:00Z".to_string(),
    });

    assert_eq!(app.activity.len(), 1);
    assert!(app.activity[0].message.contains("[RULE]"));
    assert!(app.activity[0].message.contains("no-unsafe"));
    assert!(app.activity[0].message.contains("FAIL"));

    // Test pass outcome
    let mut app2 = App::new();
    app2.on_metric_event(MetricEvent::RuleTrigger {
        id: 7,
        rule_name: "lint-check".to_string(),
        category: None,
        outcome: RuleOutcome::Pass,
        duration_ms: None,
        created_at: "2025-01-15T10:01:00Z".to_string(),
    });

    assert_eq!(app2.activity.len(), 1);
    assert!(app2.activity[0].message.contains("[RULE]"));
    assert!(app2.activity[0].message.contains("OK"));
}

// Rendering at Different Terminal Sizes

#[test]
fn renders_at_minimum_terminal_size() {
    let mut terminal = test_terminal(40, 10);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw at small terminal size should not panic");
}

#[test]
fn renders_at_large_terminal_size() {
    let mut terminal = test_terminal(200, 60);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw at large terminal size should not panic");

    let output = render_to_string(&terminal);
    assert!(output.contains("commit"));
    assert!(output.contains("review"));
    assert!(output.contains("deploy"));
}

// Panel Focus Highlighting

#[test]
fn focused_panel_has_different_style() {
    let mut terminal = test_terminal(100, 20);
    let mut app = app_with_skills();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");
    let buffer1 = terminal.backend().buffer().clone();

    app.on_key(KeyCode::Tab);
    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");
    let buffer2 = terminal.backend().buffer().clone();

    assert_ne!(
        format!("{:?}", buffer1),
        format!("{:?}", buffer2),
        "Changing focus should visually change the rendered output"
    );
}

// Full Dashboard Snapshot

#[test]
fn full_dashboard_snapshot() {
    let mut terminal = test_terminal(120, 30);
    let mut app = app_with_skills();
    app.add_activity("[INV] commit - OK".to_string());
    app.add_activity("[VAL] review - FAIL".to_string());

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);

    assert!(output.contains("skrills dashboard"), "Header present");
    assert!(output.contains("Skills"), "Skills panel present");
    assert!(output.contains("Activity"), "Activity panel present");
    assert!(output.contains("Skill Info"), "Metrics panel present");
    assert!(output.contains("q:Quit"), "Footer present");
    assert!(output.contains("[OK]"), "Validation status present");
    assert!(output.contains("commit"), "Skill names present");
    assert!(output.contains("[INV]"), "Activity events present");
    assert!(output.contains("Invocations: 42"), "Skill metrics present");
}

// MCP Servers Panel Tests

#[test]
fn mcp_panel_hidden_when_no_servers() {
    let mut terminal = test_terminal(120, 25);
    let mut app = App::new();

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);

    assert!(
        !output.contains("MCP Servers"),
        "MCP panel should not appear when no servers exist"
    );
}

#[test]
fn mcp_panel_shows_servers() {
    let mut terminal = test_terminal(120, 30);
    let mut app = App::new();

    app.mcp_servers = vec![
        McpServerInfo {
            name: "test-server".to_string(),
            source: "claude".to_string(),
            transport: "stdio".to_string(),
            command: "/usr/bin/mcp-test".to_string(),
            enabled: true,
            allowed_tools: vec![],
            disabled_tools: vec![],
        },
        McpServerInfo {
            name: "restricted-server".to_string(),
            source: "codex".to_string(),
            transport: "http".to_string(),
            command: "/bin/restricted".to_string(),
            enabled: true,
            allowed_tools: vec!["read_file".to_string()],
            disabled_tools: vec!["delete_file".to_string()],
        },
    ];

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);

    assert!(
        output.contains("MCP Servers (2)"),
        "MCP panel title should show server count"
    );
    assert!(output.contains("test-server"), "Should display server name");
    assert!(output.contains("[claude]"), "Should display server source");
    assert!(
        output.contains("allow:read_file"),
        "Should display allowed tools"
    );
    assert!(
        output.contains("deny:delete_file"),
        "Should display disabled tools"
    );
}

#[test]
fn mcp_panel_shows_disabled_server() {
    let mut terminal = test_terminal(120, 30);
    let mut app = App::new();

    app.mcp_servers = vec![McpServerInfo {
        name: "off-server".to_string(),
        source: "cursor".to_string(),
        transport: "stdio".to_string(),
        command: "/bin/off".to_string(),
        enabled: false,
        allowed_tools: vec![],
        disabled_tools: vec![],
    }];

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);

    assert!(
        output.contains("- off-server"),
        "Disabled server should show '-' prefix"
    );
}

#[test]
fn header_shows_mcp_count_when_servers_present() {
    let mut terminal = test_terminal(130, 25);
    let mut app = App::new();

    app.mcp_servers = vec![McpServerInfo {
        name: "s1".to_string(),
        source: "claude".to_string(),
        transport: "stdio".to_string(),
        command: "/bin/s".to_string(),
        enabled: true,
        allowed_tools: vec![],
        disabled_tools: vec![],
    }];

    terminal
        .draw(|f| ui::draw(f, &mut app))
        .expect("draw failed");

    let output = render_to_string(&terminal);

    assert!(
        output.contains("MCP: 1"),
        "Header should show MCP server count when servers present"
    );
}
