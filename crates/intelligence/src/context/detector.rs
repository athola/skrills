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

/// Analyze a project directory and build a comprehensive profile.
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
        .max_depth(8)
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
}
