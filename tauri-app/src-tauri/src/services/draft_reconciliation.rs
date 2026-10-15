//! 对账输出的只读编解码。所有注入文本均从原始产物重算，不接受调用方自带的解释。

use crate::services::draft_short_outline::trim_js;
use regex::Regex;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fmt;
use std::sync::LazyLock;

const WS: &str =
    r"[\t-\r \u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";
static THINK_PAIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<think>.*?(?:</think>|$)").unwrap());
static THINK_CLOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</think>").unwrap());
static THINK_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?think>").unwrap());
static REASONING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?iu)^{WS}*(?:思考|推理|分析|reasoning|analysis)"
    ))
    .unwrap()
});
static STRUCTURED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?iu)^{WS}*(?:```(?:json)?{WS}*)?[{{\[]")).unwrap());
static FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?iu)^{WS}*```(?:json)?{WS}*|{WS}*```{WS}*$")).unwrap());
static AS_WRITTEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?iu)^(?:按原文(?:落实)?|as written)[。.]?$").unwrap());

#[derive(Debug, PartialEq)]
pub struct DraftReconciliation {
    pub final_state: Vec<String>,
    pub events: Vec<ReconciliationEvent>,
}

#[derive(Debug, PartialEq)]
pub struct ReconciliationEvent {
    pub event: String,
    pub conflict: bool,
    pub realization: String,
}

// 不经 serde_json::Value 中转：默认有序映射会重排作者状态名，必须保留 JSON 输入顺序。
struct States(Vec<String>);
impl<'de> Deserialize<'de> for States {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StateVisitor;
        impl<'de> Visitor<'de> for StateVisitor {
            type Value = States;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("字符串数组或字符串值对象")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<States, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<String>()? {
                    values.push(value);
                    if values.len() > 8 {
                        return Err(de::Error::custom("状态条目过多"));
                    }
                }
                Ok(States(values))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<States, A::Error> {
                let mut fields: Vec<(String, String)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if let Some(entry) = fields.iter_mut().find(|entry| entry.0 == key) {
                        entry.1 = value;
                    } else {
                        fields.push((key, value));
                        if fields.len() > 8 {
                            return Err(de::Error::custom("状态条目过多"));
                        }
                    }
                }
                // Object.entries：数组索引键先按数值升序，其他键保留首次插入顺序。
                fields.sort_by_key(|(key, _)| {
                    key.parse::<u32>()
                        .ok()
                        .filter(|index| *index < u32::MAX && index.to_string() == *key)
                });
                let (indices, ordinary): (Vec<_>, Vec<_>) =
                    fields.into_iter().partition(|(key, _)| {
                        key.parse::<u32>()
                            .is_ok_and(|index| index < u32::MAX && index.to_string() == *key)
                    });
                Ok(States(
                    indices
                        .into_iter()
                        .chain(ordinary)
                        .map(|(key, value)| format!("{key}：{value}"))
                        .collect(),
                ))
            }
        }
        deserializer.deserialize_any(StateVisitor)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReconciliation {
    #[serde(rename = "finalState")]
    final_state: States,
    events: Vec<Value>,
}

fn line(text: &str, maximum: usize, allow_empty: bool) -> Option<String> {
    let mut normalized = String::new();
    let mut whitespace = false;
    for character in text.chars() {
        if (character.is_whitespace() && character != '\u{0085}') || character == '\u{feff}' {
            whitespace = !normalized.is_empty();
        } else {
            if whitespace {
                normalized.push(' ');
                whitespace = false;
            }
            normalized.push(character);
        }
    }
    if (!allow_empty && normalized.is_empty()) || normalized.encode_utf16().count() > maximum {
        return None;
    }
    Some(normalized)
}

fn visible_output(output: &str) -> String {
    let paired = THINK_PAIR.replace_all(output, "");
    let cleaned = if let Some(orphan) = THINK_CLOSE.find(&paired) {
        let prefix = &paired[..orphan.start()];
        let suffix = &paired[orphan.end()..];
        if REASONING.is_match(prefix) || STRUCTURED.is_match(suffix) {
            suffix.to_string()
        } else {
            format!("{prefix}{suffix}")
        }
    } else {
        paired.into_owned()
    };
    let cleaned = THINK_TAG.replace_all(&cleaned, "");
    let cleaned = FENCE.replace_all(trim_js(&cleaned), "");
    trim_js(&cleaned).to_string()
}

pub fn parse_draft_reconciliation(output: &str) -> Option<DraftReconciliation> {
    let visible = visible_output(output);
    let start = visible.find('{')?;
    let end = visible.rfind('}')?;
    if end <= start {
        return None;
    }
    // 重复的顶层合同字段、非法孤立代理项按无效输出保守拒绝，不改写材料合同。
    let raw: RawReconciliation = serde_json::from_str(&visible[start..=end]).ok()?;
    if raw.events.len() > 8 || raw.final_state.0.len() + raw.events.len() == 0 {
        return None;
    }
    let final_state = raw
        .final_state
        .0
        .iter()
        .map(|text| line(text, 300, false))
        .collect::<Option<Vec<_>>>()?;
    let events = raw
        .events
        .iter()
        .map(|value| {
            let entry = value.as_object()?;
            let event = line(entry.get("event")?.as_str()?, 200, false)?;
            let conflict = entry.get("conflict")?.as_bool()?;
            let realization = match entry.get("realization") {
                None | Some(Value::Null) => String::new(),
                Some(value) => line(value.as_str()?, 400, true)?,
            };
            if conflict && realization.is_empty() {
                return None;
            }
            Some(ReconciliationEvent {
                event,
                conflict,
                realization,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(DraftReconciliation {
        final_state,
        events,
    })
}

pub fn render_draft_reconciliation_block(english: bool, value: &DraftReconciliation) -> String {
    let mut lines = vec![if english {
        "[Reconciliation with finalized chapters (automatic pre-draft check; binding)]".to_string()
    } else {
        "【本章与定稿对账（生成前自动对账，须遵守）】".to_string()
    }];
    if !value.final_state.is_empty() {
        lines.push(if english {
            "State at the end of the finalized previous chapter (it holds until this chapter's prose shows a condition being met or a character making a new decision):".to_string()
        } else { "上一章定稿结束时的状态（本章正文写出条件满足或人物作出新决定之前保持不变）：".to_string() });
        lines.extend(value.final_state.iter().map(|state| format!("- {state}")));
    }
    if !value.events.is_empty() {
        lines.push(
            if english {
                "How to realize this chapter's required events:"
            } else {
                "本章必需事件的落实方式："
            }
            .to_string(),
        );
        for event in &value.events {
            lines.push(if event.conflict {
                if english {
                    format!(
                        "- {} (conflicts with the finalized facts if taken literally): {}",
                        event.event, event.realization
                    )
                } else {
                    format!(
                        "- {}（按字面会与定稿冲突）：{}",
                        event.event, event.realization
                    )
                }
            } else {
                let as_written =
                    event.realization.is_empty() || AS_WRITTEN.is_match(&event.realization);
                if english {
                    format!(
                        "- {}: {}",
                        event.event,
                        if as_written {
                            "as written, without contradicting the state above."
                        } else {
                            &event.realization
                        }
                    )
                } else {
                    format!(
                        "- {}：{}",
                        event.event,
                        if as_written {
                            "按原文落实，不得与上述状态矛盾。"
                        } else {
                            &event.realization
                        }
                    )
                }
            });
        }
    }
    let conflicts = value.events.iter().any(|event| event.conflict);
    lines.push(match (english, conflicts) {
        (true, true) => "Conclusion: resolve the conflicts above exactly as stated; never write a withdrawn, cancelled, or superseded plan as executed.",
        (false, true) => "对账结论：以上冲突按给出的落实方式处理；不得把已撤回、取消或被取代的计划写成已执行。",
        (true, false) => "Conclusion: the blueprint does not conflict with the finalized facts; follow the state and realizations above, and never write a withdrawn, paused, or still-waiting plan as executed.",
        (false, false) => "对账结论：蓝图与定稿不冲突；按上述状态与落实方式写，已撤回、暂停或仍在等待条件的计划不得写成已执行。",
    }.to_string());
    lines.join("\n")
}

pub fn draft_reconciliation_block(english: bool, output: &str) -> Option<String> {
    Some(render_draft_reconciliation_block(
        english,
        &parse_draft_reconciliation(output)?,
    ))
}

pub fn strip_draft_reconciliation_block(prompt: &str, output: &str) -> Option<String> {
    let parsed = parse_draft_reconciliation(output)?;
    for english in [false, true] {
        let needle = format!(
            "\n\n{}",
            render_draft_reconciliation_block(english, &parsed)
        );
        if prompt.match_indices(&needle).count() == 1 {
            return Some(prompt.replacen(&needle, "", 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = r#"{"finalState":[" 合成状态\n仍在等待 "],"events":[{"event":"合成事件","conflict":true,"realization":" 条件满足后\t再执行 "}]}"#;

    #[test]
    fn reconciliation_render_matches_readonly_typescript_baseline_full_hash_test() {
        use sha2::{Digest, Sha256};
        // 只读运行 TS 基线所得的完整 UTF-8 哈希，避免片段断言遗漏任一提示合同字节。
        let fixtures = [
            (
                r#"{"finalState":[" 合成状态\n仍在等待 "],"events":[{"event":"合成事件","conflict":true,"realization":" 条件满足后\t再执行 "}]}"#,
                [
                    "64a263840a7b595bd01eb64f5840d81772ef953073a56338fe9e71740f3e9182",
                    "3e35c148a0533b8dfe4327e357997778132aeb351c87a14b4710206e9d985e27",
                ],
            ),
            (
                r#"{"finalState":{"后写":"甲","10":"十","2":"二","先写":"旧值","先写":"新值"},"events":[]}"#,
                [
                    "193424c40cb689270f551162745a707a7700d37c23c6967b068e4d78437642e8",
                    "fa51cbb2fde001dc84b2f8f5551bdadf6d5382da6d59ec6caf49a793d734c170",
                ],
            ),
            (
                r#"{"finalState":[],"events":[{"event":"事件","conflict":false,"realization":null}]}"#,
                [
                    "fd43372b24a106f3c7ac6994c4a08a9193114fc18bd31f4d4ba5d4379b6734e2",
                    "7bc33be562f88fd032ff2f07bbcd40d5ac681d6b966361a28b2c97e44eafbb67",
                ],
            ),
            (
                r#"{"finalState":[],"events":[{"event":"事件","conflict":false,"realization":"必须先写条件满足"}]}"#,
                [
                    "44707c3c4246a7bbe7fe1f85780db73c4358cdbc886f0f35ff44ac4dc9d3f003",
                    "0814bb3b23bfaeba1bdac767bf75b9c9214c37f9db9801f1e4377aa201825076",
                ],
            ),
        ];
        for (input, hashes) in fixtures {
            for (english, expected) in [(false, hashes[0]), (true, hashes[1])] {
                let block = draft_reconciliation_block(english, input).unwrap();
                assert_eq!(format!("{:x}", Sha256::digest(block.as_bytes())), expected);
            }
        }
    }

    #[test]
    fn bilingual_reconciliation_blocks_strip_exactly_once_at_any_position_test() {
        for english in [false, true] {
            let block = draft_reconciliation_block(english, OUTPUT).unwrap();
            assert!(block.contains("条件满足后 再执行"));
            let prompt = format!("原始材料\n\n{block}\n\n后续约束");
            assert_eq!(
                strip_draft_reconciliation_block(&prompt, OUTPUT),
                Some("原始材料\n\n后续约束".to_string())
            );
            assert_eq!(
                strip_draft_reconciliation_block(&format!("{prompt}\n\n{block}"), OUTPUT),
                None
            );
            assert_eq!(
                strip_draft_reconciliation_block(
                    &prompt.replace("条件满足后 再执行", "任意新事实"),
                    OUTPUT
                ),
                None
            );
            assert_eq!(strip_draft_reconciliation_block(&block, OUTPUT), None);
        }
    }

    #[test]
    fn reconciliation_removes_only_thinking_and_preserves_js_state_order_test() {
        let source = r#"{"finalState":{"后写":"甲","10":"十","2":"二","先写":"旧值","先写":"新值"},"events":[]}"#;
        let wrapped = format!("<THINK>{{隐藏推理}}</think>\n```JSON\n{source}\n```");
        let parsed = parse_draft_reconciliation(&wrapped).unwrap();
        assert_eq!(
            parsed.final_state,
            ["2：二", "10：十", "后写：甲", "先写：新值"]
        );
        assert_eq!(
            parse_draft_reconciliation(&format!("隐藏推理</think>\n{source}")),
            Some(parsed)
        );
        assert!(parse_draft_reconciliation(
            "<think>未闭合推理 {\"finalState\":[\"不可见\"],\"events\":[]}"
        )
        .is_none());
    }

    #[test]
    fn reconciliation_rejects_shapes_limits_and_conflicts_without_realization_test() {
        for output in [
            r#"{"finalState":[],"events":[]}"#,
            r#"{"finalState":[1],"events":[]}"#,
            r#"{"finalState":{},"events":[{"event":"事件","conflict":true}]}"#,
            r#"{"finalState":[],"events":[{"event":"事件","conflict":"false"}]}"#,
            r#"{"finalState":[],"events":[{"event":"事件","conflict":false,"realization":1}]}"#,
        ] {
            assert!(
                parse_draft_reconciliation(output).is_none(),
                "必须拒绝 {output}"
            );
        }
        for state in ["😀".repeat(151), "x".repeat(301)] {
            assert!(parse_draft_reconciliation(
                &serde_json::json!({"finalState":[state],"events":[]}).to_string()
            )
            .is_none());
        }
        assert!(parse_draft_reconciliation(
            &serde_json::json!({"finalState":vec!["状态";9],"events":[]}).to_string()
        )
        .is_none());
        assert!(parse_draft_reconciliation(
            &serde_json::json!({"finalState":["😀".repeat(150)],"events":[]}).to_string()
        )
        .is_some());
        for (event, realization) in [
            ("x".repeat(201), "落实".to_string()),
            ("事件".to_string(), "😀".repeat(201)),
        ] {
            let output=serde_json::json!({"finalState":[],"events":[{"event":event,"conflict":true,"realization":realization}]}).to_string();
            assert!(parse_draft_reconciliation(&output).is_none());
        }
        let item = serde_json::json!({"event":"事件","conflict":false});
        assert!(parse_draft_reconciliation(
            &serde_json::json!({"finalState":[],"events":vec![item;9]}).to_string()
        )
        .is_none());
        for output in [
            r#"{"finalState":["前值"],"finalState":["后值"],"events":[]}"#,
            r#"{"finalState":["\ud800"],"events":[]}"#,
        ] {
            assert!(parse_draft_reconciliation(output).is_none());
        }
        assert_eq!(
            line("\u{feff}甲\u{feff}\n乙\u{feff}", 300, false),
            Some("甲 乙".to_string())
        );
        assert_eq!(line("\u{0085}", 300, false), Some("\u{0085}".to_string()));
    }

    #[test]
    fn reconciliation_nonconflicting_realizations_are_not_replaced_by_default_test() {
        for realization in ["", "按原文", "按原文落实。", "AS WRITTEN."] {
            let output = serde_json::json!({"finalState":[],"events":[{"event":"事件","conflict":false,"realization":realization}]}).to_string();
            assert!(draft_reconciliation_block(false, &output)
                .unwrap()
                .contains("按原文落实，不得与上述状态矛盾。"));
            assert!(draft_reconciliation_block(true, &output)
                .unwrap()
                .contains("as written, without contradicting the state above."));
        }
        let output = r#"{"finalState":[],"events":[{"event":"事件","conflict":false,"realization":"必须先写条件满足"}]}"#;
        assert!(draft_reconciliation_block(false, output)
            .unwrap()
            .contains("- 事件：必须先写条件满足"));
    }
}
