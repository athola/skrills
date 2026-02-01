//! Language and framework detection from project files.

use super::{
    dependencies::{parse_cargo_toml, parse_package_json, parse_pyproject_toml},
    git_context::extract_git_keywords,
    DependencyInfo, LanguageInfo, ProjectProfile, ProjectType,
};
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Maximum directory depth when scanning for language files.
const MAX_DIRECTORY_SCAN_DEPTH: usize = 8;
/// Language extension mappings.
const LANGUAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "Rust"),
    ("py", "Python"),
    ("pyw", "Python"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("js", "JavaScript"),
    ("jsx", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("go", "Go"),
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("kts", "Kotlin"),
    ("rb", "Ruby"),
    ("c", "C"),
    ("cpp", "C++"),
    ("cc", "C++"),
    ("cxx", "C++"),
    ("h", "C"),
    ("hpp", "C++"),
    ("hxx", "C++"),
    ("cs", "C#"),
    ("swift", "Swift"),
    ("php", "PHP"),
    ("scala", "Scala"),
    ("zig", "Zig"),
    ("lua", "Lua"),
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("erl", "Erlang"),
    ("hrl", "Erlang"),
    ("clj", "Clojure"),
    ("cljs", "ClojureScript"),
    ("hs", "Haskell"),
    ("ml", "OCaml"),
    ("mli", "OCaml"),
    ("fs", "F#"),
    ("fsx", "F#"),
    ("r", "R"),
    ("jl", "Julia"),
    ("dart", "Dart"),
    ("nim", "Nim"),
    ("v", "V"),
    ("cr", "Crystal"),
];

/// Directories to skip during analysis.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".tox",
    ".mypy_cache",
    ".ruff_cache",
    "coverage",
    ".nyc_output",
    ".next",
    ".nuxt",
];

/// Options for project analysis.
#[derive(Debug, Clone, Copy)]
pub struct AnalyzeProjectOptions {
    /// Whether to include git commit keyword analysis.
    pub include_git: bool,
    /// Number of recent commits to analyze when include_git is true.
    pub commit_limit: usize,
    /// Maximum number of languages to include in results.
    /// Languages are sorted by file count. Set to 0 for unlimited.
    pub max_languages: usize,
}

impl Default for AnalyzeProjectOptions {
    fn default() -> Self {
        Self {
            include_git: true,
            commit_limit: 50,
            max_languages: 10,
        }
    }
}

/// Analyze a project directory and build a profile.
pub fn analyze_project(root: &Path) -> Result<ProjectProfile> {
    analyze_project_with_options(root, AnalyzeProjectOptions::default())
}

/// Analyze a project directory with explicit options.
pub fn analyze_project_with_options(
    root: &Path,
    options: AnalyzeProjectOptions,
) -> Result<ProjectProfile> {
    let mut profile = ProjectProfile {
        root: root.to_path_buf(),
        ..Default::default()
    };

    // Detect languages from file extensions
    let all_languages = detect_languages(root)?;

    // Apply max_languages limit if set
    profile.languages = if options.max_languages > 0 && all_languages.len() > options.max_languages
    {
        // Sort by file count and take top N
        let mut sorted: Vec<_> = all_languages.into_iter().collect();
        sorted.sort_by(|a, b| b.1.file_count.cmp(&a.1.file_count));
        sorted.truncate(options.max_languages);
        sorted.into_iter().collect()
    } else {
        all_languages
    };

    // Parse dependency files
    profile.dependencies = parse_all_dependencies(root)?;

    // Detect frameworks from dependencies
    profile.frameworks = detect_frameworks(&profile.dependencies);

    // Parse README for description and keywords
    match parse_readme(root) {
        Ok((desc, keywords)) => {
            profile.description = desc;
            profile.keywords = keywords;
        }
        Err(e) => {
            tracing::debug!(error = %e, "Could not parse README for project context");
        }
    }

    // Extract git commit keywords
    if options.include_git {
        match extract_git_keywords(root, options.commit_limit) {
            Ok(git_keywords) => profile.git_keywords = git_keywords,
            Err(e) => {
                tracing::debug!(error = %e, "Could not extract git keywords");
            }
        }
    }

    // Classify project type
    profile.project_type = classify_project_type(root, &profile);

    Ok(profile)
}

/// Detect programming languages from file extensions.
pub fn detect_languages(root: &Path) -> Result<HashMap<String, LanguageInfo>> {
    let mut counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for entry in WalkDir::new(root)
        .max_depth(MAX_DIRECTORY_SCAN_DEPTH)
        .into_iter()
        .filter_entry(|e| {
            // Don't filter the root directory itself
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref())
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                for (lang_ext, lang_name) in LANGUAGE_EXTENSIONS {
                    if ext.eq_ignore_ascii_case(lang_ext) {
                        let entry = counts.entry((*lang_name).to_string()).or_default();
                        entry.0 += 1;
                        if !entry.1.contains(&ext.to_lowercase()) {
                            entry.1.push(ext.to_lowercase());
                        }
                        break;
                    }
                }
            }
        }
    }

    // Find the language with the most files
    let max_count = counts.values().map(|(c, _)| *c).max().unwrap_or(0);

    Ok(counts
        .into_iter()
        .map(|(name, (count, exts))| {
            (
                name,
                LanguageInfo {
                    file_count: count,
                    extensions: exts,
                    primary: count == max_count && max_count > 0,
                },
            )
        })
        .collect())
}

/// Parse all dependency files in a project.
fn parse_all_dependencies(root: &Path) -> Result<HashMap<String, Vec<DependencyInfo>>> {
    let mut deps = HashMap::new();

    // Rust: Cargo.toml
    let cargo_path = root.join("Cargo.toml");
    if cargo_path.exists() {
        match parse_cargo_toml(&cargo_path) {
            Ok(rust_deps) => {
                deps.insert("rust".to_string(), rust_deps);
            }
            Err(e) => {
                tracing::debug!(error = %e, path = %cargo_path.display(), "Could not parse Cargo.toml");
            }
        }
    }

    // Node.js: package.json
    let package_path = root.join("package.json");
    if package_path.exists() {
        match parse_package_json(&package_path) {
            Ok(npm_deps) => {
                deps.insert("npm".to_string(), npm_deps);
            }
            Err(e) => {
                tracing::debug!(error = %e, path = %package_path.display(), "Could not parse package.json");
            }
        }
    }

    // Python: pyproject.toml
    let pyproject_path = root.join("pyproject.toml");
    if pyproject_path.exists() {
        match parse_pyproject_toml(&pyproject_path) {
            Ok(py_deps) => {
                deps.insert("python".to_string(), py_deps);
            }
            Err(e) => {
                tracing::debug!(error = %e, path = %pyproject_path.display(), "Could not parse pyproject.toml");
            }
        }
    }

    // Python: requirements.txt (fallback)
    if !deps.contains_key("python") {
        let requirements_path = root.join("requirements.txt");
        if requirements_path.exists() {
            match parse_requirements_txt(&requirements_path) {
                Ok(py_deps) => {
                    deps.insert("python".to_string(), py_deps);
                }
                Err(e) => {
                    tracing::debug!(error = %e, path = %requirements_path.display(), "Could not parse requirements.txt");
                }
            }
        }
    }

    Ok(deps)
}

/// Parse requirements.txt for Python dependencies.
fn parse_requirements_txt(path: &Path) -> Result<Vec<DependencyInfo>> {
    let content = fs::read_to_string(path)?;
    let mut deps = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }

        // Parse name and optional version specifier
        let (name, version) = if let Some(idx) = line.find(|c| ['=', '<', '>'].contains(&c)) {
            let name = line[..idx].trim();
            let version = line[idx..].trim();
            (name.to_string(), Some(version.to_string()))
        } else {
            (line.to_string(), None)
        };

        if !name.is_empty() {
            deps.push(DependencyInfo {
                name,
                version,
                dev: false,
            });
        }
    }

    Ok(deps)
}

/// Detect frameworks from project dependencies.
pub fn detect_frameworks(deps: &HashMap<String, Vec<DependencyInfo>>) -> Vec<String> {
    let mut frameworks = Vec::new();

    // Framework patterns: (dependency pattern, framework name)
    let known_frameworks: &[(&str, &str)] = &[
        // JavaScript/TypeScript
        ("react", "React"),
        ("next", "Next.js"),
        ("vue", "Vue"),
        ("nuxt", "Nuxt"),
        ("angular", "Angular"),
        ("svelte", "Svelte"),
        ("express", "Express"),
        ("fastify", "Fastify"),
        ("koa", "Koa"),
        ("nestjs", "NestJS"),
        ("jest", "Jest"),
        ("vitest", "Vitest"),
        ("mocha", "Mocha"),
        ("playwright", "Playwright"),
        ("cypress", "Cypress"),
        // Python
        ("fastapi", "FastAPI"),
        ("django", "Django"),
        ("flask", "Flask"),
        ("pytest", "pytest"),
        ("pandas", "pandas"),
        ("numpy", "NumPy"),
        ("tensorflow", "TensorFlow"),
        ("torch", "PyTorch"),
        ("scikit-learn", "scikit-learn"),
        // Rust
        ("actix-web", "Actix"),
        ("axum", "Axum"),
        ("rocket", "Rocket"),
        ("tokio", "Tokio"),
        ("async-std", "async-std"),
        ("serde", "Serde"),
        ("clap", "Clap"),
        ("tracing", "tracing"),
        // Go
        ("gin-gonic", "Gin"),
        ("echo", "Echo"),
        ("fiber", "Fiber"),
    ];

    for deps_list in deps.values() {
        for dep in deps_list {
            let dep_lower = dep.name.to_lowercase();
            for (pattern, framework) in known_frameworks {
                if dep_lower.contains(pattern) && !frameworks.contains(&framework.to_string()) {
                    frameworks.push(framework.to_string());
                }
            }
        }
    }

    frameworks.sort();
    frameworks
}

/// Parse README file for description and keywords.
fn parse_readme(root: &Path) -> Result<(Option<String>, Vec<String>)> {
    // Use case-insensitive matching to avoid duplicating entries
    let readme_patterns = ["readme.md", "readme", "readme.rst"];

    // Read directory entries once and match case-insensitively
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(|e| e.ok()) {
            let file_name = entry.file_name();
            let name_lower = file_name.to_string_lossy().to_lowercase();
            for pattern in readme_patterns {
                if name_lower == pattern {
                    let content = fs::read_to_string(entry.path())?;
                    return Ok(parse_readme_content(&content));
                }
            }
        }
    }

    anyhow::bail!("No README found")
}

/// Parse README content for description and keywords.
fn parse_readme_content(content: &str) -> (Option<String>, Vec<String>) {
    let mut description = None;
    let mut keywords = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    // Extract first paragraph after title as description
    let mut in_description = false;
    let mut desc_lines = Vec::new();

    for line in &lines {
        // Skip badges and images
        if line.contains("![") || line.contains("](http") {
            continue;
        }

        if line.starts_with('#') && description.is_none() {
            in_description = true;
            continue;
        }

        if in_description {
            if line.is_empty() && !desc_lines.is_empty() {
                description = Some(desc_lines.join(" ").trim().to_string());
                break;
            }
            if !line.starts_with('#') && !line.is_empty() && !line.starts_with('[') {
                desc_lines.push(line.trim());
            }
        }
    }

    // If we didn't find a description, use first non-empty paragraph
    if description.is_none() && !desc_lines.is_empty() {
        description = Some(desc_lines.join(" ").trim().to_string());
    }

    // Truncate description if too long
    if let Some(ref mut desc) = description {
        if desc.len() > 500 {
            *desc = desc.chars().take(500).collect::<String>() + "...";
        }
    }

    // Extract keywords from headings
    for line in &lines {
        if line.starts_with('#') {
            let heading = line.trim_start_matches('#').trim().to_lowercase();
            let words: Vec<_> = heading
                .split_whitespace()
                .filter(|w| w.len() >= 3)
                .map(|s| s.to_string())
                .collect();
            keywords.extend(words);
        }
    }

    // Deduplicate keywords
    keywords.sort();
    keywords.dedup();

    (description, keywords)
}

/// Classify the project type based on structure and profile.
fn classify_project_type(root: &Path, profile: &ProjectProfile) -> ProjectType {
    // Check for monorepo markers
    if root.join("lerna.json").exists()
        || root.join("pnpm-workspace.yaml").exists()
        || root.join("nx.json").exists()
        || (root.join("Cargo.toml").exists()
            && fs::read_to_string(root.join("Cargo.toml"))
                .map(|c| c.contains("[workspace]"))
                .unwrap_or(false))
    {
        return ProjectType::Monorepo;
    }

    // Check for plugin markers
    if root.join("plugin.json").exists()
        || root.join(".claude-plugin").exists()
        || profile
            .keywords
            .iter()
            .any(|k| k.contains("plugin") || k.contains("extension"))
    {
        return ProjectType::Plugin;
    }

    // Check for service markers
    if root.join("Dockerfile").exists()
        || root.join("docker-compose.yml").exists()
        || root.join("docker-compose.yaml").exists()
        || profile.frameworks.iter().any(|f| {
            f == "Express"
                || f == "FastAPI"
                || f == "Django"
                || f == "Actix"
                || f == "Axum"
                || f == "Gin"
        })
    {
        return ProjectType::Service;
    }

    // Check for library markers
    if root.join("src/lib.rs").exists()
        || (profile.dependencies.contains_key("rust")
            && fs::read_to_string(root.join("Cargo.toml"))
                .map(|c| c.contains("[lib]"))
                .unwrap_or(false))
    {
        return ProjectType::Library;
    }

    // Check for application markers
    if root.join("src/main.rs").exists()
        || root.join("src/main.py").exists()
        || root.join("src/index.ts").exists()
        || root.join("src/index.js").exists()
        || root.join("app.py").exists()
    {
        return ProjectType::Application;
    }

    ProjectType::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "Test User")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test User")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("run git command");
        assert!(status.success(), "git command failed: {:?}", args);
    }

    fn commit_file(root: &Path, message: &str, content: &str) {
        let file_path = root.join("README.md");
        fs::write(&file_path, content).expect("write file");
        run_git(root, &["add", "README.md"]);
        run_git(
            root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                message,
            ],
        );
    }

    #[test]
    fn analyze_project_skips_git_when_disabled() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init"]);
        commit_file(temp.path(), "alphaunique", "alpha");

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 1,
            max_languages: 10,
        };
        let profile = analyze_project_with_options(temp.path(), options).unwrap();

        assert!(
            profile.git_keywords.is_empty(),
            "git keywords should be empty when include_git is false"
        );
    }

    #[test]
    fn analyze_project_respects_commit_limit() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init"]);
        commit_file(temp.path(), "alphaunique", "alpha");
        commit_file(temp.path(), "betaunique", "beta");

        let options = AnalyzeProjectOptions {
            include_git: true,
            commit_limit: 1,
            max_languages: 10,
        };
        let profile = analyze_project_with_options(temp.path(), options).unwrap();

        let keywords = profile
            .git_keywords
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert!(
            keywords.contains(&"betaunique"),
            "expected latest commit keyword in git keywords"
        );
        assert!(
            !keywords.contains(&"alphaunique"),
            "older commit keywords should be excluded when commit_limit is 1"
        );
    }

    #[test]
    fn test_detect_languages_empty_dir() {
        let tmp = tempdir().unwrap();
        let languages = detect_languages(tmp.path()).unwrap();
        assert!(languages.is_empty());
    }

    #[test]
    fn test_detect_languages_rust() {
        let tmp = tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn foo() {}").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();
        assert!(languages.contains_key("Rust"));
        assert_eq!(languages["Rust"].file_count, 2);
        assert!(languages["Rust"].primary);
    }

    #[test]
    fn test_detect_frameworks() {
        let mut deps = HashMap::new();
        deps.insert(
            "npm".to_string(),
            vec![
                DependencyInfo {
                    name: "react".to_string(),
                    version: Some("^18.0.0".to_string()),
                    dev: false,
                },
                DependencyInfo {
                    name: "jest".to_string(),
                    version: None,
                    dev: true,
                },
            ],
        );

        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.contains(&"React".to_string()));
        assert!(frameworks.contains(&"Jest".to_string()));
    }

    #[test]
    fn test_classify_monorepo() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("lerna.json"), "{}").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);
        assert_eq!(project_type, ProjectType::Monorepo);
    }

    #[test]
    fn test_parse_readme_content() {
        let content = r#"# My Project

This is a great project for testing things.

## Features

- Feature 1
- Feature 2
"#;

        let (desc, keywords) = parse_readme_content(content);
        assert!(desc.is_some());
        assert!(desc.unwrap().contains("great project"));
        assert!(keywords.contains(&"features".to_string()));
    }

    // ============================================
    // Additional tests for >90% coverage (Issue #60)
    // ============================================

    #[test]
    fn test_detect_languages_multiple_languages() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        // Rust files (more files, should be primary)
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.rs"), "pub fn x() {}").unwrap();
        fs::write(src.join("util.rs"), "pub fn y() {}").unwrap();

        // Python files (fewer)
        fs::write(src.join("script.py"), "print('hi')").unwrap();

        // JavaScript files
        fs::write(src.join("index.js"), "console.log()").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();

        assert!(languages.contains_key("Rust"));
        assert!(languages.contains_key("Python"));
        assert!(languages.contains_key("JavaScript"));

        assert_eq!(languages["Rust"].file_count, 3);
        assert!(languages["Rust"].primary);
        assert!(!languages["Python"].primary);
        assert!(!languages["JavaScript"].primary);
    }

    #[test]
    fn test_detect_languages_skips_hidden_dirs() {
        let tmp = tempdir().unwrap();

        // File in hidden directory should be skipped
        let hidden = tmp.path().join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("secret.rs"), "fn x() {}").unwrap();

        // File in node_modules should be skipped
        let node_modules = tmp.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("dep.js"), "x()").unwrap();

        // File in target should be skipped
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("build.rs"), "fn x() {}").unwrap();

        // Only visible file
        fs::write(tmp.path().join("main.py"), "print()").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();

        assert_eq!(languages.len(), 1);
        assert!(languages.contains_key("Python"));
        assert!(!languages.contains_key("Rust"));
        assert!(!languages.contains_key("JavaScript"));
    }

    #[test]
    fn test_detect_languages_case_insensitive_extensions() {
        let tmp = tempdir().unwrap();

        fs::write(tmp.path().join("file.RS"), "fn main() {}").unwrap();
        fs::write(tmp.path().join("file.Py"), "print()").unwrap();
        fs::write(tmp.path().join("file.JS"), "x()").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();

        assert!(languages.contains_key("Rust"));
        assert!(languages.contains_key("Python"));
        assert!(languages.contains_key("JavaScript"));
    }

    #[test]
    fn test_detect_languages_typescript_variants() {
        let tmp = tempdir().unwrap();

        fs::write(tmp.path().join("file.ts"), "const x: number = 1;").unwrap();
        fs::write(tmp.path().join("component.tsx"), "<div/>").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();

        assert!(languages.contains_key("TypeScript"));
        assert_eq!(languages["TypeScript"].file_count, 2);
        assert!(languages["TypeScript"]
            .extensions
            .contains(&"ts".to_string()));
        assert!(languages["TypeScript"]
            .extensions
            .contains(&"tsx".to_string()));
    }

    #[test]
    fn test_detect_languages_cpp_variants() {
        let tmp = tempdir().unwrap();

        fs::write(tmp.path().join("file.cpp"), "int main() {}").unwrap();
        fs::write(tmp.path().join("file.cc"), "int main() {}").unwrap();
        fs::write(tmp.path().join("file.hpp"), "#pragma once").unwrap();

        let languages = detect_languages(tmp.path()).unwrap();

        assert!(languages.contains_key("C++"));
        assert_eq!(languages["C++"].file_count, 3);
    }

    #[test]
    fn test_analyze_project_options_default() {
        let opts = AnalyzeProjectOptions::default();
        assert!(opts.include_git);
        assert_eq!(opts.commit_limit, 50);
        assert_eq!(opts.max_languages, 10);
    }

    #[test]
    fn test_analyze_project_max_languages_limit() {
        let tmp = tempdir().unwrap();

        // Create files in many languages to exceed the limit
        let langs = [
            ("file.rs", "Rust"),
            ("file.py", "Python"),
            ("file.js", "JavaScript"),
            ("file.ts", "TypeScript"),
            ("file.go", "Go"),
            ("file.java", "Java"),
            ("file.rb", "Ruby"),
            ("file.php", "PHP"),
            ("file.swift", "Swift"),
            ("file.c", "C"),
            ("file.cpp", "C++"),
            ("file.cs", "C#"),
        ];

        for (filename, _) in &langs {
            fs::write(tmp.path().join(filename), "// code").unwrap();
        }

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 3,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert_eq!(profile.languages.len(), 3);
    }

    #[test]
    fn test_classify_project_type_pnpm_workspace() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("pnpm-workspace.yaml"),
            "packages:\n  - 'packages/*'",
        )
        .unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Monorepo);
    }

    #[test]
    fn test_classify_project_type_nx() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("nx.json"), "{}").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Monorepo);
    }

    #[test]
    fn test_classify_project_type_cargo_workspace() {
        let tmp = tempdir().unwrap();
        let cargo = r#"
[workspace]
members = ["crates/*"]
"#;
        fs::write(tmp.path().join("Cargo.toml"), cargo).unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Monorepo);
    }

    #[test]
    fn test_classify_project_type_plugin() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("plugin.json"), "{}").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Plugin);
    }

    #[test]
    fn test_classify_project_type_claude_plugin() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(".claude-plugin"), "").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Plugin);
    }

    #[test]
    fn test_classify_project_type_plugin_from_keywords() {
        let tmp = tempdir().unwrap();

        let profile = ProjectProfile {
            keywords: vec!["plugin".to_string(), "cli".to_string()],
            ..Default::default()
        };

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Plugin);
    }

    #[test]
    fn test_classify_project_type_extension_keyword() {
        let tmp = tempdir().unwrap();

        let profile = ProjectProfile {
            keywords: vec!["extension".to_string()],
            ..Default::default()
        };

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Plugin);
    }

    #[test]
    fn test_classify_project_type_service_dockerfile() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("Dockerfile"), "FROM rust").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_service_docker_compose() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_service_docker_compose_yaml() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("docker-compose.yaml"), "version: '3'").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_service_from_frameworks() {
        let tmp = tempdir().unwrap();

        let profile = ProjectProfile {
            frameworks: vec!["FastAPI".to_string()],
            ..Default::default()
        };

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_service_express_framework() {
        let tmp = tempdir().unwrap();

        let profile = ProjectProfile {
            frameworks: vec!["Express".to_string()],
            ..Default::default()
        };

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_service_axum_framework() {
        let tmp = tempdir().unwrap();

        let profile = ProjectProfile {
            frameworks: vec!["Axum".to_string()],
            ..Default::default()
        };

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Service);
    }

    #[test]
    fn test_classify_project_type_library_rust_lib_rs() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "pub fn foo() {}").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Library);
    }

    #[test]
    fn test_classify_project_type_library_cargo_lib_section() {
        let tmp = tempdir().unwrap();
        let cargo = r#"
[package]
name = "mylib"
version = "0.1.0"

[lib]
name = "mylib"
"#;
        fs::write(tmp.path().join("Cargo.toml"), cargo).unwrap();

        let mut profile = ProjectProfile::default();
        profile.dependencies.insert("rust".to_string(), Vec::new());

        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Library);
    }

    #[test]
    fn test_classify_project_type_application_main_rs() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Application);
    }

    #[test]
    fn test_classify_project_type_application_main_py() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.py"), "if __name__ == '__main__': pass").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Application);
    }

    #[test]
    fn test_classify_project_type_application_index_ts() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("index.ts"), "console.log()").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Application);
    }

    #[test]
    fn test_classify_project_type_application_index_js() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("index.js"), "console.log()").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Application);
    }

    #[test]
    fn test_classify_project_type_application_app_py() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("app.py"), "app = Flask(__name__)").unwrap();

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Application);
    }

    #[test]
    fn test_classify_project_type_unknown() {
        let tmp = tempdir().unwrap();
        // Just an empty directory with no project markers

        let profile = ProjectProfile::default();
        let project_type = classify_project_type(tmp.path(), &profile);

        assert_eq!(project_type, ProjectType::Unknown);
    }

    #[test]
    fn test_parse_readme_content_with_badges() {
        let content = r#"# My Project

![Build Status](https://travis-ci.org/user/repo.svg)
[![Coverage](https://codecov.io/badge)](https://codecov.io)

This is the actual description of the project.

## Installation
"#;

        let (desc, _) = parse_readme_content(content);

        assert!(desc.is_some());
        let desc = desc.unwrap();
        // Should skip badges and get actual description
        assert!(desc.contains("actual description"));
        assert!(!desc.contains("travis-ci"));
    }

    #[test]
    fn test_parse_readme_content_truncates_long_description() {
        let content = format!("# Project\n\n{}", "A very long description. ".repeat(100));

        let (desc, _) = parse_readme_content(&content);

        assert!(desc.is_some());
        let desc = desc.unwrap();
        assert!(desc.len() <= 503); // 500 chars + "..."
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn test_parse_readme_content_extracts_multiple_keywords() {
        let content = r#"# Project Name

Description here.

## Installation Guide

## Configuration Options

## Usage Examples

## Contributing Guidelines
"#;

        let (_, keywords) = parse_readme_content(content);

        assert!(keywords.contains(&"installation".to_string()));
        assert!(keywords.contains(&"configuration".to_string()));
        assert!(keywords.contains(&"usage".to_string()));
        assert!(keywords.contains(&"contributing".to_string()));
    }

    #[test]
    fn test_parse_readme_content_filters_short_words() {
        let content = r#"# A B C

Description.

## In To Of At
"#;

        let (_, keywords) = parse_readme_content(content);

        // Words shorter than 3 chars should be filtered
        assert!(!keywords.iter().any(|k| k.len() < 3));
    }

    #[test]
    fn test_parse_readme_content_no_description() {
        let content = r#"# Project

"#;

        let (desc, _) = parse_readme_content(content);

        // Empty description case
        assert!(desc.is_none() || desc.as_ref().map(|d| d.is_empty()).unwrap_or(false));
    }

    #[test]
    fn test_parse_readme_content_description_after_multiple_headers() {
        let content = r#"# Header One

Some text under first header.

## Header Two
"#;

        let (desc, _) = parse_readme_content(content);

        assert!(desc.is_some());
        assert!(desc.unwrap().contains("text under first"));
    }

    #[test]
    fn test_detect_frameworks_python_frameworks() {
        let mut deps = HashMap::new();
        deps.insert(
            "python".to_string(),
            vec![
                DependencyInfo {
                    name: "fastapi".to_string(),
                    version: Some(">=0.100.0".to_string()),
                    dev: false,
                },
                DependencyInfo {
                    name: "pytest".to_string(),
                    version: None,
                    dev: true,
                },
                DependencyInfo {
                    name: "pandas".to_string(),
                    version: Some(">=2.0".to_string()),
                    dev: false,
                },
            ],
        );

        let frameworks = detect_frameworks(&deps);

        assert!(frameworks.contains(&"FastAPI".to_string()));
        assert!(frameworks.contains(&"pytest".to_string()));
        assert!(frameworks.contains(&"pandas".to_string()));
    }

    #[test]
    fn test_detect_frameworks_rust_frameworks() {
        let mut deps = HashMap::new();
        deps.insert(
            "rust".to_string(),
            vec![
                DependencyInfo {
                    name: "axum".to_string(),
                    version: Some("0.7".to_string()),
                    dev: false,
                },
                DependencyInfo {
                    name: "tokio".to_string(),
                    version: Some("1".to_string()),
                    dev: false,
                },
                DependencyInfo {
                    name: "serde".to_string(),
                    version: Some("1".to_string()),
                    dev: false,
                },
            ],
        );

        let frameworks = detect_frameworks(&deps);

        assert!(frameworks.contains(&"Axum".to_string()));
        assert!(frameworks.contains(&"Tokio".to_string()));
        assert!(frameworks.contains(&"Serde".to_string()));
    }

    #[test]
    fn test_detect_frameworks_no_duplicates() {
        let mut deps = HashMap::new();
        deps.insert(
            "npm".to_string(),
            vec![
                DependencyInfo {
                    name: "react".to_string(),
                    version: None,
                    dev: false,
                },
                DependencyInfo {
                    name: "react-dom".to_string(),
                    version: None,
                    dev: false,
                },
            ],
        );

        let frameworks = detect_frameworks(&deps);

        // Both contain "react" but should only have one React entry
        let react_count = frameworks.iter().filter(|f| *f == "React").count();
        assert_eq!(react_count, 1);
    }

    #[test]
    fn test_detect_frameworks_empty_deps() {
        let deps: HashMap<String, Vec<DependencyInfo>> = HashMap::new();
        let frameworks = detect_frameworks(&deps);

        assert!(frameworks.is_empty());
    }

    #[test]
    fn test_parse_requirements_txt() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("requirements.txt");

        let content = r#"
# This is a comment
requests>=2.28.0
flask==2.3.0
-e git+https://github.com/user/repo.git#egg=somepackage
numpy
pandas<2.0,>=1.5.0

pytest>=7.0
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_requirements_txt(&path).unwrap();

        assert_eq!(deps.len(), 5); // Comments and -e lines are skipped

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, Some(">=2.28.0".to_string()));

        let flask = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask.version, Some("==2.3.0".to_string()));

        let numpy = deps.iter().find(|d| d.name == "numpy").unwrap();
        assert_eq!(numpy.version, None);
    }

    #[test]
    fn test_analyze_project_with_readme() {
        let tmp = tempdir().unwrap();

        fs::write(
            tmp.path().join("README.md"),
            "# Test Project\n\nThis is a test project for CI/CD.",
        )
        .unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.description.is_some());
        assert!(profile.description.unwrap().contains("test project"));
    }

    #[test]
    fn test_analyze_project_without_readme() {
        let tmp = tempdir().unwrap();
        // No README file

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        // Should not fail, just have empty description
        assert!(profile.description.is_none());
    }

    #[test]
    fn test_analyze_project_case_insensitive_readme() {
        let tmp = tempdir().unwrap();

        // All caps README
        fs::write(
            tmp.path().join("README"),
            "# Plain README\n\nNo extension here.",
        )
        .unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.description.is_some());
    }

    #[test]
    fn test_analyze_project_detects_dependencies() {
        let tmp = tempdir().unwrap();

        let cargo = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"
"#;
        fs::write(tmp.path().join("Cargo.toml"), cargo).unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.dependencies.contains_key("rust"));
        let rust_deps = &profile.dependencies["rust"];
        assert!(rust_deps.iter().any(|d| d.name == "serde"));
    }

    #[test]
    fn test_analyze_project_detects_frameworks_from_deps() {
        let tmp = tempdir().unwrap();

        let package_json = r#"{
  "dependencies": {
    "express": "4.18.2",
    "react": "^18.0.0"
  }
}"#;
        fs::write(tmp.path().join("package.json"), package_json).unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.frameworks.contains(&"Express".to_string()));
        assert!(profile.frameworks.contains(&"React".to_string()));
    }

    #[test]
    fn test_analyze_project_uses_default_options() {
        let tmp = tempdir().unwrap();

        // Just ensure analyze_project works (wrapper for with_options)
        let profile = analyze_project(tmp.path()).unwrap();

        assert_eq!(profile.root, tmp.path());
    }

    #[test]
    fn test_requirements_txt_fallback_when_no_pyproject() {
        let tmp = tempdir().unwrap();

        // Only requirements.txt, no pyproject.toml
        fs::write(tmp.path().join("requirements.txt"), "requests>=2.0\n").unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.dependencies.contains_key("python"));
        let py_deps = &profile.dependencies["python"];
        assert!(py_deps.iter().any(|d| d.name == "requests"));
    }

    #[test]
    fn test_pyproject_takes_precedence_over_requirements() {
        let tmp = tempdir().unwrap();

        // Both files exist
        fs::write(tmp.path().join("requirements.txt"), "old-package>=1.0\n").unwrap();

        let pyproject = r#"
[project]
dependencies = ["new-package>=2.0"]
"#;
        fs::write(tmp.path().join("pyproject.toml"), pyproject).unwrap();

        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let profile = analyze_project_with_options(tmp.path(), options).unwrap();

        assert!(profile.dependencies.contains_key("python"));
        let py_deps = &profile.dependencies["python"];
        // Should have pyproject.toml deps, not requirements.txt
        assert!(py_deps.iter().any(|d| d.name == "new-package"));
        assert!(!py_deps.iter().any(|d| d.name == "old-package"));
    }

    #[test]
    fn test_parse_readme_content_skips_link_lines() {
        let content = r#"# Project

[Documentation](https://docs.example.com)
[GitHub](https://github.com/user/repo)

This is the real description.

## Features
"#;

        let (desc, _) = parse_readme_content(content);

        assert!(desc.is_some());
        let desc = desc.unwrap();
        assert!(desc.contains("real description"));
        assert!(!desc.contains("Documentation"));
    }

    #[test]
    fn test_detect_languages_all_extensions() {
        let tmp = tempdir().unwrap();

        // Test a variety of extensions to ensure coverage
        let files = [
            ("main.rs", "Rust"),
            ("script.pyw", "Python"),
            ("app.mjs", "JavaScript"),
            ("app.cjs", "JavaScript"),
            ("main.kt", "Kotlin"),
            ("main.kts", "Kotlin"),
            ("header.hxx", "C++"),
            ("source.cxx", "C++"),
            ("script.exs", "Elixir"),
            ("module.erl", "Erlang"),
            ("header.hrl", "Erlang"),
            ("main.cljs", "ClojureScript"),
            ("main.mli", "OCaml"),
            ("script.fsx", "F#"),
            ("main.nim", "Nim"),
            ("main.v", "V"),
            ("main.cr", "Crystal"),
        ];

        for (filename, _) in &files {
            fs::write(tmp.path().join(filename), "code").unwrap();
        }

        let languages = detect_languages(tmp.path()).unwrap();

        for (_, lang) in &files {
            assert!(
                languages.contains_key(*lang),
                "Expected language {} to be detected",
                lang
            );
        }
    }

    #[test]
    fn test_analyze_project_nonexistent_directory_behavior() {
        // Test that analyze_project handles nonexistent directories gracefully.
        // Current behavior: returns empty profile (not an error).
        // This is acceptable for skill recommendations - an empty profile simply
        // means no language/framework context is available, which is valid.
        let nonexistent = Path::new("/nonexistent/path/that/should/not/exist");
        let result = analyze_project(nonexistent);

        // Either an error or empty result is acceptable - key is no panic
        match result {
            Ok(profile) => {
                // Empty profile is valid for nonexistent directory
                assert!(profile.languages.is_empty());
                assert!(profile.dependencies.is_empty());
            }
            Err(e) => {
                // An error is also acceptable
                assert!(!e.to_string().is_empty());
            }
        }
    }

    #[test]
    fn test_detect_languages_nonexistent_behavior() {
        // detect_languages on nonexistent path should either error or return empty
        let nonexistent = Path::new("/nonexistent/path/that/should/not/exist");
        let result = detect_languages(nonexistent);

        match result {
            Ok(languages) => assert!(languages.is_empty()),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }

    #[test]
    fn test_analyze_project_empty_directory_succeeds() {
        // An empty but accessible directory should succeed with empty results
        let tmp = tempdir().unwrap();
        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };

        let result = analyze_project_with_options(tmp.path(), options);
        assert!(result.is_ok(), "Empty directory should succeed");

        let profile = result.unwrap();
        assert!(profile.languages.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_project_handles_permission_errors() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let restricted = tmp.path().join("restricted");
        fs::create_dir(&restricted).unwrap();
        fs::write(restricted.join("test.rs"), "fn main() {}").unwrap();

        // Remove read permissions from directory
        let mut perms = fs::metadata(&restricted).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&restricted, perms).unwrap();

        // Test should either error or succeed with partial results (not panic)
        let options = AnalyzeProjectOptions {
            include_git: false,
            commit_limit: 0,
            max_languages: 10,
        };
        let result = analyze_project_with_options(tmp.path(), options);

        // Restore permissions before assertions so cleanup works
        let mut perms = fs::metadata(&restricted).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&restricted, perms).unwrap();

        // Either succeeds with partial results or returns a clear error - both are valid
        // The key is it should NOT panic
        match result {
            Ok(profile) => {
                // If it succeeded, the restricted directory should have been skipped
                // so we shouldn't see any Rust files detected from it
                assert!(
                    profile
                        .languages
                        .get("Rust")
                        .is_none_or(|l| l.file_count == 0),
                    "Should not detect files from permission-denied directory"
                );
            }
            Err(e) => {
                // An error is acceptable - verify it's a meaningful error
                assert!(
                    !e.to_string().is_empty(),
                    "Error should have a descriptive message"
                );
            }
        }
    }
}
