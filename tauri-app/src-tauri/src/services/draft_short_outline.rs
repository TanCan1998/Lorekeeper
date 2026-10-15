//! 写前短细纲的只读提示块。计划不成为作者事实，剥离必须精确匹配已完成输出。

fn trim_js(text: &str) -> &str {
    text.trim_matches(|c: char| (c.is_whitespace() && c != '\u{0085}') || c == '\u{feff}')
}

pub fn draft_short_outline_block(english: bool, output: &str) -> Result<String, &'static str> {
    let text = trim_js(output);
    if text.is_empty() {
        return Err("GENERATION_DRAFT_SHORT_OUTLINE_FAILED");
    }
    Ok(if english {
        format!("[Generated short outline — plan, not evidence]\nThis organizes the original goals only. Author facts, constraints and actual prior prose take priority; the plan establishes no fact.\n{text}\n[End generated short outline]")
    } else {
        format!("【生成短细纲：计划，不是事实依据】\n仅用于组织原目标；作者事实、约束和实际前文优先，计划不确立任何事实。\n{text}\n【生成短细纲结束】")
    })
}

pub fn strip_draft_short_outline_block<'a>(prompt: &'a str, output: &str) -> Option<&'a str> {
    for english in [false, true] {
        let block = draft_short_outline_block(english, output).ok()?;
        if let Some(original) = prompt.strip_suffix(&format!("\n\n{block}")) {
            return Some(original);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilingual_blocks_strip_only_exact_completed_suffix_test() {
        for english in [false, true] {
            let block = draft_short_outline_block(english, "  合成计划😀\r\n ").unwrap();
            let original = "合成写作材料\r\n保留空白 ";
            let prompt = format!("{original}\n\n{block}");
            assert_eq!(
                strip_draft_short_outline_block(&prompt, "合成计划😀"),
                Some(original)
            );
            assert_eq!(
                strip_draft_short_outline_block(&format!("{prompt} "), "合成计划😀"),
                None
            );
            assert_eq!(strip_draft_short_outline_block(&prompt, "不同计划"), None);
            assert_eq!(
                strip_draft_short_outline_block(&format!("{block}\n\n{original}"), "合成计划😀"),
                None
            );
        }
    }

    #[test]
    fn trim_matches_renderer_ecmascript_not_rust_whitespace_test() {
        assert_eq!(
            draft_short_outline_block(false, "\u{feff}计划\u{feff}").unwrap(),
            draft_short_outline_block(false, "计划").unwrap()
        );
        assert!(draft_short_outline_block(false, " \u{feff}\r\n").is_err());
        assert!(draft_short_outline_block(false, "\u{0085}")
            .unwrap()
            .contains('\u{0085}'));
        assert_eq!(strip_draft_short_outline_block("材料", " "), None);
    }
}
