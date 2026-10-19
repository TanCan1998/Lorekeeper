//! 大纲恢复的纯文本合同，供读取提案和作者提交共用；不读写数据库。
use crate::repositories::generation_run_repository::{
    PlotOutlineCursor, PlotOutlineProgress, PlotOutlineRange, ProjectCoreSynopsisExpected,
};
const GENERATION_PLOT_OUTLINE_PREFIX_INVALID: &str = "GENERATION_PLOT_OUTLINE_PREFIX_INVALID";

const CHINESE_DIGITS: &str = "零〇一二两三四五六七八九";

#[derive(Debug, Clone, PartialEq)]
struct HeadingMatch {
    from: i64,
    to: i64,
    index: usize,
    end: usize,
    grouped: bool,
}

fn chinese_digit(char: char) -> Option<i64> {
    match char {
        '零' | '〇' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

fn chinese_unit(char: char) -> Option<i64> {
    match char {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        _ => None,
    }
}

fn is_chinese_number_char(char: char) -> bool {
    char.is_ascii_digit() || CHINESE_DIGITS.contains(char) || matches!(char, '十' | '百' | '千')
}

fn chapter_number(token: &str) -> i64 {
    if !token.is_empty() && token.chars().all(|char| char.is_ascii_digit()) {
        return token.parse::<i64>().unwrap_or_default();
    }
    let mut result = 0;
    let mut digit = 0;
    for char in token.chars() {
        if let Some(value) = chinese_digit(char) {
            digit = value;
        } else if let Some(unit) = chinese_unit(char) {
            result += digit.max(1) * unit;
            digit = 0;
        }
    }
    result + digit
}

fn match_chinese_number(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !is_chinese_number_char(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < chars.len() && is_chinese_number_char(chars[end]) {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    Some((chapter_number(&token), end))
}

fn match_ascii_digits(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !chars[start].is_ascii_digit() {
        return None;
    }
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    token.parse::<i64>().ok().map(|value| (value, end))
}

/// 中文支 `第 NUMBER([–—-]NUMBER)?章`；返回 `(from, to, 是否有第二数字)`。
fn match_chinese_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    let (from, mut position) = match_chinese_number(chars, start + 1)?;
    let mut to = from;
    let mut grouped = false;
    if position < chars.len()
        && (chars[position] == '–' || chars[position] == '—' || chars[position] == '-')
    {
        if let Some((second, end)) = match_chinese_number(chars, position + 1) {
            to = second;
            grouped = true;
            position = end;
        }
    }
    if position >= chars.len() || chars[position] != '章' {
        return None;
    }
    position += 1;
    let mut lookahead = position;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// 阿拉伯支 `Chapters?[\t ]+(\d+)([\t ]*[–—-][\t ]*(\d+))?`。
fn match_english_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    if start + 7 > chars.len()
        || !chars[start..start + 7]
            .iter()
            .collect::<String>()
            .eq_ignore_ascii_case("chapter")
    {
        return None;
    }
    let mut position = start + 7;
    if position < chars.len() && (chars[position] == 's' || chars[position] == 'S') {
        position += 1;
    }
    let whitespace_start = position;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    if position == whitespace_start {
        return None;
    }
    let (from, mut end) = match_ascii_digits(chars, position)?;
    let mut to = from;
    let mut grouped = false;
    let mut range = end;
    while range < chars.len() && (chars[range] == '\t' || chars[range] == ' ') {
        range += 1;
    }
    if range < chars.len() && (chars[range] == '–' || chars[range] == '—' || chars[range] == '-')
    {
        let mut dash_end = range + 1;
        while dash_end < chars.len() && (chars[dash_end] == '\t' || chars[dash_end] == ' ') {
            dash_end += 1;
        }
        if let Some((second, second_end)) = match_ascii_digits(chars, dash_end) {
            to = second;
            grouped = true;
            end = second_end;
        }
    }
    let mut lookahead = end;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// `/Chapters\s/iu`：匹配文本含大小写不敏感 "chapters" 紧接空白字符。
fn contains_chapters_followed_by_whitespace(text: &str) -> bool {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = "chapters".chars().collect();
    if lower.len() < needle.len() {
        return false;
    }
    for start in 0..=(lower.len() - needle.len()) {
        if lower[start..start + needle.len()] == needle[..] {
            let after = start + needle.len();
            if after < lower.len() && lower[after].is_whitespace() {
                return true;
            }
        }
    }
    false
}

/// 逐行匹配 JS 正则
/// `^[\t ]*(?:#{1,6}[\t ]+|[-*+][\t ]+)?(?:第(NUMBER)(?:[–—-](NUMBER))?章|Chapters?[\t ]+(\d+)(?:[\t ]*[–—-][\t ]*(\d+))?)(?=[\t ]*(?:[:：.．—-]|$))[^\r\n]*`
/// （`regex` crate 不支持前瞻断言，手工实现等价算法，含可选组回溯语义）。
fn match_heading_line(line: &str) -> Option<(i64, i64, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut position = 0;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    // 可选 markdown 列表前缀 #{1,6}[\t ]+ 或 [-*+][\t ]+；不构成时整组回退为空。
    if position < chars.len() && chars[position] == '#' {
        let mut hashes = position;
        while hashes < chars.len() && chars[hashes] == '#' && hashes - position < 6 {
            hashes += 1;
        }
        if hashes - position >= 1
            && hashes < chars.len()
            && (chars[hashes] == '\t' || chars[hashes] == ' ')
        {
            while hashes < chars.len() && (chars[hashes] == '\t' || chars[hashes] == ' ') {
                hashes += 1;
            }
            position = hashes;
        }
    } else if position < chars.len()
        && (chars[position] == '-' || chars[position] == '*' || chars[position] == '+')
    {
        let marker = position + 1;
        if marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
            let mut marker = marker;
            while marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
                marker += 1;
            }
            position = marker;
        }
    }
    if position < chars.len() && chars[position] == '第' {
        if let Some(result) = match_chinese_chapter(&chars, position) {
            return Some(result);
        }
        return None;
    }
    match_english_chapter(&chars, position)
}

fn headings(text: &str) -> Vec<HeadingMatch> {
    let mut result = Vec::new();
    let mut byte_offset = 0;
    for line in text.split('\n') {
        let line_len = line.len();
        let match_len = if line.ends_with('\r') {
            line_len - 1
        } else {
            line_len
        };
        if let Some((from, to, grouped)) = match_heading_line(line) {
            let match_text = &line[..match_len];
            result.push(HeadingMatch {
                from,
                to,
                index: byte_offset,
                end: byte_offset + match_len,
                grouped: grouped || contains_chapters_followed_by_whitespace(match_text),
            });
        }
        byte_offset += line_len + 1;
    }
    result
}

/// 基线 `joinPlotOutlineEntries`（plot-outline-contract.ts:177）。
fn join_plot_outline_entries(prefix: &str, next: &str) -> String {
    [prefix.trim(), next.trim()]
        .iter()
        .filter(|entry| !entry.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 基线 `renderPlotOutlineSynopsis`（plot-outline-contract.ts:187）。
fn render_plot_outline_synopsis(
    body: &str,
    covered_to: i64,
    expected: &ProjectCoreSynopsisExpected,
) -> String {
    let english = expected.writing_language == "en-US";
    let tail = if covered_to < expected.total_chapters {
        if english {
            format!(
                "> This outline covers chapters 1-{covered_to} of {}; the remaining chapters will be generated in later batches.",
                expected.total_chapters
            )
        } else {
            format!(
                "> 本大纲已覆盖至第 {} 章（全书 {} 章），其余章节将在后续批次继续生成。",
                covered_to, expected.total_chapters
            )
        }
    } else {
        String::new()
    };
    let title = if english {
        "Plot Outline"
    } else {
        "情节大纲"
    };
    format!("# {title}\n\n{}", join_plot_outline_entries(body, &tail))
        .trim()
        .to_string()
}

/// 基线 `plotOutlineConfirmedPrefix`（plot-outline-contract.ts）——
/// 已确认前缀的一致性校验与提取。
fn plot_outline_confirmed_prefix(
    expected: &ProjectCoreSynopsisExpected,
    range: &PlotOutlineRange,
) -> Result<String, String> {
    let entries = headings(&expected.synopsis);
    if entries.iter().any(|entry| {
        (entry.from < range.from && entry.to >= range.from)
            || (entry.from <= range.to && entry.to > range.to)
    }) {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    if range.from == 1 {
        return Ok(String::new());
    }
    let before: Vec<&HeadingMatch> = entries
        .iter()
        .filter(|entry| entry.to < range.from)
        .collect();
    let mut next = 1;
    for entry in &before {
        if entry.from != next || entry.to < entry.from {
            return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
        }
        next = entry.to + 1;
    }
    if next != range.from || before.is_empty() {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    if let Some(following) = entries.iter().find(|entry| entry.from >= range.from) {
        return Ok(expected.synopsis[before[0].index..following.index]
            .trim()
            .to_string());
    }
    let title = format!(
        "# {}\n\n",
        if expected.writing_language == "en-US" {
            "Plot Outline"
        } else {
            "情节大纲"
        }
    );
    let suffix =
        render_plot_outline_synopsis("", range.from - 1, expected)[title.len()..].to_string();
    if !expected.synopsis.starts_with(&title)
        || suffix.is_empty()
        || !expected.synopsis.ends_with(&suffix)
    {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    let prefix = expected.synopsis[title.len()..expected.synopsis.len() - suffix.len()]
        .trim()
        .to_string();
    if prefix.is_empty()
        || render_plot_outline_synopsis(&prefix, range.from - 1, expected) != expected.synopsis
    {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    Ok(prefix)
}

/// 基线 `plotOutlineSourceInvalid`：开跑时冻结的大纲按现行规则无效。
pub(crate) fn plot_outline_source_invalid(progress: &PlotOutlineProgress) -> bool {
    matches!(
        &progress.cursor,
        PlotOutlineCursor::Stopped { reason, .. } if reason == "source-invalid"
    )
}

/// 基线 `validPlotOutlineAuthorPrefix`：作者提交的大纲必须覆盖所选
/// 范围、逐章合法，且按该范围回渲染后与提交文本完全一致。
pub(crate) fn valid_plot_outline_author_prefix(
    synopsis: &str,
    progress: &PlotOutlineProgress,
    range: &PlotOutlineRange,
) -> Result<bool, String> {
    if range.from != progress.range.from || range.to < range.from || range.to > progress.range.to {
        return Ok(false);
    }
    let expected = &progress.source_expected;
    let all = headings(synopsis);
    let entries: Vec<&HeadingMatch> = all
        .iter()
        .filter(|entry| entry.from >= range.from && entry.from <= range.to)
        .collect();
    if entries.len() as i64 != range.to - range.from + 1 {
        return Ok(false);
    }
    let after = all.iter().find(|entry| entry.from > range.to);
    let title = format!(
        "# {}\n\n",
        if expected.writing_language == "en-US" {
            "Plot Outline"
        } else {
            "情节大纲"
        }
    );
    let suffix = render_plot_outline_synopsis("", range.to, expected)[title.len()..].to_string();
    let end = match after {
        Some(entry) => entry.index,
        None if !suffix.is_empty() && synopsis.ends_with(&suffix) => synopsis.len() - suffix.len(),
        None => synopsis.len(),
    };
    let authored = synopsis[entries[0].index..end].trim();
    let authored_entries = headings(authored);
    let chapters_are_valid = authored_entries.iter().enumerate().all(|(index, entry)| {
        let end = authored_entries
            .get(index + 1)
            .map(|next| next.index)
            .unwrap_or(authored.len());
        crate::repositories::generation_run_repository::valid_plot_outline_entry(
            authored[entry.index..end].trim(),
            range.from + index as i64,
        )
    });
    Ok(chapters_are_valid && render_plot_outline_range(authored, range, expected)? == synopsis)
}

/// 基线 `renderPlotOutlineRange`（plot-outline-contract.ts:212）。
pub(crate) fn render_plot_outline_range(
    body: &str,
    range: &PlotOutlineRange,
    expected: &ProjectCoreSynopsisExpected,
) -> Result<String, String> {
    // 基线 `plotOutlineConfirmedPrefix(expected, range)`：仅作
    // 一致性校验（抛错），结果在末行才使用。
    plot_outline_confirmed_prefix(expected, range)?;
    let entries = headings(&expected.synopsis);
    if entries.is_empty() {
        return Ok(render_plot_outline_synopsis(body, range.to, expected));
    }
    let start = entries.iter().find(|entry| entry.from >= range.from);
    let after = entries.iter().find(|entry| entry.from > range.to);
    if let Some(start) = start {
        let before = &expected.synopsis[..start.index];
        if let Some(after) = after {
            return Ok(format!(
                "{}{}\n\n{}",
                before,
                body.trim(),
                &expected.synopsis[after.index..]
            ));
        }
        let title = format!(
            "# {}\n\n",
            if expected.writing_language == "en-US" {
                "Plot Outline"
            } else {
                "情节大纲"
            }
        );
        let suffix =
            render_plot_outline_synopsis("", range.to, expected)[title.len()..].to_string();
        return Ok(format!(
            "{}{}{}",
            before,
            body.trim(),
            if suffix.is_empty() {
                String::new()
            } else {
                format!("\n\n{suffix}")
            }
        ));
    }
    let prefix = plot_outline_confirmed_prefix(expected, range)?;
    Ok(render_plot_outline_synopsis(
        &join_plot_outline_entries(&prefix, body),
        range.to,
        expected,
    ))
}

/// 基线 `plotOutlineRecoveryDraft`（plot-outline-contract.ts:243）：
/// 大纲恢复提案文本——已完成章数 → 下一章占位标题（或失败
/// 候选）→ 范围渲染。
pub(crate) fn plot_outline_recovery_draft(
    progress: &PlotOutlineProgress,
    failed_candidate: Option<&str>,
) -> Result<String, String> {
    let completed = progress
        .composition
        .as_ref()
        .and_then(|composition| composition.chapters.as_ref())
        .map_or(0, |chapters| chapters.len());
    let chapter = progress.range.from + completed as i64;
    let mut body = progress
        .composition
        .as_ref()
        .map(|composition| composition.text.clone())
        .unwrap_or_default();
    match &progress.cursor {
        PlotOutlineCursor::Accept { text, .. } => {
            body = join_plot_outline_entries(&body, text);
        }
        PlotOutlineCursor::Complete => {}
        _ => {
            let heading = if progress.source_expected.writing_language == "en-US" {
                format!("## Chapter {chapter}: ")
            } else {
                format!("## 第{chapter}章：")
            };
            let fallback = failed_candidate
                .map(|candidate| candidate.trim())
                .filter(|candidate| !candidate.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{heading}\n"));
            body = join_plot_outline_entries(&body, &fallback);
        }
    }
    render_plot_outline_range(
        &body,
        &PlotOutlineRange {
            from: progress.range.from,
            to: chapter.min(progress.range.to),
        },
        &progress.source_expected,
    )
}
