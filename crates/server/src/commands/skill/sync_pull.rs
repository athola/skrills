use anyhow::Result;

use crate::cli::{OutputFormat, SyncSource};

use super::SyncPullResult;

/// Handle the sync-pull command.
pub(crate) fn handle_sync_pull_command(
    source: Option<String>,
    skill: Option<String>,
    target: SyncSource,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    let result = SyncPullResult {
        source: source.clone(),
        target: target.as_str().to_string(),
        skills_pulled: 0,
        dry_run,
        message: if source.is_some() {
            "Remote skill registries not yet implemented. Use 'skill-import' for individual files or 'sync' to copy from another CLI.".to_string()
        } else {
            "No source specified. Use --source <url> to specify a remote registry.".to_string()
        },
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("sync-pull: {}", result.message);
        println!();
        println!("Available alternatives:");
        println!("  skrills skill-import <path>      Import a skill from local path");
        println!("  skrills sync --from claude       Sync skills from Claude to Codex");
        println!("  skrills sync-all --from claude   Sync all assets from Claude");

        if let Some(ref s) = skill {
            println!();
            println!(
                "To import skill '{}', use: skrills skill-import /path/to/{}.md",
                s, s
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_pull_returns_placeholder_message() {
        let result =
            handle_sync_pull_command(None, None, SyncSource::Claude, false, OutputFormat::Json);

        assert!(result.is_ok(), "sync-pull should succeed (placeholder)");
    }
}
