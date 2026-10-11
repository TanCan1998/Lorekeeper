//! import_generation —— 导入生成契约类型（对齐 `src/shared/import-generation.ts`）。
//!
//! 本波只移植 `ImportGenerationSlot` / `ImportGenerationContext` 两个类型
//! （含上下文内联章节类型 `ImportGenerationChapter`）。
//!
//! 未移植项（依赖尚未迁移的生成运行时类型，后续批次收口）：
//! - `ImportGenerationRecovery`（依赖 `MainGenerationRunView`）；
//! - `ImportGenerationChannels`（依赖 `MainGenerationRunHandle` /
//!   `MainGenerationExecuteReceipt` / `GenerationTask` /
//!   `ImportRunExecutionAuthority`）。
//!
//! 偏差说明：`prompts: Record<string, PromptTemplate>` 的
//! `PromptTemplate` 类型尚未移植（`prompt-templates` 服务未迁移），
//! 暂以 `serde_json::Value` 承载，字段名与上游逐字对齐。

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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
