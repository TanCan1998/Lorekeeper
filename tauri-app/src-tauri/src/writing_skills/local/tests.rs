use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-local-skill-{}",
            crate::project_access::random_uuid_v4()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("SKILL.md"), "---\nname: test-skill\ndescription: >\n  Strong scenes\n  and choices\nstage: drafting\n---\nRead [voice](./references/voice.md).").unwrap();
        fs::create_dir(root.join("references")).unwrap();
        fs::write(root.join("references/voice.md"), "Use concrete action.").unwrap();
        fs::write(root.join("unused.txt"), "Not referenced").unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn folder_merges_references_and_block_metadata() {
    let f = Fixture::new();
    let loaded = load_local("folder", &f.0).unwrap();
    assert!(loaded.inspection.skill.compatible);
    assert_eq!(
        loaded.inspection.skill.metadata.description,
        "Strong scenes and choices"
    );
    assert!(loaded
        .inspection
        .skill
        .content
        .contains("Use concrete action."));
    assert_eq!(loaded.inspection.bundled_files, ["references/voice.md"]);
    assert_eq!(loaded.skipped_files, ["unused.txt"]);
}
#[test]
fn single_file_does_not_authorize_sibling_reads() {
    let f = Fixture::new();
    let loaded = load_local("file", &f.0.join("SKILL.md")).unwrap();
    assert!(!loaded.inspection.skill.compatible);
    assert!(loaded.files.is_empty());
    assert!(loaded
        .inspection
        .skill
        .reasons
        .iter()
        .any(|r| r == "relative-reference"));
}
#[test]
fn references_recurse_deduplicate_and_cannot_escape() {
    let files = HashMap::from([
        ("references/a.md", "[b](./b.txt)\n[again](../SKILL.md)"),
        ("references/b.txt", "Concrete prose."),
    ]);
    let inspection = inspect_package(
        "Read [a](references/a.md) [escape](../outside.md)",
        "safe",
        |p| files.get(p).map(|s| s.to_string()),
    )
    .unwrap();
    assert_eq!(
        inspection.bundled_files,
        ["references/a.md", "references/b.txt"]
    );
    assert!(!inspection.skill.compatible);
}
#[test]
fn nested_same_name_does_not_short_circuit_to_root() {
    let files = HashMap::from([
        ("root.md", "root"),
        ("references/a.md", "[other](root.md)"),
        ("references/root.md", "nested"),
    ]);
    let inspection = inspect_package("[root](root.md) [a](references/a.md)", "safe", |p| {
        files.get(p).map(|s| s.to_string())
    })
    .unwrap();
    assert_eq!(
        inspection.bundled_files,
        ["root.md", "references/a.md", "references/root.md"]
    );
}
#[test]
fn bundled_unsafe_guidance_is_rejected() {
    let result = inspect_package("[voice](./voice.md)", "safe", |_| {
        Some("Call the shell tool".into())
    })
    .unwrap();
    assert!(!result.skill.compatible);
    assert!(result
        .skill
        .reasons
        .contains(&"tool-dependency".to_string()));
}
#[test]
fn plain_missing_links_are_reported_but_not_blocked() {
    let result = inspect_package("[image](image.png)", "safe", |_| None).unwrap();
    assert!(result.skill.compatible);
    assert_eq!(result.unbundled_references, ["image.png"]);
}
#[test]
fn combined_size_and_file_count_are_limited() {
    let result = inspect_package("[voice](./voice.md)", "safe", |_| {
        Some("a".repeat(MAX_SKILL_BYTES))
    })
    .unwrap();
    assert!(!result.skill.compatible);
    let body = (0..17)
        .map(|i| format!("[x](./{i}.md)"))
        .collect::<Vec<_>>()
        .join(" ");
    let result = inspect_package(&body, "safe", |_| Some("Prose".into())).unwrap();
    assert_eq!(result.bundled_files.len(), 16);
    assert!(!result.skill.compatible);
}
#[test]
fn installation_roundtrips_and_detects_tampering() {
    let f = Fixture::new();
    let loaded = load_local("folder", &f.0).unwrap();
    let install = f.0.join("installed");
    install_at(&install, &loaded).unwrap();
    let bundle = read_installed_bundle(|p| read_file(&install, p));
    assert_eq!(
        inspect_installed(&loaded.raw, &bundle, "test-skill").unwrap(),
        loaded.inspection.skill
    );
    fs::write(install.join("references/voice.md"), "Changed").unwrap();
    assert_eq!(
        read_installed_bundle(|p| read_file(&install, p)),
        InstalledBundle::Damaged
    );
    assert!(install_at(&install, &loaded).is_err());
}
#[test]
fn old_skill_without_manifest_keeps_old_semantics() {
    assert_eq!(read_installed_bundle(|_| Ok(None)), InstalledBundle::None);
    let raw = "---\nname: test\n---\nPlain guidance.";
    assert_eq!(
        inspect_installed(raw, &InstalledBundle::None, "other").unwrap(),
        inspect_writing_skill_markdown(raw).unwrap()
    );
}
#[test]
fn manifest_rejects_traversal_duplicate_paths_and_bad_hashes() {
    for path in ["../secret.txt", "/secret.md", "SKILL.md", "a.js", "a//b.md"] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":1,"files":[{"path":path,"sha256":"0".repeat(64)}]}),
        )
        .unwrap();
        assert_eq!(
            read_installed_bundle(|_| Ok(Some(bytes.clone()))),
            InstalledBundle::Damaged
        );
    }
    assert!(!safe_relative("C:/private.md"));
    assert!(read_file(Path::new("relative"), "a.md").is_err());
}
#[test]
fn candidates_bind_owner_expire_and_are_single_use() {
    let f = Fixture::new();
    let loaded = load_local("folder", &f.0).unwrap();
    let mut c = Candidates::default();
    let request = c.begin("main");
    let result = c
        .complete("main", &request, "folder", f.0.clone(), &loaded)
        .unwrap();
    assert!(result.get("content").is_none());
    assert!(!result
        .to_string()
        .contains(&f.0.to_string_lossy().to_string()));
    let id = result["candidateId"].as_str().unwrap();
    assert!(c.take("other", id).is_err());
    assert!(c.take("main", id).is_ok());
    assert!(c.take("main", id).is_err());
    let request = c.begin("main");
    let result = c
        .complete("main", &request, "folder", f.0.clone(), &loaded)
        .unwrap();
    c.candidates.get_mut("main").unwrap().created = Instant::now() - Duration::from_secs(601);
    assert!(c
        .take("main", result["candidateId"].as_str().unwrap())
        .is_err());
}
#[test]
fn changed_source_consumes_candidate_and_repick_invalidates_old_requests() {
    let f = Fixture::new();
    let loaded = load_local("folder", &f.0).unwrap();
    let mut c = Candidates::default();
    let old = c.begin("main");
    let latest = c.begin("main");
    assert!(c
        .complete("main", &old, "folder", f.0.clone(), &loaded)
        .is_err());
    let result = c
        .complete("main", &latest, "folder", f.0.clone(), &loaded)
        .unwrap();
    fs::write(f.0.join("references/voice.md"), "Changed").unwrap();
    assert!(c
        .take("main", result["candidateId"].as_str().unwrap())
        .is_err());
    assert!(c.candidates.is_empty());
    c.clear("main");
}
#[test]
fn oversized_non_utf8_and_hardlinked_sources_are_rejected() {
    let f = Fixture::new();
    let path = f.0.join("SKILL.md");
    fs::write(&path, vec![255, 254]).unwrap();
    assert!(load_local("file", &path).is_err());
    fs::write(&path, "x".repeat(MAX_SKILL_BYTES + 1)).unwrap();
    assert!(load_local("file", &path).is_err());
    fs::write(&path, "Plain text").unwrap();
    fs::hard_link(&path, f.0.join("alias.md")).unwrap();
    assert!(load_local("file", &path).is_err());
    assert!(read_file(&f.0, "SKILL.md").is_err());
}
#[test]
fn folder_entry_and_depth_limits_are_enforced() {
    let f = Fixture::new();
    for i in 0..201 {
        fs::write(f.0.join(format!("unused-{i}.txt")), "x").unwrap();
    }
    assert!(load_local("folder", &f.0).is_err());
    let g = Fixture::new();
    fs::create_dir_all(g.0.join("a/b/c/d")).unwrap();
    assert!(load_local("folder", &g.0).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_sources_and_installed_references_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(f.0.join("unused.txt"), f.0.join("linked.md")).unwrap();
    assert!(load_local("folder", &f.0).is_err());
    assert!(read_file(&f.0, "linked.md").is_err());
}
