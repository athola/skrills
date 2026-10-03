//! Setup and installation logic.
//!
//! Handles first-run detection, interactive setup, reinstallation, and uninstallation
//! for Claude Code, Codex, and GitHub Copilot clients, and supports a universal
//! `~/.agent/skills` directory.

use anyhow::{anyhow, Context, Result};
use inquire::{Confirm, Select, Text};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Supported client types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Claude,
    Codex,
    Copilot,
    Cursor,
}

impl Client {
    /// Returns the default base directory for this client.
    ///
    /// For Copilot, follows XDG Base Directory Specification:
    /// 1. If `~/.copilot` exists → use it (legacy compatibility)
    /// 2. Otherwise → `$XDG_CONFIG_HOME/copilot` or `~/.config/copilot`
    pub fn base_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
        Ok(match self {
            Client::Claude => home.join(".claude"),
            Client::Codex => home.join(".codex"),
            Client::Copilot => {
                // XDG-compliant path resolution for Copilot
                let legacy_path = home.join(".copilot");
                if legacy_path.exists() {
                    legacy_path
                } else if let Some(config_dir) = dirs::config_dir() {
                    config_dir.join("copilot")
                } else {
                    legacy_path // Fallback to legacy if XDG unavailable
                }
            }
            Client::Cursor => home.join(".cursor"),
        })
    }

    /// Returns the default binary directory for this client.
    pub fn default_bin_dir(&self) -> Result<PathBuf> {
        Ok(self.base_dir()?.join("bin"))
    }

    /// Returns the name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Codex => "codex",
            Client::Copilot => "copilot",
            Client::Cursor => "cursor",
        }
    }

    /// Parses from string.
    pub(crate) fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Client::Claude),
            "codex" => Ok(Client::Codex),
            "copilot" => Ok(Client::Copilot),
            "cursor" => Ok(Client::Cursor),
            _ => Err(anyhow!(
                "Invalid client: {}. Must be 'claude', 'codex', 'copilot', or 'cursor'",
                s
            )),
        }
    }
}

/// Configuration for a setup operation.
#[derive(Debug, Clone)]
pub struct SetupConfig {
    pub clients: Vec<Client>,
    pub bin_dir: PathBuf,
    pub reinstall: bool,
    pub uninstall: bool,
    pub add: bool,
    pub yes: bool,
    pub universal: bool,
    pub mirror_source: Option<PathBuf>,
}

/// Checks if skrills is already set up for a given client.
///
/// Setup is detected by checking for MCP server registration:
/// - Claude: `mcpServers.skrills` in ~/.claude.json (user scope), or the
///   fallback ~/.claude/.mcp.json containing a "skrills" entry
/// - Codex: config.toml containing [mcp_servers.skrills]
/// - Copilot: mcp_servers.json containing "skrills" entry
pub fn is_setup(client: Client) -> Result<bool> {
    let base_dir = client.base_dir()?;

    match client {
        Client::Claude => {
            // `claude mcp add --scope user` records the server in ~/.claude.json.
            if let Some(home) = dirs::home_dir() {
                if claude_json_has_skrills(&home) {
                    return Ok(true);
                }
            }
            // Fallback registration written when the CLI is unavailable.
            let mcp_path = base_dir.join(".mcp.json");
            if mcp_path.exists() {
                if let Ok(content) = fs::read_to_string(&mcp_path) {
                    if content.contains("\"skrills\"") {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Client::Codex => {
            // Check for MCP server registration in config.toml
            let config_path = base_dir.join("config.toml");
            if config_path.exists() {
                if let Ok(content) = fs::read_to_string(&config_path) {
                    if content.contains("[mcp_servers.skrills]") {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Client::Copilot => {
            // Check for MCP registration in mcp_servers.json (JSON format like Claude)
            let mcp_path = base_dir.join("mcp_servers.json");
            if mcp_path.exists() {
                if let Ok(content) = fs::read_to_string(&mcp_path) {
                    if content.contains("\"skrills\"") {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        Client::Cursor => {
            // Check for MCP registration in mcp.json
            let mcp_path = base_dir.join("mcp.json");
            if mcp_path.exists() {
                if let Ok(content) = fs::read_to_string(&mcp_path) {
                    if content.contains("\"skrills\"") {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}

/// Detects if this is the first run (no setup for any client).
pub fn is_first_run() -> Result<bool> {
    let claude_setup = is_setup(Client::Claude).unwrap_or(false);
    let codex_setup = is_setup(Client::Codex).unwrap_or(false);
    let copilot_setup = is_setup(Client::Copilot).unwrap_or(false);
    let cursor_setup = is_setup(Client::Cursor).unwrap_or(false);

    Ok(!claude_setup && !codex_setup && !copilot_setup && !cursor_setup)
}

/// Prompts the user to run setup on first run.
/// Returns true if user wants to proceed with setup.
pub fn prompt_first_run_setup() -> Result<bool> {
    println!("\nSkrills is not configured on this system.");
    println!("Setup creates hooks, registers MCP servers, and configures directories.\n");

    Confirm::new("Would you like to run setup now?")
        .with_default(true)
        .prompt()
        .context("Failed to get user confirmation")
}

/// Interactive setup flow.
#[allow(clippy::too_many_arguments)]
pub fn interactive_setup(
    client_arg: Option<String>,
    bin_dir_arg: Option<PathBuf>,
    reinstall: bool,
    uninstall: bool,
    add: bool,
    yes: bool,
    universal: bool,
    mirror_source: Option<PathBuf>,
) -> Result<SetupConfig> {
    let clients = if let Some(ref client_str) = client_arg {
        parse_clients(client_str)?
    } else if !yes {
        prompt_clients(add)?
    } else {
        return Err(anyhow!("--client required in non-interactive mode (--yes)"));
    };

    // Determine bin_dir
    let bin_dir = if let Some(dir) = bin_dir_arg {
        dir
    } else if !yes {
        prompt_bin_dir(&clients)?
    } else {
        // Use default for first client
        clients[0].default_bin_dir()?
    };

    // Prompt for universal sync if not specified
    let universal = if !yes && !universal {
        Confirm::new("Sync skills to universal ~/.agent/skills directory?")
            .with_default(false)
            .prompt()?
    } else {
        universal
    };

    Ok(SetupConfig {
        clients,
        bin_dir,
        reinstall,
        uninstall,
        add,
        yes,
        universal,
        mirror_source,
    })
}

/// Parses client string into a list of clients.
fn parse_clients(s: &str) -> Result<Vec<Client>> {
    match s.to_lowercase().as_str() {
        "both" => Ok(vec![Client::Claude, Client::Codex]),
        "all" => Ok(vec![
            Client::Claude,
            Client::Codex,
            Client::Copilot,
            Client::Cursor,
        ]),
        _ => Ok(vec![Client::from_str(s)?]),
    }
}

/// Prompts user to select clients.
fn prompt_clients(add_mode: bool) -> Result<Vec<Client>> {
    if add_mode {
        // In add mode, show only clients not already set up
        let claude_setup = is_setup(Client::Claude).unwrap_or(false);
        let codex_setup = is_setup(Client::Codex).unwrap_or(false);
        let copilot_setup = is_setup(Client::Copilot).unwrap_or(false);
        let cursor_setup = is_setup(Client::Cursor).unwrap_or(false);

        let mut options = Vec::new();
        if !claude_setup {
            options.push("Claude Code");
        }
        if !codex_setup {
            options.push("Codex");
        }
        if !copilot_setup {
            options.push("Copilot");
        }
        if !cursor_setup {
            options.push("Cursor");
        }

        if options.is_empty() {
            return Err(anyhow!(
                "All clients (Claude Code, Codex, Copilot, Cursor) are already set up. Use --reinstall to reconfigure."
            ));
        }

        if options.len() == 1 {
            println!(
                "Setting up for {} (the only client not yet configured)",
                options[0]
            );
            return Ok(vec![parse_client_selection(options[0])?]);
        }

        let selection =
            Select::new("Which client would you like to add?", options.clone()).prompt()?;

        Ok(vec![parse_client_selection(selection)?])
    } else {
        let options = vec!["Claude Code", "Codex", "Copilot", "Cursor", "All"];
        let selection =
            Select::new("Which client would you like to set up?", options.clone()).prompt()?;

        match selection {
            "All" => Ok(vec![
                Client::Claude,
                Client::Codex,
                Client::Copilot,
                Client::Cursor,
            ]),
            other => Ok(vec![parse_client_selection(other)?]),
        }
    }
}

/// Prompts user for binary installation directory.
fn prompt_bin_dir(clients: &[Client]) -> Result<PathBuf> {
    let default = clients[0].default_bin_dir()?;
    let default_str = default.display().to_string();

    let input = Text::new("Binary installation directory")
        .with_default(&default_str)
        .prompt()?;

    if input.trim().is_empty() {
        Ok(default)
    } else {
        Ok(PathBuf::from(shellexpand::tilde(&input).into_owned()))
    }
}

fn parse_client_selection(name: &str) -> Result<Client> {
    match name {
        "Claude Code" => Ok(Client::Claude),
        "Codex" => Ok(Client::Codex),
        "Copilot" => Ok(Client::Copilot),
        "Cursor" => Ok(Client::Cursor),
        other => Err(anyhow!("Unknown client: {}", other)),
    }
}

/// Runs the setup process.
pub fn run_setup(config: SetupConfig) -> Result<()> {
    if config.uninstall {
        return run_uninstall(&config);
    }

    let current_exe =
        std::env::current_exe().context("Failed to determine current executable path")?;

    for client in &config.clients {
        if !config.reinstall && !config.add && is_setup(*client)? {
            if !config.yes {
                let proceed = Confirm::new(&format!(
                    "{} is already set up. Reinstall?",
                    client.as_str()
                ))
                .with_default(false)
                .prompt()?;

                if !proceed {
                    println!("Skipping {} setup", client.as_str());
                    continue;
                }
            } else {
                println!(
                    "{} is already set up, skipping (use --reinstall to override)",
                    client.as_str()
                );
                continue;
            }
        }

        println!("\nSetting up skrills for {}...", client.as_str());

        match client {
            Client::Claude => setup_claude(&config.bin_dir, &current_exe)?,
            Client::Codex => setup_codex(&config.bin_dir, &current_exe)?,
            Client::Copilot => setup_copilot(&config.bin_dir, &current_exe)?,
            Client::Cursor => setup_cursor(&config.bin_dir, &current_exe)?,
        }

        println!("{} setup complete!", client.as_str());
    }

    // Perform universal sync if requested
    if config.universal {
        sync_universal(&config)?;
    }

    println!("\nSetup complete!");
    print_next_steps(&config)?;

    Ok(())
}

/// What a `claude` CLI invocation reported.
pub(crate) struct ClaudeCliOutcome {
    success: bool,
    stderr: String,
}

/// Runs the `claude` CLI. Injected so tests never touch the developer's real
/// Claude configuration.
type ClaudeCli<'a> = &'a dyn Fn(&[&OsStr]) -> std::io::Result<ClaudeCliOutcome>;

fn run_claude_cli(args: &[&OsStr]) -> std::io::Result<ClaudeCliOutcome> {
    let output = Command::new("claude").args(args).output()?;
    Ok(ClaudeCliOutcome {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// Path of the installed `skrills` binary inside `bin_dir`, with the
/// platform's executable suffix (`.exe` on Windows).
pub(crate) fn installed_binary_path(bin_dir: &Path) -> PathBuf {
    bin_dir.join(format!("skrills{}", std::env::consts::EXE_SUFFIX))
}

/// Copies the running binary into `bin_dir` (and `~/.cargo/bin`) and returns
/// the installed path.
fn install_binary(bin_dir: &Path, current_exe: &Path) -> Result<PathBuf> {
    fs::create_dir_all(bin_dir)
        .context(format!("Failed to create directory: {}", bin_dir.display()))?;

    let target_bin = installed_binary_path(bin_dir);
    if target_bin != current_exe {
        fs::copy(current_exe, &target_bin)
            .context(format!("Failed to copy binary to {}", target_bin.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target_bin)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_bin, perms)?;
        }

        println!("  Installed binary to {}", target_bin.display());
    }

    copy_to_cargo_bin(&target_bin)?;
    Ok(target_bin)
}

/// Sets up Claude Code integration.
fn setup_claude(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let target_bin = install_binary(bin_dir, current_exe)?;
    register_claude_mcp(&home.join(".claude"), &target_bin)
}

/// Registers `skrills` MCP server with Claude Code.
fn register_claude_mcp(base_dir: &Path, bin_path: &Path) -> Result<()> {
    register_claude_mcp_with(base_dir, bin_path, &run_claude_cli)
}

fn register_claude_mcp_with(base_dir: &Path, bin_path: &Path, claude: ClaudeCli) -> Result<()> {
    // User scope, so the registration lands in ~/.claude.json where
    // `is_setup` looks for it. The CLI default (local scope) registers the
    // server for the current directory only.
    let args: [&OsStr; 9] = [
        "mcp".as_ref(),
        "add".as_ref(),
        "--scope".as_ref(),
        "user".as_ref(),
        "--transport".as_ref(),
        "stdio".as_ref(),
        "skrills".as_ref(),
        "--".as_ref(),
        bin_path.as_os_str(),
    ];
    let mut with_serve: Vec<&OsStr> = args.to_vec();
    with_serve.push("serve".as_ref());
    match claude(&with_serve) {
        Ok(outcome) if outcome.success => {
            println!("  Registered MCP server with 'claude mcp add --scope user'");
            return Ok(());
        }
        Ok(outcome) => {
            println!(
                "  'claude mcp add' failed ({}), manually updating .mcp.json",
                if outcome.stderr.is_empty() {
                    "no error output"
                } else {
                    outcome.stderr.as_str()
                }
            );
        }
        Err(e) => {
            println!("  'claude' command not available ({e}), manually updating .mcp.json");
        }
    }

    let mcp_path = base_dir.join(".mcp.json");
    let mut mcp_config = read_mcp_json(&mcp_path)?;
    add_mcp_server_entry(&mut mcp_config, bin_path)?;
    write_mcp_json(&mcp_path, &mcp_config)?;
    println!("  Updated {}", mcp_path.display());

    Ok(())
}

/// Reads a client's MCP JSON config as an object.
///
/// A missing file is an empty object. A file that does not parse, or whose
/// root is not an object, is an error: rewriting it would delete every other
/// server the user registered.
pub(crate) fn read_mcp_json(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "{} is not valid JSON; fix or move it, then rerun setup (it was left unchanged)",
            path.display()
        )
    })?;
    match value {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(anyhow!(
            "{} does not hold a JSON object; fix or move it, then rerun setup (it was left unchanged)",
            path.display()
        )),
    }
}

fn write_mcp_json(path: &Path, config: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(config)?)
        .with_context(|| format!("Failed to write {}", path.display()))
}

/// Inserts (or replaces) the `skrills` entry under `mcpServers`.
pub(crate) fn add_mcp_server_entry(
    config: &mut serde_json::Map<String, serde_json::Value>,
    bin_path: &Path,
) -> Result<()> {
    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`mcpServers` is not a JSON object"))?;
    servers.insert(
        "skrills".to_string(),
        serde_json::json!({
            "type": "stdio",
            "command": bin_path.display().to_string(),
            "args": ["serve"]
        }),
    );
    Ok(())
}

/// Whether `mcpServers.skrills` exists in a parsed MCP config.
fn has_skrills_entry(config: &serde_json::Map<String, serde_json::Value>) -> bool {
    config
        .get("mcpServers")
        .and_then(|servers| servers.get("skrills"))
        .is_some()
}

/// Registers in a JSON MCP config unless already registered.
fn register_json_mcp(mcp_path: &Path, skrills_bin: &Path) -> Result<()> {
    let mut mcp_config = read_mcp_json(mcp_path)?;
    if has_skrills_entry(&mcp_config) {
        println!("  MCP server already registered in {}", mcp_path.display());
        return Ok(());
    }
    add_mcp_server_entry(&mut mcp_config, skrills_bin)?;
    write_mcp_json(mcp_path, &mcp_config)?;
    println!("  Registered MCP server in {}", mcp_path.display());
    Ok(())
}

/// Removes `mcpServers.skrills` from a JSON MCP config, dropping an emptied
/// `mcpServers` key and deleting the file when nothing else is left.
fn unregister_json_mcp(mcp_path: &Path) -> Result<()> {
    if !mcp_path.exists() {
        return Ok(());
    }
    let mut mcp_config = read_mcp_json(mcp_path)?;
    let Some(servers) = mcp_config
        .get_mut("mcpServers")
        .and_then(|s| s.as_object_mut())
    else {
        return Ok(());
    };
    if servers.remove("skrills").is_none() {
        return Ok(());
    }
    if servers.is_empty() {
        mcp_config.remove("mcpServers");
    }
    if mcp_config.is_empty() {
        fs::remove_file(mcp_path)?;
        println!("  Removed {}", mcp_path.display());
    } else {
        write_mcp_json(mcp_path, &mcp_config)?;
        println!("  Removed MCP registration from {}", mcp_path.display());
    }
    Ok(())
}

/// Sets up Codex integration.
fn setup_codex(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".codex");
    fs::create_dir_all(&base_dir).context("Failed to create .codex directory")?;
    let target_bin = install_binary(bin_dir, current_exe)?;
    register_codex_mcp(&base_dir, &target_bin)
}

/// Header of the table setup writes into Codex's config.toml.
const CODEX_MCP_HEADER: &str = "[mcp_servers.skrills]";

/// Comment setup writes directly above [`CODEX_MCP_HEADER`].
const CODEX_MCP_COMMENT: &str = "# Skrills MCP server for skill management";

/// Whether a config.toml line is a table or array-of-tables header.
fn is_toml_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[')
        && trimmed
            .split('#')
            .next()
            .is_some_and(|code| code.trim_end().ends_with(']'))
}

/// Renders the `[mcp_servers.skrills]` table. The path goes through the TOML
/// serializer so backslashes and quotes in it are escaped.
fn codex_mcp_entry(skrills_bin: &Path) -> String {
    let command = toml::Value::String(skrills_bin.display().to_string());
    format!(
        "\n{CODEX_MCP_COMMENT}\n{CODEX_MCP_HEADER}\ncommand = {command}\ntype = \"stdio\"\nargs = [\"serve\"]\n"
    )
}

/// Registers `skrills` MCP server in Codex's config.toml.
fn register_codex_mcp(base_dir: &Path, skrills_bin: &Path) -> Result<()> {
    let config_path = base_dir.join("config.toml");

    // Read existing config or create new
    let mut content = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    // Check if skrills MCP is already registered
    if content.contains(CODEX_MCP_HEADER) {
        println!("  MCP server already registered in config.toml");
        // Still ensure Codex skills feature flag is enabled.
        ensure_codex_skills_feature_enabled(&config_path)?;
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&codex_mcp_entry(skrills_bin));

    fs::write(&config_path, content)?;
    println!("  Registered MCP server in {}", config_path.display());

    ensure_codex_skills_feature_enabled(&config_path)?;

    Ok(())
}

/// Removes the `[mcp_servers.skrills]` table (with its sub-tables and the
/// comment setup writes above it) from a config.toml body.
///
/// Returns `None` when the table is absent.
fn remove_codex_mcp_table(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.iter().position(|l| l.trim() == CODEX_MCP_HEADER)?;

    let in_skrills_table = |line: &str| {
        let t = line.trim();
        t == CODEX_MCP_HEADER || t.starts_with("[mcp_servers.skrills.")
    };
    let mut end = start + 1;
    while end < lines.len() && (!is_toml_header(lines[end]) || in_skrills_table(lines[end])) {
        end += 1;
    }

    let mut cut_from = start;
    if cut_from > 0 && lines[cut_from - 1].trim() == CODEX_MCP_COMMENT {
        cut_from -= 1;
    }
    while cut_from > 0 && lines[cut_from - 1].trim().is_empty() {
        cut_from -= 1;
    }

    let mut kept: Vec<&str> = lines[..cut_from].to_vec();
    let rest = &lines[end..];
    let rest_start = rest.iter().position(|l| !l.trim().is_empty());
    if let Some(offset) = rest_start {
        if !kept.is_empty() {
            kept.push("");
        }
        kept.extend_from_slice(&rest[offset..]);
    }
    let body = kept.join("\n");
    let body = body.trim_end();
    Some(if body.is_empty() {
        String::new()
    } else {
        format!("{body}\n")
    })
}

/// Sets up GitHub Copilot integration.
fn setup_copilot(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let base_dir = Client::Copilot.base_dir()?;
    fs::create_dir_all(&base_dir).context("Failed to create copilot directory")?;
    let target_bin = install_binary(bin_dir, current_exe)?;

    // Register MCP server in mcp_servers.json
    register_copilot_mcp(&base_dir, &target_bin)?;

    // Create skills directory if needed
    let skills_dir = base_dir.join("skills");
    if !skills_dir.exists() {
        fs::create_dir_all(&skills_dir)?;
        println!("  Created skills directory: {}", skills_dir.display());
    }

    Ok(())
}

/// Sets up Cursor IDE integration.
fn setup_cursor(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let base_dir = Client::Cursor.base_dir()?;
    fs::create_dir_all(&base_dir).context("Failed to create .cursor directory")?;
    let target_bin = install_binary(bin_dir, current_exe)?;
    register_cursor_mcp(&base_dir, &target_bin)
}

/// Registers `skrills` MCP server in Cursor's mcp.json.
fn register_cursor_mcp(base_dir: &Path, skrills_bin: &Path) -> Result<()> {
    register_json_mcp(&base_dir.join("mcp.json"), skrills_bin)
}

/// Registers `skrills` MCP server in Copilot's mcp_servers.json.
fn register_copilot_mcp(base_dir: &Path, skrills_bin: &Path) -> Result<()> {
    register_json_mcp(&base_dir.join("mcp_servers.json"), skrills_bin)
}

/// Whether the `[features]` table already sets `skills = true`, in any of the
/// forms TOML allows (table, dotted key or inline table).
fn codex_skills_feature_on(table: &toml::Table) -> bool {
    table
        .get("features")
        .and_then(|f| f.get("skills"))
        .and_then(|v| v.as_bool())
        == Some(true)
}

/// Whether a `[features]`-table line assigns the `skills` key itself, not a
/// key that merely starts with `skills` such as `skills_dir`.
fn is_skills_key_line(line: &str) -> bool {
    let Some((key, _)) = line.split_once('=') else {
        return false;
    };
    let key = key.trim().trim_matches('"').trim_matches('\'');
    key == "skills"
}

/// Ensure the experimental Codex skills feature flag is enabled in `config.toml`.
///
/// Codex loads skills only when `[features] skills = true` is set. The file is
/// left untouched when the flag is already on, and an edit that would leave a
/// file Codex cannot parse is refused with an error instead of written.
pub fn ensure_codex_skills_feature_enabled(config_path: &Path) -> Result<()> {
    let content = if config_path.exists() {
        fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let original: Option<toml::Table> = toml::from_str(&content).ok();
    if original.as_ref().is_some_and(codex_skills_feature_on) {
        return Ok(());
    }

    let input_lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(input_lines.len() + 4);
    let mut found_features = false;

    let mut i = 0usize;
    while i < input_lines.len() {
        let line = input_lines[i];
        if line.trim() == "[features]" {
            found_features = true;
            out.push(line.to_string());
            i += 1;

            let mut saw_skills = false;
            while i < input_lines.len() && !is_toml_header(input_lines[i]) {
                let cur = input_lines[i];
                if is_skills_key_line(cur) {
                    saw_skills = true;
                    out.push("skills = true".to_string());
                } else {
                    out.push(cur.to_string());
                }
                i += 1;
            }

            if !saw_skills {
                // Keep the flag next to the header rather than after any
                // trailing blank lines that separate the next table.
                let insert_at = out
                    .iter()
                    .rposition(|l| !l.trim().is_empty())
                    .map_or(out.len(), |idx| idx + 1);
                out.insert(insert_at, "skills = true".to_string());
            }
            continue;
        }

        out.push(line.to_string());
        i += 1;
    }

    if !found_features {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[features]".to_string());
        out.push("skills = true".to_string());
    }

    let updated = out.join("\n") + "\n";
    let parsed: toml::Table = toml::from_str(&updated).map_err(|e| {
        anyhow!(
            "could not enable `[features] skills = true` in {} without breaking it ({e}); \
             add it by hand",
            config_path.display()
        )
    })?;
    if !codex_skills_feature_on(&parsed) {
        return Err(anyhow!(
            "could not enable `[features] skills = true` in {}; add it by hand",
            config_path.display()
        ));
    }

    fs::write(config_path, updated)?;
    // Logged rather than printed: this also runs under the MCP stdio
    // transport, where anything on stdout corrupts the JSON-RPC stream.
    tracing::debug!(
        target: "skrills::setup",
        path = %config_path.display(),
        "Enabled Codex experimental skills feature"
    );
    Ok(())
}

/// Copies binary to ~/.cargo/bin for consistency with cargo install.
fn copy_to_cargo_bin(source_bin: &Path) -> Result<()> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Ok(()), // Can't determine home, skip silently
    };

    let cargo_bin_dir = home.join(".cargo").join("bin");
    let cargo_bin = installed_binary_path(&cargo_bin_dir);

    // Skip if already the same path or cargo bin dir doesn't exist
    if source_bin == cargo_bin || !cargo_bin_dir.exists() {
        return Ok(());
    }

    // Try to copy, but don't fail if it doesn't work
    match fs::copy(source_bin, &cargo_bin) {
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(mut perms) = fs::metadata(&cargo_bin).map(|m| m.permissions()) {
                    perms.set_mode(0o755);
                    let _ = fs::set_permissions(&cargo_bin, perms);
                }
            }
            println!("  Also installed binary to {}", cargo_bin.display());
        }
        Err(e) => {
            // Not fatal (e.g. the file is in use), but say so.
            println!(
                "  Note: could not update {} ({e}); it may be out of date",
                cargo_bin.display()
            );
        }
    }
    Ok(())
}

/// Syncs skills to universal ~/.agent/skills directory.
///
/// Runs the same in-process skill mirror as `skrills sync`, so only
/// `SKILL.md` trees (minus hidden files and symlinks) are copied, never the
/// rest of the mirror source such as credentials or session transcripts.
fn sync_universal(config: &SetupConfig) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let agent_skills = home.join(".agent/skills");

    // Determine mirror source (default: ~/.claude)
    let mirror_source = config
        .mirror_source
        .clone()
        .unwrap_or_else(|| home.join(".claude"));

    // Prevent mirroring if source doesn't exist
    if !mirror_source.exists() {
        println!(
            "  Warning: Mirror source {} does not exist, skipping universal sync",
            mirror_source.display()
        );
        return Ok(());
    }

    println!(
        "\nSyncing skills to universal directory: {}",
        agent_skills.display()
    );
    println!("Source: {}", mirror_source.display());

    fs::create_dir_all(&agent_skills)?;
    let report = crate::sync::sync_skills_only_from_claude(&mirror_source, &agent_skills, false)?;
    println!(
        "  Sync complete: {} copied, {} unchanged",
        report.copied, report.skipped
    );

    Ok(())
}

/// Uninstalls `skrills` configuration.
fn run_uninstall(config: &SetupConfig) -> Result<()> {
    let clients_to_uninstall = if config.clients.is_empty() {
        // Detect which clients are set up
        let mut clients = Vec::new();
        if is_setup(Client::Claude)? {
            clients.push(Client::Claude);
        }
        if is_setup(Client::Codex)? {
            clients.push(Client::Codex);
        }
        if is_setup(Client::Copilot)? {
            clients.push(Client::Copilot);
        }
        if is_setup(Client::Cursor)? {
            clients.push(Client::Cursor);
        }

        if clients.is_empty() {
            println!("No skrills configuration found to uninstall.");
            return Ok(());
        }

        clients
    } else {
        config.clients.clone()
    };

    for client in clients_to_uninstall {
        if !config.yes {
            let proceed = Confirm::new(&format!("Uninstall {} configuration?", client.as_str()))
                .with_default(false)
                .prompt()?;

            if !proceed {
                println!("Skipping {} uninstall", client.as_str());
                continue;
            }
        }

        println!("\nUninstalling {} configuration...", client.as_str());

        match client {
            Client::Claude => uninstall_claude()?,
            Client::Codex => uninstall_codex()?,
            Client::Copilot => uninstall_copilot()?,
            Client::Cursor => uninstall_cursor()?,
        }

        println!("{} uninstalled", client.as_str());
    }

    println!("\nUninstall complete!");
    println!("Note: The skrills binary was not removed. To remove it:");
    println!("  rm $(which skrills)");
    println!("\nTo also remove universal skills:");
    println!("  rm -rf ~/.agent/skills");

    Ok(())
}

/// Uninstalls Claude Code configuration.
fn uninstall_claude() -> Result<()> {
    uninstall_claude_with(&run_claude_cli)
}

fn uninstall_claude_with(claude: ClaudeCli) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".claude");

    // Remove the hook older releases installed.
    let hook_path = base_dir.join("hooks/prompt.on_user_prompt_submit");
    if hook_path.exists() {
        fs::remove_file(&hook_path)?;
        println!("  Removed hook: {}", hook_path.display());
    }

    // Remove the user-scope registration `claude mcp add --scope user` made.
    if claude_json_has_skrills(&home) {
        let args: [&OsStr; 5] = [
            "mcp".as_ref(),
            "remove".as_ref(),
            "--scope".as_ref(),
            "user".as_ref(),
            "skrills".as_ref(),
        ];
        match claude(&args) {
            Ok(outcome) if outcome.success => {
                println!("  Removed MCP registration with 'claude mcp remove --scope user'");
            }
            Ok(outcome) => println!(
                "  Warning: 'claude mcp remove' failed ({}); run `claude mcp remove --scope user skrills`",
                outcome.stderr
            ),
            Err(e) => println!(
                "  Warning: 'claude' command not available ({e}); run `claude mcp remove --scope user skrills`"
            ),
        }
    }

    // Remove the fallback registration in ~/.claude/.mcp.json.
    unregister_json_mcp(&base_dir.join(".mcp.json"))
}

/// Whether `~/.claude.json` holds a user-scope `skrills` MCP registration.
fn claude_json_has_skrills(home: &Path) -> bool {
    fs::read_to_string(home.join(".claude.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .is_some_and(|value| {
            value
                .get("mcpServers")
                .and_then(|servers| servers.get("skrills"))
                .is_some()
        })
}

/// Removes the text between `start_marker` and the first `end_marker` after
/// it, markers included. Returns `None` when the pair is absent or the end
/// marker does not follow the start.
pub(crate) fn remove_marked_section(
    content: &str,
    start_marker: &str,
    end_marker: &str,
) -> Option<String> {
    let start = content.find(start_marker)?;
    let end = content[start..].find(end_marker)? + start + end_marker.len();
    let before = content[..start].trim_end();
    let after = content[end..].trim_start();
    Some(match (before.is_empty(), after.is_empty()) {
        (true, _) => after.to_string(),
        (false, true) => before.to_string(),
        (false, false) => format!("{before}\n\n{after}"),
    })
}

/// Uninstalls Codex configuration.
fn uninstall_codex() -> Result<()> {
    use crate::discovery::{
        AGENTS_AGENT_SECTION_END, AGENTS_AGENT_SECTION_START, AGENTS_SECTION_END,
        AGENTS_SECTION_START,
    };

    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".codex");

    // Remove the sections `skrills sync-agents` writes into AGENTS.md, plus
    // the marker pair older releases used.
    let agents_path = base_dir.join("AGENTS.md");
    if let Ok(content) = fs::read_to_string(&agents_path) {
        let mut updated = content.clone();
        for (start, end) in [
            (AGENTS_SECTION_START, AGENTS_SECTION_END),
            (AGENTS_AGENT_SECTION_START, AGENTS_AGENT_SECTION_END),
            (
                "<!-- skrills-integration-start -->",
                "<!-- skrills-integration-end -->",
            ),
        ] {
            if let Some(next) = remove_marked_section(&updated, start, end) {
                updated = next;
            }
        }
        if updated != content {
            if updated.trim().is_empty() {
                fs::remove_file(&agents_path)?;
                println!("  Removed AGENTS.md: {}", agents_path.display());
            } else {
                fs::write(&agents_path, format!("{}\n", updated.trim_end()))?;
                println!(
                    "  Removed skrills sections from AGENTS.md: {}",
                    agents_path.display()
                );
            }
        }
    }

    // Remove MCP server registration from config.toml
    let config_path = base_dir.join("config.toml");
    if let Ok(content) = fs::read_to_string(&config_path) {
        if let Some(new_content) = remove_codex_mcp_table(&content) {
            if toml::from_str::<toml::Table>(&content).is_ok()
                && toml::from_str::<toml::Table>(&new_content).is_err()
            {
                return Err(anyhow!(
                    "removing [mcp_servers.skrills] would leave {} unparseable; remove it by hand",
                    config_path.display()
                ));
            }
            if new_content.trim().is_empty() {
                fs::remove_file(&config_path)?;
                println!("  Removed config.toml: {}", config_path.display());
            } else {
                fs::write(&config_path, new_content)?;
                println!(
                    "  Removed skrills MCP server from config.toml: {}",
                    config_path.display()
                );
            }
        }
    }

    Ok(())
}

/// Uninstalls Copilot configuration.
fn uninstall_copilot() -> Result<()> {
    unregister_json_mcp(&Client::Copilot.base_dir()?.join("mcp_servers.json"))
}

/// Uninstalls Cursor configuration.
fn uninstall_cursor() -> Result<()> {
    unregister_json_mcp(&Client::Cursor.base_dir()?.join("mcp.json"))
}

/// Prints next steps after setup.
fn print_next_steps(config: &SetupConfig) -> Result<()> {
    println!("\nNext steps:");

    for client in &config.clients {
        match client {
            Client::Claude => {
                println!("\n  Claude Code:");
                println!("    - Restart Claude Code to load the MCP server");
                println!("    - Use 'skrills sync' to sync skills to Codex");
                println!("    - Use 'skrills validate' to check skill structure");
            }
            Client::Codex => {
                println!("\n  Codex:");
                println!("    - MCP server registered in ~/.codex/config.toml");
                println!("    - Use 'skrills sync' to sync skills from Claude");
                println!("    - Use 'skrills validate' to check skill structure");
            }
            Client::Copilot => {
                println!("\n  GitHub Copilot:");
                println!("    - MCP server registered in mcp_servers.json");
                println!("    - Use 'skrills sync' to sync skills from Claude");
                println!("    - Use 'skrills validate --target copilot' to check structure");
            }
            Client::Cursor => {
                println!("\n  Cursor:");
                println!("    - MCP server registered in mcp.json");
                println!("    - Use 'skrills sync' to sync skills from Claude");
                println!("    - Use 'skrills validate' to check structure");
            }
        }
    }

    if config.universal {
        println!("\n  Universal Skills:");
        println!("    - Skills synced to ~/.agent/skills");
        println!("    - Run 'skrills sync' to update mirrored skills");
    }

    Ok(())
}

#[cfg(test)]
mod tests;
