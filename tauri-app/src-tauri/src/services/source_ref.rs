//! source_ref —— 生成来源引用契约（对齐 `src/shared/source-ref.ts`）。
//!
//! 共享契约类型 `ProjectEpoch` / `SourceRef` / `SourceSpan` /
//! `FrozenInputFingerprint` 已在 `repositories::generation_run_repository`
//! 落地，字段形状与上游逐字段一致（projectId / epoch / sourceId /
//! revision / contentHash / span 同形），按 G4-1 约定直接 `pub use` 复用，
//! 不重复定义。其余导出（`isContentHash` / `isFinalizedSourceIdentity` /
//! `sameProjectEpoch` / `hashAuthorText` / `validateSourceRef` /
//! `CandidateState` / `CandidateSource` / `CandidateAdmission` /
//! `mayUseCandidate` / `ContextSnapshot`）在本文件移植。
//!
//! 语义对齐要点：
//! - 哈希为 UTF-8 字节 SHA-256 小写 hex（基线 `crypto.subtle` + `TextEncoder`）；
//! - `Number.isSafeInteger` 收敛到 ±2^53-1（Rust `i64` 可能超出）；
//! - `validateSourceRef` 的 `text.length` 为 UTF-16 码元数（`encode_utf16().count()`）。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::repositories::character_repository::FinalizedSourceIdentity;

pub use crate::repositories::generation_run_repository::{
    FrozenInputFingerprint, ProjectEpoch, SourceRef, SourceSpan,
};

/// SHA-256 小写 hex（`/^[a-f0-9]{64}$/u`）。
pub fn is_content_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// `FinalizedSourceIdentity` 结构校验：finalizationId 非空白、draftId /
/// chapterNumber 为正安全整数、contentHash 为合法小写 SHA-256。
pub fn is_finalized_source_identity(source: &FinalizedSourceIdentity) -> bool {
    !source.finalization_id.trim().is_empty()
        && is_safe_integer(source.draft_id)
        && source.draft_id > 0
        && is_safe_integer(source.chapter_number)
        && source.chapter_number > 0
        && is_content_hash(&source.content_hash)
}

/// TS 结构子类型的等价物：凡携带 `projectId` / `epoch` 的契约类型
/// （`ProjectEpoch` / `SourceRef` / `CandidateSource` / `CandidateAdmission`）
/// 均可参与纪元比较，与基线 `sameProjectEpoch` 的多态形态一致。
pub trait ProjectEpochLike {
    fn project_epoch_fields(&self) -> (&str, &str);
}

impl ProjectEpochLike for ProjectEpoch {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochLike for SourceRef {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

/// `sameProjectEpoch`：两侧 projectId / epoch 均非空且逐字段相等。
pub fn same_project_epoch<T: ProjectEpochLike, U: ProjectEpochLike>(a: &T, b: &U) -> bool {
    let (a_project, a_epoch) = a.project_epoch_fields();
    let (b_project, b_epoch) = b.project_epoch_fields();
    !a_project.is_empty() && !a_epoch.is_empty() && a_project == b_project && a_epoch == b_epoch
}

/// 作者正文哈希：UTF-8 字节 SHA-256 小写 hex（基线 `hashAuthorText`；
/// TS 侧为 async `crypto.subtle`，Rust 侧为同步实现，签名无 await 面）。
pub fn hash_author_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `validateSourceRef`：纪元一致、sourceId 非空、revision 为非负安全整数、
/// contentHash 合法且等于正文哈希；span 以 UTF-16 码元校验（end 排他）。
pub fn validate_source_ref(ref_: &SourceRef, current: &ProjectEpoch, text: &str) -> bool {
    let valid_span = match &ref_.span {
        None => true,
        Some(span) => {
            span.unit == "utf16-code-unit"
                && is_safe_integer(span.start)
                && is_safe_integer(span.end)
                && span.start >= 0
                && span.end > span.start
                && span.end <= text.encode_utf16().count() as i64
        }
    };
    same_project_epoch(ref_, current)
        && !ref_.source_id.is_empty()
        && is_safe_integer(ref_.revision)
        && ref_.revision >= 0
        && is_content_hash(&ref_.content_hash)
        && hash_author_text(text) == ref_.content_hash
        && valid_span
}

/// 候选状态联合（TS 七值；`source-conflict` 为 kebab-case）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateState {
    Draft,
    Partial,
    Recovery,
    SourceConflict,
    Discarded,
    Stale,
    Replaced,
}

/// 进入上下文的候选产物身份（扩展 `ProjectEpoch`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSource {
    pub project_id: String,
    pub epoch: String,
    pub artifact_id: String,
    pub revision: i64,
    pub state: CandidateState,
    pub saved: bool,
    pub batch_lineage: String,
    pub fingerprint: FrozenInputFingerprint,
}

/// 准入条件：作者选中的当前草稿或已保存的直接前驱（扩展 `ProjectEpoch`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAdmission {
    pub project_id: String,
    pub epoch: String,
    pub batch_lineage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_current_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_predecessor_id: Option<String>,
}

impl ProjectEpochLike for CandidateSource {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochLike for CandidateAdmission {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

/// 仅作者选中的当前草稿或同批次血缘下的已保存直接前驱可进入上下文。
pub fn may_use_candidate(candidate: &CandidateSource, admission: &CandidateAdmission) -> bool {
    same_project_epoch(candidate, admission)
        && candidate.state == CandidateState::Draft
        && candidate.saved
        && !candidate.artifact_id.is_empty()
        && is_safe_integer(candidate.revision)
        && candidate.revision >= 0
        && (admission.selected_current_draft_id.as_deref() == Some(candidate.artifact_id.as_str())
            || (!admission.batch_lineage.is_empty()
                && candidate.batch_lineage == admission.batch_lineage
                && admission.direct_predecessor_id.as_deref()
                    == Some(candidate.artifact_id.as_str())))
}

/// 上下文来源槽位（TS 三值联合，kebab-case）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextSourceSlot {
    FinalizedFact,
    UnconfirmedContinuity,
    AuthorConstraint,
}

/// 上下文快照中的来源条目；`ref` 为 Rust 关键字，序列化名保持 `ref`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSource {
    #[serde(rename = "ref")]
    pub source_ref: SourceRef,
    pub slot: ContextSourceSlot,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextOmission {
    pub source_id: String,
    pub reason: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEstimate {
    pub method_version: String,
    pub input_units: i64,
}

/// 仅携带可移植收据身份；绝不成为执行或作者内容权威。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferAuthority {
    pub receipt_id: String,
    pub origin_project_id: String,
    pub snapshot_generation: String,
}

/// 上下文快照（扩展 `ProjectEpoch`；`sources` / `omissions` 为只读列表）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub project_id: String,
    pub epoch: String,
    pub id: String,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_authority: Option<TransferAuthority>,
    pub sources: Vec<ContextSource>,
    pub omissions: Vec<ContextOmission>,
    pub estimate: ContextEstimate,
}

/// `Number.isSafeInteger` 等价物（JS 安全整数为 ±2^53-1；Rust i64 可能超出）。
fn is_safe_integer(value: i64) -> bool {
    (-9007199254740991..=9007199254740991).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch() -> ProjectEpoch {
        ProjectEpoch {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
        }
    }

    fn frozen_fingerprint() -> FrozenInputFingerprint {
        FrozenInputFingerprint {
            chapter_brief_hash: String::new(),
            author_guidance_hash: String::new(),
            dependency_hash: String::new(),
            context_snapshot_hash: String::new(),
            template_hash: String::new(),
            skill_snapshot_hash: String::new(),
            model_lease_revision: String::new(),
            policy_hash: String::new(),
            output_contract_hash: String::new(),
        }
    }

    #[test]
    fn is_content_hash_accepts_lowercase_sha256_only_test() {
        assert!(is_content_hash(&"a".repeat(64)));
        assert!(!is_content_hash(&"A".repeat(64)), "大写应拒绝");
        assert!(!is_content_hash(&"a".repeat(63)), "长度不足应拒绝");
        assert!(!is_content_hash(&"g".repeat(64)), "非 hex 应拒绝");
        assert!(!is_content_hash(""), "空串应拒绝");
    }

    #[test]
    fn is_finalized_source_identity_checks_all_fields_test() {
        let valid = FinalizedSourceIdentity {
            draft_id: 1,
            finalization_id: "fin-1".to_string(),
            chapter_number: 2,
            content_hash: "a".repeat(64),
        };
        assert!(is_finalized_source_identity(&valid));
        assert!(
            !is_finalized_source_identity(&FinalizedSourceIdentity {
                finalization_id: "  ".to_string(),
                ..valid.clone()
            }),
            "空白 finalizationId 应拒绝"
        );
        assert!(
            !is_finalized_source_identity(&FinalizedSourceIdentity {
                draft_id: 0,
                ..valid.clone()
            }),
            "draftId 非正应拒绝"
        );
        assert!(
            !is_finalized_source_identity(&FinalizedSourceIdentity {
                chapter_number: -1,
                ..valid.clone()
            }),
            "chapterNumber 非正应拒绝"
        );
        assert!(
            !is_finalized_source_identity(&FinalizedSourceIdentity {
                content_hash: "nope".to_string(),
                ..valid
            }),
            "contentHash 非法应拒绝"
        );
    }

    #[test]
    fn same_project_epoch_requires_both_fields_test() {
        assert!(same_project_epoch(&epoch(), &epoch()));
        assert!(!same_project_epoch(
            &epoch(),
            &ProjectEpoch {
                epoch: "other".to_string(),
                ..epoch()
            }
        ));
        assert!(
            !same_project_epoch(
                &ProjectEpoch {
                    project_id: String::new(),
                    ..epoch()
                },
                &epoch()
            ),
            "空 projectId 不构成有效纪元"
        );
        // SourceRef 按 ProjectEpoch 结构参与比较（TS 结构子类型）
        let source_ref = SourceRef {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
            source_id: "src-1".to_string(),
            revision: 0,
            content_hash: "a".repeat(64),
            span: None,
        };
        assert!(same_project_epoch(&source_ref, &epoch()));
    }

    #[test]
    fn hash_author_text_matches_sha256_hex_test() {
        assert_eq!(
            hash_author_text(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_author_text("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn validate_source_ref_checks_epoch_revision_hash_and_span_test() {
        let text = "林决在城门";
        let content_hash = hash_author_text(text);
        let mut valid = SourceRef {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
            source_id: "src-1".to_string(),
            revision: 3,
            content_hash: content_hash.clone(),
            span: None,
        };
        assert!(validate_source_ref(&valid, &epoch(), text));
        valid.revision = -1;
        assert!(
            !validate_source_ref(&valid, &epoch(), text),
            "负 revision 应拒绝"
        );
        valid.revision = 3;
        valid.project_id = "other".to_string();
        assert!(
            !validate_source_ref(&valid, &epoch(), text),
            "纪元不一致应拒绝"
        );
        valid.project_id = "proj-1".to_string();
        valid.content_hash = "0".repeat(64);
        assert!(
            !validate_source_ref(&valid, &epoch(), text),
            "正文哈希不符应拒绝"
        );
        valid.content_hash = content_hash;
        valid.source_id = String::new();
        assert!(
            !validate_source_ref(&valid, &epoch(), text),
            "空 sourceId 应拒绝"
        );
        valid.source_id = "src-1".to_string();
        // span 以 UTF-16 码元计：「𝐀」是 1 码点 / 2 码元，「𝐀林」共 3 码元
        let surrogate_text = "𝐀林";
        valid.content_hash = hash_author_text(surrogate_text);
        valid.span = Some(SourceSpan {
            start: 0,
            end: 3,
            unit: "utf16-code-unit".to_string(),
        });
        assert!(validate_source_ref(&valid, &epoch(), surrogate_text));
        valid.span = Some(SourceSpan {
            start: 0,
            end: 4,
            unit: "utf16-code-unit".to_string(),
        });
        assert!(
            !validate_source_ref(&valid, &epoch(), surrogate_text),
            "span 越过 UTF-16 长度应拒绝"
        );
        valid.span = Some(SourceSpan {
            start: 0,
            end: 3,
            unit: "utf8-code-point".to_string(),
        });
        assert!(
            !validate_source_ref(&valid, &epoch(), surrogate_text),
            "单位非 utf16-code-unit 应拒绝"
        );
    }

    #[test]
    fn may_use_candidate_allows_selected_or_predecessor_only_test() {
        let candidate = CandidateSource {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
            artifact_id: "artifact-1".to_string(),
            revision: 0,
            state: CandidateState::Draft,
            saved: true,
            batch_lineage: "batch-1".to_string(),
            fingerprint: frozen_fingerprint(),
        };
        let admission = CandidateAdmission {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
            batch_lineage: "batch-1".to_string(),
            selected_current_draft_id: Some("artifact-1".to_string()),
            direct_predecessor_id: None,
        };
        assert!(
            may_use_candidate(&candidate, &admission),
            "作者选中的当前草稿应准入"
        );
        let predecessor = CandidateAdmission {
            selected_current_draft_id: Some("other".to_string()),
            direct_predecessor_id: Some("artifact-1".to_string()),
            ..admission.clone()
        };
        assert!(
            may_use_candidate(&candidate, &predecessor),
            "同批次直接前驱应准入"
        );
        let conflict = CandidateSource {
            state: CandidateState::SourceConflict,
            ..candidate.clone()
        };
        assert!(
            !may_use_candidate(&conflict, &admission),
            "非 draft 状态应拒绝"
        );
        let unsaved = CandidateSource {
            saved: false,
            ..candidate.clone()
        };
        assert!(!may_use_candidate(&unsaved, &admission), "未保存候选应拒绝");
        let foreign = CandidateAdmission {
            epoch: "epoch-2".to_string(),
            ..admission.clone()
        };
        assert!(!may_use_candidate(&candidate, &foreign), "纪元不一致应拒绝");
        let stranger = CandidateAdmission {
            batch_lineage: "batch-2".to_string(),
            selected_current_draft_id: None,
            direct_predecessor_id: None,
            ..admission
        };
        assert!(
            !may_use_candidate(&candidate, &stranger),
            "血缘不匹配且无前驱身份应拒绝"
        );
    }
}
