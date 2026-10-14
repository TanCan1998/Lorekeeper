//! 持久化生成的可见流过滤器，平移自 electron/llm/visible-stream.ts。
//! 未完成标签与未闭合思考只暂存，永不进入可见快照；不修剪作者空白。

#[derive(Debug, Default)]
pub struct VisibleStreamFilter {
    pending: String,
    depth: usize,
    visible: String,
}

impl VisibleStreamFilter {
    pub fn push(&mut self, text: &str) -> String {
        let input: Vec<char> = self.pending.chars().chain(text.chars()).collect();
        self.pending.clear();
        let mut emitted = String::new();
        let mut cursor = 0;
        while cursor < input.len() {
            if input[cursor] != '<' {
                if self.depth == 0 {
                    emitted.push(input[cursor]);
                }
                cursor += 1;
                continue;
            }
            let tail: String = input[cursor..]
                .iter()
                .take(8)
                .map(|character| character.to_ascii_lowercase())
                .collect();
            let tag = if tail.starts_with("<think>") {
                Some(("<think>", true))
            } else if tail.starts_with("</think>") {
                Some(("</think>", false))
            } else {
                None
            };
            if let Some((tag, opening)) = tag {
                self.depth = if opening {
                    self.depth + 1
                } else {
                    self.depth.saturating_sub(1)
                };
                cursor += tag.len();
            } else if ["<think>", "</think>"]
                .iter()
                .any(|tag| tag.starts_with(&tail))
            {
                self.pending = input[cursor..].iter().collect();
                break;
            } else {
                if self.depth == 0 {
                    emitted.push('<');
                }
                cursor += 1;
            }
        }
        self.visible.push_str(&emitted);
        emitted
    }

    pub fn text(&self) -> &str {
        &self.visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_utf8_boundary_preserves_visible_whitespace() {
        let text = " \r\n甲<think>秘密</think>乙😀 \n";
        for split in (0..=text.len()).filter(|index| text.is_char_boundary(*index)) {
            let mut filter = VisibleStreamFilter::default();
            assert_eq!(
                filter.push(&text[..split]) + &filter.push(&text[split..]),
                " \r\n甲乙😀 \n"
            );
            assert_eq!(filter.text(), " \r\n甲乙😀 \n");
        }
    }

    #[test]
    fn unfinished_thought_and_markup_remain_private() {
        let mut filter = VisibleStreamFilter::default();
        assert_eq!(filter.push("甲<th"), "甲");
        assert_eq!(filter.push("ink>隐藏"), "");
        assert_eq!(filter.text(), "甲");
        let mut partial = VisibleStreamFilter::default();
        assert_eq!(partial.push("正文<thi"), "正文");
        assert_eq!(partial.text(), "正文");
    }

    #[test]
    fn nested_case_insensitive_tags_and_literal_markup() {
        let mut filter = VisibleStreamFilter::default();
        assert_eq!(
            filter.push("甲<THINK>秘密<think>内层</THINK>末</think><em>乙</em>İ"),
            "甲<em>乙</em>İ"
        );
    }

    #[test]
    fn large_chunk_is_scanned_linearly() {
        let text = "正文".repeat(50_000);
        let mut filter = VisibleStreamFilter::default();
        assert_eq!(
            filter.push(&(text.clone() + "İ<THINK>隐藏</THINK>末尾")),
            text + "İ末尾"
        );
    }
}
