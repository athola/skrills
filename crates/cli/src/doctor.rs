//! Diagnostics for Codex MCP setup.
//!
//! Inspects and validates MCP server configurations (JSON and TOML)
//! to diagnose common setup issues.

use anyhow::Result;
use skrills_state::home_dir;
use std::fs;
use std::path::Path;

/// Whether `cmd` is a bare program name (`skrills`) rather than a path.
fn is_bare_command(cmd: &str) -> bool {
    !cmd.contains('/') && !cmd.contains(std::path::MAIN_SEPARATOR)
}

/// Whether `cmd` names something Codex can start: an existing path, or a
/// bare name found on `PATH`, which is how Codex resolves it.
fn command_resolves(cmd: &str) -> bool {
    if is_bare_command(cmd) {
        crate::commands::is_on_path(cmd)
    } else {
        Path::new(cmd).exists()
    }
}

/// Validates an MCP server entry and prints diagnostics.
fn validate_mcp_entry(
    entry: &serde_json::Value,
    expected_cmd: &Path,
    config_path: &Path,
    file_label: &str,
    lines: &mut Vec<String>,
) {
    let typ = entry
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    let cmd = entry
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");

    let args_display = entry
        .get("args")
        .map(|v| format!("{:?}", v))
        .unwrap_or_else(|| "None".to_string());
    lines.push(format!(
        "{file_label}: type={typ} command={cmd} args={args_display} ({})",
        config_path.display()
    ));

    if typ != "stdio" {
        lines.push("  ! expected type=\"stdio\"".to_string());
    }
    if file_label.contains("json") && Path::new(cmd) != expected_cmd {
        lines.push("  i command differs; ensure binary path is correct and executable".to_string());
    }
    if !command_resolves(cmd) {
        lines.push(if is_bare_command(cmd) {
            "  ! command not found on PATH".to_string()
        } else {
            "  ! command path does not exist on disk".to_string()
        });
    }
}

/// Inspects a legacy `mcp_servers.json`, if one is present.
///
/// `skrills setup` registers Codex in `config.toml` only, so a missing JSON
/// file is normal and not reported. One that exists is still checked, since
/// a stale entry there can confuse an older Codex.
fn inspect_mcp_json(mcp_path: &Path, expected_cmd: &Path, lines: &mut Vec<String>) -> Result<()> {
    if !mcp_path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(mcp_path)?;
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(json) => {
            if let Some(entry) = json.get("mcpServers").and_then(|m| m.get("skrills")) {
                validate_mcp_entry(entry, expected_cmd, mcp_path, "mcp_servers.json", lines);
            } else {
                lines.push(format!(
                    "mcp_servers.json: missing skrills entry ({})",
                    mcp_path.display()
                ));
            }
        }
        Err(e) => lines.push(format!(
            "mcp_servers.json: failed to parse ({:?}): {}",
            mcp_path.display(),
            e
        )),
    }
    Ok(())
}

/// Inspects the Codex config TOML file.
fn inspect_config_toml(
    cfg_path: &Path,
    expected_cmd: &Path,
    lines: &mut Vec<String>,
) -> Result<()> {
    if !cfg_path.exists() {
        lines.push(format!(
            "config.toml:    not found at {}",
            cfg_path.display()
        ));
        return Ok(());
    }

    let raw = fs::read_to_string(cfg_path)?;
    match toml::from_str::<toml::Value>(&raw) {
        Ok(toml_val) => {
            let entry = toml_val.get("mcp_servers").and_then(|m| m.get("skrills"));
            if let Some(e) = entry {
                let json_entry = serde_json::to_value(e).unwrap_or(serde_json::Value::Null);
                validate_mcp_entry(&json_entry, expected_cmd, cfg_path, "config.toml   ", lines);
            } else {
                lines.push(format!(
                    "config.toml:    missing [mcp_servers.skrills] ({})",
                    cfg_path.display()
                ));
            }
        }
        Err(e) => lines.push(format!(
            "config.toml:    failed to parse ({:?}): {}",
            cfg_path.display(),
            e
        )),
    }
    Ok(())
}

fn doctor_report_with_paths(
    mcp_path: &Path,
    cfg_path: &Path,
    expected_cmd: &Path,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    lines.push("== skrills doctor ==".to_string());
    inspect_config_toml(cfg_path, expected_cmd, &mut lines)?;
    inspect_mcp_json(mcp_path, expected_cmd, &mut lines)?;
    lines.push("Hint: Codex CLI raises 'missing field `type`' when either file lacks type=\"stdio\" for skrills.".to_string());
    Ok(lines)
}

/// Runs diagnostics on Codex MCP configuration files.
///
/// Inspects `~/.codex/config.toml`, where `skrills setup` registers the
/// server, and a legacy `~/.codex/mcp_servers.json` when one exists.
pub fn doctor_report() -> Result<()> {
    let home = home_dir()?;
    let mcp_path = home.join(".codex/mcp_servers.json");
    let cfg_path = home.join(".codex/config.toml");
    let expected_cmd = home.join(".codex/bin/skrills");
    let lines = doctor_report_with_paths(&mcp_path, &cfg_path, &expected_cmd)?;
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn doctor_reports_missing_files() {
        /*
        GIVEN a home directory without MCP config files
        WHEN generating a doctor report
        THEN it should note missing files
        */
        let _guard = skrills_test_utils::env_guard();
        let temp = tempdir().expect("tempdir");
        let mcp_path = temp.path().join(".codex/mcp_servers.json");
        let cfg_path = temp.path().join(".codex/config.toml");
        let expected_cmd = temp.path().join(".codex/bin/skrills");

        let lines = doctor_report_with_paths(&mcp_path, &cfg_path, &expected_cmd)
            .expect("doctor report should succeed");
        let joined = lines.join("\n");

        assert!(
            !joined.contains("mcp_servers.json"),
            "setup never writes mcp_servers.json, so its absence is not a finding"
        );
        assert!(
            joined.contains("config.toml:    not found at"),
            "missing toml should be reported"
        );
    }

    #[test]
    fn doctor_reports_invalid_and_mismatched_entries() {
        /*
        GIVEN malformed JSON and mismatched MCP entries
        WHEN generating a doctor report
        THEN it should surface parsing and validation messages
        */
        let _guard = skrills_test_utils::env_guard();
        let temp = tempdir().expect("tempdir");
        let codex_dir = temp.path().join(".codex");
        fs::create_dir_all(&codex_dir).expect("create codex dir");

        let mcp_path = codex_dir.join("mcp_servers.json");
        let cfg_path = codex_dir.join("config.toml");
        let expected_cmd = codex_dir.join("bin/skrills");

        fs::write(&mcp_path, "{invalid json").expect("write invalid json");
        fs::write(
            &cfg_path,
            r#"
[mcp_servers.skrills]
type = "http"
command = "/tmp/other"
args = ["--flag"]
"#,
        )
        .expect("write toml");

        let lines = doctor_report_with_paths(&mcp_path, &cfg_path, &expected_cmd)
            .expect("doctor report should succeed");
        let joined = lines.join("\n");

        assert!(
            joined.contains("mcp_servers.json: failed to parse"),
            "invalid json should be reported"
        );
        assert!(
            joined.contains("config.toml   : type=http command=/tmp/other"),
            "config entry should be surfaced"
        );
        assert!(
            joined.contains("expected type=\"stdio\""),
            "type mismatch should be reported"
        );
        assert!(
            joined.contains("command path does not exist"),
            "missing command path should be reported"
        );
    }

    /// SB-40: the report looked for a file setup never writes, and flagged a
    /// bare command name as a missing path.
    #[test]
    fn a_setup_style_config_toml_reports_no_problems() {
        let _guard = skrills_test_utils::env_guard();
        let temp = tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("skrills");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _path = skrills_test_utils::set_env_var("PATH", Some(bin_dir.to_str().unwrap()));
        let cfg_path = temp.path().join("config.toml");
        fs::write(
            &cfg_path,
            "[mcp_servers.skrills]\ncommand = \"skrills\"\ntype = \"stdio\"\nargs = [\"serve\"]\n",
        )
        .unwrap();

        let lines =
            doctor_report_with_paths(&temp.path().join("mcp_servers.json"), &cfg_path, &bin)
                .unwrap();
        let joined = lines.join("\n");

        assert!(joined.contains("type=stdio command=skrills"), "{joined}");
        assert!(
            !joined.contains("  !"),
            "no problem should be flagged: {joined}"
        );
        assert!(!joined.contains("mcp_servers.json"), "{joined}");
    }

    #[test]
    fn a_bare_command_missing_from_path_is_reported_as_such() {
        let _guard = skrills_test_utils::env_guard();
        let temp = tempdir().expect("tempdir");
        let _path = skrills_test_utils::set_env_var("PATH", Some(temp.path().to_str().unwrap()));
        let cfg_path = temp.path().join("config.toml");
        fs::write(
            &cfg_path,
            "[mcp_servers.skrills]\ncommand = \"skrills-nowhere\"\ntype = \"stdio\"\n",
        )
        .unwrap();

        let lines =
            doctor_report_with_paths(&temp.path().join("none.json"), &cfg_path, temp.path())
                .unwrap();

        assert!(
            lines
                .iter()
                .any(|l| l.contains("command not found on PATH")),
            "{lines:?}"
        );
    }
}
