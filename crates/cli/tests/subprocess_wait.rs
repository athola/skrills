//! Guards that the CLI process can still wait on child processes.
//!
//! `run()` once installed `SIG_IGN | SA_NOCLDWAIT` for SIGCHLD, which makes
//! `waitpid` return `ECHILD`, so every `Command::status()`/`output()` in the
//! process failed with "No child processes". Unit tests never saw it: they
//! call the command functions directly and never go through `run()`. The
//! visible symptom was `analyze-project-context` returning an empty
//! `git_keywords` list in a repo with history, because the `git log` it
//! shells out to could not be waited on. This test drives the real binary.

use std::process::Command;

use anyhow::{Context, Result};

fn git(repo: &std::path::Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=skrills-test",
            "-c",
            "user.email=skrills-test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repo)
        .status()
        .with_context(|| format!("git {args:?}"))?;
    anyhow::ensure!(status.success(), "git {args:?} failed");
    Ok(())
}

#[test]
fn analyze_project_context_extracts_git_keywords_through_cli_dispatch() -> Result<()> {
    let _g = skrills_test_utils::env_guard();
    let tmp = tempfile::tempdir()?;
    let _home = skrills_test_utils::set_env_var("HOME", Some(tmp.path().to_str().unwrap()));

    // GIVEN a repository whose history carries distinctive words
    let repo = tmp.path().join("project");
    std::fs::create_dir_all(&repo)?;
    git(&repo, &["init", "-q"])?;
    std::fs::write(repo.join("main.py"), "print('hi')\n")?;
    git(&repo, &["add", "main.py"])?;
    git(
        &repo,
        &[
            "commit",
            "-q",
            "-m",
            "feat: implement telemetry ingestion pipeline",
        ],
    )?;

    // WHEN the real binary analyses it with git keywords enabled
    let output = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args([
            "analyze-project-context",
            "--project-dir",
            repo.to_str().unwrap(),
            "--include-git",
            "true",
            "--format",
            "json",
        ])
        .output()
        .context("spawn skrills analyze-project-context")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::ensure!(
        output.status.success(),
        "exit {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );

    // THEN the git log subprocess was waited on and its words came back.
    // Startup notices and log lines precede the JSON document on stdout.
    let json_start = stdout
        .find("\n{")
        .map(|i| i + 1)
        .or_else(|| stdout.starts_with('{').then_some(0))
        .with_context(|| format!("no JSON document on stdout: {stdout}"))?;
    let context: serde_json::Value = serde_json::from_str(&stdout[json_start..])
        .with_context(|| format!("stdout not JSON: {stdout}"))?;
    let keywords: Vec<&str> = context["git_keywords"]
        .as_array()
        .with_context(|| format!("git_keywords missing: {context}"))?
        .iter()
        .filter_map(|k| k.as_str())
        .collect();
    anyhow::ensure!(
        keywords.contains(&"telemetry"),
        "git_keywords should carry commit words (the process cannot wait on children?): {keywords:?}\nstderr: {stderr}"
    );
    Ok(())
}
