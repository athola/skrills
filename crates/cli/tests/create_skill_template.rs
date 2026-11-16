//! `create-skill --template` end to end: the real binary, the real validator.
//!
//! Templates are offline scaffolding, so none of these tests may need a
//! network or an LLM. A template run that tried either would hang or fail here.

use std::path::Path;
use std::process::{Command, Output};

const TEMPLATES: [&str; 5] = [
    "debugging",
    "code-review",
    "testing",
    "documentation",
    "refactoring",
];
const TARGETS: [&str; 4] = ["claude", "codex", "copilot", "cursor"];

fn run_skrills(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SKRILLS_SKILL_DIR")
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn skrills")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn template_list_prints_every_template_without_a_skill_name() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(home.path(), &["create-skill", "--template", "list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let listing = stdout(&output);
    for template in TEMPLATES {
        assert!(listing.contains(template), "{listing}");
    }
}

#[test]
fn template_list_as_json_has_one_entry_per_template() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(
        home.path(),
        &["create-skill", "--template", "list", "--format", "json"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(parsed["templates"].as_array().expect("array").len(), 5);
}

#[test]
fn generated_skill_passes_validate_for_every_template_and_target() {
    for template in TEMPLATES {
        for target in TARGETS {
            let home = tempfile::tempdir().expect("home");
            let skills = tempfile::tempdir().expect("skills");
            let skills_arg = skills.path().to_str().expect("utf8");
            let created = run_skrills(
                home.path(),
                &[
                    "create-skill",
                    "my-skill",
                    "--template",
                    template,
                    "--target",
                    target,
                    "--target-dir",
                    skills_arg,
                ],
            );
            assert!(
                created.status.success(),
                "{template}/{target}: {}",
                stderr(&created)
            );

            let validated = run_skrills(
                home.path(),
                &[
                    "validate",
                    "--skill-dir",
                    skills_arg,
                    "--target",
                    "all",
                    "--errors-only",
                ],
            );
            assert!(
                validated.status.success(),
                "{template}/{target} failed validate: {}{}",
                stdout(&validated),
                stderr(&validated)
            );
        }
    }
}

#[test]
fn template_without_target_dir_writes_under_the_target_cli_home() {
    let home = tempfile::tempdir().expect("home");
    let created = run_skrills(
        home.path(),
        &[
            "create-skill",
            "my-skill",
            "--template",
            "testing",
            "--target",
            "cursor",
        ],
    );
    assert!(created.status.success(), "{}", stderr(&created));
    assert!(home
        .path()
        .join(".cursor/skills/my-skill/SKILL.md")
        .is_file());
}

#[test]
fn template_dry_run_prints_the_skill_and_writes_nothing() {
    let home = tempfile::tempdir().expect("home");
    let skills = tempfile::tempdir().expect("skills");
    let output = run_skrills(
        home.path(),
        &[
            "create-skill",
            "my-skill",
            "--template",
            "debugging",
            "--dry-run",
            "--target-dir",
            skills.path().to_str().expect("utf8"),
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("name: my-skill"),
        "{}",
        stdout(&output)
    );
    assert_eq!(std::fs::read_dir(skills.path()).expect("read").count(), 0);
}

#[test]
fn unknown_template_fails_and_lists_the_valid_names() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(
        home.path(),
        &["create-skill", "my-skill", "--template", "nope"],
    );
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("nope"), "{message}");
    for template in TEMPLATES {
        assert!(message.contains(template), "{message}");
    }
}

#[test]
fn template_with_method_is_rejected_as_a_conflict() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(
        home.path(),
        &[
            "create-skill",
            "my-skill",
            "--template",
            "testing",
            "--method",
            "llm",
        ],
    );
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--method"), "{}", stderr(&output));
}

#[test]
fn template_requires_a_skill_name_unless_listing() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(home.path(), &["create-skill", "--template", "testing"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("NAME"), "{}", stderr(&output));
}

#[test]
fn without_a_template_description_is_still_required() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(home.path(), &["create-skill", "my-skill"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--description"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn target_without_a_template_is_rejected() {
    let home = tempfile::tempdir().expect("home");
    let output = run_skrills(
        home.path(),
        &[
            "create-skill",
            "my-skill",
            "--description",
            "x",
            "--target",
            "codex",
        ],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--template"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn rerunning_a_template_keeps_the_existing_skill_and_fails() {
    let home = tempfile::tempdir().expect("home");
    let skills = tempfile::tempdir().expect("skills");
    let skills_arg = skills.path().to_str().expect("utf8");
    let args = [
        "create-skill",
        "my-skill",
        "--template",
        "testing",
        "--target-dir",
        skills_arg,
    ];
    assert!(run_skrills(home.path(), &args).status.success());
    let skill_path = skills.path().join("my-skill/SKILL.md");
    std::fs::write(&skill_path, "hand-edited").expect("edit");

    let second = run_skrills(home.path(), &args);
    assert!(!second.status.success());
    assert!(
        stderr(&second).contains("already exists"),
        "{}",
        stderr(&second)
    );
    assert_eq!(
        std::fs::read_to_string(&skill_path).expect("read"),
        "hand-edited"
    );
}
