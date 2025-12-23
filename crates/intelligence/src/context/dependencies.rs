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
}
