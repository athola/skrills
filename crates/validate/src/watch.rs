//! Watch mode for auto-revalidation on file changes.
//!
//! Monitors skill directories and revalidates when files change,
//! with debouncing to prevent excessive revalidation.

use crate::{validate_skill, ValidationResult, ValidationSummary, ValidationTarget};
use anyhow::Result;
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Configuration for the validation watch loop.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Directories to watch for changes.
    pub paths: Vec<PathBuf>,
    /// Debounce interval in milliseconds.
    pub debounce_ms: u64,
    /// Validation target (Claude, Codex, etc.).
    pub target: ValidationTarget,
    /// Only show skills with errors.
    pub errors_only: bool,
}

impl WatchConfig {
    /// Creates a new `WatchConfig` with the given paths and target.
    pub fn new(paths: Vec<PathBuf>, target: ValidationTarget) -> Self {
        Self {
            paths,
            debounce_ms: 300,
            target,
            errors_only: false,
        }
    }

    /// Sets the debounce interval in milliseconds.
    pub fn with_debounce_ms(mut self, ms: u64) -> Self {
        self.debounce_ms = ms;
        self
    }

    /// Sets whether to show only errors.
    pub fn with_errors_only(mut self, errors_only: bool) -> Self {
        self.errors_only = errors_only;
        self
    }
}

/// Collects changed paths from pending events, applying debounce logic.
///
/// Returns the set of unique file paths that changed, or `None` if the
/// channel was disconnected (watcher dropped).
pub fn collect_debounced_paths(
    rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    debounce: Duration,
) -> Option<HashSet<PathBuf>> {
    // Block until the first event arrives (or channel closes).
    let first = match rx.recv() {
        Ok(evt) => evt,
        Err(_) => return None, // channel closed
    };

    let mut changed = HashSet::new();
    if let Ok(event) = first {
        for p in event.paths {
            changed.insert(p);
        }
    }

    // Drain additional events within the debounce window.
    let deadline = Instant::now() + debounce;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(Ok(event)) => {
                for p in event.paths {
                    changed.insert(p);
                }
            }
            Ok(Err(_)) => {} // notify error, ignore
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }

    Some(changed)
}

/// Validates a single SKILL.md file and returns the result if the path qualifies.
///
/// Returns `None` if the path is not a SKILL.md file or cannot be read.
pub fn validate_changed_file(path: &Path, target: ValidationTarget) -> Option<ValidationResult> {
    let filename = path.file_name()?.to_str()?;
    if filename != "SKILL.md" {
        // A non-SKILL.md file changed; check if the parent directory contains a SKILL.md.
        let parent = path.parent()?;
        let skill_path = parent.join("SKILL.md");
        if skill_path.is_file() {
            let content = std::fs::read_to_string(&skill_path).ok()?;
            return Some(validate_skill(&skill_path, &content, target));
        }
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    Some(validate_skill(path, &content, target))
}

/// Formats and prints a single validation result with pass/fail indicators.
pub fn print_result(result: &ValidationResult) {
    let claude_icon = if result.claude_valid {
        "\x1b[32mPASS\x1b[0m"
    } else {
        "\x1b[31mFAIL\x1b[0m"
    };
    let codex_icon = if result.codex_valid {
        "\x1b[32mPASS\x1b[0m"
    } else {
        "\x1b[31mFAIL\x1b[0m"
    };
    let copilot_icon = if result.copilot_valid {
        "\x1b[32mPASS\x1b[0m"
    } else {
        "\x1b[31mFAIL\x1b[0m"
    };

    println!(
        "  {} [Claude: {}] [Codex: {}] [Copilot: {}]",
        result.path.display(),
        claude_icon,
        codex_icon,
        copilot_icon,
    );

    for issue in &result.issues {
        let severity_color = match issue.severity {
            crate::Severity::Error => "\x1b[31m",
            crate::Severity::Warning => "\x1b[33m",
            crate::Severity::Info => "\x1b[36m",
        };
        let location = match issue.line {
            Some(line) => format!(":{}", line),
            None => String::new(),
        };
        println!(
            "    {}{:?}\x1b[0m{}: {}",
            severity_color, issue.severity, location, issue.message
        );
    }
}

/// Prints a summary line for a set of results.
pub fn print_watch_summary(results: &[ValidationResult]) {
    let summary = ValidationSummary::from_results(results);
    let all_pass = summary.error_count == 0;
    let status = if all_pass {
        "\x1b[32mAll checks passed\x1b[0m"
    } else {
        "\x1b[31mValidation errors found\x1b[0m"
    };
    println!(
        "  {} ({} skills, {} errors, {} warnings)",
        status, summary.total, summary.error_count, summary.warning_count
    );
}

/// Runs the watch loop, blocking until Ctrl+C or the watcher is dropped.
///
/// On each batch of file changes (debounced), revalidates affected SKILL.md
/// files and prints results inline.
pub fn run_watch_loop(config: &WatchConfig) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let debounce = Duration::from_millis(config.debounce_ms);

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            let _ = tx.send(event);
        },
        NotifyConfig::default(),
    )?;

    for path in &config.paths {
        if path.exists() {
            watcher.watch(path.as_path(), RecursiveMode::Recursive)?;
            tracing::info!(path = %path.display(), "Watching directory");
        } else {
            tracing::warn!(path = %path.display(), "Watch path does not exist, skipping");
        }
    }

    println!(
        "\x1b[1mWatching {} director{} for changes (debounce: {}ms)...\x1b[0m",
        config.paths.len(),
        if config.paths.len() == 1 { "y" } else { "ies" },
        config.debounce_ms
    );
    println!("Press Ctrl+C to stop.\n");

    // Install Ctrl+C handler
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    ctrlc_channel(&stop_tx);

    loop {
        // Check for Ctrl+C between iterations
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Wait for file changes (with periodic check for Ctrl+C)
        match collect_debounced_paths_interruptible(&rx, &stop_rx, debounce) {
            WatchEvent::Changed(changed) => {
                let mut results = Vec::new();
                for path in &changed {
                    if let Some(result) = validate_changed_file(path, config.target) {
                        if !config.errors_only || result.has_errors() {
                            results.push(result);
                        }
                    }
                }

                if !results.is_empty() {
                    println!(
                        "\x1b[1m[{}] Revalidating {} file{}...\x1b[0m",
                        chrono_now(),
                        results.len(),
                        if results.len() == 1 { "" } else { "s" }
                    );
                    for result in &results {
                        print_result(result);
                    }
                    print_watch_summary(&results);
                    println!();
                }
            }
            WatchEvent::Stop => break,
        }
    }

    // Explicit drop to stop the watcher before exiting
    drop(watcher);
    println!("\nWatch mode stopped.");
    Ok(())
}

/// Result from the interruptible watch.
enum WatchEvent {
    /// File changes were detected.
    Changed(HashSet<PathBuf>),
    /// Stop signal received (Ctrl+C).
    Stop,
}

/// Waits for file changes or a stop signal, whichever comes first.
fn collect_debounced_paths_interruptible(
    rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    stop_rx: &mpsc::Receiver<()>,
    debounce: Duration,
) -> WatchEvent {
    // Poll both channels with a timeout so we can check for Ctrl+C
    loop {
        // Check stop signal
        if stop_rx.try_recv().is_ok() {
            return WatchEvent::Stop;
        }

        // Try to get a file event with a short timeout
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(first) => {
                let mut changed = HashSet::new();
                if let Ok(event) = first {
                    for p in event.paths {
                        changed.insert(p);
                    }
                }

                // Drain additional events within the debounce window
                let deadline = Instant::now() + debounce;
                loop {
                    if stop_rx.try_recv().is_ok() {
                        return WatchEvent::Stop;
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
                        Ok(Ok(event)) => {
                            for p in event.paths {
                                changed.insert(p);
                            }
                        }
                        Ok(Err(_)) => {}
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if Instant::now() >= deadline {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return WatchEvent::Stop;
                        }
                    }
                }

                return WatchEvent::Changed(changed);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return WatchEvent::Stop,
        }
    }
}

/// Sets up a Ctrl+C handler that sends on the given channel.
fn ctrlc_channel(tx: &mpsc::Sender<()>) {
    let tx = tx.clone();
    // Use a simple atomic + signal handler approach
    // that works without pulling in the `ctrlc` crate.
    let _ = std::thread::spawn(move || {
        // Block on SIGINT via libc
        #[cfg(unix)]
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static SIGNALED: AtomicBool = AtomicBool::new(false);

            unsafe {
                libc::signal(libc::SIGINT, sigint_handler as libc::sighandler_t);
            }

            extern "C" fn sigint_handler(_: libc::c_int) {
                SIGNALED.store(true, std::sync::atomic::Ordering::SeqCst);
            }

            loop {
                std::thread::sleep(Duration::from_millis(100));
                if SIGNALED.load(Ordering::SeqCst) {
                    let _ = tx.send(());
                    break;
                }
            }
        }

        #[cfg(not(unix))]
        {
            // On non-unix, just sleep forever; the watcher channel closing
            // will stop the loop.
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
    });
}

/// Returns a human-readable timestamp (HH:MM:SS).
fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    format!("{:02}:{:02}:{:02}", hours, mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_config_defaults() {
        let config = WatchConfig::new(
            vec![PathBuf::from("/tmp/skills")],
            ValidationTarget::All,
        );
        assert_eq!(config.debounce_ms, 300);
        assert!(!config.errors_only);
        assert_eq!(config.paths.len(), 1);
    }

    #[test]
    fn watch_config_builder() {
        let config = WatchConfig::new(
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            ValidationTarget::Codex,
        )
        .with_debounce_ms(500)
        .with_errors_only(true);

        assert_eq!(config.debounce_ms, 500);
        assert!(config.errors_only);
        assert_eq!(config.paths.len(), 2);
    }

    #[test]
    fn validate_changed_file_non_skill_returns_none() {
        // A path that doesn't exist and isn't SKILL.md
        let result = validate_changed_file(Path::new("/nonexistent/README.md"), ValidationTarget::All);
        assert!(result.is_none());
    }

    #[test]
    fn validate_changed_file_skill_md() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let skill_path = dir.path().join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: test-skill\ndescription: A test\n---\n# Test\nBody.",
        )
        .unwrap();

        let result = validate_changed_file(&skill_path, ValidationTarget::All);
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.claude_valid);
        assert!(result.codex_valid);
    }

    #[test]
    fn validate_changed_file_sibling_triggers_skill() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let skill_path = dir.path().join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: sibling-test\ndescription: A test\n---\n# Test\nBody.",
        )
        .unwrap();

        // A sibling file change should still trigger validation of SKILL.md
        let helper_path = dir.path().join("helper.sh");
        std::fs::write(&helper_path, "#!/bin/bash\necho hello").unwrap();

        let result = validate_changed_file(&helper_path, ValidationTarget::All);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.name, "sibling-test");
    }

    #[test]
    fn collect_debounced_paths_timeout() {
        let (_tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let debounce = Duration::from_millis(50);

        // With no events, recv will block; we use recv_timeout semantics inside
        // collect_debounced_paths which blocks on recv() first.
        // Test that it returns None when channel is dropped.
        let tx_clone = _tx;
        drop(tx_clone);

        let result = collect_debounced_paths(&rx, debounce);
        assert!(result.is_none());
    }

    #[test]
    fn collect_debounced_paths_aggregates_events() {
        let (tx, rx) = mpsc::channel();
        let debounce = Duration::from_millis(100);

        // Send two events with different paths
        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/a/SKILL.md")],
            attrs: Default::default(),
        }))
        .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/b/SKILL.md")],
            attrs: Default::default(),
        }))
        .unwrap();

        let result = collect_debounced_paths(&rx, debounce);
        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("/a/SKILL.md")));
        assert!(paths.contains(&PathBuf::from("/b/SKILL.md")));
    }

    #[test]
    fn collect_debounced_paths_deduplicates() {
        let (tx, rx) = mpsc::channel();
        let debounce = Duration::from_millis(100);

        // Send same path twice
        let path = PathBuf::from("/a/SKILL.md");
        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        }))
        .unwrap();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        }))
        .unwrap();

        let result = collect_debounced_paths(&rx, debounce);
        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn chrono_now_format() {
        let ts = chrono_now();
        // Should be HH:MM:SS format
        assert_eq!(ts.len(), 8);
        assert_eq!(&ts[2..3], ":");
        assert_eq!(&ts[5..6], ":");
    }
}
