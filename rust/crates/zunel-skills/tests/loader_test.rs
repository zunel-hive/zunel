use std::fs;

use tempfile::tempdir;

use zunel_skills::SkillsLoader;

fn write_skill(dir: &std::path::Path, name: &str, contents: &str) {
    let skill_dir = dir.join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), contents).unwrap();
}

#[test]
fn lists_user_skills_from_workspace() {
    let tmp = tempdir().unwrap();
    write_skill(
        tmp.path(),
        "greet",
        "---\ndescription: Says hi.\n---\n\nHello world.\n",
    );
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let skills = loader.list_skills(true).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "greet");
    assert_eq!(skills[0].description, "Says hi.");
}

#[test]
fn builtin_skills_are_loaded_after_user_skills_and_deduplicated() {
    let ws = tempdir().unwrap();
    let builtin = tempdir().unwrap();
    write_skill(
        ws.path(),
        "greet",
        "---\ndescription: User.\n---\n\nUser body.\n",
    );
    write_skill(
        builtin.path(),
        "greet",
        "---\ndescription: Builtin.\n---\n\nBuiltin body.\n",
    );
    write_skill(builtin.path(), "wave", "---\ndescription: Wave.\n---\n\n");

    let loader = SkillsLoader::new(ws.path(), Some(builtin.path()), &[]);
    let skills = loader.list_skills(true).unwrap();
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["greet", "wave"]);
    assert_eq!(skills[0].description, "User.");
}

#[test]
fn always_skills_are_derived_from_metadata() {
    let tmp = tempdir().unwrap();
    write_skill(
        tmp.path(),
        "watcher",
        "---\ndescription: Always-on.\nmetadata:\n  zunel:\n    always: true\n---\n\nWatcher body.",
    );
    write_skill(
        tmp.path(),
        "ondemand",
        "---\ndescription: On demand.\n---\n\n",
    );
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let always = loader.get_always_skills().unwrap();
    assert_eq!(always, vec!["watcher".to_string()]);
}

#[test]
fn load_skills_for_context_concatenates_with_delimiter() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "a", "---\ndescription: A.\n---\nBody A.");
    write_skill(tmp.path(), "b", "---\ndescription: B.\n---\nBody B.");
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let blob = loader
        .load_skills_for_context(&["a".to_string(), "b".to_string()])
        .unwrap();
    assert!(blob.contains("### Skill: a"));
    assert!(blob.contains("### Skill: b"));
    assert!(blob.contains("\n\n---\n\n"));
    assert!(blob.contains("Body A."));
    assert!(blob.contains("Body B."));
    // Frontmatter is stripped.
    assert!(!blob.contains("description: A."));
}

#[test]
fn build_skills_summary_uses_expected_format() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "hello", "---\ndescription: Hi.\n---\n\n");
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let summary = loader.build_skills_summary(None).unwrap();
    assert!(summary.starts_with("- **hello** — Hi."));
    assert!(summary.contains("`"));
}

#[test]
fn disabled_skills_are_omitted() {
    let tmp = tempdir().unwrap();
    write_skill(tmp.path(), "keep", "---\ndescription: K.\n---\n\n");
    write_skill(tmp.path(), "skip", "---\ndescription: S.\n---\n\n");
    let loader = SkillsLoader::new(tmp.path(), None, &["skip".to_string()]);
    let names: Vec<String> = loader
        .list_skills(true)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["keep".to_string()]);
}

#[test]
fn metadata_as_json_string_is_still_parsed() {
    // Python accepts `metadata: "<json string>"` for zunel/openclaw
    // namespace; match that fallback so hand-edited skills keep working.
    let tmp = tempdir().unwrap();
    write_skill(
        tmp.path(),
        "alt",
        "---\ndescription: Alt.\nmetadata: '{\"zunel\":{\"always\":true}}'\n---\n\nbody",
    );
    let loader = SkillsLoader::new(tmp.path(), None, &[]);
    let always = loader.get_always_skills().unwrap();
    assert_eq!(always, vec!["alt".to_string()]);
}
