//! The shared `~/.agents/skills` directory.
//!
//! Codex, Cursor, Copilot CLI, OpenCode, Cline, Goose and Gemini CLI all load
//! user skills from `~/.agents/skills/<name>/SKILL.md`, so a target that reads
//! it gets one copy there instead of one in its own directory: two copies of
//! one skill in directories a tool scans together load it twice.
//!
//! Sources: learn.chatgpt.com/docs/build-skills, cursor.com/docs/context/skills,
//! docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills,
//! opencode.ai/docs/skills, docs.cline.bot/customization/overview,
//! goose-docs.ai/docs/guides/context-engineering/using-skills,
//! geminicli.com/docs/cli/skills (all read 2026-10-14).
//!
//! Each skill is written one level deep, and its frontmatter `name` is set to
//! the folder name: Cursor and OpenCode load a skill only when the two match,
//! and Codex and Cursor require a `description`. Every other frontmatter field
//! is kept as written.

use super::cursor::utils::parse_frontmatter;
use super::utils::{skill_folder_name, BatchWriter};
use crate::common::Command;
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::path::{Path, PathBuf};

/// `~/.agents/skills` under `home`.
pub(crate) fn shared_skills_dir(home: &Path) -> PathBuf {
    home.join(".agents").join("skills")
}

/// `content` with its frontmatter `name` set to `folder`, plus a warning when
/// there is no `description` for the tools that require one. Only the `name`
/// line changes; every other byte is kept, so an already-valid skill is
/// copied unchanged.
pub(crate) fn with_folder_name(content: &str, folder: &str) -> (String, Option<String>) {
    let (fields, _) = parse_frontmatter(content);
    let has_description = fields
        .get("description")
        .is_some_and(|d| !d.trim().is_empty());
    let warning = (!has_description).then(|| {
        format!("Skill {folder}: no `description`, which Codex and Cursor require to load a skill")
    });

    let (raw_yaml, _, _) = skrills_validate::frontmatter::split_frontmatter(content);
    if raw_yaml.is_none() {
        return (format!("---\nname: {folder}\n---\n\n{content}"), warning);
    }

    let mut out = String::with_capacity(content.len() + folder.len() + 8);
    let mut fence_count = 0;
    let mut named = !fields.contains_key("name");
    for line in content.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        let ending = &line[text.len()..];
        if fence_count == 1 && !named {
            if let Some(value) = text.strip_prefix("name:") {
                named = true;
                if value.trim().trim_matches(['"', '\'']) == folder {
                    out.push_str(line);
                } else {
                    out.push_str(&format!("name: {folder}{ending}"));
                }
                continue;
            }
        }
        out.push_str(line);
        if text.trim_end() == "---" && fence_count < 2 {
            fence_count += 1;
            if fence_count == 1 && !fields.contains_key("name") {
                let eol = if ending.starts_with('\r') {
                    "\r\n"
                } else {
                    "\n"
                };
                out.push_str(&format!("name: {folder}{eol}"));
            }
        }
    }
    (out, warning)
}

/// Writes `skills` under `dir` as `<folder>/SKILL.md` plus companion modules.
pub(crate) fn write_shared_skills(dir: &Path, skills: &[Command]) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    if skills.is_empty() {
        return Ok(report);
    }
    std::fs::create_dir_all(dir)?;

    let mut writer = BatchWriter::new(dir);
    for skill in skills {
        let content = match std::str::from_utf8(&skill.content) {
            Ok(text) => text,
            Err(e) => {
                report.skipped.push(SkipReason::ParseError {
                    item: skill.name.clone(),
                    error: format!("not valid UTF-8: {e}"),
                });
                continue;
            }
        };
        let folder = skill_folder_name(skill);
        let (body, warning) = with_folder_name(content, &folder);
        report.warnings.extend(warning);
        writer.write(
            &skill.name,
            &folder,
            "SKILL.md",
            body.as_bytes(),
            &skill.modules,
            &mut report,
        )?;
    }
    Ok(report)
}

/// What [`write_shared_skills`] would do to `dir`, without writing: a skill
/// whose transformed `SKILL.md` and modules already match the disk is
/// `Unchanged`.
pub(crate) fn preview_shared_skills(dir: &Path, skills: &[Command]) -> WriteReport {
    let mut report = WriteReport::default();
    for skill in skills {
        let Ok(content) = std::str::from_utf8(&skill.content) else {
            report.skipped.push(SkipReason::ParseError {
                item: skill.name.clone(),
                error: "not valid UTF-8".to_string(),
            });
            continue;
        };
        let folder = skill_folder_name(skill);
        let (body, _) = with_folder_name(content, &folder);
        let skill_dir = dir.join(&folder);
        let same = |path: PathBuf, bytes: &[u8]| std::fs::read(path).is_ok_and(|b| b == bytes);
        let unchanged = same(skill_dir.join("SKILL.md"), body.as_bytes())
            && skill
                .modules
                .iter()
                .all(|m| same(skill_dir.join(&m.relative_path), &m.content));
        if unchanged {
            report.skipped.push(SkipReason::Unchanged {
                item: skill.name.clone(),
            });
        } else {
            report.written += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ModuleFile;
    use std::fs;
    use std::time::SystemTime;
    use tempfile::tempdir;

    fn skill(name: &str, source_path: PathBuf, content: &str) -> Command {
        Command {
            name: name.to_string(),
            content: content.as_bytes().to_vec(),
            source_path,
            modified: SystemTime::UNIX_EPOCH,
            hash: crate::adapters::utils::hash_content(content.as_bytes()),
            modules: Vec::new(),
            content_format: Default::default(),
            plugin_origin: None,
        }
    }

    #[test]
    fn nested_skill_lands_one_level_deep_with_name_set_to_its_folder() {
        let tmp = tempdir().unwrap();
        let dir = shared_skills_dir(tmp.path());
        let src = "---\nname: Morning Brief\ndescription: Daily brief\nallowed-tools: Read\n---\n# Body\n";
        let report = write_shared_skills(
            &dir,
            &[skill(
                "synced/abc/morning",
                PathBuf::from("/x/morning/SKILL.md"),
                src,
            )],
        )
        .unwrap();

        assert_eq!(report.written, 1);
        let written = fs::read_to_string(dir.join("morning/SKILL.md")).unwrap();
        assert_eq!(
            written,
            "---\nname: morning\ndescription: Daily brief\nallowed-tools: Read\n---\n# Body\n"
        );
        assert!(report.warnings.is_empty());
    }

    /// A copy that only rewrote its frontmatter dropped the blank line after
    /// it, so a dry run always saw a change and every sync rewrote the file.
    #[test]
    fn skill_already_named_after_its_folder_is_copied_byte_for_byte() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let src = "---\r\nname: alpha\r\ndescription: A\r\n---\r\n\r\nBody\r\n";
        write_shared_skills(
            &dir,
            &[skill("alpha", PathBuf::from("/x/alpha/SKILL.md"), src)],
        )
        .unwrap();
        assert_eq!(fs::read_to_string(dir.join("alpha/SKILL.md")).unwrap(), src);
    }

    #[test]
    fn only_the_name_line_changes_when_it_differs_from_the_folder() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let src = "---\ndescription: A\nname: \"Alpha Skill\"\n---\n\nBody\n";
        write_shared_skills(
            &dir,
            &[skill("alpha", PathBuf::from("/x/alpha/SKILL.md"), src)],
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("alpha/SKILL.md")).unwrap(),
            "---\ndescription: A\nname: alpha\n---\n\nBody\n"
        );
    }

    #[test]
    fn a_missing_name_is_inserted_after_the_opening_fence() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let src = "---\ndescription: A\n---\n\nBody\n";
        write_shared_skills(
            &dir,
            &[skill("alpha", PathBuf::from("/x/alpha/SKILL.md"), src)],
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("alpha/SKILL.md")).unwrap(),
            "---\nname: alpha\ndescription: A\n---\n\nBody\n"
        );
    }

    #[test]
    fn skill_without_frontmatter_gets_a_name_and_a_missing_description_warning() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let report = write_shared_skills(
            &dir,
            &[skill(
                "plain",
                PathBuf::from("/x/plain/SKILL.md"),
                "# Plain\n",
            )],
        )
        .unwrap();

        let written = fs::read_to_string(dir.join("plain/SKILL.md")).unwrap();
        assert_eq!(written, "---\nname: plain\n---\n\n# Plain\n");
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("no `description`"));
    }

    #[test]
    fn second_identical_write_changes_nothing() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let skills = [skill(
            "a",
            PathBuf::from("/x/a/SKILL.md"),
            "---\nname: a\ndescription: d\n---\nbody\n",
        )];
        write_shared_skills(&dir, &skills).unwrap();
        let again = write_shared_skills(&dir, &skills).unwrap();
        assert_eq!(again.written, 0);
    }

    #[test]
    fn companion_modules_are_written_beside_the_skill() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let mut with_module = skill(
            "tool",
            PathBuf::from("/x/tool/SKILL.md"),
            "---\nname: tool\ndescription: d\n---\nbody\n",
        );
        with_module.modules.push(ModuleFile {
            relative_path: PathBuf::from("scripts/run.sh"),
            content: b"echo hi\n".to_vec(),
            hash: crate::adapters::utils::hash_content(b"echo hi\n"),
        });
        write_shared_skills(&dir, &[with_module]).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("tool/scripts/run.sh")).unwrap(),
            "echo hi\n"
        );
    }

    #[test]
    fn non_utf8_skill_is_skipped_not_written_lossily() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let mut bad = skill("bad", PathBuf::from("/x/bad/SKILL.md"), "");
        bad.content = vec![0xff, 0xfe];
        let report = write_shared_skills(&dir, &[bad]).unwrap();
        assert_eq!(report.written, 0);
        assert!(matches!(report.skipped[0], SkipReason::ParseError { .. }));
    }
}
