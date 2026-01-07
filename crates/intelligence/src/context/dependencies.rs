//! Parse dependency files from various ecosystems.

use super::DependencyInfo;
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Parse Cargo.toml for Rust dependencies.
pub fn parse_cargo_toml(path: &Path) -> Result<Vec<DependencyInfo>> {
    let content = fs::read_to_string(path)?;
    let doc: toml::Value = toml::from_str(&content)?;
    let mut deps = Vec::new();

    // Parse [dependencies]
    if let Some(dependencies) = doc.get("dependencies").and_then(|d| d.as_table()) {
        for (name, value) in dependencies {
            let version = extract_cargo_version(value);
            deps.push(DependencyInfo {
                name: name.clone(),
                version,
                dev: false,
            });
        }
    }

    // Parse [dev-dependencies]
    if let Some(dev_deps) = doc.get("dev-dependencies").and_then(|d| d.as_table()) {
        for (name, value) in dev_deps {
            let version = extract_cargo_version(value);
            deps.push(DependencyInfo {
                name: name.clone(),
                version,
                dev: true,
            });
        }
    }

    // Parse [build-dependencies]
    if let Some(build_deps) = doc.get("build-dependencies").and_then(|d| d.as_table()) {
        for (name, value) in build_deps {
            let version = extract_cargo_version(value);
            deps.push(DependencyInfo {
                name: name.clone(),
                version,
                dev: true, // Treat build deps as dev deps for our purposes
            });
        }
    }

    Ok(deps)
}

fn extract_cargo_version(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(v) => Some(v.clone()),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Parse package.json for Node.js dependencies.
pub fn parse_package_json(path: &Path) -> Result<Vec<DependencyInfo>> {
    let content = fs::read_to_string(path)?;
    let doc: serde_json::Value = serde_json::from_str(&content)?;
    let mut deps = Vec::new();

    // Parse dependencies
    if let Some(dependencies) = doc.get("dependencies").and_then(|d| d.as_object()) {
        for (name, version) in dependencies {
            deps.push(DependencyInfo {
                name: name.clone(),
                version: version.as_str().map(|s| s.to_string()),
                dev: false,
            });
        }
    }

    // Parse devDependencies
    if let Some(dev_deps) = doc.get("devDependencies").and_then(|d| d.as_object()) {
        for (name, version) in dev_deps {
            deps.push(DependencyInfo {
                name: name.clone(),
                version: version.as_str().map(|s| s.to_string()),
                dev: true,
            });
        }
    }

    // Parse peerDependencies (treat as non-dev)
    if let Some(peer_deps) = doc.get("peerDependencies").and_then(|d| d.as_object()) {
        for (name, version) in peer_deps {
            deps.push(DependencyInfo {
                name: name.clone(),
                version: version.as_str().map(|s| s.to_string()),
                dev: false,
            });
        }
    }

    Ok(deps)
}

/// Parse pyproject.toml for Python dependencies.
pub fn parse_pyproject_toml(path: &Path) -> Result<Vec<DependencyInfo>> {
    let content = fs::read_to_string(path)?;
    let doc: toml::Value = toml::from_str(&content)?;
    let mut deps = Vec::new();

    // Parse [project.dependencies] (PEP 621)
    if let Some(project) = doc.get("project") {
        if let Some(dependencies) = project.get("dependencies").and_then(|d| d.as_array()) {
            for dep in dependencies {
                if let Some(dep_str) = dep.as_str() {
                    let (name, version) = parse_python_dep_string(dep_str);
                    deps.push(DependencyInfo {
                        name,
                        version,
                        dev: false,
                    });
                }
            }
        }

        // Parse optional-dependencies (treat as dev)
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(|d| d.as_table())
        {
            for group_deps in optional.values() {
                if let Some(group_array) = group_deps.as_array() {
                    for dep in group_array {
                        if let Some(dep_str) = dep.as_str() {
                            let (name, version) = parse_python_dep_string(dep_str);
                            deps.push(DependencyInfo {
                                name,
                                version,
                                dev: true,
                            });
                        }
                    }
                }
            }
        }
    }

    // Parse [tool.poetry.dependencies] (Poetry)
    if let Some(tool) = doc.get("tool") {
        if let Some(poetry) = tool.get("poetry") {
            if let Some(dependencies) = poetry.get("dependencies").and_then(|d| d.as_table()) {
                for (name, value) in dependencies {
                    if name == "python" {
                        continue; // Skip Python version constraint
                    }
                    let version = extract_poetry_version(value);
                    deps.push(DependencyInfo {
                        name: name.clone(),
                        version,
                        dev: false,
                    });
                }
            }

            // Parse dev-dependencies
            if let Some(dev_deps) = poetry.get("dev-dependencies").and_then(|d| d.as_table()) {
                for (name, value) in dev_deps {
                    let version = extract_poetry_version(value);
                    deps.push(DependencyInfo {
                        name: name.clone(),
                        version,
                        dev: true,
                    });
                }
            }

            // Parse group dependencies
            if let Some(group) = poetry.get("group").and_then(|g| g.as_table()) {
                for (group_name, group_val) in group {
                    let is_dev = group_name == "dev" || group_name == "test";
                    if let Some(group_deps) =
                        group_val.get("dependencies").and_then(|d| d.as_table())
                    {
                        for (name, value) in group_deps {
                            let version = extract_poetry_version(value);
                            deps.push(DependencyInfo {
                                name: name.clone(),
                                version,
                                dev: is_dev,
                            });
                        }
                    }
                }
            }
        }

        // Parse [tool.uv.sources] and [tool.uv.dev-dependencies] (uv)
        if let Some(uv) = tool.get("uv") {
            // Parse dev-dependencies from uv (array of package specs)
            if let Some(dev_deps) = uv.get("dev-dependencies").and_then(|d| d.as_array()) {
                for dep in dev_deps {
                    if let Some(dep_str) = dep.as_str() {
                        let (name, version) = parse_python_dep_string(dep_str);
                        deps.push(DependencyInfo {
                            name,
                            version,
                            dev: true,
                        });
                    }
                }
            }
        }
    }

    Ok(deps)
}

fn extract_poetry_version(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(v) => Some(v.clone()),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn parse_python_dep_string(dep: &str) -> (String, Option<String>) {
    // Handle various formats: name, name>=1.0, name[extra]>=1.0, etc.
    let dep = dep.trim();

    // Find version specifier start
    let version_chars = ['=', '<', '>', '!', '~', '^'];
    if let Some(idx) = dep.find(|c| version_chars.contains(&c)) {
        let name_part = &dep[..idx];
        let version_part = &dep[idx..];

        // Handle extras like name[extra]
        let name = if let Some(bracket_idx) = name_part.find('[') {
            &name_part[..bracket_idx]
        } else {
            name_part
        };

        return (
            name.trim().to_string(),
            Some(version_part.trim().to_string()),
        );
    }

    // Handle extras without version
    let name = if let Some(bracket_idx) = dep.find('[') {
        &dep[..bracket_idx]
    } else {
        dep
    };

    (name.trim().to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_cargo_toml() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("Cargo.toml");

        let content = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
tempfile = "3"
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_cargo_toml(&path).unwrap();
        assert_eq!(deps.len(), 3);

        let serde = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version, Some("1.0".to_string()));
        assert!(!serde.dev);

        let tokio = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio.version, Some("1".to_string()));

        let tempfile = deps.iter().find(|d| d.name == "tempfile").unwrap();
        assert!(tempfile.dev);
    }

    #[test]
    fn test_parse_package_json() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("package.json");

        let content = r#"{
  "dependencies": {
    "react": "^18.0.0",
    "express": "4.18.2"
  },
  "devDependencies": {
    "jest": "^29.0.0"
  }
}"#;

        fs::write(&path, content).unwrap();

        let deps = parse_package_json(&path).unwrap();
        assert_eq!(deps.len(), 3);

        let react = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react.version, Some("^18.0.0".to_string()));
        assert!(!react.dev);

        let jest = deps.iter().find(|d| d.name == "jest").unwrap();
        assert!(jest.dev);
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[project]
dependencies = [
    "fastapi>=0.100.0",
    "pydantic[email]>=2.0",
]

[project.optional-dependencies]
dev = ["pytest>=7.0"]
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();
        assert_eq!(deps.len(), 3);

        let fastapi = deps.iter().find(|d| d.name == "fastapi").unwrap();
        assert_eq!(fastapi.version, Some(">=0.100.0".to_string()));
        assert!(!fastapi.dev);

        let pydantic = deps.iter().find(|d| d.name == "pydantic").unwrap();
        assert_eq!(pydantic.version, Some(">=2.0".to_string()));

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.dev);
    }

    #[test]
    fn test_parse_python_dep_string() {
        assert_eq!(
            parse_python_dep_string("requests"),
            ("requests".to_string(), None)
        );
        assert_eq!(
            parse_python_dep_string("requests>=2.0"),
            ("requests".to_string(), Some(">=2.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("requests[security]>=2.0"),
            ("requests".to_string(), Some(">=2.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("requests[security]"),
            ("requests".to_string(), None)
        );
    }

    #[test]
    fn test_parse_pyproject_toml_uv() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[project]
dependencies = [
    "httpx>=0.25.0",
]

[tool.uv]
dev-dependencies = [
    "pytest>=8.0",
    "ruff>=0.1.0",
]
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();
        assert_eq!(deps.len(), 3);

        let httpx = deps.iter().find(|d| d.name == "httpx").unwrap();
        assert_eq!(httpx.version, Some(">=0.25.0".to_string()));
        assert!(!httpx.dev);

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert_eq!(pytest.version, Some(">=8.0".to_string()));
        assert!(pytest.dev);

        let ruff = deps.iter().find(|d| d.name == "ruff").unwrap();
        assert_eq!(ruff.version, Some(">=0.1.0".to_string()));
        assert!(ruff.dev);
    }

    // ============================================
    // Additional tests for >90% coverage (Issue #60)
    // ============================================

    #[test]
    fn test_parse_cargo_toml_build_dependencies() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("Cargo.toml");

        let content = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"

[build-dependencies]
cc = "1.0"
bindgen = { version = "0.60" }
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_cargo_toml(&path).unwrap();
        assert_eq!(deps.len(), 3);

        // Build dependencies should be treated as dev deps
        let cc = deps.iter().find(|d| d.name == "cc").unwrap();
        assert_eq!(cc.version, Some("1.0".to_string()));
        assert!(cc.dev); // Build deps are treated as dev deps

        let bindgen = deps.iter().find(|d| d.name == "bindgen").unwrap();
        assert_eq!(bindgen.version, Some("0.60".to_string()));
        assert!(bindgen.dev);
    }

    #[test]
    fn test_parse_cargo_toml_no_version_in_table() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("Cargo.toml");

        let content = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
# Path dependency without version
local-crate = { path = "../local-crate" }
# Git dependency without version
git-crate = { git = "https://github.com/user/repo" }
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_cargo_toml(&path).unwrap();
        assert_eq!(deps.len(), 2);

        let local = deps.iter().find(|d| d.name == "local-crate").unwrap();
        assert_eq!(local.version, None);

        let git = deps.iter().find(|d| d.name == "git-crate").unwrap();
        assert_eq!(git.version, None);
    }

    #[test]
    fn test_parse_cargo_toml_empty_sections() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("Cargo.toml");

        let content = r#"
[package]
name = "test"
version = "0.1.0"
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_cargo_toml(&path).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_package_json_peer_dependencies() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("package.json");

        let content = r#"{
  "dependencies": {
    "lodash": "4.17.21"
  },
  "devDependencies": {
    "typescript": "^5.0.0"
  },
  "peerDependencies": {
    "react": ">=17.0.0"
  }
}"#;

        fs::write(&path, content).unwrap();

        let deps = parse_package_json(&path).unwrap();
        assert_eq!(deps.len(), 3);

        // Peer dependencies should be treated as non-dev
        let react = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react.version, Some(">=17.0.0".to_string()));
        assert!(!react.dev);
    }

    #[test]
    fn test_parse_package_json_empty_sections() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("package.json");

        let content = r#"{
  "name": "my-package",
  "version": "1.0.0"
}"#;

        fs::write(&path, content).unwrap();

        let deps = parse_package_json(&path).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_package_json_null_versions() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("package.json");

        let content = r#"{
  "dependencies": {
    "workspace-pkg": null
  }
}"#;

        fs::write(&path, content).unwrap();

        let deps = parse_package_json(&path).unwrap();
        assert_eq!(deps.len(), 1);

        let pkg = deps.iter().find(|d| d.name == "workspace-pkg").unwrap();
        assert_eq!(pkg.version, None);
    }

    #[test]
    fn test_parse_pyproject_toml_poetry() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[tool.poetry]
name = "my-project"
version = "1.0.0"

[tool.poetry.dependencies]
python = "^3.8"
requests = "^2.28"
pydantic = { version = "^2.0", extras = ["email"] }

[tool.poetry.dev-dependencies]
pytest = "^7.0"
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();

        // Python should be skipped
        assert!(!deps.iter().any(|d| d.name == "python"));

        let requests = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, Some("^2.28".to_string()));
        assert!(!requests.dev);

        let pydantic = deps.iter().find(|d| d.name == "pydantic").unwrap();
        assert_eq!(pydantic.version, Some("^2.0".to_string()));

        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.dev);
    }

    #[test]
    fn test_parse_pyproject_toml_poetry_groups() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[tool.poetry]
name = "my-project"
version = "1.0.0"

[tool.poetry.dependencies]
python = "^3.8"

[tool.poetry.group.dev.dependencies]
pytest = "^7.0"
black = "^23.0"

[tool.poetry.group.test.dependencies]
coverage = "^7.0"

[tool.poetry.group.docs.dependencies]
sphinx = "^6.0"
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();

        // dev and test groups should be marked as dev
        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.dev);

        let coverage = deps.iter().find(|d| d.name == "coverage").unwrap();
        assert!(coverage.dev);

        // docs group is not dev/test, so not marked as dev
        let sphinx = deps.iter().find(|d| d.name == "sphinx").unwrap();
        assert!(!sphinx.dev);
    }

    #[test]
    fn test_parse_pyproject_toml_multiple_optional_dependency_groups() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[project]
dependencies = ["requests"]

[project.optional-dependencies]
dev = ["pytest", "black"]
test = ["coverage"]
docs = ["sphinx"]
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();

        // All optional-dependencies are treated as dev
        let pytest = deps.iter().find(|d| d.name == "pytest").unwrap();
        assert!(pytest.dev);

        let sphinx = deps.iter().find(|d| d.name == "sphinx").unwrap();
        assert!(sphinx.dev);
    }

    #[test]
    fn test_parse_python_dep_string_various_operators() {
        // Test different version operators
        assert_eq!(
            parse_python_dep_string("pkg==1.0.0"),
            ("pkg".to_string(), Some("==1.0.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("pkg!=1.0.0"),
            ("pkg".to_string(), Some("!=1.0.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("pkg<2.0"),
            ("pkg".to_string(), Some("<2.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("pkg>1.0"),
            ("pkg".to_string(), Some(">1.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("pkg~=1.4.2"),
            ("pkg".to_string(), Some("~=1.4.2".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("pkg^1.0"),
            ("pkg".to_string(), Some("^1.0".to_string()))
        );
    }

    #[test]
    fn test_parse_python_dep_string_with_whitespace() {
        assert_eq!(
            parse_python_dep_string("  requests  "),
            ("requests".to_string(), None)
        );
        assert_eq!(
            parse_python_dep_string("  requests >= 2.0  "),
            ("requests".to_string(), Some(">= 2.0".to_string()))
        );
    }

    #[test]
    fn test_parse_python_dep_string_complex_extras() {
        assert_eq!(
            parse_python_dep_string("package[extra1,extra2]>=1.0"),
            ("package".to_string(), Some(">=1.0".to_string()))
        );
        assert_eq!(
            parse_python_dep_string("package[extra1,extra2]"),
            ("package".to_string(), None)
        );
    }

    #[test]
    fn test_extract_cargo_version_array_value() {
        // Test that non-string, non-table values return None
        let array_value = toml::Value::Array(vec![toml::Value::String("1.0".to_string())]);
        assert_eq!(extract_cargo_version(&array_value), None);
    }

    #[test]
    fn test_extract_poetry_version_array_value() {
        // Test that non-string, non-table values return None
        let array_value = toml::Value::Array(vec![toml::Value::String("1.0".to_string())]);
        assert_eq!(extract_poetry_version(&array_value), None);
    }

    #[test]
    fn test_parse_pyproject_toml_empty() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        let content = r#"
[build-system]
requires = ["setuptools"]
"#;

        fs::write(&path, content).unwrap();

        let deps = parse_pyproject_toml(&path).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_parse_cargo_toml_invalid_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("Cargo.toml");

        fs::write(&path, "this is not valid toml {{{").unwrap();

        let result = parse_cargo_toml(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_package_json_invalid_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("package.json");

        fs::write(&path, "this is not valid json {{{").unwrap();

        let result = parse_package_json(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pyproject_toml_invalid_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("pyproject.toml");

        fs::write(&path, "this is not valid toml {{{").unwrap();

        let result = parse_pyproject_toml(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nonexistent.toml");

        let result = parse_cargo_toml(&path);
        assert!(result.is_err());
    }
}
