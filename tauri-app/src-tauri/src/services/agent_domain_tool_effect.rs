//! 域工具效应 —— `propose_novel_config` / `propose_chapter_blueprint`
//! 域工具的当前性判定与事务内提交（agent-generation 域子波次 C）。
//!
//! 平移自 `electron/services/agent-domain-tool-effect.ts`（50 行）。
//! 返回类型镜像 `src/shared/agent-generation.ts` 的
//! `AgentDomainToolReceipt`；仓储依赖 `project_core_repository` /
//! `blueprint_repository`（均已迁移）。
//!
//! 与基线的一致性约定（对齐 `agent_generation.rs` 头注）：
//! - 基线 `if (!db.inTransaction || ...) fail()` 事务门禁的
//!   Tauri 等价物为**调用方结构保证**：本模块的两个入口均由
//!   `AgentGeneration::commit_domain_tool` 在自开的
//!   `.immediate()` 等价事务内调用（rusqlite 0.40.2 无
//!   `is_in_transaction()` API，方法内不重复检查）。
//! - `structuredClone` → `Value` / 派生 `Clone` 的深拷贝。
//! - 基线 `Number.isSafeInteger(x)` 对 `x.0` 形式浮点判真；
//!   `serde_json` 把 `1.0` 解析为 f64，整数校验按 `as_i64()`
//!   （运行期参数恒为整数写出，不可达）。
//! - 基线对仓储读取错误的传播（`ProjectCoreRepository.get` /
//!   `BlueprintRepository.getAll` 抛错原样上抛）保留为
//!   `Result` 传播；`current` 判定（基线返回 boolean）中
//!   仓储读取失败同样上抛（调用方经事务回滚折叠）。
//!
//! ⚠️ 哈希口径：`beforeHash` / `afterHash` 均为
//! `textHash(JSON.stringify(data))` 的 typed 序列化哈希
//! （Rust 侧以结构体声明序序列化）——与写入侧、读取侧同为
//! 本模块 / `agent_generation.rs` 内的同一序列化路径，
//! 口径逐字自洽（基线 TS 侧同理，全程同一进程内序列化）。

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::blueprint_repository::{
    get_all as blueprint_get_all, upsert as blueprint_upsert,
};
use crate::repositories::project_core_repository::{
    get as project_core_get, update as project_core_update,
};
use crate::services::agent_generation::{
    AgentDomainToolReceipt, GENERATION_AGENT_DOMAIN_TOOL_INVALID,
};

/// 基线 `configStrings`（agent-domain-tool-effect.ts:7）——
/// `propose_novel_config` 的字符串字段白名单。
const CONFIG_STRINGS: [&str; 10] = [
    "genre",
    "subGenre",
    "targetAudience",
    "coreOutline",
    "worldSetting",
    "goldenFinger",
    "protagonistProfile",
    "globalGuidance",
    "writingStyle",
    "referenceWorks",
];

/// 基线 `configEnums`（agent-domain-tool-effect.ts:8-9）——
/// 枚举字段白名单（值域逐字保留）。
const CONFIG_ENUMS: [(&str, &[&str]); 3] = [
    (
        "plotStructure",
        &[
            "three_act",
            "heros_journey",
            "save_the_cat",
            "kishotenketsu",
            "multi_thread",
            "freeform",
        ],
    ),
    (
        "narrativePOV",
        &[
            "third_limited",
            "first_person",
            "third_omniscient",
            "multi_pov",
        ],
    ),
    ("writingLanguage", &["zh-CN", "en-US"]),
];

/// 基线 `blueprintStrings`（agent-domain-tool-effect.ts:10）——
/// `propose_chapter_blueprint` 的字符串字段白名单。
const BLUEPRINT_STRINGS: [&str; 7] = [
    "title",
    "role",
    "purpose",
    "keyEvents",
    "suspenseHook",
    "userGuidance",
    "notes",
];

/// 基线 `textHash`（SHA-256 十六进制小写）——文件内私有副本
/// （对齐 `agent_generation.rs` / `graph_generation.rs` 先例）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `fail()`——`GENERATION_AGENT_DOMAIN_TOOL_INVALID`。
fn invalid() -> String {
    GENERATION_AGENT_DOMAIN_TOOL_INVALID.to_string()
}

/// 基线 `textHash(JSON.stringify(current))`——typed 序列化哈希
/// （结构体声明序，与提交侧 beforeHash / afterHash 口径逐字
/// 一致）。
fn serialized_hash<T: Serialize>(current: &T) -> String {
    text_hash(&serde_json::to_string(current).unwrap_or_default())
}

/// 基线 `isAgentDomainToolCurrent(db, receipt)`（:17-21）——
/// 新鲜读投影比对：config → `ProjectCoreRepository.get` 哈希
/// 比对；blueprint → `BlueprintRepository.getAll` 按
/// chapterNumber 定位比对。
///
/// 基线对仓储读取错误原样上抛（经调用方事务回滚折叠）；
/// 行缺失（config 无主台账行 / blueprint 无对应章节行）或
/// blueprint 收据缺 chapterNumber 时按基线 `!!current` 语义
/// 返回 `false`。
pub fn is_agent_domain_tool_current(
    conn: &Connection,
    receipt: &AgentDomainToolReceipt,
) -> Result<bool, String> {
    if receipt.kind == "config" {
        let current = project_core_get(conn)?;
        Ok(current.is_some_and(|current| serialized_hash(&current) == receipt.after_hash))
    } else {
        let rows = blueprint_get_all(conn)?;
        // 基线 `find(row => row.chapterNumber === receipt.chapterNumber)`——
        // chapterNumber 缺失（undefined）时恒不命中（false）。
        let current = receipt.chapter_number.and_then(|chapter_number| {
            rows.iter().find(|row| row.chapter_number == chapter_number)
        });
        Ok(current.is_some_and(|current| serialized_hash(current) == receipt.after_hash))
    }
}

/// 基线 `commitAgentDomainTool(db, toolCallId, name, args)`
/// （:24-50）——**必须在捕获项目的事务内调用**（调用方结构
/// 保证，见模块头注），且 action 已经确认（name ∈
/// {propose_novel_config, propose_chapter_blueprint}）。
///
/// `args.changes` 字段白名单 + 类型 / 枚举校验
/// （configStrings / configEnums / blueprintStrings、
/// totalChapters / wordsPerChapter 正整数、characters 字符串
/// 数组、writingLanguage 中文 / English → zh-CN / en-US 映射、
/// 作者微操指导 / 用户指引 → userGuidance、narrativePov 归一
/// 化）；config → `project_core_repository::update`；blueprint
/// → `blueprint_repository::upsert`；返回
/// `{ kind, toolCallId, changes, chapterNumber?, beforeHash,
/// afterHash }`；非法输入 → `GENERATION_AGENT_DOMAIN_TOOL_INVALID`。
pub fn commit_agent_domain_tool(
    conn: &Connection,
    tool_call_id: &str,
    name: &str,
    args: &Map<String, Value>,
) -> Result<AgentDomainToolReceipt, String> {
    // 基线 :25——`!['propose_novel_config', 'propose_chapter_blueprint'].includes(name)`。
    let config = match name {
        "propose_novel_config" => true,
        "propose_chapter_blueprint" => false,
        _ => return Err(invalid()),
    };
    // 基线 :26-28——`args.changes` 必须为非空对象（非数组 /
    // 非 null / 键数 ≥ 1）。
    let Some(candidate) = args.get("changes").and_then(Value::as_object) else {
        return Err(invalid());
    };
    if candidate.is_empty() {
        return Err(invalid());
    }
    // 基线 :29-41——逐字段归一化 + 白名单 + 类型 / 枚举校验。
    let mut changes: Map<String, Value> = Map::new();
    for (original, raw) in candidate {
        // 基线 :31——字段名归一化：config 的 `narrativePov` →
        // `narrativePOV`；blueprint 的 `作者微操指导` /
        // `用户指引` → `userGuidance`。
        let field: &str = if config {
            if original == "narrativePov" {
                "narrativePOV"
            } else {
                original
            }
        } else if original == "作者微操指导" || original == "用户指引" {
            "userGuidance"
        } else {
            original
        };
        // 基线 :32——`writingLanguage`：`简体中文` → `zh-CN`、
        // `English` → `en-US`（其余原样）。
        let value = if field == "writingLanguage" {
            match raw.as_str() {
                Some("简体中文") => Value::String("zh-CN".to_string()),
                Some("English") => Value::String("en-US".to_string()),
                _ => raw.clone(),
            }
        } else {
            raw.clone()
        };
        // 基线 :33——`Object.hasOwn(changes, field)`：不同原名
        // 归一到同一字段（如 `用户指引` 与 `userGuidance` 并存）
        // 时拒绝。
        if changes.contains_key(field) {
            return Err(invalid());
        }
        // 基线 :34-39——白名单 + 类型 / 枚举校验。
        let valid =
            if CONFIG_STRINGS.contains(&field) || (!config && BLUEPRINT_STRINGS.contains(&field)) {
                // 基线 `(config ? configStrings : blueprintStrings).has(field)`
                // 分支：字符串字段。
                value.is_string()
            } else if config && matches!(field, "totalChapters" | "wordsPerChapter") {
                // 基线 config 正整数分支。
                value.as_i64().is_some_and(|number| number >= 1)
            } else if config
                && CONFIG_ENUMS
                    .iter()
                    .any(|(enum_field, _)| *enum_field == field)
            {
                // 基线 config 枚举分支。
                value.as_str().is_some_and(|text| {
                    CONFIG_ENUMS
                        .iter()
                        .find(|(enum_field, _)| *enum_field == field)
                        .is_some_and(|(_, values)| values.contains(&text))
                })
            } else if !config && field == "characters" {
                // 基线 blueprint characters 分支：字符串数组。
                value
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
            } else {
                // 基线 :39——其余字段一律拒绝。
                false
            };
        if !valid {
            return Err(invalid());
        }
        changes.insert(field.to_string(), value);
    }
    // 基线 :42-48——config：更新 project_core 主台账。
    if config {
        let Some(before) = project_core_get(conn)? else {
            return Err(invalid());
        };
        // 基线 :45——写回仓储时 `narrativePOV` → `narrativePov`
        // （仓储按驼峰键映射列）。
        let data: Map<String, Value> = changes
            .iter()
            .map(|(key, value)| {
                let field = if key == "narrativePOV" {
                    "narrativePov"
                } else {
                    key.as_str()
                };
                (field.to_string(), value.clone())
            })
            .collect();
        project_core_update(conn, &data)?;
        // 基线 :46-47——beforeHash / afterHash（更新后再读；
        // 更新后行缺失时基线 `JSON.stringify(null)` === `"null"`）。
        let after_hash = match project_core_get(conn)? {
            Some(after) => serialized_hash(&after),
            None => text_hash("null"),
        };
        return Ok(AgentDomainToolReceipt {
            kind: "config".to_string(),
            tool_call_id: tool_call_id.to_string(),
            changes,
            chapter_number: None,
            before_hash: serialized_hash(&before),
            after_hash,
            current: None,
        });
    }
    // 基线 :49-57——blueprint：按 chapter_number 定位并 upsert。
    let chapter_number = args
        .get("chapter_number")
        .and_then(Value::as_i64)
        .filter(|number| *number >= 1)
        .ok_or_else(invalid)?;
    let before = blueprint_get_all(conn)?
        .into_iter()
        .find(|row| row.chapter_number == chapter_number)
        .ok_or_else(invalid)?;
    // 基线 :53——`{ ...before, ...changes }`。
    let mut updated = before.clone();
    for (field, value) in &changes {
        match field.as_str() {
            "title" => updated.title = value.as_str().unwrap_or_default().to_string(),
            "role" => updated.role = value.as_str().unwrap_or_default().to_string(),
            "purpose" => updated.purpose = value.as_str().unwrap_or_default().to_string(),
            "keyEvents" => updated.key_events = value.as_str().unwrap_or_default().to_string(),
            "suspenseHook" => {
                updated.suspense_hook = value.as_str().unwrap_or_default().to_string()
            }
            "userGuidance" => {
                updated.user_guidance = value.as_str().unwrap_or_default().to_string()
            }
            "notes" => updated.notes = value.as_str().unwrap_or_default().to_string(),
            "characters" => {
                updated.characters = value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| item.as_str().unwrap_or_default().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
            }
            // 白名单已校验——其余字段不可达。
            _ => return Err(invalid()),
        }
    }
    blueprint_upsert(conn, &updated)?;
    let after_hash = blueprint_get_all(conn)?
        .into_iter()
        .find(|row| row.chapter_number == chapter_number)
        .map(|after| serialized_hash(&after))
        .unwrap_or_else(|| text_hash("null"));
    Ok(AgentDomainToolReceipt {
        kind: "blueprint".to_string(),
        tool_call_id: tool_call_id.to_string(),
        changes,
        chapter_number: Some(chapter_number),
        before_hash: serialized_hash(&before),
        after_hash,
        current: None,
    })
}
