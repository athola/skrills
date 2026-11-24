//! Guards the snapshot recorder: a sync-crate write, copy or removal that
//! bypasses the helpers in `adapters/utils.rs` is never recorded, so
//! `skrills sync-rollback` could not undo it.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Call shapes that mutate files. Directory creation is among them: a
/// directory the recorder never saw is one a rollback cannot remove.
const RAW_MUTATIONS: &[&str] = &[
    "fs::create_dir_all(",
    "fs::write(",
    "fs::copy(",
    "fs::rename(",
    "fs::remove_file(",
    "fs::remove_dir(",
    "fs::remove_dir_all(",
    "File::create(",
    "OpenOptions::new(",
    "File::options(",
    "fs::create_dir(",
    "set_permissions(",
    "DirBuilder::new(",
    "fs::symlink(",
    "fs::hard_link(",
];

/// Mutating functions of `std::fs` and `std::os::unix::fs` that a `use`
/// brings into scope, so a call names them bare, with no `fs::` to match.
const IMPORTABLE_MUTATIONS: &[&str] = &[
    "write",
    "copy",
    "rename",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "create_dir",
    "create_dir_all",
    "set_permissions",
    "hard_link",
    "symlink",
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
    // The snapshot's own directories, made private with `DirBuilder`.
    ("write_session.rs", "create_private_dir"),
    ("snapshot.rs", "begin_at"),
];

/// Returns `(line number, line)` for each raw mutation in non-test code
/// outside the functions named in `allowed_fns`.
///
/// Test code is skipped by item: a `#[cfg(test)]` attribute hides the item
/// under it, which ends at a `;` on its header line, with the header line
/// when it holds the whole item, or at the closing brace at the attribute's
/// own indentation (rustfmt guarantees these). An allowed
/// function is skipped the same way, from its `fn` line.
fn raw_mutations_in(source: &str, allowed_fns: &[&str]) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let imported = imported_mutations(&lines);
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
        let mutates = RAW_MUTATIONS.iter().any(|call| line.contains(call))
            || imported.iter().any(|name| calls_bare(line, name));
        if !is_comment && mutates {
            found.push((index + 1, line.trim().to_string()));
        }
        index += 1;
    }
    found
}

/// The local names a `use std::fs::...` or `use std::os::unix::fs::...` line
/// gives to the functions in [`IMPORTABLE_MUTATIONS`].
fn imported_mutations(lines: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        let Some(imports) = ["use std::fs::", "use std::os::unix::fs::"]
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
        else {
            continue;
        };
        let imports = imports
            .trim_end_matches(';')
            .trim_matches(|c| c == '{' || c == '}');
        for import in imports.split(',') {
            let mut parts = import.split(" as ").map(str::trim);
            let original = parts.next().unwrap_or_default();
            let local = parts.next().unwrap_or(original);
            if IMPORTABLE_MUTATIONS.contains(&original) {
                names.push(local.to_string());
            }
        }
    }
    names
}

/// Whether `line` calls `name` as a free function: not a method (`.name(`),
/// not a path (`x::name(`, which [`RAW_MUTATIONS`] covers for `fs::`), not
/// its definition, and not the tail of a longer name (`is_symlink(`).
fn calls_bare(line: &str, name: &str) -> bool {
    let call = format!("{name}(");
    line.match_indices(&call).any(|(at, _)| {
        let before = &line[..at];
        let joined = before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':'));
        !joined && !before.trim_end().ends_with("fn")
    })
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Index of the last line of the item whose header is at `start`.
///
/// A header that opens and closes its braces on its own line is the whole
/// item. Only that line is brace-counted: across lines a brace inside a
/// string literal would throw a counter off.
fn end_of_item(lines: &[&str], start: usize, indent: &str) -> usize {
    if start >= lines.len() || lines[start].trim_end().ends_with(';') {
        return start;
    }
    let opens = lines[start].matches('{').count();
    if opens > 0 && opens == lines[start].matches('}').count() {
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
fn scanner_flags_a_bare_call_imported_from_std_fs() {
    let source = "use std::fs::{read, remove_file};\nfn f() {\n    read(a).ok();\n    remove_file(a).ok();\n}\n";
    let found = raw_mutations_in(source, &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 4);
}

#[test]
fn scanner_flags_a_bare_symlink_imported_from_the_unix_fs_module() {
    let source = "use std::os::unix::fs::symlink;\nfn f() {\n    symlink(a, b).unwrap();\n}\n";
    assert_eq!(raw_mutations_in(source, &[]).len(), 1);
}

#[test]
fn scanner_does_not_take_a_method_or_a_definition_for_a_bare_call() {
    let source = "use std::fs::write;\nfn write(a: &str) {}\nfn f() {\n    file.write(b).ok();\n    meta.file_type().is_symlink();\n}\n";
    assert!(raw_mutations_in(source, &[]).is_empty());
}

#[test]
fn scanner_does_not_flag_the_dir_builder_extension_trait_import() {
    let source = "fn f() {\n    use std::os::unix::fs::DirBuilderExt;\n}\n";
    assert!(raw_mutations_in(source, &[]).is_empty());
}

#[test]
fn scanner_resumes_after_a_one_line_cfg_test_item() {
    let source = "#[cfg(test)]\nfn t() { std::fs::write(a, b).unwrap(); }\nfn live() {\n    std::fs::write(a, b).unwrap();\n}\n";
    let found = raw_mutations_in(source, &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 4);
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
