//! 审稿证据的偏移一律是JS UTF-16码元，不是UTF-8字节或Unicode标量。
use super::*;
use serde_json::json;

fn goal(quote: &str, start: i64, end: i64) -> Value {
    json!({"version":1,"chapterNumber":1,"coverage":"complete","items":[{
        "id":"goal-1","text":"合成目标","status":"completed","description":"已展示",
        "evidence":[{"quote":quote,"start":start,"end":end}]
    }]})
}
fn report(quote: &str, start: i64, end: i64) -> String {
    json!({"summary":"合成核对","goalReview":goal(quote,start,end),"items":[{
        "category":"本章目标","severity":"pass","description":"已展示","goalId":"goal-1","quote":quote
    }]}).to_string()
}
#[test]
fn goal_evidence_accepts_utf16_astral_span_in_both_readers() {
    for (quote, start, end) in [
        ("🔑", 3, 5),
        ("甲🔑乙", 2, 6),
        ("e\u{301}", 0, 2),
        ("甲\r\n乙", 0, 4),
    ] {
        assert!(
            parse_saved_report(&report(quote, start, end)).is_some(),
            "{quote}"
        );
        assert!(
            parse_chapter_goal_review(&goal(quote, start, end)).is_some(),
            "{quote}"
        );
    }
}
#[test]
fn goal_evidence_rejects_codepoint_and_byte_offsets() {
    for (start, end) in [(3, 4), (3, 7)] {
        assert!(parse_saved_report(&report("🔑", start, end)).is_none());
        assert!(parse_chapter_goal_review(&goal("🔑", start, end)).is_none());
    }
}
#[test]
fn unique_anchor_rejects_multibyte_and_overlapping_duplicates() {
    for (source, quote) in [
        ("汉文汉文", "汉文"),
        ("🔑🔑", "🔑"),
        ("哈哈哈", "哈哈"),
        ("aaa", "aa"),
        ("", ""),
    ] {
        assert!(locate_unique(source, quote).is_none(), "{source}/{quote}");
    }
}
#[test]
fn unique_anchor_preserves_utf16_bounds_after_astral_prefix() {
    assert_eq!(locate_unique("🔑甲乙。尾声", "甲乙"), Some((2, 4)));
    assert_eq!(locate_unique("甲🔑乙。尾声", "🔑乙"), Some((1, 4)));
}
