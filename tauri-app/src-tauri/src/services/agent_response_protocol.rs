//! Agent 响应协议解析（1:1 移植自 `src/shared/agent-response-protocol.ts`）。
//!
//! 基线契约：agent 协议文本可能以「原始调用」（名称 + JSON 参数块）、
//! 信封（`{name, arguments}` JSON）或标记块（XML/DSML 标签）形式到达；
//! 解析器把**已定居**响应解析为可见文本与工具调用——这不是流式前缀
//! 投影，不完整的原始调用仍可能被解析为动作。中断投影则相反：终端
//! 中断可以保留散文，但从不把部分调用提升为工具动作。
//!
//! B3-2b 起为 `generation_run_service` 的 `agentToolNames` 路径供给
//! 解析原语（基线 generation-run-service.ts:177-181/:267-273 消费）。
//!
//! 正则语义对齐说明：基线 JS 正则的 `\w` 为 ASCII 词字符，Rust 侧以
//! 显式 ASCII 类 `[A-Za-z0-9_.-]` 逐字对齐（`(?-u)` 模式下 `\s`/`[\s\S]`
//! 会匹配字节而遭 regex crate 拒绝）；`MARKER` 含全角「｜DSML｜」字面量，
//! 保持 Unicode 模式（基线无 `u` 标志时 `\s` 同为 Unicode 空白集）。
//! 动态正则（基线 `new RegExp(...)`）以字符串查找等价——标签名来自
//! 固定白名单，不含正则元字符。

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

/// 延迟编译正则（对齐 `repositories/generation_run_repository.rs:1710`
/// 的 `LazyLock` 先例；`regex::Regex` 为 `Sync`，可静态共享）。
static RE_RAW_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z][A-Za-z0-9_.-]*)[ \t]*\r?\n\s*(\{[\s\S]*\})\s*$")
        .expect("raw-call 协议正则为基线逐字平移，编译不可失败")
});
static RE_TAGGED_EMPTY_TOOL: LazyLock<Regex> = LazyLock::new(|| {
    // 基线 `/^<([A-Za-z][\w.-]*)>\s*<\/\1>$/` 的反向引用 `\1` 在
    // regex crate 不受支持；以双捕获组等价替代并比较两组同名。
    Regex::new(r"^<([A-Za-z][A-Za-z0-9_.-]*)>\s*</([A-Za-z][A-Za-z0-9_.-]*)>$")
        .expect("tagged 协议正则为基线逐字平移，编译不可失败")
});
static RE_TAGGED_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^<name>\s*([A-Za-z][A-Za-z0-9_.-]*)\s*</name>\s*<arguments>\s*(\{[\s\S]*\})\s*</arguments>$",
    )
    .expect("tagged-call 协议正则为基线逐字平移，编译不可失败")
});
static RE_BLOCK_TOOL_TAG: LazyLock<Regex> = LazyLock::new(|| {
    // 基线 look-ahead `(?=[\s>])` 在 regex crate 不受支持；
    // `is_match` 只检查存在性，以 `[\s>]` 消耗等价。
    Regex::new(r"(?i)</?(?:tool_call|｜DSML｜tool_call)[\s>]")
        .expect("block 工具标签正则为基线逐字平移，编译不可失败")
});
static RE_EMBEDDED_JSON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{[\s\S]*\}").expect("内嵌 JSON 正则为基线逐字平移，编译不可失败")
});
static RE_PROTOCOL_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    // 基线 `MARKER` 常量（`gi` 标志经 `new RegExp(MARKER)` 复制到
    // `scanProtocol` 的迭代正则）：alternation 以长标签在前
    // （`thinking` 先于 `think`）——基线依赖 JS 回溯达成
    // 同一匹配集，regex crate 无回溯，故显式排序；look-ahead
    // `(?=[\s>])` 以捕获组边界校验等价（见
    // `marker_tag_boundary_valid`）。
    Regex::new(
        r"(?i)<(\/?)(thinking|think|analysis|reasoning|tool_call|｜DSML｜tool_call|tool_result)([^<>]*)>",
    )
    .expect("协议标记正则为基线逐字平移，编译不可失败")
});
static RE_MULTI_NEWLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("多空行压缩正则为基线逐字平移，编译不可失败"));

/// 基线 `HIDDEN_TAGS`：推理/分析块永不进入可见文本。
const HIDDEN_TAGS: &[&str] = &["think", "thinking", "analysis", "reasoning"];
/// 基线 `PROTOCOL_TAGS`：部分标记尾判定用白名单。
const PROTOCOL_TAGS: &[&str] = &[
    "think",
    "thinking",
    "analysis",
    "reasoning",
    "tool_call",
    "｜DSML｜tool_call",
    "tool_result",
];

/// 基线 `AgentProtocolToolCall`。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentProtocolToolCall {
    pub name: String,
    pub arguments: Map<String, Value>,
}

/// 基线 `AgentResponseProtocol`。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentResponseProtocol {
    pub text_parts: Vec<String>,
    pub tool_calls: Vec<AgentProtocolToolCall>,
    pub visible_text: String,
}

/// 基线 `objectArguments`：非数组对象守卫。
fn object_arguments(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

/// 基线 `parseRawCall`：`名称\n{...}` 原始调用形式。
fn parse_raw_call(text: &str, registered: &HashSet<String>) -> Option<AgentProtocolToolCall> {
    let caps = RE_RAW_CALL.captures(text)?;
    let name = caps.get(1)?.as_str();
    if !registered.contains(name) {
        return None;
    }
    let arguments: Value = serde_json::from_str(caps.get(2)?.as_str()).ok()?;
    let arguments = object_arguments(&arguments)?.clone();
    Some(AgentProtocolToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// 基线 `parseEnvelope`：`{name, arguments}` 信封形式。
fn parse_envelope(text: &str, registered: &HashSet<String>) -> Option<AgentProtocolToolCall> {
    let envelope = text.trim();
    if !envelope.starts_with('{') || !envelope.ends_with('}') {
        return None;
    }
    let value: Value = serde_json::from_str(envelope).ok()?;
    let map = object_arguments(&value)?;
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    if keys.len() != 2 || !keys.contains(&"name") || !keys.contains(&"arguments") {
        return None;
    }
    let name = map.get("name")?.as_str()?;
    if !registered.contains(name) {
        return None;
    }
    let arguments = object_arguments(map.get("arguments")?)?.clone();
    Some(AgentProtocolToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// 基线 `parseTaggedCall`：`<name>…</name><arguments>…</arguments>`
/// 标记块形式（含空工具 `<name></name>` 特例）。
fn parse_tagged_call(text: &str, registered: &HashSet<String>) -> Option<AgentProtocolToolCall> {
    // 基线空工具特例：`/^<([A-Za-z][\w.-]*)>\s*<\/\1>$/`。
    if let Some(caps) = RE_TAGGED_EMPTY_TOOL.captures(text) {
        let name = caps.get(1)?.as_str();
        let closing = caps.get(2)?.as_str();
        if name == closing && registered.contains(name) {
            return Some(AgentProtocolToolCall {
                name: name.to_string(),
                arguments: Map::new(),
            });
        }
    }
    let caps = RE_TAGGED_CALL.captures(text)?;
    let name = caps.get(1)?.as_str();
    let arguments: Value = serde_json::from_str(caps.get(2)?.as_str()).ok()?;
    let arguments = object_arguments(&arguments)?.clone();
    Some(AgentProtocolToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// 基线 `parseBlock` 内层 `parseJson`：`{name, arguments}` JSON 块。
fn parse_block_json(content: &str) -> Option<AgentProtocolToolCall> {
    let value: Value = serde_json::from_str(content).ok()?;
    let map = object_arguments(&value)?;
    let name = map.get("name")?.as_str()?;
    if name.is_empty() {
        return None;
    }
    let arguments = map
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let arguments = object_arguments(&arguments)?.clone();
    Some(AgentProtocolToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// 基线 `parseBlock`：工具调用块解析。标记调用历史上保留未知名称，
/// 让宿主返回「未知工具」观测；解析从不授予工具权限。
fn parse_block(text: &str, registered: &HashSet<String>) -> Option<AgentProtocolToolCall> {
    // 直接 JSON 形式优先。
    if let Some(direct) = parse_block_json(text) {
        return Some(direct);
    }
    // 内层包装不能证明外层控制块完整。
    if RE_BLOCK_TOOL_TAG.is_match(text) {
        return None;
    }
    let embedded_json = RE_EMBEDDED_JSON
        .find(text)
        .map(|matched| matched.as_str().to_string());
    let tagged = parse_tagged_call(text, registered);
    tagged.or_else(|| embedded_json.as_deref().and_then(parse_block_json))
}

/// 基线 `removePartialMarkerTail`：剥离末尾未闭合标记的部分尾。
fn remove_partial_marker_tail(text: &str) -> String {
    let Some(start) = text.rfind('<') else {
        return text.to_string();
    };
    let suffix = text[start + 1..].trim_start_matches('/').to_lowercase();
    if suffix.is_empty() || suffix.contains('>') {
        return text.to_string();
    }
    let trim = PROTOCOL_TAGS.iter().any(|tag| {
        let tag = tag.to_lowercase();
        tag.starts_with(&suffix)
            || (suffix.starts_with(&tag)
                && suffix[tag.len()..].starts_with([' ', '\t', '\r', '\n', '/']))
    });
    if trim {
        text[..start].to_string()
    } else {
        text.to_string()
    }
}

/// 基线 `scanProtocol` 的中间扫描结果。
struct ScannedProtocol {
    text_parts: Vec<String>,
    tool_calls: Vec<AgentProtocolToolCall>,
    cleaned: String,
}

/// 基线 `flushText`：非空待定文本入 `textParts`（trim 后），原文入
/// 可见文本。
fn flush_pending(pending_text: &mut String, visible: &mut String, text_parts: &mut Vec<String>) {
    let trimmed = pending_text.trim();
    if !trimmed.is_empty() {
        text_parts.push(trimmed.to_string());
    }
    visible.push_str(pending_text);
    pending_text.clear();
}

/// 基线 `MARKER` 正则 look-ahead `(?=[\s>])` 的等价
/// 校验（regex crate 不支持 look-around）：`tail` 为
/// 标签名与 `>` 之间的 `[^<>]*` 捕获组——为空（`>`
/// 紧随标签名）或以空白开头才视为合法标记边界。
fn marker_tag_boundary_valid(tail: &str) -> bool {
    tail.is_empty() || tail.starts_with(char::is_whitespace)
}

/// 基线 `scanProtocol`：标记迭代剥离隐藏块并提取工具调用。
///
/// 基线以 `RegExp` 的 `lastIndex` 手动控制迭代位置；Rust 侧以
/// `find_at`/`captures_at`（从指定偏移起匹配）逐字等价。
fn scan_protocol(text: &str, registered: &HashSet<String>) -> ScannedProtocol {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<AgentProtocolToolCall> = Vec::new();
    let mut visible = String::new();
    let mut pending_text = String::new();
    let mut offset = 0usize;
    loop {
        // `markers.exec(text)`（g 标志，lastIndex = offset）。
        let Some(marker) = RE_PROTOCOL_MARKER.find_at(text, offset) else {
            break;
        };
        let Some(caps) = RE_PROTOCOL_MARKER.captures_at(text, marker.start()) else {
            break;
        };
        // 基线 look-ahead `(?=[\s>])` 的等价校验：标签名后
        // 必须紧跟空白或 `>`。不满足时正则引擎在基线中
        // 回溯，Rust 侧从 `<` 之后继续查找。
        if !marker_tag_boundary_valid(caps.get(3).map(|m| m.as_str()).unwrap_or("")) {
            offset = marker.start() + 1;
            continue;
        }
        pending_text.push_str(&text[offset..marker.start()]);
        offset = marker.end();
        let is_closing = caps.get(1).map(|m| !m.as_str().is_empty()).unwrap_or(false);
        let name = caps
            .get(2)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();

        if is_closing {
            // 缺失起始推理标签时不留已证可见前缀。
            if HIDDEN_TAGS.contains(&name.as_str()) {
                pending_text.clear();
                visible.clear();
                text_parts.clear();
                tool_calls.clear();
            }
            continue;
        }

        if HIDDEN_TAGS.contains(&name.as_str()) {
            // 隐藏块：栈式配对扫描；未闭合时丢弃其后全部文本。
            let mut hidden_stack: Vec<String> = vec![name.clone()];
            let mut inner_offset = offset;
            while !hidden_stack.is_empty() {
                let Some(inner) = RE_PROTOCOL_MARKER.find_at(text, inner_offset) else {
                    break;
                };
                let Some(inner_caps) = RE_PROTOCOL_MARKER.captures_at(text, inner.start()) else {
                    break;
                };
                if !marker_tag_boundary_valid(inner_caps.get(3).map(|m| m.as_str()).unwrap_or("")) {
                    inner_offset = inner.start() + 1;
                    continue;
                }
                let inner_name = inner_caps
                    .get(2)
                    .map(|m| m.as_str().to_lowercase())
                    .unwrap_or_default();
                let inner_is_closing = inner_caps
                    .get(1)
                    .map(|m| !m.as_str().is_empty())
                    .unwrap_or(false);
                if HIDDEN_TAGS.contains(&inner_name.as_str()) {
                    if !inner_is_closing {
                        hidden_stack.push(inner_name);
                    } else if hidden_stack
                        .last()
                        .map(|top| top == &inner_name)
                        .unwrap_or(false)
                    {
                        hidden_stack.pop();
                    }
                }
                inner_offset = inner.end();
            }
            if !hidden_stack.is_empty() {
                offset = text.len();
                break;
            }
            offset = inner_offset;
            continue;
        }

        // 非隐藏、非闭合标签（tool_call / tool_result）：配对闭合标签。
        let closing_pattern = format!("</{}>", caps.get(2).map(|m| m.as_str()).unwrap_or(""));
        let Some(end_rel) = text[offset..].find(&closing_pattern) else {
            offset = text.len();
            break;
        };
        let end = offset + end_rel;
        if name != "tool_result" {
            flush_pending(&mut pending_text, &mut visible, &mut text_parts);
            if let Some(parsed) = parse_block(text[offset..end].trim(), registered) {
                tool_calls.push(parsed);
            }
        }
        offset = end + closing_pattern.len();
    }

    pending_text.push_str(&remove_partial_marker_tail(&text[offset..]));
    flush_pending(&mut pending_text, &mut visible, &mut text_parts);
    let cleaned = RE_MULTI_NEWLINE
        .replace_all(&visible, "\n\n")
        .trim()
        .to_string();
    ScannedProtocol {
        text_parts,
        tool_calls,
        cleaned,
    }
}

/// 基线 `parseAgentResponseProtocol`：对已定居响应按冻结注册表解析。
/// 这不是流式前缀投影——不完整的原始调用仍可能转化为动作。
pub fn parse_agent_response_protocol(
    content: &str,
    registered_tool_names: &[String],
) -> AgentResponseProtocol {
    let registered: HashSet<String> = registered_tool_names.iter().cloned().collect();
    if let Some(raw_call) =
        parse_raw_call(content, &registered).or_else(|| parse_envelope(content, &registered))
    {
        return AgentResponseProtocol {
            text_parts: Vec::new(),
            tool_calls: vec![raw_call],
            visible_text: String::new(),
        };
    }
    let parsed = scan_protocol(content, &registered);
    let visible_text = RE_MULTI_NEWLINE
        .replace_all(&parsed.text_parts.join(""), "\n\n")
        .trim()
        .to_string();
    AgentResponseProtocol {
        text_parts: parsed.text_parts,
        tool_calls: parsed.tool_calls,
        visible_text,
    }
}

/// 基线 `cleanAgentProtocolVisibleText`：清洗已选可见响应时保留
/// 散文间距。
pub fn clean_agent_protocol_visible_text(
    content: &str,
    registered_tool_names: &[String],
) -> String {
    let registered: HashSet<String> = registered_tool_names.iter().cloned().collect();
    if parse_raw_call(content, &registered)
        .or_else(|| parse_envelope(content, &registered))
        .is_some()
    {
        return String::new();
    }
    scan_protocol(content, &registered).cleaned
}

/// 基线 `projectInterruptedAgentVisibleText`：终端中断可以保留散文，
/// 但从不把部分调用提升为工具动作。含糊的原始 JSON/名称前缀被扣留。
pub fn project_interrupted_agent_visible_text(
    content: &str,
    registered_tool_names: &[String],
) -> String {
    let visible = clean_agent_protocol_visible_text(content, registered_tool_names);
    let trimmed = visible.trim_start();
    if trimmed.starts_with('{') {
        return String::new();
    }
    let first_line = trimmed
        .split_once("\r\n")
        .map(|(line, _)| line)
        .unwrap_or(trimmed);
    let first_line = first_line.split('\n').next().unwrap_or("").trim();
    if !first_line.is_empty()
        && registered_tool_names.iter().any(|name| {
            name == first_line || (name.starts_with(first_line) && !trimmed.contains('\n'))
        })
    {
        return String::new();
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn parse_raw_call_accepts_registered_name() {
        let text = "make_section\n{\"title\":\"序章\"}";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "make_section");
        assert_eq!(
            parsed.tool_calls[0]
                .arguments
                .get("title")
                .and_then(Value::as_str),
            Some("序章")
        );
        assert!(parsed.visible_text.is_empty());
        assert!(parsed.text_parts.is_empty());
    }

    #[test]
    fn parse_raw_call_rejects_unregistered_name() {
        let parsed = parse_agent_response_protocol("unknown_tool\n{}", &tools(&["make_section"]));
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn parse_envelope_requires_exact_two_keys() {
        let envelope = r#"{"name":"make_section","arguments":{"x":1}}"#;
        let parsed = parse_agent_response_protocol(envelope, &tools(&["make_section"]));
        assert_eq!(parsed.tool_calls.len(), 1);
        // 三键信封被拒绝。
        let extra = r#"{"name":"make_section","arguments":{},"extra":1}"#;
        let parsed = parse_agent_response_protocol(extra, &tools(&["make_section"]));
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn parse_tagged_call_extracts_block() {
        let text = "前言\n<tool_call><name>make_section</name>\n<arguments>{\"title\":\"序章\"}</arguments></tool_call>";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "make_section");
        // 标记前的散文保留在可见文本。
        assert!(parsed.visible_text.contains("前言"));
    }

    #[test]
    fn parse_tagged_empty_tool_yields_empty_arguments() {
        let parsed = parse_agent_response_protocol(
            "<tool_call><make_section></make_section></tool_call>",
            &tools(&["make_section"]),
        );
        assert_eq!(parsed.tool_calls.len(), 1);
        assert!(parsed.tool_calls[0].arguments.is_empty());
    }

    #[test]
    fn hidden_tags_are_stripped_from_visible_text() {
        let text = "之前之后";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.visible_text, "之前之后");
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn unclosed_hidden_tag_discards_rest() {
        let text = "可见<think>未闭合推理";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.visible_text, "可见");
    }

    #[test]
    fn nested_hidden_tags_stack_correctly() {
        let text = "外<think>内<analysis>更深</analysis>回</think>尾";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.visible_text, "外尾");
    }

    #[test]
    fn interrupted_projection_holds_back_json_prefix() {
        let result = project_interrupted_agent_visible_text(
            r#"{"name":"make_section""#,
            &tools(&["make_section"]),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn interrupted_projection_holds_back_tool_name_prefix() {
        // 首行即工具名且无换行——扣留（可能是未完成调用名）。
        let result =
            project_interrupted_agent_visible_text("make_section", &tools(&["make_section"]));
        assert!(result.is_empty());
        // 带换行的散文首行是工具名前缀（非全同）——保留。
        let result =
            project_interrupted_agent_visible_text("make_sec\n继续写", &tools(&["make_section"]));
        assert!(result.contains("make_sec"));
    }

    #[test]
    fn interrupted_projection_retains_prose() {
        let result =
            project_interrupted_agent_visible_text("正文中断处", &tools(&["make_section"]));
        assert_eq!(result, "正文中断处");
    }

    #[test]
    fn multi_blank_runs_collapse() {
        let text = "段一\n\n\n\n\n段二";
        let parsed = parse_agent_response_protocol(text, &tools(&["make_section"]));
        assert_eq!(parsed.visible_text, "段一\n\n段二");
    }
}
