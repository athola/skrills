use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::cli::{OutputFormat, Platform};

use super::ImportResult;

/// The directory a CLI loads skills from, where an import lands.
///
/// Claude and Codex use `~/.claude/skills` and `~/.codex/skills`. Copilot and
/// Cursor resolve through their sync adapters, as `sync-status` does, so a
/// Copilot import lands in the XDG directory when that is where Copilot
/// lives rather than in a hard-coded `~/.copilot/skills`.
fn import_target_dir(target: Platform, home: &Path) -> PathBuf {
    match target {
        Platform::Claude => home.join(".claude/skills"),
        other => crate::commands::sync::source_skill_root(other, home),
    }
}

/// The skill file and name behind `source`: a `SKILL.md`, a directory that
/// holds one, or any other markdown file. A `SKILL.md` takes its directory's
/// name; another file takes its stem.
fn resolve_source(source: &str) -> Result<(PathBuf, String)> {
    if source.starts_with("http://") || source.starts_with("https://") {
        bail!("URL imports are not supported. Download the skill and pass its local path.");
    }
    // A local directory named `skills.git` is a path, not a git URL.
    if source.starts_with("git://") || (source.ends_with(".git") && !Path::new(source).exists()) {
        bail!(
            "Git imports are not supported. Clone the repository and pass the skill's local path."
        );
    }
    let mut path = PathBuf::from(source);
    if !path.exists() {
        bail!("Source path does not exist: {}", source);
    }
    if path.is_dir() {
        path = path.join("SKILL.md");
        if !path.is_file() {
            bail!("Source directory has no SKILL.md: {}", source);
        }
    }
    let name = if path.file_name().is_some_and(|f| f == "SKILL.md") {
        path.parent().and_then(|p| p.file_name())
    } else {
        path.file_stem()
    }
    .map(|s| s.to_string_lossy().into_owned())
    .unwrap_or_else(|| "imported-skill".to_string());
    Ok((path, name))
}

/// Copies every file under `source_dir` except `SKILL.md` (written
/// separately) into `target_dir`, keeping relative paths. Hidden entries
/// (`.git`) and symlinks are skipped: a link could point anywhere on the
/// source machine.
fn copy_skill_tree(source_dir: &Path, target_dir: &Path) -> Result<()> {
    let walker = walkdir::WalkDir::new(source_dir)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'));
    for entry in walker {
        let entry = entry
            .with_context(|| format!("Failed to read source tree {}", source_dir.display()))?;
        let rel = entry
            .path()
            .strip_prefix(source_dir)
            .context("walked entry lies outside the source directory")?;
        let dest = target_dir.join(rel);
        let file_type = entry.file_type();
        if file_type.is_dir() {
            std::fs::create_dir_all(&dest)
                .with_context(|| format!("Failed to create {}", dest.display()))?;
        } else if file_type.is_file() && rel != Path::new("SKILL.md") {
            std::fs::copy(entry.path(), &dest)
                .with_context(|| format!("Failed to copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// `content` with its frontmatter `name` set to `name`. Content without
/// frontmatter, or without a `name` line, is returned unchanged.
fn frontmatter_with_name(content: &str, name: &str) -> String {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content.to_string();
    };
    let Some(end) = rest.find("\n---") else {
        return content.to_string();
    };
    let (header, body) = rest.split_at(end);
    let value = if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        name.to_string()
    } else {
        format!("\"{}\"", super::escape_yaml_string(name))
    };
    let mut changed = false;
    let lines: Vec<String> = header
        .lines()
        .map(|line| match line.strip_prefix("name:") {
            Some(current) if current.trim().trim_matches(|c| c == '"' || c == '\'') != name => {
                changed = true;
                format!("name: {value}")
            }
            _ => line.to_string(),
        })
        .collect();
    if !changed {
        return content.to_string();
    }
    format!("---\n{}{}", lines.join("\n"), body)
}

/// Handle the skill-import command.
///
/// The skill is written as `<target>/<name>/SKILL.md`, the only layout skill
/// discovery recognises; `<target>/<name>.md` was invisible to every other
/// command.
pub(crate) fn handle_skill_import_command(
    source: String,
    target: Platform,
    force: bool,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    let home = skrills_state::home_dir()?;
    let target_dir = import_target_dir(target, &home);

    let (source_path, skill_name) = resolve_source(&source)?;
    let skill_content = std::fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read source file: {}", source_path.display()))?;

    let target_path = target_dir.join(&skill_name).join("SKILL.md");

    if target_path.exists() && !force {
        bail!(
            "Skill '{}' already exists at {}. Use --force to overwrite.",
            skill_name,
            target_path.display()
        );
    }

    // Cursor and OpenCode load a skill only when its frontmatter `name`
    // matches its folder, so the imported name follows the folder.
    let renamed = frontmatter_with_name(&skill_content, &skill_name);
    if renamed != skill_content {
        eprintln!("Note: frontmatter name set to '{skill_name}' to match the target folder");
    }
    let skill_content = renamed;

    if dry_run {
        let result = ImportResult {
            source,
            target_path: target_path.clone(),
            imported: false,
            skill_name: Some(skill_name.clone()),
            message: format!("Would import '{}' to {}", skill_name, target_path.display()),
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("[dry-run] {}", result.message);
        }
        return Ok(());
    }

    let skill_dir = target_path.parent().expect("target path has a parent");
    std::fs::create_dir_all(skill_dir)
        .with_context(|| format!("Failed to create {}", skill_dir.display()))?;
    // A directory source brings its scripts, references and assets along;
    // copying SKILL.md alone left its relative links dangling.
    if let Some(source_dir) = source_path.parent().filter(|_| Path::new(&source).is_dir()) {
        copy_skill_tree(source_dir, skill_dir)?;
    }
    super::write_skill_file(&target_path, &skill_content)
        .with_context(|| format!("Failed to write skill to {}", target_path.display()))?;

    let result = ImportResult {
        source,
        target_path: target_path.clone(),
        imported: true,
        skill_name: Some(skill_name.clone()),
        message: format!(
            "Successfully imported '{}' to {}",
            skill_name,
            target_path.display()
        ),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("✓ {}", result.message);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, TestFixture};
    use tempfile::tempdir;

    #[test]
    fn import_local_file_succeeds() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("source-skill.md");
        std::fs::write(
            &source_path,
            "---\nname: imported-skill\ndescription: Test\n---\nContent",
        )
        .expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            Platform::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        result.expect("import should succeed");

        let target = fixture.claude_skills.join("source-skill/SKILL.md");
        assert!(target.exists(), "imported skill should exist at target");

        // SB-30: the import has to be visible to discovery, which only
        // recognises SKILL.md.
        let skills = skrills_discovery::discover_skills(
            &[skrills_discovery::SkillRoot {
                root: fixture.claude_skills.clone(),
                source: skrills_discovery::SkillSource::Claude,
            }],
            None,
        )
        .unwrap();
        assert!(
            skills.iter().any(|s| s.path == target),
            "the imported skill should be discovered: {skills:?}"
        );
    }

    #[test]
    fn import_dry_run_does_not_create_file() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("dry-run-skill.md");
        std::fs::write(&source_path, "---\nname: dry\n---\nContent").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            Platform::Claude,
            false,
            true,
            OutputFormat::Json,
        );

        result.expect("dry run should succeed");

        let target = fixture.claude_skills.join("dry-run-skill/SKILL.md");
        assert!(!target.exists(), "dry run should not create file");
    }

    #[test]
    fn import_force_overwrites_existing() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let existing = fixture.claude_skills.join("overwrite-skill/SKILL.md");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "old content").expect("write existing");

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("overwrite-skill.md");
        std::fs::write(&source_path, "new content").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            Platform::Claude,
            true,
            false,
            OutputFormat::Json,
        );

        result.expect("force import should succeed");

        let content = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(content, "new content", "content should be overwritten");
    }

    #[test]
    fn import_without_force_fails_and_keeps_the_existing_skill() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let existing = fixture.claude_skills.join("keep-skill/SKILL.md");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "keep this").expect("write existing");

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("keep-skill.md");
        std::fs::write(&source_path, "replace this").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            Platform::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        let err = result.expect_err("an import that imports nothing must not exit 0");
        assert!(err.to_string().contains("--force"), "{err}");

        let content = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(content, "keep this", "content should not be overwritten");
    }

    #[test]
    fn import_nonexistent_source_errors() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let result = handle_skill_import_command(
            "/nonexistent/path/skill.md".to_string(),
            Platform::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_err(), "import nonexistent should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist"),
            "error should mention missing file"
        );
    }

    #[test]
    fn import_from_a_directory_copies_the_whole_skill_tree() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let source_dir = tempdir().expect("tempdir");
        let skill = source_dir.path().join("tree-skill");
        std::fs::create_dir_all(skill.join("scripts")).unwrap();
        std::fs::create_dir_all(skill.join("references")).unwrap();
        std::fs::create_dir_all(skill.join(".git")).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: tree-skill\ndescription: T\n---\nSee references/guide.md\n",
        )
        .unwrap();
        std::fs::write(skill.join("scripts/run.sh"), "echo hi\n").unwrap();
        std::fs::write(skill.join("references/guide.md"), "guide\n").unwrap();
        std::fs::write(skill.join(".git/HEAD"), "ref\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", skill.join("hosts-link")).unwrap();

        handle_skill_import_command(
            skill.to_string_lossy().to_string(),
            Platform::Claude,
            false,
            false,
            OutputFormat::Json,
        )
        .expect("directory import should succeed");

        let target = fixture.claude_skills.join("tree-skill");
        assert!(target.join("SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(target.join("scripts/run.sh")).unwrap(),
            "echo hi\n"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("references/guide.md")).unwrap(),
            "guide\n"
        );
        assert!(!target.join(".git").exists(), "hidden entries are skipped");
        assert!(
            std::fs::symlink_metadata(target.join("hosts-link")).is_err(),
            "symlinks are skipped"
        );
    }

    #[test]
    fn imported_frontmatter_name_is_set_to_the_target_folder() {
        assert_eq!(
            frontmatter_with_name("---\nname: other\ndescription: D\n---\nBody\n", "folder"),
            "---\nname: folder\ndescription: D\n---\nBody\n"
        );
        let same = "---\nname: folder\n---\nBody\n";
        assert_eq!(frontmatter_with_name(same, "folder"), same);
        assert_eq!(
            frontmatter_with_name("no frontmatter\n", "folder"),
            "no frontmatter\n"
        );
    }

    #[test]
    fn a_skill_md_source_takes_its_directory_name() {
        let dir = tempdir().unwrap();
        let skill = dir.path().join("my-skill/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "x").unwrap();

        let (path, name) = resolve_source(skill.to_str().unwrap()).unwrap();
        assert_eq!((path, name.as_str()), (skill.clone(), "my-skill"));

        let (path, name) = resolve_source(dir.path().join("my-skill").to_str().unwrap()).unwrap();
        assert_eq!((path, name.as_str()), (skill, "my-skill"));
    }

    /// SB-38: the URL error pointed users at git URLs, which the next branch
    /// rejected.
    #[test]
    fn url_and_git_sources_are_refused_with_a_local_path_hint() {
        let url = resolve_source("https://example.com/s.md")
            .unwrap_err()
            .to_string();
        assert!(
            url.contains("local path") && !url.contains("git URL"),
            "{url}"
        );
        let git = resolve_source("git://example.com/r.git")
            .unwrap_err()
            .to_string();
        assert!(git.contains("local path"), "{git}");
    }

    #[test]
    fn a_local_directory_ending_in_dot_git_is_imported_as_a_path() {
        let dir = tempdir().unwrap();
        let skill = dir.path().join("skills.git/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "x").unwrap();

        let (path, name) = resolve_source(dir.path().join("skills.git").to_str().unwrap()).unwrap();

        assert_eq!((path, name.as_str()), (skill, "skills.git"));
    }

    /// SB-38: Copilot imports went to `~/.copilot/skills` even when Copilot
    /// lives in the XDG config directory.
    #[test]
    fn copilot_imports_follow_the_copilot_adapter() {
        let _g = env_guard();
        let home = tempdir().unwrap();
        let _h = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let xdg = home.path().join("xdg");
        let _x = skrills_test_utils::set_env_var("XDG_CONFIG_HOME", Some(xdg.to_str().unwrap()));
        std::fs::create_dir_all(xdg.join("copilot")).unwrap();

        let expected = {
            use skrills_sync::adapters::traits::AgentAdapter;
            skrills_sync::CopilotAdapter::new()
                .unwrap()
                .config_root()
                .join("skills")
        };
        assert!(expected.starts_with(&xdg), "{expected:?}");
        assert_eq!(import_target_dir(Platform::Copilot, home.path()), expected);
        assert_eq!(
            import_target_dir(Platform::Claude, home.path()),
            home.path().join(".claude/skills")
        );
    }
}
