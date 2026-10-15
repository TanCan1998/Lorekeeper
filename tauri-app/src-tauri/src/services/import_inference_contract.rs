//! import_inference_contract —— 导入推演产物解码（批次 B101）。
//!
//! 1:1 移植：
//! - `src/services/workflows/commands/import-inference-contract.ts` 的解码链
//!   （`parseImportInferenceJsonObject` / `decodeImportInferenceJson` /
//!   `importInferenceCards` / `unresolvedImportRelationshipTargets` /
//!   `parseImportEndpointCorrectionDelta` / `decodeImportInferenceWithEndpointDelta`）；
//! - `src/services/workflows/structured-syntax-repair.ts` 的
//!   `preservesStructuredJsonEvidence`（含 `lexicalEvidence`）。
//!
//! 提示词合同文本（`IMPORT_INFERENCE_JSON_CONTRACT` 等）不属于本波次范围。
//! 诊断消息复用 `character_proposal_parser::structured_contract_diagnostic`（同文案）。

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::services::character_proposal_parser::structured_contract_diagnostic;

/// 不可见包装残留（`EDGE_INVISIBLE_WRAPPER_RESIDUE`）。
const EDGE_INVISIBLE_WRAPPER_RESIDUE: [char; 5] =
    ['\u{FEFF}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}'];
const PLOT_STRUCTURES: [&str; 6] = [
    "three_act",
    "heros_journey",
    "save_the_cat",
    "kishotenketsu",
    "multi_thread",
    "freeform",
];
const NARRATIVE_POVS: [&str; 4] = [
    "third_limited",
    "first_person",
    "third_omniscient",
    "multi_pov",
];
const CHARACTER_ROSTER_ROLES: [&str; 4] = ["protagonist", "antagonist", "supporting", "minor"];
const IMPORT_ENDPOINT_DELTA_CARD_KEYS: [&str; 13] = [
    "abilities",
    "age",
    "appearance",
    "arc",
    "background",
    "currentState",
    "gender",
    "motivation",
    "name",
    "notes",
    "personality",
    "relationships",
    "role",
];
const IMPORT_ENDPOINT_DELTA_CURRENT_STATE_KEYS: [&str; 7] = [
    "keyItems",
    "location",
    "mentalState",
    "physicalState",
    "powerLevel",
    "recentEvents",
    "updatedAtChapter",
];
const IMPORT_ENDPOINT_DELTA_RELATIONSHIP_KEYS: [&str; 2] = ["relation", "target"];

static RE_JSON_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:true|false|null)").expect("字面量正则合法"));
static RE_JSON_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?").expect("数字正则合法")
});

/// 基线 `UiText` 回调（调用点恒取英文文案；中文原文保留便于将来接入 i18n）。
fn ui_text(_zh_cn: &str, en_us: &str) -> String {
    en_us.to_string()
}

fn is_record(value: &Value) -> bool {
    value.is_object()
}

/// `trimEdgeWrapperResidue`：剥离首尾空白与不可见包装字符。
fn trim_edge_wrapper_residue(content: &str) -> String {
    let is_residue = |character: char| {
        character.is_whitespace() || EDGE_INVISIBLE_WRAPPER_RESIDUE.contains(&character)
    };
    let trimmed_start = content.trim_start_matches(is_residue);
    trimmed_start.trim_end_matches(is_residue).to_string()
}

/// `findCompleteJsonObjectEnd`：仅统计花括号深度。
fn find_complete_json_object_end(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    let bytes = source.as_bytes();
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
            index += 1;
            continue;
        }
        if character == b'"' {
            in_string = true;
        } else if character == b'{' {
            depth += 1;
        } else if character == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
            if depth < 0 {
                return None;
            }
        }
        index += 1;
    }
    None
}

/// `extractSingleCompleteJsonObject`：必须恰好一个可解析对象。
fn extract_single_complete_json_object(source: &str) -> Result<String, String> {
    let mut candidates: Vec<String> = Vec::new();
    let mut search_from = 0usize;
    while search_from < source.len() {
        let Some(relative) = source[search_from..].find('{') else {
            break;
        };
        let start = search_from + relative;
        let Some(end) = find_complete_json_object_end(source, start) else {
            return Err(structured_contract_diagnostic("invalid_json", "$"));
        };
        let candidate = source[start..=end].to_string();
        let parsed = serde_json::from_str::<Value>(&candidate)
            .map_err(|_| structured_contract_diagnostic("invalid_json", "$"))?;
        if !is_record(&parsed) {
            return Err(structured_contract_diagnostic("invalid_json", "$"));
        }
        candidates.push(candidate);
        search_from = end + 1;
    }
    if candidates.len() != 1 {
        return Err(structured_contract_diagnostic("invalid_json", "$"));
    }
    Ok(candidates.remove(0))
}

/// `parseImportInferenceJsonObject`。
pub(crate) fn parse_import_inference_json_object(
    content: &str,
) -> Result<Map<String, Value>, String> {
    let normalized = extract_single_complete_json_object(&trim_edge_wrapper_residue(content))?;
    let parsed: Value = serde_json::from_str(&normalized)
        .map_err(|_| structured_contract_diagnostic("invalid_json", "$"))?;
    record(&parsed, "$")
}

/// `record(value, path)`。
fn record(value: &Value, path: &str) -> Result<Map<String, Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| structured_contract_diagnostic("invalid_type", path))
}

/// `required(value, field, path)`。
fn required(value: &Map<String, Value>, field: &str, path: &str) -> Result<Value, String> {
    value
        .get(field)
        .cloned()
        .ok_or_else(|| structured_contract_diagnostic("missing_field", &format!("{path}.{field}")))
}

/// `text(value, path)`：非字符串 → invalid_type；trim 后空 → invalid_value。
fn text(value: &Value, path: &str) -> Result<String, String> {
    let Some(raw) = value.as_str() else {
        return Err(structured_contract_diagnostic("invalid_type", path));
    };
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(structured_contract_diagnostic("invalid_value", path));
    }
    Ok(normalized.to_string())
}

/// `age(value, path)`：有限数字转文本，否则按文本校验。
fn age(value: &Value, path: &str) -> Result<String, String> {
    if let Some(number) = value.as_f64() {
        if number.is_finite() {
            let rendered = if number.fract() == 0.0 && number.abs() < 1e15 {
                format!("{}", number as i64)
            } else {
                format!("{number}")
            };
            return Ok(rendered);
        }
    }
    text(value, path)
}

/// `currentState(value, path)`。
fn current_state(value: &Value, path: &str) -> Result<Value, String> {
    let state = record(value, path)?;
    let updated_at_chapter = required(&state, "updatedAtChapter", path)?;
    let chapter = updated_at_chapter.as_i64().filter(|chapter| *chapter >= 0);
    let Some(chapter) = chapter else {
        return Err(structured_contract_diagnostic(
            "invalid_value",
            &format!("{path}.updatedAtChapter"),
        ));
    };
    let field = |name: &str| -> Result<String, String> {
        text(&required(&state, name, path)?, &format!("{path}.{name}"))
    };
    Ok(json!({
        "location": field("location")?,
        "powerLevel": field("powerLevel")?,
        "physicalState": field("physicalState")?,
        "mentalState": field("mentalState")?,
        "keyItems": field("keyItems")?,
        "recentEvents": field("recentEvents")?,
        "updatedAtChapter": chapter,
    }))
}

/// `decodeCards`：3–8 张、角色枚举、关系闭合与唯一名称自指约束。
fn decode_cards(value: &Value) -> Result<Vec<Value>, String> {
    let Some(items) = value.as_array() else {
        return Err(structured_contract_diagnostic(
            "invalid_type",
            "characterCards",
        ));
    };
    if items.len() < 3 || items.len() > 8 {
        return Err(structured_contract_diagnostic(
            "invalid_value",
            "characterCards",
        ));
    }
    let mut cards: Vec<Value> = Vec::with_capacity(items.len());
    for (index, raw) in items.iter().enumerate() {
        let path = format!("characterCards[{index}]");
        let card = record(raw, &path)?;
        let name = text(&required(&card, "name", &path)?, &format!("{path}.name"))?;
        let role_value = required(&card, "role", &path)?;
        let role = role_value
            .as_str()
            .filter(|value| CHARACTER_ROSTER_ROLES.contains(value))
            .ok_or_else(|| {
                structured_contract_diagnostic("invalid_value", &format!("{path}.role"))
            })?
            .to_string();
        let raw_relationships = required(&card, "relationships", &path)?;
        let Some(relationships) = raw_relationships.as_array() else {
            return Err(structured_contract_diagnostic(
                "invalid_type",
                &format!("{path}.relationships"),
            ));
        };
        let mut decoded_relationships: Vec<Value> = Vec::with_capacity(relationships.len());
        for (relationship_index, raw_relationship) in relationships.iter().enumerate() {
            let relationship_path = format!("{path}.relationships[{relationship_index}]");
            let relationship = record(raw_relationship, &relationship_path)?;
            decoded_relationships.push(json!({
                "target": text(
                    &required(&relationship, "target", &relationship_path)?,
                    &format!("{relationship_path}.target"),
                )?,
                "relation": text(
                    &required(&relationship, "relation", &relationship_path)?,
                    &format!("{relationship_path}.relation"),
                )?,
            }));
        }
        cards.push(json!({
            "name": name,
            "role": role,
            "gender": text(&required(&card, "gender", &path)?, &format!("{path}.gender"))?,
            "age": age(&required(&card, "age", &path)?, &format!("{path}.age"))?,
            "appearance": text(&required(&card, "appearance", &path)?, &format!("{path}.appearance"))?,
            "personality": text(&required(&card, "personality", &path)?, &format!("{path}.personality"))?,
            "background": text(&required(&card, "background", &path)?, &format!("{path}.background"))?,
            "abilities": text(&required(&card, "abilities", &path)?, &format!("{path}.abilities"))?,
            "motivation": text(&required(&card, "motivation", &path)?, &format!("{path}.motivation"))?,
            "relationships": decoded_relationships,
            "arc": text(&required(&card, "arc", &path)?, &format!("{path}.arc"))?,
            "notes": text(&required(&card, "notes", &path)?, &format!("{path}.notes"))?,
            "currentState": current_state(
                &required(&card, "currentState", &path)?,
                &format!("{path}.currentState"),
            )?,
        }));
    }
    let names: HashSet<String> = cards
        .iter()
        .filter_map(|card| card["name"].as_str().map(str::to_string))
        .collect();
    if !cards
        .iter()
        .any(|card| card["role"].as_str() == Some("protagonist"))
    {
        return Err(structured_contract_diagnostic(
            "missing_item",
            "characterCards.protagonist",
        ));
    }
    for (index, card) in cards.iter().enumerate() {
        let Some(relationships) = card["relationships"].as_array() else {
            continue;
        };
        let card_name = card["name"].as_str().unwrap_or_default();
        for (relationship_index, relationship) in relationships.iter().enumerate() {
            let path =
                format!("characterCards[{index}].relationships[{relationship_index}].target");
            let target = relationship["target"].as_str().unwrap_or_default();
            // 出现是候选而不是按名字绑定的身份：同名标签可指向另一个出现。
            let same_name_count = cards
                .iter()
                .filter(|candidate| candidate["name"].as_str() == Some(card_name))
                .count();
            if target == card_name && same_name_count == 1 {
                return Err(structured_contract_diagnostic(
                    "relationship_self_reference",
                    &path,
                ));
            }
            if !names.contains(target) {
                return Err(structured_contract_diagnostic(
                    "relationship_endpoint_not_in_characters",
                    &path,
                ));
            }
        }
    }
    Ok(cards)
}

/// `decodeImportInferenceJson`。
pub(crate) fn decode_import_inference_json(content: &str) -> Result<Value, String> {
    let root = parse_import_inference_json_object(content)?;
    let config = record(&required(&root, "novelConfig", "$")?, "novelConfig")?;
    let architecture = record(
        &required(&root, "architectureFiles", "$")?,
        "architectureFiles",
    )?;
    let plot_structure = required(&config, "plotStructure", "novelConfig")?;
    let plot_structure = plot_structure
        .as_str()
        .filter(|value| PLOT_STRUCTURES.contains(value))
        .ok_or_else(|| {
            structured_contract_diagnostic("invalid_value", "novelConfig.plotStructure")
        })?
        .to_string();
    let narrative_pov = required(&config, "narrativePOV", "novelConfig")?;
    let narrative_pov = narrative_pov
        .as_str()
        .filter(|value| NARRATIVE_POVS.contains(value))
        .ok_or_else(|| structured_contract_diagnostic("invalid_value", "novelConfig.narrativePOV"))?
        .to_string();
    let novel_config = json!({
        "genre": text(&required(&config, "genre", "novelConfig")?, "novelConfig.genre")?,
        "subGenre": text(&required(&config, "subGenre", "novelConfig")?, "novelConfig.subGenre")?,
        "targetAudience": text(
            &required(&config, "targetAudience", "novelConfig")?,
            "novelConfig.targetAudience",
        )?,
        "plotStructure": plot_structure,
        "narrativePOV": narrative_pov,
        "coreOutline": text(
            &required(&config, "coreOutline", "novelConfig")?,
            "novelConfig.coreOutline",
        )?,
        "worldSetting": text(
            &required(&config, "worldSetting", "novelConfig")?,
            "novelConfig.worldSetting",
        )?,
        "goldenFinger": text(
            &required(&config, "goldenFinger", "novelConfig")?,
            "novelConfig.goldenFinger",
        )?,
        "protagonistProfile": text(
            &required(&config, "protagonistProfile", "novelConfig")?,
            "novelConfig.protagonistProfile",
        )?,
        "globalGuidance": text(
            &required(&config, "globalGuidance", "novelConfig")?,
            "novelConfig.globalGuidance",
        )?,
    });
    let architecture_files = json!({
        "premise": text(
            &required(&architecture, "premise", "architectureFiles")?,
            "architectureFiles.premise",
        )?,
        "worldbuilding": text(
            &required(&architecture, "worldbuilding", "architectureFiles")?,
            "architectureFiles.worldbuilding",
        )?,
        "synopsis": text(
            &required(&architecture, "synopsis", "architectureFiles")?,
            "architectureFiles.synopsis",
        )?,
    });
    let character_cards = decode_cards(&required(&root, "characterCards", "$")?)?;
    Ok(json!({
        "novelConfig": novel_config,
        "architectureFiles": architecture_files,
        "characterCards": character_cards,
    }))
}

/// `importInferenceCards`：取出可比较的原始角色卡。
pub(crate) fn import_inference_cards(
    root: &Map<String, Value>,
) -> Result<Vec<Map<String, Value>>, String> {
    let Some(cards) = root.get("characterCards").and_then(Value::as_array) else {
        return Err(ui_text(
            "导入推演受限补卡校正缺少可比较的原始角色卡",
            "The bounded import correction is missing comparable original character cards.",
        ));
    };
    if !cards.iter().all(is_record) {
        return Err(ui_text(
            "导入推演受限补卡校正缺少可比较的原始角色卡",
            "The bounded import correction is missing comparable original character cards.",
        ));
    }
    Ok(cards.iter().filter_map(Value::as_object).cloned().collect())
}

/// `assertExactImportEndpointDeltaKeys`：键集合必须与期望完全一致。
fn assert_exact_import_endpoint_delta_keys(
    value: &Map<String, Value>,
    expected_keys: &[&str],
    path: &str,
) -> Result<(), String> {
    let mut actual: Vec<&str> = value.keys().map(String::as_str).collect();
    actual.sort_unstable();
    let mut expected: Vec<&str> = expected_keys.to_vec();
    expected.sort_unstable();
    if actual != expected {
        return Err(ui_text(
            &format!("导入推演受限补卡校正 delta {path} 包含缺失或额外字段"),
            &format!("The bounded import correction delta at {path} has missing or extra fields."),
        ));
    }
    Ok(())
}

/// `unresolvedImportRelationshipTargets`。
pub(crate) fn unresolved_import_relationship_targets(
    root: &Map<String, Value>,
) -> Result<Vec<String>, String> {
    let cards = import_inference_cards(root)?;
    let names: HashSet<String> = cards
        .iter()
        .filter_map(|card| card.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut unresolved: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for card in &cards {
        let card_name = card.get("name").and_then(Value::as_str);
        let Some(relationships) = card.get("relationships").and_then(Value::as_array) else {
            continue;
        };
        for relationship in relationships {
            let Some(relationship) = relationship.as_object() else {
                continue;
            };
            let Some(target) = relationship.get("target").and_then(Value::as_str) else {
                continue;
            };
            if Some(target) != card_name
                && !names.contains(target)
                && seen.insert(target.to_string())
            {
                unresolved.push(target.to_string());
            }
        }
    }
    if unresolved.is_empty() {
        return Err(ui_text(
            "导入推演受限补卡校正缺少未闭合的关系端点",
            "The bounded import correction has no unresolved relationship endpoint.",
        ));
    }
    Ok(unresolved)
}

/// `parseImportEndpointCorrectionDelta`：受限补卡只允许补缺失关系端点。
pub(crate) fn parse_import_endpoint_correction_delta(
    content: &str,
    unresolved_targets: &[String],
) -> Result<Vec<Map<String, Value>>, String> {
    let delta_root = parse_import_inference_json_object(content)?;
    assert_exact_import_endpoint_delta_keys(&delta_root, &["characterCards"], "$")?;
    let delta_cards = import_inference_cards(&delta_root)?;
    if delta_cards.len() != unresolved_targets.len() {
        return Err(ui_text(
            "导入推演受限补卡校正只能新增缺失关系端点角色",
            "The bounded import correction may add only characters required by missing relationship endpoints.",
        ));
    }
    for (index, delta_card) in delta_cards.iter().enumerate() {
        let path = format!("characterCards[{index}]");
        assert_exact_import_endpoint_delta_keys(
            delta_card,
            &IMPORT_ENDPOINT_DELTA_CARD_KEYS,
            &path,
        )?;
        if let Some(current_state) = delta_card.get("currentState").and_then(Value::as_object) {
            assert_exact_import_endpoint_delta_keys(
                current_state,
                &IMPORT_ENDPOINT_DELTA_CURRENT_STATE_KEYS,
                &format!("{path}.currentState"),
            )?;
        }
        if let Some(relationships) = delta_card.get("relationships").and_then(Value::as_array) {
            for (relationship_index, relationship) in relationships.iter().enumerate() {
                if let Some(relationship) = relationship.as_object() {
                    assert_exact_import_endpoint_delta_keys(
                        relationship,
                        &IMPORT_ENDPOINT_DELTA_RELATIONSHIP_KEYS,
                        &format!("{path}.relationships[{relationship_index}]"),
                    )?;
                }
            }
        }
    }
    let expected_added_names: HashSet<&str> =
        unresolved_targets.iter().map(String::as_str).collect();
    let added_names: Vec<Option<&str>> = delta_cards
        .iter()
        .map(|card| card.get("name").and_then(Value::as_str))
        .collect();
    if added_names
        .iter()
        .any(|name| name.is_none_or(|name| !expected_added_names.contains(name)))
    {
        return Err(ui_text(
            "导入推演受限补卡校正新增角色必须精确匹配原始未闭合关系端点",
            "Characters added by the bounded import correction must exactly match the original unresolved endpoints.",
        ));
    }
    let unique_added: HashSet<&str> = added_names.iter().flatten().copied().collect();
    if unique_added.len() != added_names.len() {
        return Err(ui_text(
            "导入推演受限补卡校正 delta 包含重复缺失关系端点角色",
            "The bounded import correction delta contains duplicate missing-endpoint characters.",
        ));
    }
    if added_names.len() != expected_added_names.len() {
        return Err(ui_text(
            "导入推演受限补卡校正 delta 缺失关系端点角色",
            "The bounded import correction delta omits a missing-endpoint character.",
        ));
    }
    Ok(delta_cards)
}

/// `decodeImportInferenceWithEndpointDelta`：原始推演 + 端点补卡合并后重解码。
pub(crate) fn decode_import_inference_with_endpoint_delta(
    original: &str,
    delta: &str,
) -> Result<Value, String> {
    let root = parse_import_inference_json_object(original)?;
    let unresolved = unresolved_import_relationship_targets(&root)?;
    let base_cards = import_inference_cards(&root)?;
    let delta_cards = parse_import_endpoint_correction_delta(delta, &unresolved)?;
    let mut corrected = root;
    let mut cards: Vec<Value> = base_cards.into_iter().map(Value::Object).collect();
    cards.extend(delta_cards.into_iter().map(Value::Object));
    corrected.insert("characterCards".to_string(), Value::Array(cards));
    decode_import_inference_json(&Value::Object(corrected).to_string())
}

/// `JsonLexicalEvidence`：标量序列与容器形状两类证据。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct JsonLexicalEvidence {
    pub(crate) scalars: Vec<String>,
    pub(crate) containers: String,
}

/// `lexicalEvidence`：逐词法单元收集证据，任一处无法解析即失败。
fn lexical_evidence(source: &str) -> Option<JsonLexicalEvidence> {
    let mut scalars: Vec<String> = Vec::new();
    let mut containers = String::new();
    let characters: Vec<char> = source.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let character = characters[index];
        if character.is_whitespace() || character == ',' || character == ':' {
            index += 1;
            continue;
        }
        if "{}[]".contains(character) {
            containers.push(character);
            index += 1;
            continue;
        }
        if character == '"' {
            let start = index;
            index += 1;
            let mut escaped = false;
            while index < characters.len() {
                let current = characters[index];
                if !escaped && current == '"' {
                    index += 1;
                    let raw: String = characters[start..index].iter().collect();
                    let parsed: Value = serde_json::from_str(&raw).ok()?;
                    scalars.push(format!("s:{}", serde_json::to_string(&parsed).ok()?));
                    break;
                }
                if !escaped && (current as u32) < 0x20 {
                    return None;
                }
                if escaped {
                    escaped = false;
                } else if current == '\\' {
                    escaped = true;
                }
                index += 1;
            }
            if index == 0 || characters.get(index - 1) != Some(&'"') {
                return None;
            }
            continue;
        }
        let rest: String = characters[index..].iter().collect();
        if let Some(matched) = RE_JSON_LITERAL.find(&rest) {
            scalars.push(format!("l:{}", matched.as_str()));
            index += matched.as_str().chars().count();
            continue;
        }
        if let Some(matched) = RE_JSON_NUMBER.find(&rest) {
            scalars.push(format!("n:{}", matched.as_str()));
            index += matched.as_str().chars().count();
            continue;
        }
        return None;
    }
    Some(JsonLexicalEvidence {
        scalars,
        containers,
    })
}

/// `preservesStructuredJsonEvidence`：修复只允许改标点，唯一例外是补全缺失闭合符。
pub(crate) fn preserves_structured_json_evidence(candidate: &str, repaired: &str) -> bool {
    if serde_json::from_str::<Value>(repaired.trim()).is_err() {
        return false;
    }
    let Some(before) = lexical_evidence(candidate.trim()) else {
        return false;
    };
    let Some(after) = lexical_evidence(repaired.trim()) else {
        return false;
    };
    if before.scalars.len() != after.scalars.len() || before.scalars != after.scalars {
        return false;
    }
    if before.containers == after.containers {
        return true;
    }
    if !after.containers.starts_with(&before.containers) {
        return false;
    }
    let residue = &after.containers[before.containers.len()..];
    !residue.is_empty()
        && residue
            .chars()
            .all(|character| character == ']' || character == '}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, role: &str, targets: Vec<(&str, &str)>) -> Value {
        json!({
            "name": name,
            "role": role,
            "gender": "男",
            "age": "二十",
            "appearance": "外貌",
            "personality": "性格",
            "background": "背景",
            "abilities": "能力",
            "motivation": "动机",
            "relationships": targets
                .into_iter()
                .map(|(target, relation)| json!({ "target": target, "relation": relation }))
                .collect::<Vec<Value>>(),
            "arc": "弧光",
            "notes": "备注",
            "currentState": {
                "location": "地点",
                "powerLevel": "战力",
                "physicalState": "身体",
                "mentalState": "心理",
                "keyItems": "剑",
                "recentEvents": "事件",
                "updatedAtChapter": 0,
            },
        })
    }

    fn payload(cards: Vec<Value>) -> String {
        json!({
            "novelConfig": {
                "genre": "玄幻",
                "subGenre": "东方",
                "targetAudience": "男频",
                "plotStructure": "three_act",
                "narrativePOV": "third_limited",
                "coreOutline": "大纲",
                "worldSetting": "世界观",
                "goldenFinger": "金手指",
                "protagonistProfile": "主角",
                "globalGuidance": "总纲",
            },
            "architectureFiles": {
                "premise": "前提",
                "worldbuilding": "世界观档案",
                "synopsis": "梗概",
            },
            "characterCards": cards,
        })
        .to_string()
    }

    fn valid_cards() -> Vec<Value> {
        vec![
            card("林澈", "protagonist", vec![("苏晚", "挚友")]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ]
    }

    #[test]
    fn decode_import_inference_json_accepts_valid_payload() {
        let decoded = decode_import_inference_json(&payload(valid_cards())).unwrap();
        assert_eq!(decoded["novelConfig"]["plotStructure"], json!("three_act"));
        assert_eq!(decoded["architectureFiles"]["premise"], json!("前提"));
        let cards = decoded["characterCards"].as_array().unwrap();
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0]["name"], json!("林澈"));
        assert_eq!(cards[0]["relationships"][0]["target"], json!("苏晚"));
        assert_eq!(cards[0]["currentState"]["updatedAtChapter"], json!(0));
        // 不可见包装残留与 Markdown 围栏均可剥离。
        let wrapped = format!("\u{FEFF}\n```json\n{}\n```\n", payload(valid_cards()));
        assert!(decode_import_inference_json(&wrapped).is_ok());
    }

    #[test]
    fn decode_import_inference_json_reports_paths() {
        // 卡片数越界。
        let two_cards = vec![
            card("林澈", "protagonist", vec![]),
            card("苏晚", "supporting", vec![]),
        ];
        assert_eq!(
            decode_import_inference_json(&payload(two_cards)).unwrap_err(),
            structured_contract_diagnostic("invalid_value", "characterCards")
        );
        // 角色枚举。
        let bad_role = vec![
            card("林澈", "主角", vec![]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ];
        assert_eq!(
            decode_import_inference_json(&payload(bad_role)).unwrap_err(),
            structured_contract_diagnostic("invalid_value", "characterCards[0].role")
        );
        // 缺主角。
        let no_protagonist = vec![
            card("林澈", "supporting", vec![]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ];
        assert_eq!(
            decode_import_inference_json(&payload(no_protagonist)).unwrap_err(),
            structured_contract_diagnostic("missing_item", "characterCards.protagonist")
        );
        // 唯一名称自指。
        let self_reference = vec![
            card("林澈", "protagonist", vec![("林澈", "自己")]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ];
        assert_eq!(
            decode_import_inference_json(&payload(self_reference)).unwrap_err(),
            structured_contract_diagnostic(
                "relationship_self_reference",
                "characterCards[0].relationships[0].target"
            )
        );
        // 关系端点不在名单内。
        let dangling = vec![
            card("林澈", "protagonist", vec![("无名氏", "旧识")]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ];
        assert_eq!(
            decode_import_inference_json(&payload(dangling)).unwrap_err(),
            structured_contract_diagnostic(
                "relationship_endpoint_not_in_characters",
                "characterCards[0].relationships[0].target"
            )
        );
        // 缺字段与空文本。
        let mut missing = valid_cards();
        missing[0].as_object_mut().unwrap().remove("arc");
        assert_eq!(
            decode_import_inference_json(&payload(missing)).unwrap_err(),
            structured_contract_diagnostic("missing_field", "characterCards[0].arc")
        );
        let mut blank = valid_cards();
        blank[0]["notes"] = json!("   ");
        assert_eq!(
            decode_import_inference_json(&payload(blank)).unwrap_err(),
            structured_contract_diagnostic("invalid_value", "characterCards[0].notes")
        );
        // JSON 非法 / 多对象。
        assert_eq!(
            decode_import_inference_json("不是 JSON").unwrap_err(),
            structured_contract_diagnostic("invalid_json", "$")
        );
        let duplicated = format!("{}{}", payload(valid_cards()), payload(valid_cards()));
        assert_eq!(
            decode_import_inference_json(&duplicated).unwrap_err(),
            structured_contract_diagnostic("invalid_json", "$")
        );
    }

    #[test]
    fn decode_import_inference_json_normalizes_numeric_age() {
        let mut cards = valid_cards();
        cards[0]["age"] = json!(20);
        let decoded = decode_import_inference_json(&payload(cards)).unwrap();
        assert_eq!(decoded["characterCards"][0]["age"], json!("20"));
    }

    #[test]
    fn endpoint_delta_requires_exact_missing_targets() {
        // 原始载荷含未闭合端点「神秘人」。
        let original = payload(vec![
            card("林澈", "protagonist", vec![("神秘人", "宿敌")]),
            card("苏晚", "supporting", vec![]),
            card("石决", "antagonist", vec![]),
        ]);
        let root = parse_import_inference_json_object(&original).unwrap();
        assert_eq!(
            unresolved_import_relationship_targets(&root).unwrap(),
            vec!["神秘人".to_string()]
        );
        // 合法 delta：恰好补上该端点。
        let delta = json!({
            "characterCards": [card("神秘人", "minor", vec![])],
        })
        .to_string();
        let merged = decode_import_inference_with_endpoint_delta(&original, &delta).unwrap();
        let names: Vec<&str> = merged["characterCards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| card["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["林澈", "苏晚", "石决", "神秘人"]);

        // delta 名字不匹配。
        let wrong_delta =
            json!({ "characterCards": [card("别的人", "minor", vec![])] }).to_string();
        let message =
            decode_import_inference_with_endpoint_delta(&original, &wrong_delta).unwrap_err();
        assert!(
            message.contains("must exactly match the original unresolved endpoints"),
            "{message}"
        );

        // delta 携带额外字段。
        let mut extra_card = card("神秘人", "minor", vec![]);
        extra_card["extra"] = json!(1);
        let extra_delta = json!({ "characterCards": [extra_card] }).to_string();
        let message =
            decode_import_inference_with_endpoint_delta(&original, &extra_delta).unwrap_err();
        assert!(message.contains("missing or extra fields"), "{message}");

        // 无未闭合端点时拒绝端点补卡。
        let closed = payload(valid_cards());
        let closed_root = parse_import_inference_json_object(&closed).unwrap();
        let message = unresolved_import_relationship_targets(&closed_root).unwrap_err();
        assert!(
            message.contains("no unresolved relationship endpoint"),
            "{message}"
        );
    }

    #[test]
    fn structured_json_evidence_preserves_scalars_and_container_shape() {
        // 仅补全缺失闭合符：允许。
        assert!(preserves_structured_json_evidence(
            "{\"a\":1,\"b\":[1,2",
            "{\"a\":1,\"b\":[1,2]}"
        ));
        // 完全相同：允许。
        assert!(preserves_structured_json_evidence("{\"a\":1}", "{\"a\":1}"));
        // 标量变化：拒绝。
        assert!(!preserves_structured_json_evidence(
            "{\"a\":1}",
            "{\"a\":2}"
        ));
        // 修复结果不可解析：拒绝。
        assert!(!preserves_structured_json_evidence("{\"a\":1}", "{\"a\":1"));
        // 容器形状改变：拒绝。
        assert!(!preserves_structured_json_evidence(
            "{\"a\":[1]}",
            "[{\"a\":1}]"
        ));
        // 闭合符之外的多余内容：拒绝。
        assert!(!preserves_structured_json_evidence(
            "{\"a\":1",
            "{\"a\":1}x"
        ));
    }
}
