//! character_proposal_parser —— 角色提案结构化解析（批次 B100）。
//!
//! 1:1 移植 `src/shared/character-proposal-parser.ts`：
//! - `extractSingleCompleteJsonObject`（思考标签剥离 + 唯一完整 JSON 对象定位）；
//! - `decodeCharacterIdentityManifest` / `normalizeCharacterSlotId` /
//!   `validateCharacterDetail` / `decodeCharacterDetails`（架构派生版本 1/2）；
//! - `parseMaterialExtraction`（规划资料抽取的信封诊断）；
//! - `characterProposalMaterialChunks`（UTF-16 码元分块与边界回退）；
//! - `parseArchitectureCharacterProposal` / `parsePlanningMaterialCharacterProposal`
//!   （覆盖度、槽位匹配与关系投影）。
//!
//! 文档化偏差：JS `Array.from(...)` 按码点计长、`slice` 按 UTF-16 码元；本模块分别用
//! `chars()` 与 UTF-16 码元视图等价实现，孤立代理项按替换字符处理（导入文本不含）。

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

/// 结构化产物引用（`{artifactId, text}`）。
#[derive(Debug, Clone)]
pub(crate) struct Artifact {
    pub(crate) artifact_id: String,
    pub(crate) text: String,
}

/// 规划资料输入（`{fileName, text}`）。
#[derive(Debug, Clone)]
pub(crate) struct MaterialInput {
    pub(crate) file_name: String,
    pub(crate) text: String,
}

/// 规划资料分块（`characterProposalMaterialChunks` 元素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterialChunk {
    pub(crate) source_id: String,
    /// 基线形状保留：分块文件名随产物返回，本栈只消费 `source_id`。
    #[allow(dead_code)]
    pub(crate) file_name: String,
    pub(crate) text: String,
}

/// 规划资料抽取结果（`MaterialExtraction`）。
#[derive(Debug, Clone)]
pub(crate) struct MaterialExtraction {
    pub(crate) source_id: String,
    pub(crate) character_cards: Vec<Value>,
}

/// 角色身份清单槽位（`CharacterIdentitySlot`）。
#[derive(Debug, Clone)]
pub(crate) struct CharacterIdentitySlot {
    pub(crate) slot_id: String,
    pub(crate) name: String,
    pub(crate) role: String,
    #[allow(dead_code)]
    pub(crate) narrative_duty: String,
    pub(crate) relations: Vec<(String, String)>,
}

const MIN_CHARACTER_SLOTS: usize = 3;
const MAX_CHARACTER_SLOTS: usize = 8;
const CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS: usize = 120;
const CHARACTER_STATE_TEXT_MAX_CHARS: usize = 80;
const CHARACTER_DETAIL_DESCRIPTION_FIELDS: [&str; 7] = [
    "appearance",
    "personality",
    "background",
    "abilities",
    "motivation",
    "arc",
    "notes",
];
const CHARACTER_STATE_TEXT_FIELDS: [&str; 6] = [
    "location",
    "powerLevel",
    "physicalState",
    "mentalState",
    "keyItems",
    "recentEvents",
];
const MATERIAL_CHARACTER_TEXT_FIELDS: [&str; 9] = [
    "gender",
    "age",
    "appearance",
    "personality",
    "background",
    "abilities",
    "motivation",
    "arc",
    "notes",
];
/// 架构清单只接受这四种定位（对齐 `CHARACTER_ROSTER_ROLES`）。
const CHARACTER_ROSTER_ROLES: [&str; 4] = ["protagonist", "antagonist", "supporting", "minor"];
/// 规划资料分块的 UTF-16 码元上限（`MATERIAL_CHUNK_CHARACTERS`）。
const MATERIAL_CHUNK_CHARACTERS: usize = 12_000;
const MATERIAL_CHUNK_BOUNDARY_START: usize = MATERIAL_CHUNK_CHARACTERS * 8 / 10;

static RE_MATERIAL_FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)^```json[ \t]*\r?\n(.*?)\r?\n```$").expect("围栏正则合法"));
static RE_DIAGNOSTIC_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)(?:\[\d+\])*$").expect("诊断字段正则合法")
});
static RE_MATERIAL_BOUNDARY: LazyLock<Regex> = LazyLock::new(|| {
    // Rust `regex` 不支持 lookahead：`.…` 分支的「后随空白或结尾」改由调用点检查。
    Regex::new(r#"(?:\r?\n[ \t]*\r?\n|[。！？!?][”’"'）)\]】」』]*|\.[”’"')\]】」』]*)"#)
        .expect("分块边界正则合法")
});

fn is_record(value: &Value) -> bool {
    value.is_object()
}

fn has_only_keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_none_or(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
}

/// 唯一键校验（`uniqueKeys`）。
fn unique_keys(keys: &[String]) -> Result<(), String> {
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err("CHARACTER_PROPOSAL_KEYS_INVALID".to_string());
    }
    let unique: HashSet<&str> = keys.iter().map(String::as_str).collect();
    if unique.len() != keys.len() {
        return Err("CHARACTER_PROPOSAL_KEYS_INVALID".to_string());
    }
    Ok(())
}

/// `fieldFromPath`：取 schema path 末尾字段名，无匹配回落 `$`。
fn field_from_path(path: &str) -> String {
    RE_DIAGNOSTIC_FIELD
        .find(path)
        .map(|matched| matched.as_str().to_string())
        .map(|field| {
            field
                .split(|character: char| character == '[')
                .next()
                .unwrap_or("$")
                .to_string()
        })
        .unwrap_or_else(|| "$".to_string())
}

/// 结构化合同诊断（`StructuredContractDiagnostic`）：
/// 消息逐字对齐基线，仅携带稳定 code 与 schema path。
pub(crate) fn structured_contract_diagnostic(code: &str, path: &str) -> String {
    format!(
        "结构化合同诊断 code={} path={} field={}",
        code,
        path,
        field_from_path(path)
    )
}

/// 定位从 `start` 起第一个完整 JSON 对象/数组的结束下标。
fn find_complete_json_object_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut closers: Vec<u8> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut index = start;
    while index < bytes.len() {
        let character = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if character == b'\\' {
                escaped = true;
            } else if character == b'"' {
                in_string = false;
            }
        } else if character == b'"' {
            in_string = true;
        } else if character == b'{' || character == b'[' {
            closers.push(if character == b'{' { b'}' } else { b']' });
        } else if character == b'}' || character == b']' {
            if closers.pop() != Some(character) {
                return None;
            }
            if closers.is_empty() {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

/// `extractSingleCompleteJsonObject`：剥离思考标签后，必须恰好存在一个完整 JSON 对象。
pub(crate) fn extract_single_complete_json_object(
    content: &str,
    exclude_array_roots: bool,
) -> Result<String, String> {
    let source = crate::repositories::generation_run_repository::strip_draft_thinking_tags(content)
        .trim()
        .to_string();
    let mut candidates: Vec<String> = Vec::new();
    let mut search_from = 0usize;
    while search_from < source.len() {
        let object_start = source[search_from..]
            .find('{')
            .map(|index| index + search_from);
        let array_start = if exclude_array_roots {
            source[search_from..]
                .find('[')
                .map(|index| index + search_from)
        } else {
            None
        };
        let start = match (object_start, array_start) {
            (None, None) => break,
            (Some(object), None) => object,
            (None, Some(array)) => array,
            (Some(object), Some(array)) => object.min(array),
        };
        let Some(end) = find_complete_json_object_end(&source, start) else {
            return Err("AI 返回包含截断 JSON 对象片段".to_string());
        };
        let candidate = source[start..=end].to_string();
        if let Ok(parsed) = serde_json::from_str::<Value>(&candidate) {
            if is_record(&parsed) {
                candidates.push(candidate);
            }
        }
        search_from = end + 1;
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err("AI 返回未包含一个完整 JSON 对象".to_string()),
        _ => Err("AI 返回包含多个完整 JSON 对象，无法确定唯一结构化结果".to_string()),
    }
}

/// `normalizeCharacterSlotId`：字符串 trim 非空，或非负安全整数转文本。
pub(crate) fn normalize_character_slot_id(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    value
        .as_i64()
        .filter(|number| *number >= 0 && number.abs() <= 9_007_199_254_740_991)
        .map(|number| number.to_string())
}

/// `decodeCharacterIdentityManifest`。
pub(crate) fn decode_character_identity_manifest(
    content: &str,
) -> Result<Vec<CharacterIdentitySlot>, String> {
    let extracted = extract_single_complete_json_object(content, false)?;
    let parsed: Value =
        serde_json::from_str(&extracted).map_err(|_| "角色身份清单缺少 slots".to_string())?;
    let Some(slots) = parsed.get("slots").and_then(Value::as_array) else {
        return Err("角色身份清单缺少 slots".to_string());
    };
    if !has_only_keys(&parsed, &["slots"]) {
        return Err("角色身份清单包含未知字段".to_string());
    }
    if slots.len() < MIN_CHARACTER_SLOTS || slots.len() > MAX_CHARACTER_SLOTS {
        return Err(format!(
            "角色身份清单必须包含 {MIN_CHARACTER_SLOTS}–{MAX_CHARACTER_SLOTS} 个角色"
        ));
    }
    let mut decoded: Vec<CharacterIdentitySlot> = Vec::new();
    for (index, candidate) in slots.iter().enumerate() {
        if !is_record(candidate) {
            return Err(format!("角色身份清单第 {} 项无效", index + 1));
        }
        if !has_only_keys(
            candidate,
            &["slotId", "name", "role", "narrativeDuty", "relations"],
        ) {
            return Err(format!("角色身份清单第 {} 项包含未知字段", index + 1));
        }
        let Some(relations) = candidate.get("relations").and_then(Value::as_array) else {
            return Err(format!("角色身份清单第 {} 项缺少关系列表", index + 1));
        };
        let slot_id = normalize_character_slot_id(candidate.get("slotId").unwrap_or(&Value::Null));
        let name = candidate
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let role = candidate
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let narrative_duty = candidate
            .get("narrativeDuty")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if slot_id.is_none()
            || name.trim().is_empty()
            || !CHARACTER_ROSTER_ROLES.contains(&role)
            || narrative_duty.trim().is_empty()
        {
            return Err(format!("角色身份清单第 {} 项字段不完整", index + 1));
        }
        let mut normalized_relations: Vec<(String, String)> = Vec::new();
        for (relation_index, relation) in relations.iter().enumerate() {
            let target_slot_id = if is_record(relation) {
                normalize_character_slot_id(relation.get("targetSlotId").unwrap_or(&Value::Null))
            } else {
                None
            };
            let relation_text = relation
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !is_record(relation)
                || !has_only_keys(relation, &["targetSlotId", "relation"])
                || target_slot_id.is_none()
                || relation_text.trim().is_empty()
            {
                return Err(format!(
                    "角色身份清单第 {} 项关系 {} 无效",
                    index + 1,
                    relation_index + 1
                ));
            }
            normalized_relations.push((
                target_slot_id.unwrap_or_default(),
                relation_text.trim().to_string(),
            ));
        }
        decoded.push(CharacterIdentitySlot {
            slot_id: slot_id.unwrap_or_default(),
            name: name.trim().to_string(),
            role: role.to_string(),
            narrative_duty: narrative_duty.trim().to_string(),
            relations: normalized_relations,
        });
    }
    let slot_ids: HashSet<&str> = decoded.iter().map(|slot| slot.slot_id.as_str()).collect();
    if slot_ids.len() != decoded.len() {
        return Err("角色身份清单包含重复 slotId".to_string());
    }
    if !decoded.iter().any(|slot| slot.role == "protagonist") {
        return Err("角色身份清单必须至少包含一个主角".to_string());
    }
    for slot in &decoded {
        for (target, _) in &slot.relations {
            if !slot_ids.contains(target.as_str()) || target == &slot.slot_id {
                return Err("角色身份清单关系端点不闭合或存在自指".to_string());
            }
        }
    }
    Ok(decoded)
}

/// `validateCharacterDetail`：返回首个违规说明（`None` 表示通过）。
pub(crate) fn validate_character_detail(output: &Value, version: u8) -> Option<String> {
    let slot_id = output
        .get("slotId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let invalid = |field: &str, reason: &str| {
        Some(format!("角色详情 slotId={slot_id} 字段 {field} {reason}"))
    };
    for field in [
        "slotId",
        "name",
        "gender",
        "age",
        "appearance",
        "personality",
        "background",
        "abilities",
        "motivation",
        "arc",
        "notes",
    ] {
        let text = output
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if text.trim().is_empty() {
            return invalid(field, "必须是非空文本");
        }
    }
    if version == 1 {
        for field in CHARACTER_DETAIL_DESCRIPTION_FIELDS {
            let length = output
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .chars()
                .count();
            if length > CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS {
                return invalid(
                    field,
                    &format!("不得超过 {CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS} 字符"),
                );
            }
        }
    }
    let role = output
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !CHARACTER_ROSTER_ROLES.contains(&role) {
        return invalid("role", "不是允许的定位");
    }
    if output.get("relationships").is_some() {
        return invalid("relationships", "不得出现");
    }
    let Some(current_state) = output.get("currentState") else {
        return invalid("currentState", "必填");
    };
    if !is_record(current_state) {
        return invalid("currentState", "必须是对象");
    }
    for field in CHARACTER_STATE_TEXT_FIELDS {
        let text = current_state
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if text.trim().is_empty() {
            return invalid(&format!("currentState.{field}"), "必须是非空文本");
        }
        if version == 1 && text.trim().chars().count() > CHARACTER_STATE_TEXT_MAX_CHARS {
            return invalid(
                &format!("currentState.{field}"),
                &format!("不得超过 {CHARACTER_STATE_TEXT_MAX_CHARS} 字符"),
            );
        }
    }
    if current_state
        .get("updatedAtChapter")
        .and_then(Value::as_i64)
        .is_none_or(|value| value < 0)
    {
        return invalid("currentState.updatedAtChapter", "必须是非负整数");
    }
    None
}

/// `normalizeDetailStringList`：字符串 trim；字符串数组按分隔符拼接；其余原样。
fn normalize_detail_string_list(value: &Value, separator: &str) -> Value {
    if let Some(text) = value.as_str() {
        return Value::String(text.trim().to_string());
    }
    let Some(items) = value.as_array() else {
        return value.clone();
    };
    if items.is_empty() {
        return value.clone();
    }
    let mut normalized: Vec<&str> = Vec::with_capacity(items.len());
    for item in items {
        let Some(text) = item.as_str() else {
            return value.clone();
        };
        if text.trim().is_empty() {
            return value.clone();
        }
        normalized.push(text.trim());
    }
    Value::String(normalized.join(separator))
}

/// `normalizeBoundedDetailText`：非字符串原样；trim；可选按码点截断。
fn normalize_bounded_detail_text(value: &Value, max_chars: Option<usize>) -> Value {
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    let trimmed = text.trim();
    match max_chars {
        None => Value::String(trimmed.to_string()),
        Some(limit) => Value::String(trimmed.chars().take(limit).collect()),
    }
}

/// `parseMaterialExtraction`：规划资料抽取的结构化合同校验（仅稳定 code 与 path 出栈）。
pub(crate) fn parse_material_extraction(content: &str) -> Result<Vec<MaterialExtraction>, String> {
    let trimmed = content.trim();
    let payload = RE_MATERIAL_FENCE
        .captures(trimmed)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().trim().to_string())
        .unwrap_or_else(|| trimmed.to_string());
    let root: Value = serde_json::from_str(&payload)
        .map_err(|_| structured_contract_diagnostic("invalid_json", "$"))?;
    if !is_record(&root) {
        return Err(structured_contract_diagnostic("invalid_envelope", "$"));
    }
    if !has_only_keys(&root, &["results"]) {
        return Err(structured_contract_diagnostic("unexpected_item", "$"));
    }
    let Some(results) = root.get("results") else {
        return Err(structured_contract_diagnostic("missing_field", "$.results"));
    };
    let Some(results) = results.as_array() else {
        return Err(structured_contract_diagnostic("invalid_type", "$.results"));
    };
    let result_keys = ["sourceId", "characterCards"];
    let mut card_keys: Vec<&str> = vec!["name", "role", "relationships"];
    card_keys.extend(MATERIAL_CHARACTER_TEXT_FIELDS);
    let require_non_empty_text = |value: Option<&Value>, path: &str| -> Result<String, String> {
        let Some(value) = value else {
            return Err(structured_contract_diagnostic("invalid_type", path));
        };
        let Some(text) = value.as_str() else {
            return Err(structured_contract_diagnostic("invalid_type", path));
        };
        if text.trim().is_empty() {
            return Err(structured_contract_diagnostic("empty_value", path));
        }
        Ok(text.to_string())
    };
    let mut parsed_results: Vec<MaterialExtraction> = Vec::new();
    for (result_index, candidate) in results.iter().enumerate() {
        let result_path = format!("$.results[{result_index}]");
        if !is_record(candidate) {
            return Err(structured_contract_diagnostic("invalid_type", &result_path));
        }
        if let Some(map) = candidate.as_object() {
            if let Some(key) = map.keys().find(|key| !result_keys.contains(&key.as_str())) {
                return Err(structured_contract_diagnostic(
                    "unexpected_item",
                    &format!("{result_path}.{key}"),
                ));
            }
        }
        let source_id = require_non_empty_text(
            candidate.get("sourceId"),
            &format!("{result_path}.sourceId"),
        )?;
        let Some(cards) = candidate.get("characterCards") else {
            return Err(structured_contract_diagnostic(
                "missing_field",
                &format!("{result_path}.characterCards"),
            ));
        };
        let Some(cards) = cards.as_array() else {
            return Err(structured_contract_diagnostic(
                "invalid_type",
                &format!("{result_path}.characterCards"),
            ));
        };
        let mut character_cards: Vec<Value> = Vec::new();
        for (card_index, card) in cards.iter().enumerate() {
            let card_path = format!("{result_path}.characterCards[{card_index}]");
            if !is_record(card) {
                return Err(structured_contract_diagnostic("invalid_type", &card_path));
            }
            if let Some(map) = card.as_object() {
                if let Some(key) = map.keys().find(|key| !card_keys.contains(&key.as_str())) {
                    return Err(structured_contract_diagnostic(
                        "unexpected_item",
                        &format!("{card_path}.{key}"),
                    ));
                }
            }
            require_non_empty_text(card.get("name"), &format!("{card_path}.name"))?;
            require_non_empty_text(card.get("role"), &format!("{card_path}.role"))?;
            for field in MATERIAL_CHARACTER_TEXT_FIELDS {
                if card.get(field).is_some() {
                    require_non_empty_text(card.get(field), &format!("{card_path}.{field}"))?;
                }
            }
            if card.get("relationships").is_some() {
                let Some(relationships) = card.get("relationships").and_then(Value::as_array)
                else {
                    return Err(structured_contract_diagnostic(
                        "invalid_type",
                        &format!("{card_path}.relationships"),
                    ));
                };
                for (relationship_index, relationship) in relationships.iter().enumerate() {
                    let relationship_path =
                        format!("{card_path}.relationships[{relationship_index}]");
                    if !is_record(relationship) {
                        return Err(structured_contract_diagnostic(
                            "invalid_type",
                            &relationship_path,
                        ));
                    }
                    if !has_only_keys(relationship, &["target", "relation"]) {
                        return Err(structured_contract_diagnostic(
                            "unexpected_item",
                            &relationship_path,
                        ));
                    }
                    require_non_empty_text(
                        relationship.get("target"),
                        &format!("{relationship_path}.target"),
                    )?;
                    require_non_empty_text(
                        relationship.get("relation"),
                        &format!("{relationship_path}.relation"),
                    )?;
                }
            }
            character_cards.push(card.clone());
        }
        parsed_results.push(MaterialExtraction {
            source_id,
            character_cards,
        });
    }
    Ok(parsed_results)
}

/// `decodeCharacterDetails`：字段归一化、状态默认章号 0、数字年龄转字符串。
pub(crate) fn decode_character_details(content: &str, version: u8) -> Result<Vec<Value>, String> {
    let extracted = extract_single_complete_json_object(content, false)?;
    let parsed: Value =
        serde_json::from_str(&extracted).map_err(|_| "角色详情响应缺少 entries".to_string())?;
    let Some(entries) = parsed.get("entries").and_then(Value::as_array) else {
        return Err("角色详情响应缺少 entries".to_string());
    };
    let description_limit = (version == 1).then_some(CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS);
    let state_limit = (version == 1).then_some(CHARACTER_STATE_TEXT_MAX_CHARS);
    let mut decoded: Vec<Value> = Vec::new();
    for candidate in entries {
        if !is_record(candidate) {
            decoded.push(candidate.clone());
            continue;
        }
        let mut normalized = candidate.clone();
        let Some(map) = normalized.as_object_mut() else {
            continue;
        };
        for field in CHARACTER_DETAIL_DESCRIPTION_FIELDS {
            let value = map.get(field).cloned().unwrap_or(Value::Null);
            map.insert(
                field.to_string(),
                normalize_bounded_detail_text(&value, description_limit),
            );
        }
        let current_state = map.get("currentState").cloned().unwrap_or(Value::Null);
        if let Some(state) = current_state.as_object() {
            let mut normalized_state = state.clone();
            normalized_state.insert(
                "keyItems".to_string(),
                normalize_detail_string_list(
                    normalized_state.get("keyItems").unwrap_or(&Value::Null),
                    "、",
                ),
            );
            normalized_state.insert(
                "recentEvents".to_string(),
                normalize_detail_string_list(
                    normalized_state.get("recentEvents").unwrap_or(&Value::Null),
                    "；",
                ),
            );
            // 架构生成描述的是分章前基线：模型给出的未来章号绝不落为事实。
            normalized_state.insert("updatedAtChapter".to_string(), json!(0));
            for field in CHARACTER_STATE_TEXT_FIELDS {
                let value = normalized_state.get(field).cloned().unwrap_or(Value::Null);
                normalized_state.insert(
                    field.to_string(),
                    normalize_bounded_detail_text(&value, state_limit),
                );
            }
            map.insert("currentState".to_string(), Value::Object(normalized_state));
        }
        if let Some(age) = map.get("age").and_then(Value::as_f64) {
            if age.is_finite() {
                let text = if age.fract() == 0.0 && age.abs() < 1e15 {
                    format!("{}", age as i64)
                } else {
                    format!("{age}")
                };
                map.insert("age".to_string(), Value::String(text));
            }
        }
        decoded.push(normalized);
    }
    Ok(decoded)
}

/// `materialChunkEnd`：UTF-16 码元分块终点（边界优先，其次代理对回退）。
fn material_chunk_end(units: &[u16], offset: usize) -> usize {
    let length = units.len();
    let hard_end = (offset + MATERIAL_CHUNK_CHARACTERS).min(length);
    if hard_end == length {
        return hard_end;
    }
    let tail_start = MATERIAL_CHUNK_BOUNDARY_START.min(hard_end - offset);
    let tail: String = String::from_utf16_lossy(&units[offset + tail_start..hard_end]);
    let mut boundary_end = 0usize;
    for matched in RE_MATERIAL_BOUNDARY.find_iter(&tail) {
        // 英文句点分支要求后随空白或字符串结尾（基线的 `(?=\s|$)`）。
        if matched.as_str().starts_with('.') {
            let followed_by_blank = tail[matched.end()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
            if !followed_by_blank {
                continue;
            }
        }
        let before = tail[..matched.start()].encode_utf16().count();
        let matched_units = matched.as_str().encode_utf16().count();
        boundary_end = tail_start + before + matched_units;
    }
    if boundary_end > 0 {
        return offset + boundary_end;
    }
    let previous = units[hard_end - 1];
    let next = units[hard_end];
    let splits_surrogate_pair =
        (0xD800..=0xDBFF).contains(&previous) && (0xDC00..=0xDFFF).contains(&next);
    if splits_surrogate_pair {
        hard_end - 1
    } else {
        hard_end
    }
}

/// `characterProposalMaterialChunks`：按 `序号:分块号` 生成稳定 sourceId。
pub(crate) fn character_proposal_material_chunks(
    materials: &[MaterialInput],
) -> Vec<MaterialChunk> {
    let mut chunks: Vec<MaterialChunk> = Vec::new();
    for (material_index, material) in materials.iter().enumerate() {
        let text = material.text.trim();
        if text.is_empty() {
            continue;
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut offset = 0usize;
        let mut chunk_index = 0usize;
        while offset < units.len() {
            let end = material_chunk_end(&units, offset).max(offset + 1);
            chunks.push(MaterialChunk {
                source_id: format!("{}:{}", material_index + 1, chunk_index + 1),
                file_name: material.file_name.clone(),
                text: String::from_utf16_lossy(&units[offset..end]),
            });
            offset = end;
            chunk_index += 1;
        }
    }
    chunks
}

/// `parseArchitectureCharacterProposal`：清单 + 详情产物 → 提案条目。
pub(crate) fn parse_architecture_character_proposal(
    manifest: &Artifact,
    details: &[Artifact],
    version: u8,
) -> Result<Vec<Value>, String> {
    let mut keys: Vec<String> = Vec::with_capacity(details.len() + 1);
    keys.push(manifest.artifact_id.clone());
    keys.extend(details.iter().map(|item| item.artifact_id.clone()));
    unique_keys(&keys)?;
    let slots = decode_character_identity_manifest(&manifest.text)?;
    let by_id: HashMap<&str, &CharacterIdentitySlot> = slots
        .iter()
        .map(|slot| (slot.slot_id.as_str(), slot))
        .collect();
    let mut records: Vec<Value> = Vec::new();
    for artifact in details {
        let extracted = extract_single_complete_json_object(&artifact.text, false)?;
        let root: Value = serde_json::from_str(&extracted)
            .map_err(|_| "CHARACTER_PROPOSAL_ENVELOPE_INVALID".to_string())?;
        if !has_only_keys(&root, &["entries"]) {
            return Err("CHARACTER_PROPOSAL_ENVELOPE_INVALID".to_string());
        }
        let decoded = decode_character_details(&artifact.text, version)?;
        if decoded.is_empty() {
            return Err("CHARACTER_PROPOSAL_EMPTY_DETAILS".to_string());
        }
        let raw_entries = root
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut allowed: Vec<&str> =
            vec!["slotId", "name", "role", "gender", "age", "currentState"];
        allowed.extend(CHARACTER_DETAIL_DESCRIPTION_FIELDS);
        for (index, entry) in decoded.iter().enumerate() {
            if let Some(invalid) = validate_character_detail(entry, version) {
                return Err(invalid);
            }
            let entry_slot_id = entry
                .get("slotId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let entry_name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let entry_role = entry
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(slot) = by_id.get(entry_slot_id) else {
                return Err("CHARACTER_PROPOSAL_SLOT_MISMATCH".to_string());
            };
            if slot.name != entry_name || slot.role != entry_role {
                return Err("CHARACTER_PROPOSAL_SLOT_MISMATCH".to_string());
            }
            let raw = raw_entries.get(index).cloned().unwrap_or(Value::Null);
            if !is_record(&raw) || !has_only_keys(&raw, &allowed) {
                return Err("CHARACTER_PROPOSAL_DETAIL_FIELD_INVALID".to_string());
            }
            let Some(entry_map) = entry.as_object() else {
                return Err("CHARACTER_PROPOSAL_DETAIL_FIELD_INVALID".to_string());
            };
            let mut static_fields = entry_map.clone();
            static_fields.remove("relationships");
            static_fields.remove("slotId");
            static_fields.remove("currentState");
            let relationships: Vec<Value> = slot
                .relations
                .iter()
                .map(|(target, relation)| {
                    json!({
                        "targetSelectionKey": target,
                        "relation": relation,
                    })
                })
                .collect();
            records.push(json!({
                "selectionKey": slot.slot_id,
                "sourceId": format!("{}:{}", artifact.artifact_id, slot.slot_id),
                "fields": Value::Object(static_fields),
                "relationships": relationships,
                "rawValue": {
                    "manifest": {
                        "slotId": slot.slot_id,
                        "name": slot.name,
                        "role": slot.role,
                        "narrativeDuty": slot.narrative_duty,
                        "relations": slot
                            .relations
                            .iter()
                            .map(|(target, relation)| {
                                json!({
                                    "targetSlotId": target,
                                    "relation": relation,
                                })
                            })
                            .collect::<Vec<Value>>(),
                    },
                    "detail": raw,
                },
            }));
        }
    }
    let record_keys: Vec<String> = records
        .iter()
        .map(|record| {
            record["selectionKey"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    unique_keys(&record_keys)?;
    if records.len() != slots.len() {
        return Err("CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE".to_string());
    }
    let mut ordered: Vec<Value> = Vec::with_capacity(slots.len());
    for slot in &slots {
        let Some(record) = records
            .iter()
            .find(|record| record["selectionKey"].as_str() == Some(slot.slot_id.as_str()))
        else {
            return Err("CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE".to_string());
        };
        ordered.push(record.clone());
    }
    Ok(ordered)
}

/// `parsePlanningMaterialCharacterProposal`：抽取产物 → 按期望 sourceId 顺序展开提案条目。
pub(crate) fn parse_planning_material_character_proposal(
    artifacts: &[Artifact],
    expected_source_ids: &[String],
) -> Result<Vec<Value>, String> {
    unique_keys(
        &artifacts
            .iter()
            .map(|artifact| artifact.artifact_id.clone())
            .collect::<Vec<_>>(),
    )?;
    unique_keys(expected_source_ids)?;
    let mut results: Vec<MaterialExtraction> = Vec::new();
    for artifact in artifacts {
        results.extend(parse_material_extraction(&artifact.text)?);
    }
    unique_keys(
        &results
            .iter()
            .map(|result| result.source_id.clone())
            .collect::<Vec<_>>(),
    )?;
    if results.len() != expected_source_ids.len()
        || results
            .iter()
            .any(|result| !expected_source_ids.contains(&result.source_id))
    {
        return Err("CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE".to_string());
    }
    let mut items: Vec<Value> = Vec::new();
    for source_id in expected_source_ids {
        let Some(result) = results.iter().find(|result| &result.source_id == source_id) else {
            return Err("CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE".to_string());
        };
        for (index, card) in result.character_cards.iter().enumerate() {
            let role = card.get("role").and_then(Value::as_str).unwrap_or_default();
            if !CHARACTER_ROSTER_ROLES.contains(&role) {
                return Err("CHARACTER_PROPOSAL_ROLE_INVALID".to_string());
            }
            let mut fields = card.as_object().cloned().unwrap_or_default();
            let relationships_value = fields.remove("relationships").unwrap_or(Value::Null);
            let relationships: Vec<Value> = relationships_value
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|relation| {
                    json!({
                        "targetName": relation.get("target"),
                        "relation": relation.get("relation"),
                        "raw": relation,
                    })
                })
                .collect();
            items.push(json!({
                "selectionKey": format!("{}:character:{}", source_id, index + 1),
                "sourceId": source_id,
                "fields": Value::Object(fields),
                "relationships": relationships,
                "rawValue": card,
            }));
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{"slots":[
        {"slotId":"s1","name":"林澈","role":"protagonist","narrativeDuty":"主线","relations":[{"targetSlotId":"s2","relation":"挚友"}]},
        {"slotId":"s2","name":"苏晚","role":"supporting","narrativeDuty":"情感线","relations":[{"targetSlotId":"s1","relation":"挚友"}]},
        {"slotId":"s3","name":"石决","role":"antagonist","narrativeDuty":"对抗","relations":[]}
    ]}"#;

    fn detail_entry(slot_id: &str, name: &str, role: &str, chapter: i64) -> String {
        json!({
            "slotId": slot_id,
            "name": name,
            "role": role,
            "gender": "男",
            "age": "二十",
            "appearance": "外貌",
            "personality": "性格",
            "background": "背景",
            "abilities": "能力",
            "motivation": "动机",
            "arc": "弧光",
            "notes": "备注",
            "currentState": {
                "location": "地点",
                "powerLevel": "战力",
                "physicalState": "身体",
                "mentalState": "心理",
                "keyItems": "剑、盾",
                "recentEvents": "事件一；事件二",
                "updatedAtChapter": chapter,
            },
        })
        .to_string()
    }

    fn details_json() -> String {
        format!(
            r#"{{"entries":[{},{},{}]}}"#,
            detail_entry("s1", "林澈", "protagonist", 7),
            detail_entry("s2", "苏晚", "supporting", 7),
            detail_entry("s3", "石决", "antagonist", 7)
        )
    }

    fn artifact(artifact_id: &str, text: &str) -> Artifact {
        Artifact {
            artifact_id: artifact_id.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn extract_single_complete_json_object_matches_baseline_messages() {
        assert_eq!(
            extract_single_complete_json_object("前文 {\"a\":1} 后文", false).unwrap(),
            r#"{"a":1}"#
        );
        // 思考标签内容整体剥离后才定位对象。
        assert_eq!(
            extract_single_complete_json_object("<think>推理 {\"x\":1}</think>{\"a\":1}", false)
                .unwrap(),
            r#"{"a":1}"#
        );
        assert_eq!(
            extract_single_complete_json_object("{\"a\":1}{\"b\":2}", false).unwrap_err(),
            "AI 返回包含多个完整 JSON 对象，无法确定唯一结构化结果"
        );
        assert_eq!(
            extract_single_complete_json_object("{\"a\":", false).unwrap_err(),
            "AI 返回包含截断 JSON 对象片段"
        );
        assert_eq!(
            extract_single_complete_json_object("没有对象", false).unwrap_err(),
            "AI 返回未包含一个完整 JSON 对象"
        );
        // 字符串内的括号不参与配对。
        assert_eq!(
            extract_single_complete_json_object(r#"{"a":"}{"}"#, false).unwrap(),
            r#"{"a":"}{"}"#
        );
    }

    #[test]
    fn decode_identity_manifest_validates_shape_and_closure() {
        let slots = decode_character_identity_manifest(MANIFEST).unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].slot_id, "s1");
        assert_eq!(
            slots[0].relations[0],
            ("s2".to_string(), "挚友".to_string())
        );

        // 槽位数量边界。
        assert_eq!(
            decode_character_identity_manifest(r#"{"slots":[]}"#).unwrap_err(),
            "角色身份清单必须包含 3–8 个角色"
        );
        // 未知顶层字段。
        assert_eq!(
            decode_character_identity_manifest(
                &MANIFEST.replace(r#"{"slots":"#, r#"{"extra":1,"slots":"#)
            )
            .unwrap_err(),
            "角色身份清单包含未知字段"
        );
        // 缺主角。
        let no_protagonist = MANIFEST.replace(r#""role":"protagonist""#, r#""role":"minor""#);
        assert_eq!(
            decode_character_identity_manifest(&no_protagonist).unwrap_err(),
            "角色身份清单必须至少包含一个主角"
        );
        // 重复 slotId。
        let duplicated = MANIFEST.replace(r#""slotId":"s3""#, r#""slotId":"s2""#);
        assert_eq!(
            decode_character_identity_manifest(&duplicated).unwrap_err(),
            "角色身份清单包含重复 slotId"
        );
        // 关系端点缺失。
        let dangling = MANIFEST.replace(
            r#""targetSlotId":"s2","relation":"挚友""#,
            r#""targetSlotId":"s9","relation":"挚友""#,
        );
        assert_eq!(
            decode_character_identity_manifest(&dangling).unwrap_err(),
            "角色身份清单关系端点不闭合或存在自指"
        );
        // 自指。
        let self_reference = MANIFEST.replace(
            r#""targetSlotId":"s2","relation":"挚友""#,
            r#""targetSlotId":"s1","relation":"挚友""#,
        );
        assert_eq!(
            decode_character_identity_manifest(&self_reference).unwrap_err(),
            "角色身份清单关系端点不闭合或存在自指"
        );
        // 字段不完整。
        let missing_name = MANIFEST.replace(r#""name":"林澈""#, "\"name\":\"  \"");
        assert_eq!(
            decode_character_identity_manifest(&missing_name).unwrap_err(),
            "角色身份清单第 1 项字段不完整"
        );
    }

    #[test]
    fn normalize_slot_id_accepts_text_and_nonnegative_integers() {
        assert_eq!(
            normalize_character_slot_id(&json!("  s1 ")),
            Some("s1".to_string())
        );
        assert_eq!(
            normalize_character_slot_id(&json!(3)),
            Some("3".to_string())
        );
        assert_eq!(
            normalize_character_slot_id(&json!(0)),
            Some("0".to_string())
        );
        assert_eq!(normalize_character_slot_id(&json!(-1)), None);
        assert_eq!(normalize_character_slot_id(&json!("   ")), None);
        assert_eq!(normalize_character_slot_id(&json!(1.5)), None);
        assert_eq!(normalize_character_slot_id(&json!(null)), None);
    }

    #[test]
    fn validate_character_detail_enforces_version_rules() {
        let base = || -> Value {
            serde_json::from_str(&detail_entry("s1", "林澈", "protagonist", 7)).unwrap()
        };
        assert_eq!(validate_character_detail(&base(), 2), None);
        // version 1 的描述字段按码点限长。
        let mut entry = base();
        entry["appearance"] = json!("甲".repeat(CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS + 1));
        assert_eq!(
            validate_character_detail(&entry, 1).unwrap(),
            format!("角色详情 slotId=s1 字段 appearance 不得超过 {CHARACTER_DETAIL_DESCRIPTION_MAX_CHARS} 字符")
        );
        assert!(validate_character_detail(&entry, 2).is_none());
        // 空字段 / 非法定位 / 出现 relationships / 缺 currentState / 状态章号非法。
        let mut broken = base();
        broken["name"] = json!("  ");
        assert_eq!(
            validate_character_detail(&broken, 2).unwrap(),
            "角色详情 slotId=s1 字段 name 必须是非空文本"
        );
        let mut broken = base();
        broken["role"] = json!("主角");
        assert_eq!(
            validate_character_detail(&broken, 2).unwrap(),
            "角色详情 slotId=s1 字段 role 不是允许的定位"
        );
        let mut broken = base();
        broken["relationships"] = json!([]);
        assert_eq!(
            validate_character_detail(&broken, 2).unwrap(),
            "角色详情 slotId=s1 字段 relationships 不得出现"
        );
        let mut broken = base();
        broken.as_object_mut().unwrap().remove("currentState");
        assert_eq!(
            validate_character_detail(&broken, 2).unwrap(),
            "角色详情 slotId=s1 字段 currentState 必填"
        );
        let mut broken = base();
        broken["currentState"]["updatedAtChapter"] = json!(-1);
        assert_eq!(
            validate_character_detail(&broken, 2).unwrap(),
            "角色详情 slotId=s1 字段 currentState.updatedAtChapter 必须是非负整数"
        );
        // slotId 缺失时以 unknown 占位。
        assert!(validate_character_detail(&json!({ "name": "林澈" }), 2)
            .unwrap()
            .starts_with("角色详情 slotId=unknown 字段 slotId"));
    }

    #[test]
    fn decode_character_details_normalizes_fields_and_state() {
        let decoded = decode_character_details(&details_json(), 2).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0]["currentState"]["updatedAtChapter"], json!(0));
        // 数组形态的状态字段按分隔符拼接为文本。
        let arrays = r#"{"entries":[{"slotId":"s1","currentState":{"keyItems":["剑","盾"],"recentEvents":["事件一","事件二"]}}]}"#;
        let decoded_arrays = decode_character_details(arrays, 2).unwrap();
        assert_eq!(
            decoded_arrays[0]["currentState"]["keyItems"],
            json!("剑、盾")
        );
        assert_eq!(
            decoded_arrays[0]["currentState"]["recentEvents"],
            json!("事件一；事件二")
        );
        assert_eq!(decoded[0]["currentState"]["keyItems"], json!("剑、盾"));
        // 数字年龄转字符串；非对象条目原样保留。
        let numeric = r#"{"entries":[{"slotId":"s1","age":20},42]}"#;
        let decoded = decode_character_details(numeric, 2).unwrap();
        assert_eq!(decoded[0]["age"], json!("20"));
        assert_eq!(decoded[1], json!(42));
        assert_eq!(
            decode_character_details("{}", 2).unwrap_err(),
            "角色详情响应缺少 entries"
        );
    }

    #[test]
    fn material_chunks_prefer_boundaries_and_keep_utf16_units() {
        let chunks = character_proposal_material_chunks(&[MaterialInput {
            file_name: "a.txt".to_string(),
            text: "  第一段。\n\n第二段。  ".to_string(),
        }]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_id, "1:1");
        assert_eq!(chunks[0].text, "第一段。\n\n第二段。");
        assert!(character_proposal_material_chunks(&[MaterialInput {
            file_name: "b.txt".to_string(),
            text: "   ".to_string(),
        }])
        .is_empty());

        // 超限时按段落边界提前切分。
        let boundary_text = format!("{}{}{}", "甲".repeat(11_000), "\n\n", "乙".repeat(3_000));
        let chunks = character_proposal_material_chunks(&[MaterialInput {
            file_name: "c.txt".to_string(),
            text: boundary_text,
        }]);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text.encode_utf16().count(), 11_002);
        assert!(chunks[0].text.ends_with("\n\n"));
        assert_eq!(chunks[1].source_id, "1:2");

        // 无边界时按硬上限切分，总量守恒。
        let chunks = character_proposal_material_chunks(&[MaterialInput {
            file_name: "d.txt".to_string(),
            text: "甲".repeat(13_000),
        }]);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[0].text.encode_utf16().count() + chunks[1].text.encode_utf16().count(),
            13_000
        );

        // 不切断代理对：emoji 跨越硬上限时回退一位码元。
        let mut surrogate = "甲".repeat(11_999);
        surrogate.push('\u{1F600}');
        surrogate.push_str(&"乙".repeat(100));
        let chunks = character_proposal_material_chunks(&[MaterialInput {
            file_name: "e.txt".to_string(),
            text: surrogate,
        }]);
        assert_eq!(chunks[0].text.encode_utf16().count(), 11_999);
    }

    #[test]
    fn parse_material_extraction_emits_diagnostic_codes_and_paths() {
        let fenced = "```json\n{\"results\":[{\"sourceId\":\"1:1\",\"characterCards\":[{\"name\":\"林澈\",\"role\":\"protagonist\"}]}]}\n```";
        let parsed = parse_material_extraction(fenced).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].source_id, "1:1");
        assert_eq!(parsed[0].character_cards[0]["name"], json!("林澈"));

        assert_eq!(
            parse_material_extraction("[]").unwrap_err(),
            structured_contract_diagnostic("invalid_envelope", "$")
        );
        assert_eq!(
            parse_material_extraction("{}").unwrap_err(),
            structured_contract_diagnostic("missing_field", "$.results")
        );
        assert_eq!(
            parse_material_extraction(r#"{"results":{}}"#).unwrap_err(),
            structured_contract_diagnostic("invalid_type", "$.results")
        );
        assert_eq!(
            parse_material_extraction(r#"{"other":1}"#).unwrap_err(),
            structured_contract_diagnostic("unexpected_item", "$")
        );
        assert_eq!(
            parse_material_extraction(
                r#"{"results":[{"sourceId":"1:1","characterCards":[{"name":" ","role":"supporting"}]}]}"#
            )
            .unwrap_err(),
            structured_contract_diagnostic("empty_value", "$.results[0].characterCards[0].name")
        );
        assert_eq!(
            parse_material_extraction(
                r#"{"results":[{"sourceId":"1:1","characterCards":[{"name":"林澈","role":"supporting","extra":1}]}]}"#
            )
            .unwrap_err(),
            structured_contract_diagnostic("unexpected_item", "$.results[0].characterCards[0].extra")
        );
        assert_eq!(
            parse_material_extraction(r#"{"results":[{"sourceId":"1:1"}]}"#).unwrap_err(),
            structured_contract_diagnostic("missing_field", "$.results[0].characterCards")
        );
    }

    #[test]
    fn parse_architecture_character_proposal_builds_records() {
        let manifest = artifact("manifest-1", MANIFEST);
        let details = vec![artifact("detail-1", &details_json())];
        let items = parse_architecture_character_proposal(&manifest, &details, 2).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["selectionKey"], json!("s1"));
        assert_eq!(items[0]["sourceId"], json!("detail-1:s1"));
        assert_eq!(
            items[0]["relationships"][0]["targetSelectionKey"],
            json!("s2")
        );
        assert_eq!(items[0]["relationships"][0]["relation"], json!("挚友"));
        assert_eq!(items[2]["relationships"], json!([]));
        assert!(items[0]["fields"].get("slotId").is_none());
        assert!(items[0]["fields"].get("currentState").is_none());
        assert!(items[0]["fields"].get("relationships").is_none());
        assert_eq!(items[0]["fields"]["name"], json!("林澈"));
        // rawValue 保留清单槽位与未归一化的原始详情。
        assert_eq!(items[0]["rawValue"]["manifest"]["slotId"], json!("s1"));
        assert_eq!(
            items[0]["rawValue"]["manifest"]["narrativeDuty"],
            json!("主线")
        );
        assert_eq!(
            items[0]["rawValue"]["detail"]["currentState"]["updatedAtChapter"],
            json!(7)
        );
        assert_eq!(
            items[1]["rawValue"]["detail"]["currentState"]["keyItems"],
            json!("剑、盾")
        );

        // 覆盖不完整 / 槽位不匹配 / 键集非法 / 信封未知字段。
        let partial = format!(
            r#"{{"entries":[{}]}}"#,
            detail_entry("s1", "林澈", "protagonist", 7)
        );
        assert_eq!(
            parse_architecture_character_proposal(&manifest, &[artifact("detail-1", &partial)], 2)
                .unwrap_err(),
            "CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE"
        );
        let mismatched = format!(
            r#"{{"entries":[{},{},{}]}}"#,
            detail_entry("s1", "别的名字", "protagonist", 7),
            detail_entry("s2", "苏晚", "supporting", 7),
            detail_entry("s3", "石决", "antagonist", 7)
        );
        assert_eq!(
            parse_architecture_character_proposal(
                &manifest,
                &[artifact("detail-1", &mismatched)],
                2
            )
            .unwrap_err(),
            "CHARACTER_PROPOSAL_SLOT_MISMATCH"
        );
        assert_eq!(
            parse_architecture_character_proposal(
                &manifest,
                &[artifact("manifest-1", &details_json())],
                2
            )
            .unwrap_err(),
            "CHARACTER_PROPOSAL_KEYS_INVALID"
        );
        let envelope_extra = details_json().replace(r#"{"entries":"#, r#"{"extra":1,"entries":"#);
        assert_eq!(
            parse_architecture_character_proposal(
                &manifest,
                &[artifact("detail-1", &envelope_extra)],
                2
            )
            .unwrap_err(),
            "CHARACTER_PROPOSAL_ENVELOPE_INVALID"
        );
    }

    #[test]
    fn parse_planning_material_character_proposal_projects_relationships() {
        let text = r#"{"results":[{"sourceId":"1:1","characterCards":[{"name":"林澈","role":"protagonist","relationships":[{"target":"苏晚","relation":"挚友"}]}]}]}"#;
        let items = parse_planning_material_character_proposal(
            &[artifact("a1", text)],
            &["1:1".to_string()],
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["selectionKey"], json!("1:1:character:1"));
        assert_eq!(items[0]["sourceId"], json!("1:1"));
        assert_eq!(items[0]["relationships"][0]["targetName"], json!("苏晚"));
        assert_eq!(
            items[0]["relationships"][0]["raw"]["relation"],
            json!("挚友")
        );
        assert!(items[0]["fields"].get("relationships").is_none());
        assert_eq!(items[0]["fields"]["role"], json!("protagonist"));
        assert_eq!(items[0]["rawValue"]["role"], json!("protagonist"));

        let invalid_role = text.replace(r#""role":"protagonist""#, r#""role":"主角""#);
        assert_eq!(
            parse_planning_material_character_proposal(
                &[artifact("a1", &invalid_role)],
                &["1:1".to_string()]
            )
            .unwrap_err(),
            "CHARACTER_PROPOSAL_ROLE_INVALID"
        );
        assert_eq!(
            parse_planning_material_character_proposal(
                &[artifact("a1", text)],
                &["2:1".to_string()]
            )
            .unwrap_err(),
            "CHARACTER_PROPOSAL_COVERAGE_INCOMPLETE"
        );
    }
}
