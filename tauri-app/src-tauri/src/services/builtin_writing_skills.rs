//! 内置写作技能的可信只读目录。
//! 正文与 frontmatter 平移自 src/shared/builtin-writing-skills.ts 的
//! readBuiltinWritingSkill；这是运行时资产，不复制前端类型。
use crate::services::prompt_system_role::WritingLanguage;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static SKILLS: LazyLock<BTreeMap<String, BTreeMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("builtin_writing_skills.json"))
        .expect("内置技能目录必须是有效的双语 JSON")
});

pub fn read_builtin_writing_skill(name: &str, language: WritingLanguage) -> Option<String> {
    let language = match language {
        WritingLanguage::ZhCn => "zh-CN",
        WritingLanguage::EnUs => "en-US",
    };
    SKILLS.get(name)?.get(language).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_builtin_skills_have_valid_bilingual_metadata_test() {
        assert_eq!(SKILLS.len(), 7);
        for name in SKILLS.keys() {
            for language in [WritingLanguage::ZhCn, WritingLanguage::EnUs] {
                let text = read_builtin_writing_skill(name, language).unwrap();
                let inspected =
                    crate::writing_skills::inspect_writing_skill_markdown(&text).unwrap();
                assert_eq!(inspected.metadata.name, *name);
                assert_eq!(inspected.metadata.language, "bilingual");
                assert!(!inspected.content.is_empty());
            }
        }
        assert!(read_builtin_writing_skill("missing", WritingLanguage::ZhCn).is_none());
    }
    #[test]
    fn builtin_skill_read_returns_an_independent_copy_test() {
        let original =
            read_builtin_writing_skill("long-form-continuity", WritingLanguage::ZhCn).unwrap();
        let mut changed = original.clone();
        changed.push_str("不改变目录");
        assert_eq!(
            read_builtin_writing_skill("long-form-continuity", WritingLanguage::ZhCn),
            Some(original)
        );
    }
}
