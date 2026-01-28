use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use crate::cli::{OutputFormat, SyncSource};

use super::ImportResult;

/// Handle the skill-import command.
pub(crate) fn handle_skill_import_command(
    source: String,
    target: SyncSource,
    force: bool,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;

    let target_dir = match target {
        SyncSource::Claude => home.join(".claude/skills"),
        SyncSource::Codex => home.join(".codex/skills"),
        SyncSource::Copilot => home.join(".github/copilot/skills"),
    };

    if !dry_run {
        std::fs::create_dir_all(&target_dir).with_context(|| {
            format!(
                "Failed to create target directory: {}",
                target_dir.display()
            )
        })?;
    }

    let (skill_content, skill_name) =
        if source.starts_with("http://") || source.starts_with("https://") {
            bail!(
            "URL imports require the 'http-transport' feature. Use a local path or git URL instead."
        );
        } else if source.starts_with("git://") || source.ends_with(".git") {
            bail!("Git imports not yet implemented. Clone the repo manually and use a local path.");
        } else {
            let path = PathBuf::from(&source);
            if !path.exists() {
                bail!("Source path does not exist: {}", source);
            }

            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read source file: {}", source))?;

            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "imported-skill".to_string());

            (content, name)
        };

    let target_path = target_dir.join(format!("{}.md", skill_name));

    if target_path.exists() && !force {
        let result = ImportResult {
            source: source.clone(),
            target_path: target_path.clone(),
            imported: false,
            skill_name: Some(skill_name.clone()),
            message: format!(
                "Skill '{}' already exists at {}. Use --force to overwrite.",
                skill_name,
                target_path.display()
            ),
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            eprintln!("{}", result.message);
        }
        return Ok(());
    }

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

    std::fs::write(&target_path, &skill_content)
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
            SyncSource::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "import should succeed");

        let target = fixture.claude_skills.join("source-skill.md");
        assert!(target.exists(), "imported skill should exist at target");
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
            SyncSource::Claude,
            false,
            true,
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "dry run should succeed");

        let target = fixture.claude_skills.join("dry-run-skill.md");
        assert!(!target.exists(), "dry run should not create file");
    }

    #[test]
    fn import_force_overwrites_existing() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let existing = fixture.claude_skills.join("overwrite-skill.md");
        std::fs::write(&existing, "old content").expect("write existing");

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("overwrite-skill.md");
        std::fs::write(&source_path, "new content").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            true,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "force import should succeed");

        let content = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(content, "new content", "content should be overwritten");
    }

    #[test]
    fn import_without_force_skips_existing() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let existing = fixture.claude_skills.join("keep-skill.md");
        std::fs::write(&existing, "keep this").expect("write existing");

        let source_dir = tempdir().expect("tempdir");
        let source_path = source_dir.path().join("keep-skill.md");
        std::fs::write(&source_path, "replace this").expect("write source");

        let result = handle_skill_import_command(
            source_path.to_string_lossy().to_string(),
            SyncSource::Claude,
            false,
            false,
            OutputFormat::Json,
        );

        assert!(result.is_ok());

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
            SyncSource::Claude,
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
}
