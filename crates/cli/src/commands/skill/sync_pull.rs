use anyhow::{bail, Result};

use crate::cli::{OutputFormat, SyncSource};

use super::SyncPullResult;

/// Commands that do what `sync-pull` cannot yet, with what each does.
const ALTERNATIVES: [(&str, &str); 3] = [
    (
        "skrills skill-import <path>",
        "Import a skill from local path",
    ),
    ("skrills sync", "Sync skills from Claude to Codex"),
    (
        "skrills sync-all --from claude",
        "Sync all assets from Claude",
    ),
];

/// Handle the sync-pull command.
///
/// Remote registries are not implemented, so a pull from `--source` is an
/// error: exiting 0 with `skills_pulled: 0` told scripts the pull worked.
pub(crate) fn handle_sync_pull_command(
    source: Option<String>,
    skill: Option<String>,
    target: SyncSource,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    if let Some(source) = source {
        let alternatives: Vec<&str> = ALTERNATIVES.iter().map(|(c, _)| *c).collect();
        bail!(
            "sync-pull from {source}: remote skill registries are not implemented. Use one of: {}",
            alternatives.join(", ")
        );
    }

    let result = SyncPullResult {
        source: None,
        target: target.as_str().to_string(),
        skills_pulled: 0,
        dry_run,
        message: "No source specified. Use --source <url> to specify a remote registry."
            .to_string(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("sync-pull: {}", result.message);
        println!();
        println!("Available alternatives:");
        for (command, what) in ALTERNATIVES {
            println!("  {command:<33}{what}");
        }

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
    fn sync_pull_without_a_source_explains_and_succeeds() {
        let result =
            handle_sync_pull_command(None, None, SyncSource::Claude, false, OutputFormat::Json);

        result.expect("sync-pull without --source only prints guidance");
    }

    #[test]
    fn sync_pull_with_a_source_fails_because_remote_registries_are_not_implemented() {
        let err = handle_sync_pull_command(
            Some("https://example.com/registry".into()),
            None,
            SyncSource::Claude,
            false,
            OutputFormat::Json,
        )
        .expect_err("a pull that pulls nothing must not exit 0");

        assert!(err.to_string().contains("not implemented"), "{err}");
    }

    #[test]
    fn sync_pull_alternatives_are_commands_that_parse() {
        use clap::Parser;
        for (command, _) in ALTERNATIVES {
            let argv = command
                .split_whitespace()
                .map(|arg| if arg == "<path>" { "/tmp/x" } else { arg });
            crate::cli::Cli::try_parse_from(argv)
                .unwrap_or_else(|e| panic!("`{command}` does not parse: {e}"));
        }
    }
}
