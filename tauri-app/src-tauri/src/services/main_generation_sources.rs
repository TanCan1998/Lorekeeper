//! 主生成 owner 的模型只读收据与源绑定依赖组合。
//! 仅从 Lorekeeper 的既有授权根读取；不回退 Electron 数据目录。
use crate::llm::lease::{
    create_model_execution_lease_receipt, LeaseModelProfile, ModelExecutionLeaseReceipt,
};
use crate::repositories::generation_run_repository::RunBinding;
use crate::services::builtin_writing_skills::read_builtin_writing_skill;
use crate::services::generation_source_binding::{
    rebuild_generation_source_binding, GenerationSourceBindingDependencies,
    SafeGenerationModelReceipt,
};
use crate::services::prompt_system_role::get_builtin_prompt_template;
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

pub(crate) fn safe_model_receipt(
    receipt: &ModelExecutionLeaseReceipt,
) -> SafeGenerationModelReceipt {
    SafeGenerationModelReceipt {
        model_id: receipt.model_id.clone(),
        provider: Some(receipt.provider.clone()),
        protocol: Some(receipt.protocol.clone()),
        model_name: Some(receipt.model_name.clone()),
        model_revision: receipt.model_revision.clone(),
        endpoint_fingerprint: receipt.endpoint_fingerprint.clone(),
        capability_evidence: serde_json::to_value(&receipt.capability_evidence)
            .expect("能力证据可序列化"),
    }
}

pub(crate) fn load_model(global_root: &Path, model_id: &str) -> Result<Value, String> {
    let model =
        crate::commands::read_models_at(&crate::app_paths::models_config_path_at(global_root))
            .into_iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
            .ok_or_else(|| "GENERATION_MODEL_MISSING".to_string())?;
    validate_model(&model)?;
    Ok(model)
}

pub(crate) fn validate_model(model: &Value) -> Result<(), String> {
    let profile = LeaseModelProfile::from_value(model);
    if [
        profile.id,
        profile.provider,
        profile.protocol,
        profile.model_name,
        profile.base_url,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
        || !matches!(profile.protocol, "openai" | "gemini")
    {
        return Err("GENERATION_MODEL_MISSING".to_string());
    }
    Ok(())
}

pub(crate) fn read_only_model_receipt(
    global_root: &Path,
    model_id: &str,
) -> Result<SafeGenerationModelReceipt, String> {
    let model = load_model(global_root, model_id)?;
    model_read_only_receipt(&model)
}

pub(crate) fn model_read_only_receipt(model: &Value) -> Result<SafeGenerationModelReceipt, String> {
    // 只读证据不是可执行租约，不进租约注册表或过期判定。
    let receipt = create_model_execution_lease_receipt(
        &LeaseModelProfile::from_value(&model),
        "read-only-evidence".to_string(),
        0,
        0,
    )
    .map_err(|error| error.message().to_string())?;
    Ok(safe_model_receipt(&receipt))
}

pub(crate) fn current_model_receipt(
    global_root: &Path,
    binding: &RunBinding,
) -> Result<SafeGenerationModelReceipt, String> {
    let model_id = binding
        .source_manifest
        .get("modelReceipt")
        .and_then(|receipt| receipt.get("modelId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "GENERATION_MODEL_MISSING".to_string())?;
    read_only_model_receipt(global_root, model_id)
}

pub(crate) fn with_source_dependencies<T>(
    conn: &Connection,
    project_storage_root: &Path,
    global_root: &Path,
    action: impl FnOnce(&GenerationSourceBindingDependencies<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let prompt = |key: &str, language| {
        get_builtin_prompt_template(key, language)
            .and_then(|template| serde_json::to_value(&template).ok())
            // 变量目录是 HashMap，先转规范 JSON，避免随机遍历顺序造成伪源漂移。
            .and_then(|template| serde_json::to_string(&template).ok())
    };
    action(&GenerationSourceBindingDependencies {
        db: conn,
        project_storage_root,
        global_data_root: global_root,
        read_builtin_prompt: &prompt,
        read_builtin_skill: Some(&read_builtin_writing_skill),
    })
}

pub(crate) fn rebuild_binding(
    conn: &Connection,
    project_storage_root: &Path,
    global_root: &Path,
    epoch: &str,
    previous: &RunBinding,
) -> Result<RunBinding, String> {
    let receipt = current_model_receipt(global_root, previous)?;
    rebuild_binding_with_receipt(
        conn,
        project_storage_root,
        global_root,
        epoch,
        previous,
        &receipt,
    )
}

pub(crate) fn rebuild_binding_with_receipt(
    conn: &Connection,
    project_storage_root: &Path,
    global_root: &Path,
    epoch: &str,
    previous: &RunBinding,
    receipt: &SafeGenerationModelReceipt,
) -> Result<RunBinding, String> {
    let policy = previous
        .source_manifest
        .get("policy")
        .cloned()
        .unwrap_or(Value::Null);
    // 解析器要求输入逐字段等于规范策略，校验通过后可原样透传。
    crate::services::main_generation_plan::read_main_generation_policy(&policy)?;
    if let Some(snapshot) = previous
        .source_manifest
        .get("knowledgeSnapshot")
        .filter(|value| !value.is_null())
    {
        let snapshot = serde_json::from_value(snapshot.clone())
            .map_err(|_| "GENERATION_KNOWLEDGE_INVALID_SNAPSHOT".to_string())?;
        crate::services::generation_knowledge_source::verify(
            conn,
            project_storage_root,
            &snapshot,
        )?;
    }
    with_source_dependencies(conn, project_storage_root, global_root, |deps| {
        Ok(rebuild_generation_source_binding(
            deps,
            previous,
            epoch,
            Some(&receipt),
            Some(&policy),
            None,
        )?
        .binding)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::services::generation_source_binding::{
        build_generation_source_binding, GenerationSourceBindingInput,
    };
    use serde_json::json;
    use std::path::PathBuf;

    pub(crate) struct TempRoot(PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "lorekeeper-generation-sources-{}",
                crate::project_access::random_uuid_v4()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn model() -> Value {
        json!({"id":"fixture", "provider":"custom", "protocol":"openai", "modelName":"fixture",
            "baseUrl":"https://example.invalid/v1", "apiKey":"synthetic-only", "maxTokens":4096})
    }
    pub(crate) fn write_model(global: &Path, model: &Value) {
        std::fs::write(
            crate::app_paths::models_config_path_at(global),
            serde_json::to_vec(&json!([model])).unwrap(),
        )
        .unwrap();
    }
    pub(crate) fn fixture(language: &str) -> (TempRoot, Connection, RunBinding, PathBuf, PathBuf) {
        let temp = TempRoot::new();
        let project = temp.path().join("project");
        let global = temp.path().join("global");
        std::fs::create_dir_all(project.join(".lore")).unwrap();
        std::fs::create_dir_all(&global).unwrap();
        write_model(&global, &model());
        std::fs::write(
            project.join(".lore/writing-skills.json"),
            r#"{"version":1,"bindings":{"planning":"builtin:long-form-continuity"}}"#,
        )
        .unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        crate::repositories::project_core_repository::init(&conn, "合成项目", language).unwrap();
        let selection = json!({"operation":"brainstorm","uiActionNonce":"fixture","modelId":"fixture",
            "selectedDraftIds":[], "selectedFinalizedDraftIds":[], "promptKeys":["edit_selected_text"],
            "skillStages":["planning"],"output":"visible-text"});
        let policy =
            crate::services::main_generation_plan::new_main_generation_policy(&selection, &model())
                .unwrap();
        assert_eq!(
            crate::services::main_generation_plan::read_main_generation_policy(&policy.to_value())
                .unwrap(),
            policy
        );
        let mut input = selection;
        input["projectId"] = json!("project-1");
        input["epoch"] = json!("lease-1");
        input["modelReceipt"] =
            serde_json::to_value(read_only_model_receipt(&global, "fixture").unwrap()).unwrap();
        input["policy"] = policy.to_value();
        input["outputContract"] = json!("visible-text");
        let input: GenerationSourceBindingInput = serde_json::from_value(input).unwrap();
        let binding = with_source_dependencies(&conn, &project.join(".lore"), &global, |deps| {
            Ok(build_generation_source_binding(deps, &input)?.binding)
        })
        .unwrap();
        (temp, conn, binding, global, project)
    }

    #[test]
    fn read_only_receipt_excludes_credentials_and_execution_lease_test() {
        let (_temp, _conn, binding, global, _) = fixture("zh-CN");
        let receipt = current_model_receipt(&global, &binding).unwrap();
        let text = serde_json::to_string(&receipt).unwrap();
        for private in [
            "apiKey",
            "synthetic-only",
            "leaseId",
            "createdAt",
            "expiresAt",
            "baseUrl",
        ] {
            assert!(!text.contains(private), "不得泄漏 {private}");
        }
        let mut rotated = model();
        rotated["apiKey"] = json!("rotated-synthetic");
        write_model(&global, &rotated);
        assert_eq!(current_model_receipt(&global, &binding).unwrap(), receipt);
    }

    #[test]
    fn current_receipt_missing_invalid_and_removed_models_fail_closed_test() {
        let (_temp, _, mut binding, global, _) = fixture("zh-CN");
        let mut invalid = model();
        invalid["protocol"] = json!("unsupported");
        write_model(&global, &invalid);
        assert_eq!(
            current_model_receipt(&global, &binding).unwrap_err(),
            "GENERATION_MODEL_MISSING"
        );
        std::fs::write(crate::app_paths::models_config_path_at(&global), "[]").unwrap();
        assert_eq!(
            current_model_receipt(&global, &binding).unwrap_err(),
            "GENERATION_MODEL_MISSING"
        );
        binding.source_manifest.remove("modelReceipt");
        assert_eq!(
            current_model_receipt(&global, &binding).unwrap_err(),
            "GENERATION_MODEL_MISSING"
        );
    }

    #[test]
    fn unchanged_bilingual_binding_is_stable_with_builtin_skills_and_prompt_variables_test() {
        for language in ["zh-CN", "en-US"] {
            let (_temp, conn, binding, global, project) = fixture(language);
            for _ in 0..20 {
                let current =
                    rebuild_binding(&conn, &project.join(".lore"), &global, "lease-1", &binding)
                        .unwrap();
                assert_eq!(current, binding);
            }
        }
    }

    #[test]
    fn rebuild_detects_author_facts_model_and_skill_asset_changes_test() {
        let (_temp, conn, binding, global, project) = fixture("zh-CN");
        let root = project.join(".lore");
        let mut changed = model();
        changed["temperature"] = json!(0.4);
        write_model(&global, &changed);
        let model_changed = rebuild_binding(&conn, &root, &global, "lease-1", &binding).unwrap();
        assert_ne!(
            model_changed.fingerprint.model_lease_revision,
            binding.fingerprint.model_lease_revision
        );
        write_model(&global, &model());
        std::fs::write(
            root.join("writing-skills.json"),
            r#"{"version":1,"bindings":{"planning":"builtin:brainstorm"}}"#,
        )
        .unwrap();
        let skills_changed = rebuild_binding(&conn, &root, &global, "lease-1", &binding).unwrap();
        assert_ne!(
            skills_changed.fingerprint.skill_snapshot_hash,
            binding.fingerprint.skill_snapshot_hash
        );
        crate::repositories::project_core_repository::update(
            &conn,
            json!({"globalGuidance":"合成作者约束"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let facts_changed = rebuild_binding(&conn, &root, &global, "lease-1", &binding).unwrap();
        assert_ne!(
            facts_changed.fingerprint.author_guidance_hash,
            skills_changed.fingerprint.author_guidance_hash
        );
    }

    #[test]
    fn epoch_rebuild_changes_identity_but_not_source_evidence_test() {
        let (_temp, conn, binding, global, project) = fixture("zh-CN");
        let next =
            rebuild_binding(&conn, &project.join(".lore"), &global, "lease-2", &binding).unwrap();
        assert_eq!(next.epoch, "lease-2");
        assert_ne!(next, binding);
        assert!(
            crate::services::generation_source_binding::compare_generation_source_bindings(
                &binding, &next
            )
        );
    }

    #[test]
    fn unsafe_skill_identity_fails_before_any_external_read_test() {
        let (_temp, conn, binding, global, project) = fixture("zh-CN");
        std::fs::write(
            project.join(".lore/writing-skills.json"),
            r#"{"version":1,"bindings":{"planning":"project:../../outside"}}"#,
        )
        .unwrap();
        assert_eq!(
            rebuild_binding(&conn, &project.join(".lore"), &global, "lease-1", &binding)
                .unwrap_err(),
            "GENERATION_SKILL_ID_INVALID"
        );
    }
}
