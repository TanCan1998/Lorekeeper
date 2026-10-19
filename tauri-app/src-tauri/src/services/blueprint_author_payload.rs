//! 蓝图作者恢复载荷解码 —— 对齐 `src/shared/blueprint-semantic-contract.ts`
//! 的 `decodeBlueprintAuthorPayload` / `decodeBlueprintSemanticPayload` /
//! `normalizeBlueprintSemanticItem`。
//!
//! 渲染层已按同一合同归一化过一次；本模块在提交前重新校验，避免旧客户端、
//! 手工构造请求或字段漂移把不完整载荷写进蓝图事实。
//!
//! 覆盖范围：章节精确覆盖、必填文本、角色列表、关系端点与去重、
//! 新角色候选的角色枚举。不覆盖：模型侧语法修复路径（不属于作者恢复）。

use serde_json::{json, Value};

use crate::repositories::blueprint_repository::BlueprintData;

/// 角色枚举（对齐 `src/shared/character-role.ts` 的 `CHARACTER_ROLES`）。
const CHARACTER_ROLES: [&str; 4] = ["protagonist", "antagonist", "supporting", "minor"];

fn invalid(kind: &str, path: &str) -> String {
    format!("蓝图作者载荷无效：{kind}（{path}）")
}

fn required_text(value: Option<&Value>, path: &str) -> Result<String, String> {
    match value {
        None | Some(Value::Null) => Err(invalid("missing_field", path)),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Err(invalid("empty_value", path))
            } else {
                Ok(trimmed.to_string())
            }
        }
        Some(_) => Err(invalid("invalid_type", path)),
    }
}

/// `fieldValue(object, canonical, aliases)`：仅对**缺失**回退别名
/// （`null` 仍按字面值参与校验）。
fn field<'a>(
    object: &'a serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) -> Option<&'a Value> {
    object
        .get(canonical)
        .or_else(|| aliases.iter().find_map(|alias| object.get(*alias)))
}

fn characters_of(value: Option<&Value>, path: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Err(invalid("missing_field", path));
    };
    let Some(items) = value.as_array() else {
        return Err(invalid("invalid_type", path));
    };
    if items.is_empty() {
        return Err(invalid("invalid_value", path));
    }
    let mut characters = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let name = required_text(Some(item), &format!("{path}[{index}]"))?;
        if characters.contains(&name) {
            return Err(invalid("duplicate_item", path));
        }
        characters.push(name);
    }
    Ok(characters)
}

fn relationships_of(
    value: Option<&Value>,
    path: &str,
    characters: &[String],
) -> Result<Value, String> {
    let Some(value) = value else {
        return Err(invalid("missing_field", path));
    };
    let Some(items) = value.as_array() else {
        return Err(invalid("invalid_type", path));
    };
    let mut seen: Vec<String> = Vec::new();
    let mut facts = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(object) = item.as_object() else {
            return Err(invalid("invalid_type", &item_path));
        };
        let from = required_text(
            field(object, "from", &["source"]),
            &format!("{item_path}.from"),
        )?;
        let to = required_text(field(object, "to", &["target"]), &format!("{item_path}.to"))?;
        let relation = required_text(object.get("relation"), &format!("{item_path}.relation"))?;
        if from == to {
            return Err(invalid("relationship_self_reference", &item_path));
        }
        if !characters.contains(&from) || !characters.contains(&to) {
            return Err(invalid(
                "relationship_endpoint_not_in_characters",
                &item_path,
            ));
        }
        let key = [from.as_str(), to.as_str(), relation.as_str()].join("\u{0}");
        if seen.contains(&key) {
            return Err(invalid("duplicate_item", path));
        }
        seen.push(key);
        facts.push(json!({ "from": from, "to": to, "relation": relation }));
    }
    Ok(Value::Array(facts))
}

fn candidates_of(
    value: Option<&Value>,
    path: &str,
    characters: &[String],
) -> Result<Option<Vec<Value>>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err(invalid("invalid_type", path));
    };
    let mut seen: Vec<String> = Vec::new();
    let mut candidates = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(object) = item.as_object() else {
            return Err(invalid("invalid_type", &item_path));
        };
        let name = required_text(object.get("name"), &format!("{item_path}.name"))?;
        if !characters.contains(&name) {
            return Err(invalid("invalid_value", &format!("{item_path}.name")));
        }
        if seen.contains(&name) {
            return Err(invalid("duplicate_item", path));
        }
        seen.push(name.clone());
        let role = required_text(object.get("role"), &format!("{item_path}.role"))?;
        if !CHARACTER_ROLES.contains(&role.as_str()) {
            return Err(invalid("invalid_value", &format!("{item_path}.role")));
        }
        candidates.push(json!({ "name": name, "role": role }));
    }
    Ok(if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    })
}

fn chapter_number_of(blueprint: &BlueprintData, index: usize) -> Result<i64, String> {
    if blueprint.chapter_number < 1 {
        return Err(invalid(
            "invalid_value",
            &format!("blueprints[{index}].chapterNumber"),
        ));
    }
    Ok(blueprint.chapter_number)
}

/// 逐条归一化并在最后校验章节精确覆盖；返回按章号升序的载荷。
pub(crate) fn normalize_blueprint_author_payload(
    blueprints: Vec<BlueprintData>,
    chapters: &[i64],
) -> Result<Vec<BlueprintData>, String> {
    let mut expected: Vec<i64> = Vec::with_capacity(chapters.len());
    for chapter in chapters {
        if *chapter < 1 || expected.contains(chapter) {
            return Err(invalid("invalid_value", "expectedChapterNumbers"));
        }
        expected.push(*chapter);
    }
    let mut normalized = Vec::with_capacity(blueprints.len());
    let mut seen: Vec<i64> = Vec::with_capacity(blueprints.len());
    for (index, blueprint) in blueprints.into_iter().enumerate() {
        let path = format!("blueprints[{index}]");
        let chapter_number = chapter_number_of(&blueprint, index)?;
        let title = required_text(
            Some(&Value::String(blueprint.title)),
            &format!("{path}.title"),
        )?;
        let role = required_text(
            Some(&Value::String(blueprint.role)),
            &format!("{path}.role"),
        )?;
        let purpose = required_text(
            Some(&Value::String(blueprint.purpose)),
            &format!("{path}.purpose"),
        )?;
        let key_events = required_text(
            Some(&Value::String(blueprint.key_events)),
            &format!("{path}.keyEvents"),
        )?;
        let suspense_hook = required_text(
            Some(&Value::String(blueprint.suspense_hook)),
            &format!("{path}.suspenseHook"),
        )?;
        let characters = characters_of(
            Some(&json!(blueprint.characters)),
            &format!("{path}.characters"),
        )?;
        let relationship_hints = relationships_of(
            blueprint.relationship_hints.as_ref(),
            &format!("{path}.relationships"),
            &characters,
        )?;
        let candidates_value = blueprint
            .new_character_candidates
            .as_ref()
            .map(|items| Value::Array(items.clone()));
        let new_character_candidates = candidates_of(
            candidates_value.as_ref(),
            &format!("{path}.newCharacterCandidates"),
            &characters,
        )?;
        if seen.contains(&chapter_number) {
            return Err(invalid("duplicate_item", "blueprints"));
        }
        seen.push(chapter_number);
        if !expected.contains(&chapter_number) {
            return Err(invalid("unexpected_item", "blueprints"));
        }
        normalized.push(BlueprintData {
            chapter_number,
            title,
            role,
            purpose,
            key_events,
            characters,
            new_character_candidates,
            relationship_hints: Some(relationship_hints),
            suspense_hook,
            user_guidance: blueprint.user_guidance,
            notes: blueprint.notes,
            notes_updated_at: blueprint.notes_updated_at,
        });
    }
    if expected.iter().any(|chapter| !seen.contains(chapter)) {
        return Err(invalid("missing_item", "blueprints"));
    }
    normalized.sort_by_key(|blueprint| blueprint.chapter_number);
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blueprint(chapter: i64) -> BlueprintData {
        BlueprintData {
            chapter_number: chapter,
            title: format!("第{chapter}章"),
            role: "推进".to_string(),
            purpose: "推进主线".to_string(),
            key_events: "发生事件".to_string(),
            characters: vec!["林见".to_string()],
            new_character_candidates: None,
            relationship_hints: Some(json!([])),
            suspense_hook: "悬而未决".to_string(),
            user_guidance: String::new(),
            notes: String::new(),
            notes_updated_at: String::new(),
        }
    }

    #[test]
    fn accepts_exact_coverage_and_sorts_by_chapter_test() {
        let payload =
            normalize_blueprint_author_payload(vec![blueprint(2), blueprint(1)], &[1, 2]).unwrap();
        assert_eq!(
            payload
                .iter()
                .map(|item| item.chapter_number)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(payload[0].relationship_hints, Some(json!([])));
    }

    #[test]
    fn rejects_missing_extra_and_duplicate_chapters_test() {
        for (payload, chapters) in [
            (vec![blueprint(1)], vec![1, 2]),
            (vec![blueprint(1), blueprint(3)], vec![1, 2]),
            (vec![blueprint(1), blueprint(1)], vec![1, 2]),
            (vec![blueprint(1)], vec![1, 1]),
        ] {
            assert!(normalize_blueprint_author_payload(payload, &chapters).is_err());
        }
    }

    #[test]
    fn rejects_blank_fields_empty_characters_and_duplicates_test() {
        let mut blank = blueprint(1);
        blank.title = "   ".to_string();
        let mut empty_characters = blueprint(1);
        empty_characters.characters = Vec::new();
        let mut duplicate_characters = blueprint(1);
        duplicate_characters.characters = vec!["林见".to_string(), " 林见 ".to_string()];
        let mut missing_hook = blueprint(1);
        missing_hook.suspense_hook = String::new();
        for payload in [blank, empty_characters, duplicate_characters, missing_hook] {
            assert!(normalize_blueprint_author_payload(vec![payload], &[1]).is_err());
        }
    }

    #[test]
    fn normalizes_relationships_with_aliases_and_rejects_invalid_ones_test() {
        let mut item = blueprint(1);
        item.characters = vec!["林见".to_string(), "沈舟".to_string()];
        item.relationship_hints =
            Some(json!([{ "source": "林见", "target": "沈舟", "relation": "同盟" }]));
        let payload = normalize_blueprint_author_payload(vec![item], &[1]).unwrap();
        assert_eq!(
            payload[0].relationship_hints,
            Some(json!([{ "from": "林见", "to": "沈舟", "relation": "同盟" }]))
        );

        for hints in [
            json!([{ "from": "林见", "to": "林见", "relation": "自指" }]),
            json!([{ "from": "林见", "to": "外人", "relation": "同盟" }]),
            json!([{ "from": "林见", "to": "沈舟", "relation": " " }]),
            json!([
                { "from": "林见", "to": "沈舟", "relation": "同盟" },
                { "from": "林见", "to": "沈舟", "relation": "同盟" }
            ]),
            json!("不是数组"),
        ] {
            let mut item = blueprint(1);
            item.characters = vec!["林见".to_string(), "沈舟".to_string()];
            item.relationship_hints = Some(hints);
            assert!(normalize_blueprint_author_payload(vec![item], &[1]).is_err());
        }

        let mut missing = blueprint(1);
        missing.relationship_hints = None;
        assert!(normalize_blueprint_author_payload(vec![missing], &[1]).is_err());
    }

    #[test]
    fn rejects_candidate_outside_characters_and_unknown_role_test() {
        let mut outside = blueprint(1);
        outside.new_character_candidates = Some(vec![json!({ "name": "外人", "role": "minor" })]);
        let mut bad_role = blueprint(1);
        bad_role.new_character_candidates =
            Some(vec![json!({ "name": "林见", "role": "narrator" })]);
        let mut duplicate = blueprint(1);
        duplicate.new_character_candidates = Some(vec![
            json!({ "name": "林见", "role": "minor" }),
            json!({ "name": "林见", "role": "minor" }),
        ]);
        for payload in [outside, bad_role, duplicate] {
            assert!(normalize_blueprint_author_payload(vec![payload], &[1]).is_err());
        }

        let mut valid = blueprint(1);
        valid.new_character_candidates =
            Some(vec![json!({ "name": "林见", "role": "protagonist" })]);
        let payload = normalize_blueprint_author_payload(vec![valid], &[1]).unwrap();
        assert_eq!(
            payload[0].new_character_candidates,
            Some(vec![json!({ "name": "林见", "role": "protagonist" })])
        );
    }
}
