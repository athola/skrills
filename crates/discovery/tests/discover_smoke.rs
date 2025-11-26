use skrills_discovery::{discover_skills, extra_skill_roots, hash_file, SkillSource};
use std::fs;
use tempfile::tempdir;

#[test]
fn discovers_single_skill_with_hash() {
    let tmp = tempdir().unwrap();
    let skill_dir = tmp.path().join("skills");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("alpha/SKILL.md");
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(&skill_path, "name: alpha\n").unwrap();

    let roots = extra_skill_roots(&[skill_dir]);
    let mut dup_log = vec![];
    let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "alpha/SKILL.md");
    assert_eq!(skills[0].source, SkillSource::Extra(0));
    assert_eq!(skills[0].hash, hash_file(&skill_path).unwrap());
    assert!(dup_log.is_empty());
}
