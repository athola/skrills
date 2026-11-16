//! Adapters registered by the user instead of compiled in.
//!
//! One TOML file per adapter under `~/.skrills/adapters/`. A file names a
//! config root and, for each artifact kind the tool has, where it lives below
//! that root. A kind with no key is unsupported, so the orchestrator skips it
//! instead of writing into a layout nobody described.
//!
//! Sync writes into these paths, and they come from a file the user (or a
//! script) edited, so every path is held inside `root`: relative, no `..`,
//! and no symlink that resolves elsewhere.

use crate::Platform;
use anyhow::{anyhow, Context};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

/// Where adapter files live under `home`.
pub fn adapters_dir(home: &Path) -> PathBuf {
    home.join(".skrills").join("adapters")
}

/// The home directory adapter files are looked up from: an absolute `HOME`
/// first, then the platform lookup, as the snapshot store resolves it.
pub(crate) fn user_home() -> crate::Result<PathBuf> {
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if home.is_absolute() {
            return Ok(home);
        }
    }
    dirs::home_dir()
        .filter(|home| home.is_absolute())
        .context("Could not determine home directory")
}

/// What is wrong with one adapter file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigProblem {
    /// Not valid TOML, or a key the schema does not have.
    #[error("invalid config: {0}")]
    Parse(String),
    /// A required key is absent.
    #[error("missing required key `{0}`")]
    MissingKey(&'static str),
    /// The name cannot be typed as a `--from`/`--to` value.
    #[error(
        "name `{0}` must be lowercase letters, digits, `-` or `_`, starting with a letter or digit"
    )]
    InvalidName(String),
    /// The name is one of the built-in platforms.
    #[error("name `{0}` is a built-in platform")]
    BuiltinNameClash(String),
    /// Another file declares the same name.
    #[error("name `{name}` is also declared by {}", other.display())]
    DuplicateName {
        /// The shared name.
        name: String,
        /// The other file.
        other: PathBuf,
    },
    /// `root` is not absolute once `~` is expanded.
    #[error("root `{0}` is not an absolute path (use `~/...` or `/...`)")]
    RootNotAbsolute(String),
    /// A path that must stay below `root` does not.
    #[error(
        "`{key}` = `{path}` leaves root: use a relative path without `..` that no symlink redirects out"
    )]
    PathEscapesRoot {
        /// The offending key, `mcp.path` for the MCP file.
        key: &'static str,
        /// The value as written.
        path: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    name: Option<String>,
    root: Option<String>,
    skills_dir: Option<String>,
    commands_dir: Option<String>,
    agents_dir: Option<String>,
    instructions_file: Option<String>,
    mcp: Option<RawMcp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    path: Option<String>,
    servers_key: Option<String>,
}

/// Where a custom adapter keeps its MCP servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpLocation {
    /// The JSON file, absolute and below the root.
    pub path: PathBuf,
    /// The top-level key holding the servers object, e.g. `mcpServers`.
    pub servers_key: String,
}

/// A validated adapter file. Every path is absolute and inside `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterConfig {
    /// The name used on the command line.
    pub name: String,
    /// The tool's config directory, `~` expanded.
    pub root: PathBuf,
    /// Skill folders (`<name>/SKILL.md`).
    pub skills_dir: Option<PathBuf>,
    /// Slash commands, one markdown file each.
    pub commands_dir: Option<PathBuf>,
    /// Agents, one markdown file each.
    pub agents_dir: Option<PathBuf>,
    /// A single instructions markdown file.
    pub instructions_file: Option<PathBuf>,
    /// The MCP servers file.
    pub mcp: Option<McpLocation>,
}

impl AdapterConfig {
    /// Parses and validates `text`, expanding `~` against `home`. Returns
    /// every problem found, not just the first.
    pub fn from_toml(text: &str, home: &Path) -> Result<Self, Vec<ConfigProblem>> {
        let raw: RawConfig = toml::from_str(text)
            .map_err(|e| vec![ConfigProblem::Parse(e.message().to_string())])?;
        let mut problems = Vec::new();

        let name = match raw.name {
            None => {
                problems.push(ConfigProblem::MissingKey("name"));
                String::new()
            }
            Some(name) => {
                if name.parse::<Platform>().is_ok() {
                    problems.push(ConfigProblem::BuiltinNameClash(name.clone()));
                } else if !is_valid_name(&name) {
                    problems.push(ConfigProblem::InvalidName(name.clone()));
                }
                name
            }
        };

        let root = match raw.root {
            None => {
                problems.push(ConfigProblem::MissingKey("root"));
                None
            }
            Some(written) => {
                let expanded = expand_tilde(&written, home);
                if expanded.is_absolute() {
                    Some(expanded)
                } else {
                    problems.push(ConfigProblem::RootNotAbsolute(written));
                    None
                }
            }
        };

        let mut under_root = |key: &'static str, written: Option<String>| {
            let (root, written) = (root.as_ref()?, written?);
            match contained(root, &written) {
                Some(path) => Some(path),
                None => {
                    problems.push(ConfigProblem::PathEscapesRoot { key, path: written });
                    None
                }
            }
        };
        let skills_dir = under_root("skills_dir", raw.skills_dir);
        let commands_dir = under_root("commands_dir", raw.commands_dir);
        let agents_dir = under_root("agents_dir", raw.agents_dir);
        let instructions_file = under_root("instructions_file", raw.instructions_file);
        let mcp = match raw.mcp {
            None => None,
            Some(RawMcp { path, servers_key }) => {
                let missing_path = path.is_none();
                let servers_key = servers_key.filter(|key| !key.is_empty());
                let missing_key = servers_key.is_none();
                let located = under_root("mcp.path", path);
                if missing_path {
                    problems.push(ConfigProblem::MissingKey("mcp.path"));
                }
                if missing_key {
                    problems.push(ConfigProblem::MissingKey("mcp.servers_key"));
                }
                match (located, servers_key) {
                    (Some(path), Some(servers_key)) => Some(McpLocation { path, servers_key }),
                    _ => None,
                }
            }
        };

        match (problems.is_empty(), root) {
            (true, Some(root)) => Ok(Self {
                name,
                root,
                skills_dir,
                commands_dir,
                agents_dir,
                instructions_file,
                mcp,
            }),
            _ => Err(problems),
        }
    }

    /// The artifact kinds this adapter supports, in a fixed order.
    pub fn categories(&self) -> Vec<&'static str> {
        [
            ("skills", self.skills_dir.is_some()),
            ("commands", self.commands_dir.is_some()),
            ("agents", self.agents_dir.is_some()),
            ("instructions", self.instructions_file.is_some()),
            ("mcp_servers", self.mcp.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
    }

    /// Findings that do not stop the adapter from loading.
    pub fn warnings(&self) -> Vec<String> {
        if self.root.exists() {
            Vec::new()
        } else {
            vec![format!(
                "root {} does not exist yet; sync creates it on the first write",
                self.root.display()
            )]
        }
    }
}

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn expand_tilde(written: &str, home: &Path) -> PathBuf {
    match written {
        "~" => home.to_path_buf(),
        _ => match written.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(written),
        },
    }
}

/// `root` joined with `relative`, or `None` when `relative` is absolute,
/// climbs with `..`, or resolves outside `root` through a symlink.
fn contained(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative_path = Path::new(relative);
    let leaves = relative_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if leaves {
        return None;
    }
    let joined = root.join(relative_path);
    resolves_inside(root, &joined).then_some(joined)
}

/// Whether the deepest part of `path` that exists resolves inside `root`.
/// A root that does not exist has nothing to follow, so it passes.
fn resolves_inside(root: &Path, path: &Path) -> bool {
    let Ok(real_root) = root.canonicalize() else {
        return true;
    };
    let mut probe = path;
    loop {
        if let Ok(real) = probe.canonicalize() {
            return real.starts_with(&real_root);
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return true,
        }
    }
}

/// One file in the adapters directory and what loading it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterFile {
    /// The file's path, for reporting.
    pub path: PathBuf,
    /// The adapter, or every problem that keeps it from loading.
    pub outcome: Result<AdapterConfig, Vec<ConfigProblem>>,
}

/// Loads every `*.toml` file in [`adapters_dir`], sorted by path. A missing
/// directory is an empty registry. Two files that declare one name are both
/// rejected, so no file silently wins.
pub fn load_adapter_files(home: &Path) -> crate::Result<Vec<AdapterFile>> {
    let dir = adapters_dir(home);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(anyhow!("Failed to read {}: {e}", dir.display())),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(e) => {
                tracing::warn!(error = %e, dir = %dir.display(), "Skipping unreadable directory entry");
                None
            }
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml") && path.is_file())
        .collect();
    paths.sort();

    let mut files: Vec<AdapterFile> = paths
        .into_iter()
        .map(|path| {
            let outcome = match std::fs::read_to_string(&path) {
                Ok(text) => AdapterConfig::from_toml(&text, home),
                Err(e) => Err(vec![ConfigProblem::Parse(format!(
                    "could not read file: {e}"
                ))]),
            };
            AdapterFile { path, outcome }
        })
        .collect();

    let declared: Vec<(String, PathBuf)> = files
        .iter()
        .filter_map(|file| Some((file.outcome.as_ref().ok()?.name.clone(), file.path.clone())))
        .collect();
    for file in &mut files {
        let Ok(config) = &file.outcome else { continue };
        let clash = declared
            .iter()
            .find(|(name, path)| *name == config.name && *path != file.path);
        if let Some((name, other)) = clash {
            file.outcome = Err(vec![ConfigProblem::DuplicateName {
                name: name.clone(),
                other: other.clone(),
            }]);
        }
    }
    Ok(files)
}

/// The configured adapter called `name`, or an error that lists the built-in
/// platforms and the configured adapters.
pub fn load_adapter(name: &str, home: &Path) -> crate::Result<AdapterConfig> {
    let files = load_adapter_files(home)?;
    if let Some(config) = files
        .iter()
        .find_map(|file| file.outcome.as_ref().ok().filter(|c| c.name == name))
    {
        return Ok(config.clone());
    }
    let invalid_file = files.iter().find(|file| {
        file.outcome.is_err()
            && file
                .path
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy() == name)
    });
    if let Some(AdapterFile {
        path,
        outcome: Err(problems),
    }) = invalid_file
    {
        let listed: Vec<String> = problems.iter().map(ToString::to_string).collect();
        return Err(anyhow!(
            "Adapter '{name}' is invalid in {}: {}",
            path.display(),
            listed.join("; ")
        ));
    }
    let configured: Vec<&str> = files
        .iter()
        .filter_map(|file| file.outcome.as_ref().ok().map(|c| c.name.as_str()))
        .collect();
    let invalid = files.iter().filter(|file| file.outcome.is_err()).count();
    let builtins: Vec<&str> = Platform::ALL.iter().map(|p| p.as_str()).collect();
    Err(anyhow!(
        "Unknown adapter '{name}'. Built-in: {}. Configured in {}: {}{}",
        builtins.join(", "),
        adapters_dir(home).display(),
        if configured.is_empty() {
            "none configured".to_string()
        } else {
            configured.join(", ")
        },
        if invalid == 0 {
            String::new()
        } else {
            format!(" ({invalid} invalid file(s); run `skrills doctor`)")
        }
    ))
}
