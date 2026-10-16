//! 首版范围外的拒绝边界；材料均为合成，不解码任何旧库。
use super::*;
use crate::external_grant::ExternalGrantRegistry;
use std::time::Duration;

#[test]
fn legacy_import_is_blocked_for_empty_invalid_and_repeated_ids() {
    for (source, target) in [
        ("", ""),
        ("missing-source", "missing-target"),
        ("same", "same"),
        ("../source", "\0"),
    ] {
        for _ in 0..2 {
            let result = project_import_legacy_copy(source.into(), target.into());
            assert_eq!(
                serde_json::to_value(result).unwrap(),
                serde_json::json!({
                    "state":"blocked","code":"LEGACY_IMPORT_UNSUPPORTED"
                })
            );
        }
    }
}

#[test]
fn legacy_import_response_matches_renderer_scope_policy() {
    let policy: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/shared/unsupported-channels.json"
    ))
    .unwrap();
    let result = project_import_legacy_copy("source".into(), "target".into());
    assert_eq!(
        result.code,
        policy["project:import-legacy-copy"].as_str().unwrap()
    );
    assert_eq!(result.state, "blocked");
}

#[test]
fn legacy_import_does_not_write_copy_change_source_or_consume_grants() {
    let root = std::env::temp_dir().join(format!(
        "lorekeeper-legacy-blocked-{}",
        crate::project_access::random_uuid_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let file = source.join("synthetic.db");
    let original = b"synthetic legacy input: intentionally not a database";
    std::fs::write(&file, original).unwrap();
    let target = root.join("copy");
    let mut grants = ExternalGrantRegistry::default();
    let source_id = grants
        .issue_directory_with_operations(
            &source,
            vec![GrantOperation::LegacyImport],
            Duration::from_secs(60),
            Some(1),
        )
        .unwrap();
    let target_id = grants
        .issue_new_child(
            &root,
            "copy",
            vec![GrantOperation::LegacyImport],
            Duration::from_secs(60),
            Some(1),
        )
        .unwrap();
    for _ in 0..2 {
        let result = project_import_legacy_copy(source_id.clone(), target_id.clone());
        assert_eq!(result.state, "blocked");
    }
    assert_eq!(std::fs::read(&file).unwrap(), original);
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(&source).unwrap().count(), 1);
    // 原始单次授权仍可首次消费，第二次消费才应拒绝。
    assert!(grants
        .resolve(&source_id, GrantOperation::LegacyImport)
        .is_ok());
    assert!(grants
        .resolve(&source_id, GrantOperation::LegacyImport)
        .is_err());
    assert!(grants
        .resolve_exact_path(&target_id, GrantOperation::LegacyImport)
        .is_ok());
    assert!(grants
        .resolve_exact_path(&target_id, GrantOperation::LegacyImport)
        .is_err());
    let _ = std::fs::remove_dir_all(root);
}
