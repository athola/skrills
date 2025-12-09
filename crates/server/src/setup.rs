//! Setup and installation logic.
//!
//! Handles first-run detection, interactive setup, reinstallation, and uninstallation
//! for Claude Code and Codex clients, and supports a universal `~/.agent/skills` directory.

use anyhow::{anyhow, Context, Result};
use inquire::{Confirm, Select, Text};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Supported client types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Claude,
    Codex,
}

impl Client {
    /// Returns the default base directory for this client.
    pub fn base_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
        Ok(match self {
            Client::Claude => home.join(".claude"),
            Client::Codex => home.join(".codex"),
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
        }
    }

    /// Parses from string.
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Client::Claude),
            "codex" => Ok(Client::Codex),
            _ => Err(anyhow!(
                "Invalid client: {}. Must be 'claude' or 'codex'",
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
pub fn is_setup(client: Client) -> Result<bool> {
    let base_dir = client.base_dir()?;

    match client {
        Client::Claude => {
            // Check for MCP registration in .mcp.json
            let mcp_path = base_dir.join(".mcp.json");
            if mcp_path.exists() {
                if let Ok(content) = fs::read_to_string(&mcp_path) {
                    if content.contains("\"skrills\"") {
                        return Ok(true);
                    }
                }
            }

            // Check for hook
            let hook_path = base_dir.join("hooks/prompt.on_user_prompt_submit");
            if hook_path.exists() {
                if let Ok(content) = fs::read_to_string(&hook_path) {
                    if content.contains("skrills") {
                        return Ok(true);
                    }
                }
            }

            Ok(false)
        }
        Client::Codex => {
            // Check for AGENTS.md with skrills integration
            let agents_path = base_dir.join("AGENTS.md");
            if agents_path.exists() {
                if let Ok(content) = fs::read_to_string(&agents_path) {
                    if content.contains("<!-- skrills-integration-start -->") {
                        return Ok(true);
                    }
                }
            }

            // Also check for MCP server registration in config.toml
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
    }
}

/// Detects if this is the first run (no setup for any client).
pub fn is_first_run() -> Result<bool> {
    let claude_setup = is_setup(Client::Claude).unwrap_or(false);
    let codex_setup = is_setup(Client::Codex).unwrap_or(false);

    Ok(!claude_setup && !codex_setup)
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
        _ => Ok(vec![Client::from_str(s)?]),
    }
}

/// Prompts user to select clients.
fn prompt_clients(add_mode: bool) -> Result<Vec<Client>> {
    if add_mode {
        // In add mode, show only clients not already set up
        let claude_setup = is_setup(Client::Claude).unwrap_or(false);
        let codex_setup = is_setup(Client::Codex).unwrap_or(false);

        let mut options = Vec::new();
        if !claude_setup {
            options.push("Claude Code");
        }
        if !codex_setup {
            options.push("Codex");
        }

        if options.is_empty() {
            return Err(anyhow!(
                "Both Claude Code and Codex are already set up. Use --reinstall to reconfigure."
            ));
        }

        if options.len() == 1 {
            println!(
                "Setting up for {} (the only client not yet configured)",
                options[0]
            );
            return Ok(vec![if options[0] == "Claude Code" {
                Client::Claude
            } else {
                Client::Codex
            }]);
        }

        let selection =
            Select::new("Which client would you like to add?", options.clone()).prompt()?;

        Ok(vec![if selection == "Claude Code" {
            Client::Claude
        } else {
            Client::Codex
        }])
    } else {
        let options = vec!["Claude Code", "Codex", "Both"];
        let selection =
            Select::new("Which client would you like to set up?", options.clone()).prompt()?;

        match selection {
            "Claude Code" => Ok(vec![Client::Claude]),
            "Codex" => Ok(vec![Client::Codex]),
            "Both" => Ok(vec![Client::Claude, Client::Codex]),
            _ => unreachable!(),
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

/// Sets up Claude Code integration.
fn setup_claude(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".claude");

    // Create directories
    fs::create_dir_all(bin_dir)
        .context(format!("Failed to create directory: {}", bin_dir.display()))?;
    fs::create_dir_all(base_dir.join("hooks")).context("Failed to create hooks directory")?;

    // Copy/link binary to bin_dir
    let target_bin = bin_dir.join("skrills");
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

    // Also copy to ~/.cargo/bin for consistency
    copy_to_cargo_bin(&target_bin)?;

    // Create hook
    create_claude_hook(&base_dir, &target_bin)?;

    // Register MCP server
    register_claude_mcp(&base_dir, &target_bin)?;

    Ok(())
}

/// Creates Claude Code prompt hook.
fn create_claude_hook(base_dir: &Path, bin_path: &Path) -> Result<()> {
    let hook_path = base_dir.join("hooks/prompt.on_user_prompt_submit");

    let hook_content = format!(
        r#"#!/usr/bin/env bash
# Inject SKILL.md content into Claude Code on prompt submit via skrills
set -euo pipefail

BIN="{bin}"
CMD_ARGS=(emit-autoload)

# Optionally capture prompt text from stdin
PROMPT_INPUT=""
if [ ! -t 0 ]; then
  if IFS= read -r -t 0.05 first_line; then
    rest=$(cat)
    PROMPT_INPUT="${{first_line}}${{rest}}"
  fi
fi

if [ -n "${{SKRILLS_PROMPT:-}}" ]; then
  PROMPT_INPUT="$SKRILLS_PROMPT"
fi

if [ -n "$PROMPT_INPUT" ]; then
  CMD_ARGS+=(--prompt "$PROMPT_INPUT")
fi

if [ -x "$BIN" ]; then
  "$BIN" "${{CMD_ARGS[@]}}"
else
  echo "{{}}" && exit 0
fi
"#,
        bin = bin_path.display()
    );

    fs::write(&hook_path, hook_content)
        .context(format!("Failed to write hook to {}", hook_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    println!("  Created hook at {}", hook_path.display());
    Ok(())
}

/// Registers `skrills` MCP server with Claude Code.
fn register_claude_mcp(base_dir: &Path, bin_path: &Path) -> Result<()> {
    // Try using `claude mcp add` command first
    if let Ok(output) = Command::new("claude")
        .args(["mcp", "add", "--transport", "stdio", "skrills", "--"])
        .arg(bin_path)
        .arg("serve")
        .output()
    {
        if output.status.success() {
            println!("  Registered MCP server with 'claude mcp add'");
            return Ok(());
        }
    }

    // Fallback: manually edit .mcp.json
    println!("  'claude' command not available, manually updating .mcp.json");

    let mcp_path = base_dir.join(".mcp.json");
    let mut mcp_config: serde_json::Value = if mcp_path.exists() {
        let content = fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Add skrills server
    if let Some(servers) = mcp_config.get_mut("mcpServers") {
        if let Some(obj) = servers.as_object_mut() {
            obj.insert(
                "skrills".to_string(),
                serde_json::json!({
                    "type": "stdio",
                    "command": bin_path.display().to_string(),
                    "args": ["serve"]
                }),
            );
        }
    } else {
        mcp_config["mcpServers"] = serde_json::json!({
            "skrills": {
                "type": "stdio",
                "command": bin_path.display().to_string(),
                "args": ["serve"]
            }
        });
    }

    fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_config)?)?;
    println!("  Updated {}", mcp_path.display());

    Ok(())
}

/// Sets up Codex integration.
fn setup_codex(bin_dir: &Path, current_exe: &Path) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".codex");

    // Create directories
    fs::create_dir_all(bin_dir)
        .context(format!("Failed to create directory: {}", bin_dir.display()))?;
    fs::create_dir_all(&base_dir).context("Failed to create .codex directory")?;

    // Copy/link binary
    let target_bin = bin_dir.join("skrills");
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

    // Also copy to ~/.cargo/bin for consistency
    copy_to_cargo_bin(&target_bin)?;

    // Install AGENTS.md with skill-loading instructions
    install_codex_agents_md(&base_dir)?;

    // Register MCP server in config.toml
    register_codex_mcp(&base_dir, &target_bin)?;

    Ok(())
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
    if content.contains("[mcp_servers.skrills]") {
        println!("  MCP server already registered in config.toml");
        return Ok(());
    }

    // Build the MCP server entry
    let bin_path = skrills_bin.display();
    let mcp_entry = format!(
        r#"
# Skrills MCP server for skill management
[mcp_servers.skrills]
command = "{bin_path}"
args = ["serve"]
"#
    );

    // Append to config
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&mcp_entry);

    // Write config
    fs::write(&config_path, content)?;
    println!("  Registered MCP server in {}", config_path.display());

    Ok(())
}

/// Copies binary to ~/.cargo/bin for consistency with cargo install.
fn copy_to_cargo_bin(source_bin: &Path) -> Result<()> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Ok(()), // Can't determine home, skip silently
    };

    let cargo_bin_dir = home.join(".cargo").join("bin");
    let cargo_bin = cargo_bin_dir.join("skrills");

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
        Err(_) => {
            // Silently skip if copy fails (e.g., file in use)
        }
    }
    Ok(())
}

/// Marker to detect if AGENTS.md already has `skrills` content.
const SKRILLS_AGENTS_MARKER: &str = "<!-- skrills-integration-start -->";

/// Template content for AGENTS.md skill-loading instructions
const SKRILLS_AGENTS_TEMPLATE: &str = r#"
<!-- skrills-integration-start -->
# Skrills Autoload Protocol

## MANDATORY: Skill Loading Protocol

Before responding to ANY user request, you MUST first call the `autoload-snippet` MCP tool to load relevant skills to complete the task.

### Required Workflow

1. **User sends a message**
2. **Immediately call `autoload-snippet`** with the user's message as the `prompt` parameter
3. **Read the returned skill content** - these are specialized instructions and patterns
4. **Use the loaded skills** to inform your response to the user

### Example Tool Call

```json
{
  "name": "autoload-snippet",
  "arguments": {
    "prompt": "<paste the user's full message here>"
  }
}
```

### Why This Matters

- Skills contain domain-specific best practices, patterns, and instructions
- They are dynamically matched to your current task
- Skipping skill loading means missing critical context required to perform your task correctly.
- The skill content is working instructions, not just reference material

### When to Load Skills

- At the START of every new user message
- When the user's task changes significantly within a conversation
- After any context reset

DO NOT skip this step. The skill content is essential for providing high-quality, contextually-appropriate responses.
<!-- skrills-integration-end -->
"#;

/// Installs or appends AGENTS.md with skill-loading instructions for Codex.
fn install_codex_agents_md(base_dir: &Path) -> Result<()> {
    let agents_path = base_dir.join("AGENTS.md");

    // Read existing content if file exists
    let existing_content = if agents_path.exists() {
        fs::read_to_string(&agents_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Check if skrills content is already present
    if existing_content.contains(SKRILLS_AGENTS_MARKER) {
        println!("  AGENTS.md already contains skrills integration");
        return Ok(());
    }

    // Append the skrills content
    let new_content = if existing_content.is_empty() {
        SKRILLS_AGENTS_TEMPLATE.trim_start().to_string()
    } else {
        // Ensure there's a blank line before appending
        let separator = if existing_content.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        format!(
            "{}{}{}",
            existing_content,
            separator,
            SKRILLS_AGENTS_TEMPLATE.trim()
        )
    };

    fs::write(&agents_path, new_content).context(format!(
        "Failed to write AGENTS.md to {}",
        agents_path.display()
    ))?;

    println!("  Updated AGENTS.md at {}", agents_path.display());
    Ok(())
}

/// Syncs skills to universal ~/.agent/skills directory.
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

    // Use the existing sync command if available
    let skrills_bin = config.bin_dir.join("skrills");
    if skrills_bin.exists() {
        println!("  Running skrills sync...");
        match Command::new(&skrills_bin)
            .arg("sync")
            .env("SKRILLS_MIRROR_SOURCE", &mirror_source)
            .output()
        {
            Ok(output) if output.status.success() => {
                println!("  Sync complete");
            }
            Ok(output) => {
                println!(
                    "  Warning: skrills sync failed (exit code {:?}), using fallback copy",
                    output.status.code()
                );
                fallback_copy_tree(&mirror_source, &agent_skills)?;
            }
            Err(e) => {
                println!("  Warning: skrills sync error ({}), using fallback copy", e);
                fallback_copy_tree(&mirror_source, &agent_skills)?;
            }
        }
    } else {
        // Fallback: manual copy
        fallback_copy_tree(&mirror_source, &agent_skills)?;
    }

    Ok(())
}

/// Fallback: copies skills tree using Rust fs operations.
fn fallback_copy_tree(src: &Path, dest: &Path) -> Result<()> {
    use std::fs;

    fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
        if !dest.exists() {
            fs::create_dir_all(dest)?;
        }

        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                fs::copy(&src_path, &dest_path)?;
            }
        }

        Ok(())
    }

    copy_dir_recursive(src, dest)?;
    println!("  Copied skills using fallback copy");
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
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".claude");

    // Remove hook
    let hook_path = base_dir.join("hooks/prompt.on_user_prompt_submit");
    if hook_path.exists() {
        fs::remove_file(&hook_path)?;
        println!("  Removed hook: {}", hook_path.display());
    }

    // Remove MCP registration
    let mcp_path = base_dir.join(".mcp.json");
    if mcp_path.exists() {
        let content = fs::read_to_string(&mcp_path)?;
        if let Ok(mut mcp_config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = mcp_config.get_mut("mcpServers") {
                if let Some(obj) = servers.as_object_mut() {
                    obj.remove("skrills");
                    fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_config)?)?;
                    println!("  Removed MCP registration from {}", mcp_path.display());
                }
            }
        }
    }

    Ok(())
}

/// Uninstalls Codex configuration.
fn uninstall_codex() -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot determine home directory"))?;
    let base_dir = home.join(".codex");

    // Remove skrills integration from AGENTS.md
    let agents_path = base_dir.join("AGENTS.md");
    if agents_path.exists() {
        if let Ok(content) = fs::read_to_string(&agents_path) {
            if content.contains("<!-- skrills-integration-start -->") {
                // Remove the skrills section from AGENTS.md
                let start_marker = "<!-- skrills-integration-start -->";
                let end_marker = "<!-- skrills-integration-end -->";
                if let Some(start) = content.find(start_marker) {
                    if let Some(end) = content.find(end_marker) {
                        let end_pos = end + end_marker.len();
                        let new_content = format!(
                            "{}{}",
                            content[..start].trim_end(),
                            content[end_pos..].trim_start()
                        );
                        let new_content = new_content.trim().to_string();
                        if new_content.is_empty() {
                            fs::remove_file(&agents_path)?;
                            println!("  Removed AGENTS.md: {}", agents_path.display());
                        } else {
                            fs::write(&agents_path, new_content)?;
                            println!(
                                "  Removed skrills integration from AGENTS.md: {}",
                                agents_path.display()
                            );
                        }
                    }
                }
            }
        }
    }

    // Remove MCP server registration from config.toml
    let config_path = base_dir.join("config.toml");
    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if content.contains("[mcp_servers.skrills]") {
                // Remove the skrills MCP server section
                // Find the section start and end
                if let Some(start) = content.find("# Skrills MCP server") {
                    // Find next section or end of file
                    let section_end = content[start..]
                        .find("\n[")
                        .map(|pos| start + pos)
                        .unwrap_or(content.len());
                    let new_content =
                        format!("{}{}", content[..start].trim_end(), &content[section_end..])
                            .trim()
                            .to_string();
                    if new_content.is_empty() {
                        fs::remove_file(&config_path)?;
                        println!("  Removed config.toml: {}", config_path.display());
                    } else {
                        fs::write(&config_path, new_content)?;
                        println!(
                            "  Removed skrills MCP server from config.toml: {}",
                            config_path.display()
                        );
                    }
                } else if let Some(start) = content.find("[mcp_servers.skrills]") {
                    // Find next section or end of file
                    let section_end = content[start + 1..]
                        .find("\n[")
                        .map(|pos| start + 1 + pos)
                        .unwrap_or(content.len());
                    let new_content =
                        format!("{}{}", content[..start].trim_end(), &content[section_end..])
                            .trim()
                            .to_string();
                    if new_content.is_empty() {
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
        }
    }

    Ok(())
}

/// Prints next steps after setup.
fn print_next_steps(config: &SetupConfig) -> Result<()> {
    println!("\nNext steps:");

    for client in &config.clients {
        match client {
            Client::Claude => {
                println!("\n  Claude Code:");
                println!("    - Restart Claude Code to load the MCP server");
                println!("    - Skills will be auto-loaded on each prompt");
            }
            Client::Codex => {
                println!("\n  Codex:");
                println!("    - MCP server registered in ~/.codex/config.toml");
                println!("    - AGENTS.md installed with skill-loading instructions");
                println!("    - Skills will be auto-loaded via MCP when Codex starts");
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
