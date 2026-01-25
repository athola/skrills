//! Dependency tests - Dependency graph resolution and transitive dependencies

use super::super::*;
use skrills_discovery::SkillRoot;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_dependency_graph_integration() {
    // Initialize tracing for test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("skrills::deps=debug")
        .try_init();

    let temp = tempdir().expect("create temp directory");
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).expect("create skills directory");

    // Create skill A (depends on B and C)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).expect("create skill-a directory");
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B and C
---
# Skill A
See [skill-b](../skill-b/SKILL.md) and [skill-c](../skill-c/SKILL.md) for details.
"#,
    )
    .expect("write skill-a");

    // Create skill B (depends on D)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).expect("create skill-b directory");
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on D
---
# Skill B
Uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .expect("write skill-b");

    // Create skill C (depends on D)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).expect("create skill-c directory");
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C depends on D
---
# Skill C
Also uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .expect("write skill-c");

    // Create skill D (no dependencies)
    let skill_d_dir = skills_dir.join("skill-d");
    fs::create_dir_all(&skill_d_dir).expect("create skill-d directory");
    fs::write(
        skill_d_dir.join("SKILL.md"),
        r#"---
name: skill-d
description: Skill D has no dependencies
---
# Skill D
Base skill with no dependencies.
"#,
    )
    .expect("write skill-d");

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60))
        .expect("create skill service");

    // Force refresh to build the graph
    service.invalidate_cache().expect("invalidate cache");
    let skills = service.current_skills_with_dups().expect("get skills").0;

    // Verify skills were discovered
    assert_eq!(skills.len(), 4);

    // Test dependency resolution
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
    let deps = service
        .resolve_dependencies(skill_a_uri)
        .expect("resolve skill-a dependencies");

    // Debug output
    eprintln!("skill-a dependencies: {:?}", deps);
    eprintln!(
        "Skills discovered: {:?}",
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );

    // skill-a should have transitive dependencies: skill-b, skill-c, skill-d
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()),
        "Expected skill-b in deps"
    );
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()),
        "Expected skill-c in deps"
    );
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-d/SKILL.md".to_string()),
        "Expected skill-d in deps"
    );

    // Test reverse dependencies
    let skill_d_uri = "skill://skrills/extra0/skill-d/SKILL.md";
    let dependents = service
        .get_dependents(skill_d_uri)
        .expect("get skill-d dependents");

    // skill-d should be used by skill-b and skill-c
    assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(dependents.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

    // Test transitive dependents
    let trans_deps = service
        .get_transitive_dependents(skill_d_uri)
        .expect("get skill-d transitive dependents");

    // skill-d should transitively affect skill-a, skill-b, skill-c
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));
}

#[test]
fn test_resolve_dependencies_tool() {
    // Initialize tracing for test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("skrills::deps=debug")
        .try_init();

    let temp = tempdir().expect("create temp directory");
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).expect("create skills directory");

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).expect("create skill-a directory");
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md) for details.
"#,
    )
    .expect("write skill-a");

    // Create skill B (depends on C)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).expect("create skill-b directory");
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on C
---
# Skill B
Uses [skill-c](../skill-c/SKILL.md).
"#,
    )
    .expect("write skill-b");

    // Create skill C (no dependencies)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).expect("create skill-c directory");
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C has no dependencies
---
# Skill C
Base skill.
"#,
    )
    .expect("write skill-c");

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60))
        .expect("create skill service");

    // Force refresh to build the graph
    service.invalidate_cache().expect("invalidate cache");

    // Test 1: Transitive dependencies for A (should get B and C)
    let deps = service
        .resolve_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
        .expect("resolve skill-a dependencies");
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

    // Test 2: Direct dependencies for A (should only get B)
    let mut cache = service.cache.lock();
    let direct_deps = cache
        .get_direct_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
        .expect("get direct dependencies");
    assert_eq!(direct_deps.len(), 1);
    assert!(direct_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    drop(cache);

    // Test 3: Direct dependents of C (should only get B)
    let dependents = service
        .get_dependents("skill://skrills/extra0/skill-c/SKILL.md")
        .expect("get skill-c dependents");
    assert_eq!(dependents.len(), 1);
    assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));

    // Test 4: Transitive dependents of C (should get A and B)
    let trans_dependents = service
        .get_transitive_dependents("skill://skrills/extra0/skill-c/SKILL.md")
        .expect("get transitive dependents");
    assert_eq!(trans_dependents.len(), 2);
    assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
    assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
}
