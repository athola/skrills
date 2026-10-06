//! Unit tests for the setup module.
//!
//! These tests follow TDD/BDD principles, focusing on business logic and use cases.

use super::*;
use std::fs;
use tempfile::TempDir;

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::env_guard()
}

/// Test fixture helper for creating temporary home directories
fn create_test_home() -> Result<TempDir> {
    TempDir::new().context("Failed to create temp dir")
}

/// Set HOME environment variable to test directory, returning an RAII guard
/// that restores the original value on drop.
/// Note: dirs::home_dir() may not respect this on all platforms
fn set_test_home(dir: &TempDir) -> skrills_test_utils::EnvVarGuard {
    crate::test_support::set_env_var("HOME", Some(dir.path().to_str().unwrap()))
}

/// The installer passes `--bin-dir ~/.skrills/bin` while running the binary
/// it just put there. When the two paths name the same file through a
/// symlink, copying would truncate the binary onto itself.
#[cfg(unix)]
#[test]
fn install_binary_leaves_the_running_binary_alone_behind_a_symlink() -> Result<()> {
    let _guard = env_guard();
    let home = create_test_home()?;
    let _home = set_test_home(&home);

    let real_dir = home.path().join("real-bin");
    fs::create_dir_all(&real_dir)?;
    let exe = installed_binary_path(&real_dir);
    fs::write(&exe, b"binary contents")?;
    let linked_dir = home.path().join("linked-bin");
    std::os::unix::fs::symlink(&real_dir, &linked_dir)?;

    let installed = install_binary(&linked_dir, &exe)?;

    assert_eq!(installed, installed_binary_path(&linked_dir));
    assert_eq!(fs::read(&exe)?, b"binary contents");
    Ok(())
}

#[cfg(test)]
mod client_tests {
    use super::*;

    #[test]
    fn test_client_base_dir_claude() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let base_dir = Client::Claude.base_dir()?;
        assert!(base_dir.ends_with(".claude"));
        Ok(())
    }

    #[test]
    fn test_client_base_dir_codex() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let base_dir = Client::Codex.base_dir()?;
        assert!(base_dir.ends_with(".codex"));
        Ok(())
    }

    #[test]
    fn test_client_default_bin_dir() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let bin_dir = Client::Claude.default_bin_dir()?;
        assert!(bin_dir.ends_with(".claude/bin"));
        Ok(())
    }

    #[test]
    fn test_client_base_dir_copilot() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // With no legacy .copilot dir, falls back to config dir or legacy path
        let base_dir = Client::Copilot.base_dir()?;
        // Should end with "copilot" regardless of XDG or legacy path
        assert!(
            base_dir.ends_with("copilot") || base_dir.ends_with(".copilot"),
            "Copilot base_dir should end with copilot: {:?}",
            base_dir
        );
        Ok(())
    }

    #[test]
    fn test_client_base_dir_copilot_legacy_takes_precedence() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create legacy .copilot directory
        fs::create_dir_all(temp.path().join(".copilot"))?;

        let base_dir = Client::Copilot.base_dir()?;
        assert!(
            base_dir.ends_with(".copilot"),
            "Legacy .copilot should take precedence: {:?}",
            base_dir
        );
        Ok(())
    }

    #[test]
    fn test_client_base_dir_cursor() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let base_dir = Client::Cursor.base_dir()?;
        assert!(base_dir.ends_with(".cursor"));
        Ok(())
    }

    #[test]
    fn test_client_as_str() {
        assert_eq!(Client::Claude.as_str(), "claude");
        assert_eq!(Client::Codex.as_str(), "codex");
        assert_eq!(Client::Copilot.as_str(), "copilot");
        assert_eq!(Client::Cursor.as_str(), "cursor");
    }

    #[test]
    fn test_client_from_str_valid() -> Result<()> {
        assert_eq!(Client::from_str("claude")?, Client::Claude);
        assert_eq!(Client::from_str("codex")?, Client::Codex);
        assert_eq!(Client::from_str("copilot")?, Client::Copilot);
        assert_eq!(Client::from_str("cursor")?, Client::Cursor);
        assert_eq!(Client::from_str("CLAUDE")?, Client::Claude);
        assert_eq!(Client::from_str("Codex")?, Client::Codex);
        assert_eq!(Client::from_str("COPILOT")?, Client::Copilot);
        assert_eq!(Client::from_str("Cursor")?, Client::Cursor);
        Ok(())
    }

    #[test]
    fn test_client_from_str_invalid() {
        assert!(Client::from_str("invalid").is_err());
        assert!(Client::from_str("").is_err());
        assert!(Client::from_str("vscode").is_err());
    }
}

#[cfg(test)]
mod detection_tests {
    use super::*;

    #[test]
    fn test_is_setup_detects_fresh_install() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Fresh install - no setup
        assert!(!is_setup(Client::Claude)?);
        assert!(!is_setup(Client::Codex)?);
        Ok(())
    }

    // Note: These tests verify the logic but may not work with dirs::home_dir()
    // which doesn't always respect HOME env var. They demonstrate the intended behavior.

    #[test]
    fn test_is_first_run_no_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        assert!(is_first_run()?);
        Ok(())
    }

    // Note: These tests demonstrate expected behavior but may be skipped
    // due to HOME directory limitations in test environment
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn test_parse_clients_claude() -> Result<()> {
        let clients = parse_clients("claude")?;
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0], Client::Claude);
        Ok(())
    }

    #[test]
    fn test_parse_clients_codex() -> Result<()> {
        let clients = parse_clients("codex")?;
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0], Client::Codex);
        Ok(())
    }

    #[test]
    fn test_parse_clients_both() -> Result<()> {
        let clients = parse_clients("both")?;
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0], Client::Claude);
        assert_eq!(clients[1], Client::Codex);
        Ok(())
    }

    #[test]
    fn test_parse_clients_copilot() -> Result<()> {
        let clients = parse_clients("copilot")?;
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0], Client::Copilot);
        Ok(())
    }

    #[test]
    fn test_parse_clients_cursor() -> Result<()> {
        let clients = parse_clients("cursor")?;
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0], Client::Cursor);
        Ok(())
    }

    #[test]
    fn test_parse_clients_all() -> Result<()> {
        let clients = parse_clients("all")?;
        assert_eq!(clients.len(), 4);
        assert!(clients.contains(&Client::Claude));
        assert!(clients.contains(&Client::Codex));
        assert!(clients.contains(&Client::Copilot));
        assert!(clients.contains(&Client::Cursor));
        Ok(())
    }

    #[test]
    fn test_parse_clients_case_insensitive() -> Result<()> {
        for (mixed, lower) in [
            ("CLAUDE", "claude"),
            ("Codex", "codex"),
            ("BOTH", "both"),
            ("ALL", "all"),
            ("Copilot", "copilot"),
            ("CURSOR", "cursor"),
        ] {
            assert_eq!(parse_clients(mixed)?, parse_clients(lower)?, "{mixed}");
        }
        Ok(())
    }

    #[test]
    fn test_parse_clients_invalid() {
        assert!(parse_clients("invalid").is_err());
        assert!(parse_clients("").is_err());
        assert!(parse_clients("vscode").is_err());
    }
}

#[cfg(test)]
mod bdd_scenarios {
    use super::*;

    // BDD: Given-When-Then style tests for business scenarios

    #[test]
    fn scenario_fresh_install_detects_no_setup() -> Result<()> {
        // Given: A fresh system with no existing skrills setup
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // When: I check if setup exists
        let is_first = is_first_run()?;

        // Then: The system should detect this is a first run
        assert!(is_first);
        Ok(())
    }

    // Note: Integration tests use actual HOME directory - see Makefile demos

    #[test]
    fn scenario_user_wants_both_clients() -> Result<()> {
        // Given: A user wants to set up both Claude and Codex
        let client_spec = "both";

        // When: I parse the client specification
        let clients = parse_clients(client_spec)?;

        // Then: I should get both clients
        assert_eq!(clients.len(), 2);
        assert!(clients.contains(&Client::Claude));
        assert!(clients.contains(&Client::Codex));
        Ok(())
    }
}

#[cfg(test)]
mod is_setup_detection_tests {
    use super::*;

    #[test]
    fn test_is_setup_claude_with_mcp_json() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .claude directory
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;

        // Create .mcp.json with skrills entry
        let mcp_path = claude_dir.join(".mcp.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers": {"skrills": {"command": "skrills"}}}"#,
        )?;

        // Should detect setup
        assert!(is_setup(Client::Claude)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_claude_no_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .claude directory but no setup files
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;

        // Should not detect setup
        assert!(!is_setup(Client::Claude)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_codex_with_config_toml() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .codex directory
        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        // Create config.toml with skrills MCP server
        let config_path = codex_dir.join("config.toml");
        fs::write(&config_path, "[mcp_servers.skrills]\ncommand = \"skrills\"")?;

        // Should detect setup
        assert!(is_setup(Client::Codex)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_codex_no_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .codex directory but no setup files
        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        // Should not detect setup
        assert!(!is_setup(Client::Codex)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_copilot_with_mcp_servers_json() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .copilot directory (legacy path, so base_dir finds it)
        let copilot_dir = temp.path().join(".copilot");
        fs::create_dir_all(&copilot_dir)?;

        // Create mcp_servers.json with skrills entry
        let mcp_path = copilot_dir.join("mcp_servers.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers": {"skrills": {"command": "skrills"}}}"#,
        )?;

        assert!(is_setup(Client::Copilot)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_copilot_no_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .copilot directory but no setup files
        let copilot_dir = temp.path().join(".copilot");
        fs::create_dir_all(&copilot_dir)?;

        assert!(!is_setup(Client::Copilot)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_cursor_with_mcp_json() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .cursor directory
        let cursor_dir = temp.path().join(".cursor");
        fs::create_dir_all(&cursor_dir)?;

        // Create mcp.json with skrills entry
        let mcp_path = cursor_dir.join("mcp.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers": {"skrills": {"command": "skrills"}}}"#,
        )?;

        assert!(is_setup(Client::Cursor)?);
        Ok(())
    }

    #[test]
    fn test_is_setup_cursor_no_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Create .cursor directory but no setup files
        let cursor_dir = temp.path().join(".cursor");
        fs::create_dir_all(&cursor_dir)?;

        assert!(!is_setup(Client::Cursor)?);
        Ok(())
    }

    #[test]
    fn test_is_first_run_partial_setup() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        // Set up Claude only
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;
        let mcp_path = claude_dir.join(".mcp.json");
        fs::write(&mcp_path, r#"{"mcpServers": {"skrills": {}}}"#)?;

        // Should not be first run (Claude is set up)
        assert!(!is_first_run()?);
        Ok(())
    }
}

#[cfg(test)]
mod register_claude_mcp_tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;

    /// A `claude` CLI stand-in that records its arguments and answers with a
    /// fixed outcome, so no test touches the developer's real Claude config.
    struct FakeClaude {
        calls: RefCell<Vec<Vec<String>>>,
        succeed: Option<bool>,
    }

    impl FakeClaude {
        fn missing() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                succeed: None,
            }
        }

        fn answering(success: bool) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                succeed: Some(success),
            }
        }

        fn run(&self, args: &[&OsStr]) -> std::io::Result<ClaudeCliOutcome> {
            self.calls.borrow_mut().push(
                args.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect(),
            );
            match self.succeed {
                None => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
                Some(success) => Ok(ClaudeCliOutcome {
                    success,
                    stderr: if success {
                        String::new()
                    } else {
                        "boom".to_string()
                    },
                }),
            }
        }
    }

    #[test]
    fn test_register_claude_mcp_creates_new_file() -> Result<()> {
        let temp = create_test_home()?;
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;
        let fake = FakeClaude::missing();

        register_claude_mcp_with(&claude_dir, &temp.path().join("skrills"), &|a| fake.run(a))?;

        let content = fs::read_to_string(claude_dir.join(".mcp.json"))?;
        let parsed: serde_json::Value = serde_json::from_str(&content)?;
        assert_eq!(parsed["mcpServers"]["skrills"]["args"][0], "serve");
        Ok(())
    }

    #[test]
    fn test_register_claude_mcp_updates_existing() -> Result<()> {
        let temp = create_test_home()?;
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;
        let mcp_path = claude_dir.join(".mcp.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers": {"other": {"command": "other"}}}"#,
        )?;
        let fake = FakeClaude::answering(false);

        register_claude_mcp_with(&claude_dir, &temp.path().join("skrills"), &|a| fake.run(a))?;

        let parsed: serde_json::Value = serde_json::from_str(&fs::read_to_string(&mcp_path)?)?;
        assert!(parsed["mcpServers"]["skrills"].is_object());
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        Ok(())
    }

    /// The CLI's default scope is per-directory, which `is_setup` never sees.
    #[test]
    fn register_claude_mcp_uses_user_scope_and_skips_the_fallback() -> Result<()> {
        let temp = create_test_home()?;
        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;
        let fake = FakeClaude::answering(true);

        register_claude_mcp_with(&claude_dir, Path::new("/opt/skrills"), &|a| fake.run(a))?;

        let calls = fake.calls.borrow();
        assert_eq!(
            calls[0],
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "stdio",
                "skrills",
                "--",
                "/opt/skrills",
                "serve"
            ]
        );
        assert!(!claude_dir.join(".mcp.json").exists());
        Ok(())
    }

    #[test]
    fn is_setup_claude_reads_the_user_scope_registration() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        assert!(!is_setup(Client::Claude)?);
        fs::write(
            temp.path().join(".claude.json"),
            r#"{"mcpServers": {"skrills": {"command": "skrills"}}}"#,
        )?;
        assert!(is_setup(Client::Claude)?);
        Ok(())
    }

    #[test]
    fn uninstall_claude_removes_the_user_scope_registration() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);
        fs::write(
            temp.path().join(".claude.json"),
            r#"{"mcpServers": {"skrills": {"command": "skrills"}}}"#,
        )?;
        let fake = FakeClaude::answering(true);

        uninstall_claude_with(&|a| fake.run(a))?;

        assert_eq!(
            fake.calls.borrow()[0],
            ["mcp", "remove", "--scope", "user", "skrills"]
        );
        Ok(())
    }

    #[test]
    fn uninstall_claude_leaves_the_cli_alone_when_nothing_is_registered() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);
        let fake = FakeClaude::answering(true);

        uninstall_claude_with(&|a| fake.run(a))?;

        assert!(fake.calls.borrow().is_empty());
        Ok(())
    }
}

#[cfg(test)]
mod marked_section_tests {
    use super::*;

    #[test]
    fn remove_marked_section_strips_the_markers_and_their_body() {
        let out = remove_marked_section(
            "# A\n<!-- s -->\nbody\n<!-- e -->\n# B\n",
            "<!-- s -->",
            "<!-- e -->",
        );
        assert_eq!(out.as_deref(), Some("# A\n\n# B\n"));
    }

    /// An end marker above the start marker used to slice backwards.
    #[test]
    fn remove_marked_section_ignores_an_end_marker_before_the_start() {
        let text = "<!-- e -->\n# A\n<!-- s -->\nbody\n";
        assert_eq!(
            remove_marked_section(text, "<!-- s -->", "<!-- e -->"),
            None
        );
    }
}

#[cfg(test)]
mod register_codex_mcp_tests {
    use super::*;

    #[test]
    fn test_register_codex_mcp_new_file() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        let skrills_bin = temp.path().join("skrills");

        // Register MCP (creates new config.toml)
        register_codex_mcp(&codex_dir, &skrills_bin)?;

        let config_path = codex_dir.join("config.toml");
        assert!(config_path.exists());

        let content = fs::read_to_string(&config_path)?;
        assert!(content.contains("[mcp_servers.skrills]"));
        Ok(())
    }

    #[test]
    fn test_register_codex_mcp_already_registered() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        // Create existing config with skrills already registered
        let config_path = codex_dir.join("config.toml");
        fs::write(
            &config_path,
            "[mcp_servers.skrills]\ncommand = \"old-path\"",
        )?;

        let skrills_bin = temp.path().join("skrills");

        // Should not duplicate
        register_codex_mcp(&codex_dir, &skrills_bin)?;

        let content = fs::read_to_string(&config_path)?;
        let count = content.matches("[mcp_servers.skrills]").count();
        assert_eq!(count, 1);
        Ok(())
    }
}

#[cfg(test)]
mod uninstall_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_uninstall_claude_removes_hook() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(claude_dir.join("hooks"))?;

        // Create hook file
        let hook_path = claude_dir.join("hooks/prompt.on_user_prompt_submit");
        fs::write(&hook_path, "#!/bin/bash\necho test")?;

        // Uninstall should remove hook
        uninstall_claude_with(&|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)))?;

        assert!(!hook_path.exists());
        Ok(())
    }

    #[test]
    fn test_uninstall_claude_removes_mcp() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let claude_dir = temp.path().join(".claude");
        fs::create_dir_all(&claude_dir)?;

        // Create .mcp.json with skrills
        let mcp_path = claude_dir.join(".mcp.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers": {"skrills": {"command": "skrills"}, "other": {}}}"#,
        )?;

        // Uninstall should remove skrills entry
        uninstall_claude_with(&|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)))?;

        let content = fs::read_to_string(&mcp_path)?;
        assert!(!content.contains("skrills"));
        assert!(content.contains("other"));
        Ok(())
    }

    #[test]
    fn test_uninstall_codex_removes_agents_md_section() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        // Create AGENTS.md with skrills section
        let agents_path = codex_dir.join("AGENTS.md");
        fs::write(
            &agents_path,
            "# Start\n<!-- skrills-integration-start -->\nContent\n<!-- skrills-integration-end -->\n# Middle\n<!-- available_skills:start -->\nskills\n<!-- available_skills:end -->\n<!-- available_agents:start -->\nagents\n<!-- available_agents:end -->\n# End",
        )?;

        // Uninstall should remove skrills section
        uninstall_codex()?;

        let content = fs::read_to_string(&agents_path)?;
        assert!(!content.contains("skrills-integration"));
        assert!(!content.contains("available_skills"), "{content}");
        assert!(!content.contains("available_agents"), "{content}");
        assert!(content.contains("# Middle"));
        assert!(content.contains("# Start"));
        assert!(content.contains("# End"));
        Ok(())
    }

    #[test]
    fn test_uninstall_codex_removes_config_toml_section() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;

        // Create config.toml with skrills section (without the comment)
        let config_path = codex_dir.join("config.toml");
        fs::write(
            &config_path,
            "[mcp_servers.skrills]\ncommand = \"skrills\"\n\n[other]",
        )?;

        // Uninstall should remove skrills section
        uninstall_codex()?;

        let content = fs::read_to_string(&config_path)?;
        assert!(!content.contains("skrills"));
        assert!(content.contains("[other]"));
        Ok(())
    }
}

#[cfg(test)]
mod sync_universal_tests {
    use super::*;

    #[test]
    fn test_sync_universal_no_source() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let config = SetupConfig {
            clients: vec![Client::Claude],
            bin_dir: temp.path().join("bin"),
            reinstall: false,
            uninstall: false,
            add: false,
            yes: true,
            universal: true,
            mirror_source: Some(temp.path().join("nonexistent")),
        };

        // Should handle missing source gracefully
        sync_universal(&config)?;
        Ok(())
    }

    /// The fallback copied the whole mirror source, so `~/.claude` secrets and
    /// transcripts landed in the shared skills directory. Only skills belong
    /// there, and they must land even with no installed binary to shell out to.
    #[test]
    fn sync_universal_copies_skills_and_nothing_else() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let claude = temp.path().join(".claude");
        fs::create_dir_all(claude.join("skills/demo"))?;
        fs::write(claude.join("skills/demo/SKILL.md"), "demo")?;
        fs::write(claude.join(".credentials.json"), "{\"token\":\"secret\"}")?;
        fs::create_dir_all(claude.join("projects/p"))?;
        fs::write(claude.join("projects/p/session.jsonl"), "transcript")?;

        let config = SetupConfig {
            clients: vec![Client::Claude],
            bin_dir: temp.path().join("bin"),
            reinstall: false,
            uninstall: false,
            add: false,
            yes: true,
            universal: true,
            mirror_source: None,
        };
        sync_universal(&config)?;

        let agent_skills = temp.path().join(".agent/skills");
        assert_eq!(
            fs::read_to_string(agent_skills.join("skills/demo/SKILL.md"))?,
            "demo"
        );
        assert!(!agent_skills.join(".credentials.json").exists());
        assert!(!agent_skills.join("projects/p/session.jsonl").exists());
        Ok(())
    }
}

#[cfg(test)]
mod mcp_json_tests {
    use super::*;

    /// A config that does not parse used to be replaced by `{}` and written
    /// back, deleting every other server the user had registered.
    #[test]
    fn read_mcp_json_refuses_a_file_that_does_not_parse() -> Result<()> {
        let temp = create_test_home()?;
        let path = temp.path().join("mcp.json");
        let original = "{\"mcpServers\": {\"other\": {}},}";
        fs::write(&path, original)?;

        let err = read_mcp_json(&path).expect_err("a trailing comma should be an error");
        assert!(format!("{err:#}").contains("mcp.json"), "got: {err:#}");
        assert_eq!(fs::read_to_string(&path)?, original);
        Ok(())
    }

    #[test]
    fn read_mcp_json_refuses_a_root_that_is_not_an_object() -> Result<()> {
        let temp = create_test_home()?;
        let path = temp.path().join("mcp.json");
        fs::write(&path, "[]")?;

        assert!(read_mcp_json(&path).is_err());
        Ok(())
    }

    #[test]
    fn read_mcp_json_treats_a_missing_file_as_empty() -> Result<()> {
        let temp = create_test_home()?;
        let config = read_mcp_json(&temp.path().join("absent.json"))?;
        assert!(config.is_empty());
        Ok(())
    }

    #[test]
    fn add_mcp_server_entry_refuses_a_non_object_mcp_servers() {
        let mut config = serde_json::Map::new();
        config.insert("mcpServers".into(), serde_json::json!([]));
        assert!(add_mcp_server_entry(&mut config, Path::new("/bin/skrills")).is_err());
    }

    #[test]
    fn register_cursor_mcp_keeps_a_malformed_file_untouched() -> Result<()> {
        let temp = create_test_home()?;
        let original = "{\"mcpServers\": {\"other\": {}},}";
        fs::write(temp.path().join("mcp.json"), original)?;

        assert!(register_cursor_mcp(temp.path(), Path::new("/bin/skrills")).is_err());
        assert_eq!(fs::read_to_string(temp.path().join("mcp.json"))?, original);
        Ok(())
    }

    #[test]
    fn register_copilot_mcp_keeps_other_servers() -> Result<()> {
        let temp = create_test_home()?;
        fs::write(
            temp.path().join("mcp_servers.json"),
            r#"{"mcpServers": {"other": {"command": "other"}}, "keep": 1}"#,
        )?;

        register_copilot_mcp(temp.path(), Path::new("/bin/skrills"))?;

        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("mcp_servers.json"))?)?;
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(parsed["mcpServers"]["skrills"]["command"], "/bin/skrills");
        assert_eq!(parsed["keep"], 1);
        Ok(())
    }
}

#[cfg(test)]
mod codex_toml_tests {
    use super::*;

    /// Setup writes a comment above the table, and uninstall used to stop at
    /// the table header right after it, leaving the registration in place.
    #[test]
    fn register_then_uninstall_codex_removes_the_registration() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;
        let config_path = codex_dir.join("config.toml");
        fs::write(&config_path, "model = \"o3\"\n\n[other]\nkey = 1\n")?;

        register_codex_mcp(&codex_dir, &temp.path().join("skrills"))?;
        assert!(is_setup(Client::Codex)?);

        uninstall_codex()?;

        let content = fs::read_to_string(&config_path)?;
        assert!(!is_setup(Client::Codex)?, "still registered:\n{content}");
        assert!(!content.contains("Skrills MCP server"), "{content}");
        assert!(content.contains("[other]"), "{content}");
        assert!(content.contains("model = \"o3\""), "{content}");
        let parsed: toml::Table = toml::from_str(&content)?;
        assert!(parsed.get("mcp_servers").is_none(), "{content}");
        Ok(())
    }

    #[test]
    fn uninstall_codex_keeps_tables_that_follow_the_registration() -> Result<()> {
        let _guard = env_guard();
        let temp = create_test_home()?;
        let _home = set_test_home(&temp);

        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir)?;
        let config_path = codex_dir.join("config.toml");
        fs::write(
            &config_path,
            "[mcp_servers.skrills]\ncommand = \"skrills\"\nargs = [\"serve\"]\n\n[mcp_servers.other]\ncommand = \"other\"\n",
        )?;

        uninstall_codex()?;

        let parsed: toml::Table = toml::from_str(&fs::read_to_string(&config_path)?)?;
        let servers = parsed["mcp_servers"].as_table().unwrap();
        assert!(servers.get("skrills").is_none());
        assert!(servers.get("other").is_some());
        Ok(())
    }

    /// A Windows path carries backslashes, which a basic TOML string reads as
    /// escapes; the file must still parse and name the same path.
    #[test]
    fn register_codex_mcp_escapes_the_binary_path() -> Result<()> {
        let temp = create_test_home()?;
        let bin = PathBuf::from(r"C:\Users\me\.codex\bin\skrills.exe");

        register_codex_mcp(temp.path(), &bin)?;

        let parsed: toml::Table =
            toml::from_str(&fs::read_to_string(temp.path().join("config.toml"))?)?;
        assert_eq!(
            parsed["mcp_servers"]["skrills"]["command"].as_str(),
            Some(r"C:\Users\me\.codex\bin\skrills.exe")
        );
        Ok(())
    }

    #[test]
    fn installed_binary_name_carries_the_platform_suffix() {
        assert_eq!(
            installed_binary_path(Path::new("/opt/bin")),
            Path::new("/opt/bin").join(format!("skrills{}", std::env::consts::EXE_SUFFIX))
        );
    }

    fn feature_config(initial: &str) -> Result<(TempDir, PathBuf)> {
        let temp = create_test_home()?;
        let path = temp.path().join("config.toml");
        fs::write(&path, initial)?;
        Ok((temp, path))
    }

    /// `skills_dir` starts with `skills` but is a different key.
    #[test]
    fn feature_flag_leaves_keys_that_only_start_with_skills() -> Result<()> {
        let (_temp, path) = feature_config("[features]\nskills_dir = \"x\"\n")?;

        ensure_codex_skills_feature_enabled(&path)?;

        let parsed: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(parsed["features"]["skills_dir"].as_str(), Some("x"));
        assert_eq!(parsed["features"]["skills"].as_bool(), Some(true));
        Ok(())
    }

    /// An array-of-tables header ends `[features]` like any other header.
    #[test]
    fn feature_flag_stops_at_an_array_of_tables_header() -> Result<()> {
        let (_temp, path) =
            feature_config("[features]\nother = 1\n\n[[profiles]]\nname = \"a\"\n")?;

        ensure_codex_skills_feature_enabled(&path)?;

        let parsed: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(parsed["features"]["skills"].as_bool(), Some(true));
        let profile = parsed["profiles"].as_array().unwrap()[0]
            .as_table()
            .unwrap();
        assert!(profile.get("skills").is_none(), "{profile:?}");
        Ok(())
    }

    /// Inline and dotted forms that already enable skills must be left alone;
    /// appending a `[features]` table would make the file invalid.
    #[test]
    fn feature_flag_accepts_inline_and_dotted_forms() -> Result<()> {
        for initial in ["features = { skills = true }\n", "features.skills = true\n"] {
            let (_temp, path) = feature_config(initial)?;
            ensure_codex_skills_feature_enabled(&path)?;
            assert_eq!(
                fs::read_to_string(&path)?,
                initial,
                "should not rewrite an enabled flag"
            );
        }
        Ok(())
    }

    /// A form the line editor cannot change safely must not be rewritten into
    /// a file Codex can no longer parse.
    #[test]
    fn feature_flag_refuses_to_write_an_invalid_file() -> Result<()> {
        let initial = "features = { other = 1 }\n";
        let (_temp, path) = feature_config(initial)?;

        assert!(ensure_codex_skills_feature_enabled(&path).is_err());
        assert_eq!(fs::read_to_string(&path)?, initial);
        Ok(())
    }

    #[test]
    fn feature_flag_is_idempotent() -> Result<()> {
        let (_temp, path) = feature_config("model = \"o3\"\n")?;
        ensure_codex_skills_feature_enabled(&path)?;
        let once = fs::read_to_string(&path)?;
        ensure_codex_skills_feature_enabled(&path)?;
        assert_eq!(fs::read_to_string(&path)?, once);
        let parsed: toml::Table = toml::from_str(&once)?;
        assert_eq!(parsed["features"]["skills"].as_bool(), Some(true));
        Ok(())
    }
}
