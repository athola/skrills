//! Instructions (CLAUDE.md) reading/writing for the Claude adapter.
//!
//! Claude only supports a single CLAUDE.md file at the config root, and that
//! file is the user's own. Synced instructions therefore go into one block
//! delimited by [`BLOCK_BEGIN`] and [`BLOCK_END`]; everything outside the block
//! is left exactly as the user wrote it. The sync used to replace the whole
//! file with the source's rules.

use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::fs;
use std::time::SystemTime;

use super::ClaudeAdapter;

pub(super) fn read_instructions_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    let path = adapter.instructions_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read(&path)?;
    // The managed block came from another tool. Reading it back would send
    // those rules to every other target a second time, inside CLAUDE.
    let content = match String::from_utf8(raw) {
        Ok(text) => strip_block(&text).into_bytes(),
        Err(e) => e.into_bytes(),
    };
    if content.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let metadata = fs::metadata(&path)?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let hash = hash_content(&content);

    // Use "CLAUDE" as the instruction name (derived from CLAUDE.md)
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("CLAUDE")
        .to_string();

    Ok(vec![Command {
        name,
        content,
        source_path: path.clone(),
        modified,
        hash,
        modules: Vec::new(),

        content_format: ContentFormat::default(),
        plugin_origin: None,
    }])
}

/// Opens the skrills-managed block in CLAUDE.md.
const BLOCK_BEGIN: &str =
    "<!-- skrills:begin synced instructions (edits inside this block are replaced on sync) -->";
/// Closes the skrills-managed block in CLAUDE.md.
const BLOCK_END: &str = "<!-- skrills:end synced instructions -->";

pub(super) fn write_instructions_impl(
    adapter: &ClaudeAdapter,
    instructions: &[Command],
) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    if instructions.is_empty() {
        return Ok(report);
    }

    let path = adapter.instructions_path();
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };

    let block = render_block(instructions);
    let updated = splice_block(&existing, &block)?;

    if updated == existing {
        report.skipped.push(SkipReason::Unchanged {
            item: "CLAUDE.md".to_string(),
        });
        return Ok(report);
    }

    // The user's CLAUDE.md is a config file: keep the previous version.
    crate::adapters::utils::write_config(&path, updated.as_bytes(), false)?;
    report.written += 1;

    Ok(report)
}

/// Renders the managed block, one section per source instruction.
///
/// Leading frontmatter (a Cursor rule's `description`/`globs`/`alwaysApply`)
/// means nothing to Claude and is dropped.
fn render_block(instructions: &[Command]) -> String {
    let mut block = String::new();
    block.push_str(BLOCK_BEGIN);
    block.push('\n');
    for instruction in instructions {
        let text = String::from_utf8_lossy(&instruction.content);
        let (_, body, _) = skrills_validate::frontmatter::split_frontmatter(&text);
        block.push_str(&format!("\n<!-- Source: {} -->\n\n", instruction.name));
        block.push_str(body.trim_end());
        block.push('\n');
    }
    block.push('\n');
    block.push_str(BLOCK_END);
    block
}

/// Replaces the managed block in `existing`, or appends one after the user's
/// content when there is none yet.
fn splice_block(existing: &str, block: &str) -> Result<String> {
    let begin = existing.find(BLOCK_BEGIN);
    let end = begin.and_then(|b| existing[b..].find(BLOCK_END).map(|e| b + e));
    match (begin, end) {
        (Some(b), Some(e)) => Ok(format!(
            "{}{}{}",
            &existing[..b],
            block,
            &existing[e + BLOCK_END.len()..]
        )),
        (Some(_), None) => Err(anyhow::anyhow!(
            "CLAUDE.md has a skrills begin marker with no end marker after it; fix or remove \
             the marker so sync can tell your text from the synced block"
        )),
        (None, _) if existing.trim().is_empty() => Ok(format!("{block}\n")),
        (None, _) => {
            let separator = if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Ok(format!("{existing}{separator}{block}\n"))
        }
    }
}

/// `text` without the managed block (and without the blank line that
/// separated it from the user's content).
fn strip_block(text: &str) -> String {
    let Some(begin) = text.find(BLOCK_BEGIN) else {
        return text.to_string();
    };
    let Some(end) = text[begin..]
        .find(BLOCK_END)
        .map(|e| begin + e + BLOCK_END.len())
    else {
        return text.to_string();
    };
    let before = text[..begin].trim_end_matches('\n');
    let after = text[end..].trim_start_matches('\n');
    match (before.is_empty(), after.is_empty()) {
        (true, _) => after.to_string(),
        (false, true) => format!("{before}\n"),
        (false, false) => format!("{before}\n\n{after}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::utils::test_helpers::make_command;
    use crate::adapters::AgentAdapter;

    fn adapter(root: &std::path::Path) -> ClaudeAdapter {
        ClaudeAdapter::with_root(root.to_path_buf())
    }

    /// cursor -> claude with one rule replaced the user's whole CLAUDE.md with
    /// that rule, `.mdc` frontmatter included.
    #[test]
    fn user_text_in_claude_md_survives_an_instruction_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, "# My rules\n\nAlways run tests.\n").unwrap();
        let rule = make_command(
            "style",
            "---\ndescription: Style\nalwaysApply: true\n---\nUse tabs.\n",
        );

        adapter(tmp.path()).write_instructions(&[rule]).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# My rules\n\nAlways run tests.\n"),
            "{text}"
        );
        assert!(text.contains("Use tabs."), "{text}");
        assert!(!text.contains("alwaysApply"), "frontmatter leaked: {text}");
    }

    #[test]
    fn a_second_sync_replaces_only_the_block() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        fs::write(&path, "mine\n").unwrap();
        let claude = adapter(tmp.path());

        claude
            .write_instructions(&[make_command("style", "old rule\n")])
            .unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("\nadded after the block\n");
        fs::write(&path, &text).unwrap();
        claude
            .write_instructions(&[make_command("style", "new rule\n")])
            .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("mine\n"));
        assert!(
            text.contains("new rule") && !text.contains("old rule"),
            "{text}"
        );
        assert!(text.ends_with("added after the block\n"), "{text}");
        assert_eq!(text.matches(BLOCK_BEGIN).count(), 1);
    }

    /// Reading the block back would echo another tool's rules to every other
    /// target a second time.
    #[test]
    fn reading_claude_md_leaves_out_the_synced_block() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("CLAUDE.md"), "mine\n").unwrap();
        let claude = adapter(tmp.path());
        claude
            .write_instructions(&[make_command("style", "theirs\n")])
            .unwrap();

        let read = claude.read_instructions().unwrap();

        assert_eq!(read.len(), 1);
        assert_eq!(String::from_utf8_lossy(&read[0].content), "mine\n");
    }

    #[test]
    fn a_claude_md_holding_only_the_block_reads_as_no_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = adapter(tmp.path());
        claude
            .write_instructions(&[make_command("style", "theirs\n")])
            .unwrap();
        assert!(claude.read_instructions().unwrap().is_empty());
    }

    #[test]
    fn an_identical_sync_is_reported_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = adapter(tmp.path());
        let rules = [make_command("style", "rule\n")];
        claude.write_instructions(&rules).unwrap();
        let report = claude.write_instructions(&rules).unwrap();
        assert_eq!(report.written, 0);
    }

    #[test]
    fn the_previous_claude_md_is_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("CLAUDE.md"), "mine\n").unwrap();
        adapter(tmp.path())
            .write_instructions(&[make_command("style", "rule\n")])
            .unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("CLAUDE.md.skrills-bak")).unwrap(),
            "mine\n"
        );
    }

    #[test]
    fn a_dangling_begin_marker_is_an_error_not_a_rewrite() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let original = format!("mine\n{BLOCK_BEGIN}\nhalf\n");
        fs::write(&path, &original).unwrap();
        let result = adapter(tmp.path()).write_instructions(&[make_command("style", "rule\n")]);
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}
