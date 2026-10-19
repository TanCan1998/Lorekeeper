//! import_generation —— 导入生成契约类型（对齐 `src/shared/import-generation.ts`）。
//!
//! 本模块只承载**上下文契约类型**（`ImportGenerationSlot` /
//! `ImportGenerationContext`，含内联章节类型 `ImportGenerationChapter`）。
//!
//! 两个频道已实现：`commands::generation::import_generation_read` /
//! `import_generation_execute`（owner 侧 `read_import_generation` /
//! 执行面委托，经 `lib.rs` 注册）。`ImportGenerationRecovery` 与
//! `ImportGenerationChannels` 只作为 TS 侧的**读出形状**：与其它生成类
//! 命令一致，Rust 侧直接以 `serde_json::Value` 返回运行视图，不声明
//! 平行结构体。
//!
//! 偏差说明：`prompts: Record<string, PromptTemplate>` 的
//! `PromptTemplate` 在 Rust 侧为 `prompt_system_role::PromptTemplate`；
//! 该字段按上游形状以 `serde_json::Value` 承载，字段名逐字对齐。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::repositories::project_core_repository::ProjectCoreData;

/// 导入生成的阶段槽位（TS 联合 `'global' | 'style' | 'blueprints'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportGenerationSlotStage {
    Global,
    Style,
    Blueprints,
}

impl ImportGenerationSlotStage {
    /// 数据库 `import_run_receipts.stage` 的取值（对齐 TS 字面量联合）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Style => "style",
            Self::Blueprints => "blueprints",
        }
    }
}

/// 持久导入检查点，绝不是渲染层选择的生成根。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGenerationSlot {
    pub run_id: String,
    pub stage: ImportGenerationSlotStage,
    pub batch_id: String,
}

/// 冻结章节快照（上游内联于 `ImportGenerationContext.chapters`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGenerationChapter {
    pub number: i64,
    pub title: String,
    pub content: String,
    pub content_fingerprint: String,
    pub word_count: i64,
}

/// 导入生成的冻结上下文（不可变素材，非渲染层采样）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGenerationContext {
    pub slot: ImportGenerationSlot,
    pub manifest_fingerprint: String,
    pub total_chapters: i64,
    pub total_words: i64,
    pub core: ProjectCoreData,
    pub prompts: Map<String, Value>,
    pub chapters: Vec<ImportGenerationChapter>,
}

/// 基线 `ImportGeneration.assertMutable`：导入槽位的效应已封存时不再可写。
///
/// 非导入来源的运行（`importSlot` 缺失）直接放行；导入来源必须带
/// 上下文与一致的槽位键，否则 `GENERATION_IMPORT_CONTEXT_REQUIRED`。
pub(crate) fn assert_import_mutable(
    conn: &rusqlite::Connection,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
) -> Result<(), String> {
    let manifest = &run.binding.source_manifest;
    if manifest
        .get("importSlot")
        .is_none_or(serde_json::Value::is_null)
    {
        return Ok(());
    }
    let context: ImportGenerationContext = manifest
        .get("importContext")
        .filter(|value| !value.is_null())
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| "GENERATION_IMPORT_CONTEXT_REQUIRED".to_string())?;
    let slot_key =
        crate::services::import_generation_source::import_generation_slot_key(&context.slot)?;
    if manifest.get("importSlotKey").and_then(Value::as_str) != Some(slot_key.as_str()) {
        return Err("GENERATION_IMPORT_CONTEXT_REQUIRED".to_string());
    }
    let receipt = crate::repositories::import_run_repository::get_effect_receipt(
        conn,
        &context.slot.run_id,
        context.slot.stage.as_str(),
        &context.slot.batch_id,
    )?;
    if receipt.is_some() {
        return Err("GENERATION_IMPORT_EFFECT_SEALED".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_serializes_camel_case_test() {
        let slot = ImportGenerationSlot {
            run_id: "run-1".to_string(),
            stage: ImportGenerationSlotStage::Blueprints,
            batch_id: "1-2-aaaaaaaa.bbbbbbbb".to_string(),
        };
        let value = serde_json::to_value(&slot).unwrap();
        assert_eq!(value["runId"], "run-1");
        assert_eq!(value["stage"], "blueprints");
        assert_eq!(value["batchId"], "1-2-aaaaaaaa.bbbbbbbb");
        let round_trip: ImportGenerationSlot = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, slot);
    }

    #[test]
    fn slot_stage_round_trips_lowercase_test() {
        for (stage, expected) in [
            (ImportGenerationSlotStage::Global, "global"),
            (ImportGenerationSlotStage::Style, "style"),
            (ImportGenerationSlotStage::Blueprints, "blueprints"),
        ] {
            assert_eq!(serde_json::to_value(stage).unwrap(), expected);
            let parsed: ImportGenerationSlotStage =
                serde_json::from_value(expected.into()).unwrap();
            assert_eq!(parsed, stage);
        }
    }

    #[test]
    fn chapter_serializes_camel_case_test() {
        let chapter = ImportGenerationChapter {
            number: 3,
            title: "第三章".to_string(),
            content: "正文".to_string(),
            content_fingerprint: "a".repeat(64),
            word_count: 5,
        };
        let value = serde_json::to_value(&chapter).unwrap();
        assert_eq!(value["number"], 3);
        assert_eq!(value["contentFingerprint"].as_str().map(str::len), Some(64));
        assert_eq!(value["wordCount"], 5);
        let round_trip: ImportGenerationChapter = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, chapter);
    }

    #[test]
    fn context_serializes_camel_case_test() {
        let context = ImportGenerationContext {
            slot: ImportGenerationSlot {
                run_id: "run-1".to_string(),
                stage: ImportGenerationSlotStage::Global,
                batch_id: "done".to_string(),
            },
            manifest_fingerprint: "b".repeat(64),
            total_chapters: 2,
            total_words: 10,
            core: core_data(),
            prompts: Map::new(),
            chapters: vec![ImportGenerationChapter {
                number: 1,
                title: "第一章".to_string(),
                content: "正文".to_string(),
                content_fingerprint: "c".repeat(64),
                word_count: 5,
            }],
        };
        let value = serde_json::to_value(&context).unwrap();
        assert_eq!(value["slot"]["runId"], "run-1");
        assert_eq!(
            value["manifestFingerprint"].as_str().map(str::len),
            Some(64)
        );
        assert_eq!(value["totalChapters"], 2);
        assert_eq!(value["totalWords"], 10);
        assert_eq!(value["core"]["projectName"], "项目");
        assert!(value["prompts"]
            .as_object()
            .map(|map| map.is_empty())
            .unwrap_or(false));
        assert_eq!(
            value["chapters"][0]["contentFingerprint"]
                .as_str()
                .map(str::len),
            Some(64)
        );
    }

    fn core_data() -> ProjectCoreData {
        ProjectCoreData {
            project_name: "项目".to_string(),
            genre: "仙侠".to_string(),
            sub_genre: String::new(),
            target_audience: String::new(),
            total_chapters: 100,
            words_per_chapter: 3000,
            writing_language: "zh-CN".to_string(),
            creative_strategy: "auto".to_string(),
            narrative_thread_dormant_chapter_threshold: 3,
            plot_structure: String::new(),
            narrative_pov: String::new(),
            writing_style: String::new(),
            reference_works: String::new(),
            global_guidance: String::new(),
            golden_finger: String::new(),
            core_outline: String::new(),
            world_setting: String::new(),
            protagonist_profile: String::new(),
            premise: String::new(),
            worldbuilding: String::new(),
            characters_arch: String::new(),
            synopsis: String::new(),
            character_states: String::new(),
        }
    }
}
