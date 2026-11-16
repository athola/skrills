//! Guards the snapshot recorder: a sync-crate write, copy or removal that
//! bypasses the helpers in `adapters/utils.rs` is never recorded, so
//! `skrills sync-rollback` could not undo it.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Call shapes that mutate files. Directory creation is absent on purpose:
/// an empty directory holds nothing a rollback needs to restore.
const RAW_MUTATIONS: &[&str] = &[
    "fs::write(",
    "fs::copy(",
    "fs::rename(",
    "fs::remove_file(",
    "fs::remove_dir(",
    "fs::remove_dir_all(",
    "File::create(",
    "OpenOptions::new(",
];

/// Files that may call `std::fs` directly: the helpers themselves, and the
/// kill-switch pid marker, which is process state and not user configuration.
const ALLOWLIST: &[&str] = &["adapters/utils.rs", "kill_switch_marker.rs"];

/// Single functions, by file, that may open files directly. Each opens a file
/// that is not user content, and recording it would make a rollback undo it.
const ALLOWED_FUNCTIONS: &[(&str, &str)] = &[
    // The lock file coordinates concurrent syncs of one target. It is not
    // user content, and rolling it back would delete a lock another sync may
    // hold, so it must never be recorded.
    ("orchestrator.rs", "lock_target"),
    // The snapshot's own manifest; recording it would snapshot the snapshot.
    ("write_session.rs", "open_manifest"),
];

/// Returns `(line number, line)` for each raw mutation in non-test code
/// outside the functions named in `allowed_fns`.
///
/// Test code is skipped by item: a `#[cfg(test)]` attribute hides the item
/// under it, which ends at a `;` on its header line or at the closing brace
/// at the attribute's own indentation (rustfmt guarantees both). An allowed
/// function is skipped the same way, from its `fn` line.
fn raw_mutations_in(source: &str, allowed_fns: &[&str]) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.trim() == "#[cfg(test)]" {
            index += 1;
            while index < lines.len() && lines[index].trim_start().starts_with("#[") {
                index += 1;
            }
            index = end_of_item(&lines, index, indent_of(line)) + 1;
            continue;
        }
        if allowed_fns.iter().any(|name| declares_fn(line, name)) {
            index = end_of_item(&lines, index, indent_of(line)) + 1;
            continue;
        }
        let is_comment = line.trim_start().starts_with("//");
        if !is_comment && RAW_MUTATIONS.iter().any(|call| line.contains(call)) {
            found.push((index + 1, line.trim().to_string()));
        }
        index += 1;
    }
    found
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Index of the last line of the item whose header is at `start`.
fn end_of_item(lines: &[&str], start: usize, indent: &str) -> usize {
    if start >= lines.len() || lines[start].trim_end().ends_with(';') {
        return start;
    }
    let closing = format!("{indent}}}");
    let mut index = start;
    while index < lines.len() && lines[index].trim_end() != closing {
        index += 1;
    }
    index
}

/// Whether `line` is the header of a function called `name`, with any
/// visibility in front.
fn declares_fn(line: &str, name: &str) -> bool {
    let header = format!("fn {name}(");
    let trimmed = line.trim_start();
    trimmed.starts_with(&header)
        || (trimmed.starts_with("pub") && trimmed.contains(&format!(" {header}")))
}

fn allowed_fns_for(path: &Path) -> Vec<&'static str> {
    ALLOWED_FUNCTIONS
        .iter()
        .filter(|(file, _)| path.ends_with(file))
        .map(|(_, name)| *name)
        .collect()
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name == "tests.rs" || name == "tests_common.rs"
}

fn is_allowlisted(path: &Path) -> bool {
    ALLOWLIST.iter().any(|allowed| path.ends_with(allowed))
}

#[test]
fn sync_crate_has_no_raw_fs_mutation_outside_the_recording_helpers() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in WalkDir::new(&src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let is_rust = path.extension().is_some_and(|ext| ext == "rs");
        if !is_rust || is_test_file(path) || is_allowlisted(path) {
            continue;
        }
        let source = std::fs::read_to_string(path).expect("read sync source");
        for (number, line) in raw_mutations_in(&source, &allowed_fns_for(path)) {
            offenders.push(format!("{}:{number}: {line}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "raw fs mutation bypasses the snapshot recorder; use the helpers in adapters/utils.rs:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn scanner_flags_each_mutation_shape() {
    for call in RAW_MUTATIONS {
        let source = format!("fn go() {{ std::{call}a); }}\n");
        assert_eq!(
            raw_mutations_in(&source, &[]).len(),
            1,
            "{call} not flagged"
        );
    }
}

#[test]
fn scanner_ignores_an_inline_cfg_test_module() {
    let source = "fn keep() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        std::fs::write(a, b).unwrap();\n    }\n}\nfn after() {\n    std::fs::remove_file(a).ok();\n}\n";
    let found = raw_mutations_in(source, &[]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 9);
}

#[test]
fn scanner_ignores_an_indented_cfg_test_item() {
    let source = "impl A {\n    #[cfg(test)]\n    fn t() {\n        std::fs::copy(a, b).unwrap();\n    }\n    fn live() {\n        std::fs::copy(a, b).unwrap();\n    }\n}\n";
    let found = raw_mutations_in(source, &[]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 7);
}

#[test]
fn scanner_resumes_after_an_external_cfg_test_module_declaration() {
    let source = "#[cfg(test)]\nmod tests;\nfn live() {\n    std::fs::rename(a, b).unwrap();\n}\n";
    assert_eq!(raw_mutations_in(source, &[]).len(), 1);
}

#[test]
fn scanner_skips_attributes_between_cfg_test_and_its_item() {
    let source = "#[cfg(test)]\n#[derive(Debug)]\nmod tests {\n    fn t() { std::fs::write(a, b).unwrap(); }\n}\n";
    assert!(raw_mutations_in(source, &[]).is_empty());
}

#[test]
fn scanner_ignores_comments_that_mention_a_raw_call() {
    let source =
        "/// Unlike `std::fs::write`, this is atomic.\n// fs::remove_file( is wrapped\nfn f() {}\n";
    assert!(raw_mutations_in(source, &[]).is_empty());
}

#[test]
fn allowlist_matches_by_path_suffix() {
    assert!(is_allowlisted(Path::new("/x/src/adapters/utils.rs")));
    assert!(is_allowlisted(Path::new("/x/src/kill_switch_marker.rs")));
    assert!(!is_allowlisted(Path::new(
        "/x/src/adapters/cursor/utils.rs"
    )));
}

#[test]
fn scanner_skips_only_the_allowed_function() {
    let source = "fn lock_target(root: &Path) -> File {\n    std::fs::OpenOptions::new().open(root)\n}\nfn other() {\n    std::fs::OpenOptions::new().open(a);\n}\n";
    let found = raw_mutations_in(source, &["lock_target"]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 5);
}

#[test]
fn scanner_matches_an_allowed_function_with_visibility() {
    let source = "    pub(crate) fn open_manifest(p: &Path) {\n        std::fs::OpenOptions::new();\n    }\n";
    assert!(raw_mutations_in(source, &["open_manifest"]).is_empty());
}

#[test]
fn scanner_does_not_treat_a_call_to_an_allowed_function_as_its_body() {
    let source = "fn sync() {\n    let f = lock_target(root)?;\n    std::fs::OpenOptions::new().open(a);\n}\n";
    assert_eq!(raw_mutations_in(source, &["lock_target"]).len(), 1);
}

#[test]
fn scanner_ignores_the_open_options_extension_trait_import() {
    let source = "fn f() {\n    use std::os::unix::fs::OpenOptionsExt;\n}\n";
    assert!(raw_mutations_in(source, &[]).is_empty());
}

#[test]
fn allowed_functions_are_scoped_to_their_file() {
    assert_eq!(
        allowed_fns_for(Path::new("/x/src/orchestrator.rs")),
        ["lock_target"]
    );
    assert!(allowed_fns_for(Path::new("/x/src/adapters/codex.rs")).is_empty());
}
