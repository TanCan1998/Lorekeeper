//! 便携项目恢复计划 —— 平移自 `electron/services/project-restore-service.ts` 的**清单校验与落点映射**段，
//! 按**本项目口径**落地（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）。
//!
//! # 职责边界
//!
//! 本模块只做**纯逻辑**：把已解析的 manifest 校验成一份「归档条目 → 目标相对路径」计划。
//! 真正的安装（暂存目录所有权、安装日志、原子移动、转移权威改写、回执）由 B113b 落地；
//! 命令层（消费两个授权 + 会话门禁）由 B113c 落地。
//!
//! # 本项目口径与基线差异
//!
//! | 面 | 基线（Electron） | 本项目 |
//! |---|---|---|
//! | 项目存储根 | `.ai-novel/` | `.lore/` |
//! | 库文件 | `project.db` | `lorekeeper.db` |
//! | 转移权威 | 嵌套在 `.ai-novel/` 下 | 归档根目录（与库同级），恢复时改写后放入 `.lore/` |
//! | 知识面 | 独立 `portable-knowledge-source.json`（LanceDB 投影） | **无**：知识事实在库内，
//! 恢复后只需重建 FTS 索引 |
//! | 禁止的派生面 | `lancedb` / `vectors.json` / `embedding-spaces.json` | `kb/`（HNSW 快照） |
//!
//! 因此基线那套「`.ai-novel/` 保留前缀 + 知识快照条目」的校验在本项目里简化为：
//! 条目路径一律相对存储根、不得出现 `.lore/` 前缀（否则会双重嵌套）、不得出现库副文件与
//! 项目清单（这两类由恢复过程自行生成）。

use std::collections::BTreeSet;

use crate::services::portable_project_format::{
    portable_path_key, PortableEntryDisposition, PortableProjectManifest,
};

/// 便携库在归档内的条目名（与导出侧一致）
pub const RESTORE_DATABASE_ENTRY: &str = "lorekeeper.db";
/// 转移权威在归档内的条目名（与导出侧一致）
pub const RESTORE_TRANSFER_AUTHORITY_ENTRY: &str = "portable-transfer-authority.json";
/// 项目清单文件名（由恢复过程重新生成，归档内不得出现）
pub const RESTORE_PROJECT_MANIFEST_ENTRY: &str = "project.json";
/// 库副文件（归档内不得出现）
pub const RESTORE_DATABASE_SIDECARS: &[&'static str] = &[
    "lorekeeper.db-wal",
    "lorekeeper.db-shm",
    "lorekeeper.db-journal",
];
/// 头像资产的顶层目录（`avatar-asset` 条目必须落在其下）
const AVATAR_DIRECTORY: &str = "avatars";
/// 派生向量索引顶层目录（本项目口径下禁止入档）
const DERIVED_VECTOR_DIRECTORY: &str = "kb";
/// 本项目口径下不产出知识快照文件（知识事实在库内）
const BASELINE_KNOWLEDGE_ENTRY: &str = "portable-knowledge-source.json";

fn restore_invalid() -> String {
    "PORTABLE_RESTORE_INVALID".to_string()
}

/// 恢复计划中的单个条目
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePlanEntry {
    /// 归档内的条目路径（原样，用于在暂存目录定位文件）
    pub archive_path: String,
    /// 相对项目存储根（`.lore/`）的目标路径（`/` 分隔）
    pub target_relative_path: String,
    pub disposition: PortableEntryDisposition,
}

/// 恢复计划
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePlan {
    pub entries: Vec<RestorePlanEntry>,
    /// 便携库条目（唯一）
    pub database_entry: RestorePlanEntry,
    /// 转移权威条目（唯一）
    pub transfer_authority_entry: RestorePlanEntry,
    /// 头像资产条目数（`omittedItems` 之外的统计用）
    pub avatar_entries: usize,
}

impl RestorePlan {
    /// 目标项目存储根下的全部落点（相对路径）
    pub fn target_paths(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.target_relative_path.as_str())
            .collect()
    }
}

/// 把 manifest 校验成恢复计划（本项目口径）。
///
/// 拒绝（一律 `PORTABLE_RESTORE_INVALID`）：
/// - 条目路径重复（便携路径键去重）、或出现 `.lore/` 前缀（会双重嵌套）；
/// - 出现项目清单 `project.json`、库副文件 `lorekeeper.db-*`、派生向量目录 `kb/`；
/// - 出现基线知识快照 `portable-knowledge-source.json`（本项目不产出，知识事实在库内）；
/// - 便携库条目缺失 / 重复 / 处置不是 `portable-database`；
/// - 转移权威条目缺失 / 重复 / 处置不是 `transfer-receipt`；
/// - `avatar-asset` 条目落在 `avatars/` 之外；
/// - `knowledge-source` / `history-projection` 处置在本项目口径下不产出。
pub fn plan_restore(manifest: &PortableProjectManifest) -> Result<RestorePlan, String> {
    let max_path_bytes =
        crate::services::portable_project_format::default_portable_archive_limits().max_path_bytes;
    let mut seen_keys: BTreeSet<String> = BTreeSet::new();
    let mut entries: Vec<RestorePlanEntry> = Vec::with_capacity(manifest.entries.len());
    let mut database_entry: Option<RestorePlanEntry> = None;
    let mut authority_entry: Option<RestorePlanEntry> = None;
    let mut avatar_entries = 0usize;

    for entry in &manifest.entries {
        let key = portable_path_key(&entry.path, max_path_bytes).map_err(|_| restore_invalid())?;
        if !seen_keys.insert(key.clone()) {
            return Err(restore_invalid());
        }
        // 归档条目相对存储根，故不得再带存储根前缀
        if key == ".lore" || key.starts_with(".lore/") {
            return Err(restore_invalid());
        }
        if key == RESTORE_PROJECT_MANIFEST_ENTRY
            || RESTORE_DATABASE_SIDECARS.contains(&key.as_str())
            || key == DERIVED_VECTOR_DIRECTORY
            || key.starts_with("kb/")
            || key == BASELINE_KNOWLEDGE_ENTRY
        {
            return Err(restore_invalid());
        }

        let plan_entry = RestorePlanEntry {
            archive_path: entry.path.clone(),
            target_relative_path: entry.path.clone(),
            disposition: entry.disposition,
        };
        match entry.disposition {
            PortableEntryDisposition::PortableDatabase => {
                if key != RESTORE_DATABASE_ENTRY || database_entry.is_some() {
                    return Err(restore_invalid());
                }
                database_entry = Some(plan_entry.clone());
            }
            PortableEntryDisposition::TransferReceipt => {
                if key != RESTORE_TRANSFER_AUTHORITY_ENTRY || authority_entry.is_some() {
                    return Err(restore_invalid());
                }
                authority_entry = Some(plan_entry.clone());
            }
            PortableEntryDisposition::AvatarAsset => {
                if !(key == AVATAR_DIRECTORY || key.starts_with("avatars/")) {
                    return Err(restore_invalid());
                }
                avatar_entries += 1;
            }
            PortableEntryDisposition::AuthorContent
            | PortableEntryDisposition::Prompt
            | PortableEntryDisposition::Skill => {}
            PortableEntryDisposition::KnowledgeSource
            | PortableEntryDisposition::HistoryProjection => return Err(restore_invalid()),
        }
        entries.push(plan_entry);
    }

    let Some(database_entry) = database_entry else {
        return Err(restore_invalid());
    };
    let Some(transfer_authority_entry) = authority_entry else {
        return Err(restore_invalid());
    };
    Ok(RestorePlan {
        entries,
        database_entry,
        transfer_authority_entry,
        avatar_entries,
    })
}

/// 目标项目根下的存储根（`<target>/.lore`）。
///
/// 用本项目的 [`crate::db::PROJECT_DIR_NAME`]（`.lore`），**不是**上游的
/// `CANONICAL_PROJECT_DIRECTORY`（`.ai-novel`）—— 后者是便携归档内的目录约定，
/// Tauri 侧项目存储根是 `.lore`（L3 双栈隔离，见 AGENTS.md）。
pub fn restore_storage_root(target_project_root: &std::path::Path) -> std::path::PathBuf {
    target_project_root.join(crate::db::PROJECT_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::portable_project_format::{
        PortableOmissionReason, PortableProjectManifestEntry, PORTABLE_PROJECT_FORMAT_VERSION,
        PORTABLE_SOURCE_SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;

    fn entry(path: &str, disposition: PortableEntryDisposition) -> PortableProjectManifestEntry {
        PortableProjectManifestEntry {
            path: path.to_string(),
            byte_size: 1,
            sha256: "a".repeat(64),
            disposition,
        }
    }

    fn manifest(entries: Vec<PortableProjectManifestEntry>) -> PortableProjectManifest {
        PortableProjectManifest {
            format_version: PORTABLE_PROJECT_FORMAT_VERSION,
            source_schema_version: PORTABLE_SOURCE_SCHEMA_VERSION,
            origin_project_id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
            snapshot_generation: "gen-1".to_string(),
            created_at: "2026-10-16T05:00:00.000Z".to_string(),
            declared_uncompressed_bytes: entries.iter().map(|e| e.byte_size).sum(),
            declared_compressed_bytes: entries.iter().map(|e| e.byte_size).sum(),
            entries,
            semantic_counts: BTreeMap::new(),
            omitted_items: vec![
                crate::services::portable_project_format::PortableOmissionItem {
                    id: "vectors".to_string(),
                    reason: PortableOmissionReason::RebuildStale,
                },
            ],
            transfer_receipt_ids: vec!["portable-transfer:abc".to_string()],
            history_projection_ids: Vec::new(),
        }
    }

    fn base_entries() -> Vec<PortableProjectManifestEntry> {
        vec![
            entry("lorekeeper.db", PortableEntryDisposition::PortableDatabase),
            entry(
                "portable-transfer-authority.json",
                PortableEntryDisposition::TransferReceipt,
            ),
            entry("prompts/a.md", PortableEntryDisposition::Prompt),
            entry("skills/b.md", PortableEntryDisposition::Skill),
            entry("avatars/ab/1-x.png", PortableEntryDisposition::AvatarAsset),
        ]
    }

    #[test]
    fn plan_maps_entries_to_storage_root_test() {
        let plan = plan_restore(&manifest(base_entries())).unwrap();
        assert_eq!(plan.entries.len(), 5);
        assert_eq!(plan.database_entry.target_relative_path, "lorekeeper.db");
        assert_eq!(
            plan.transfer_authority_entry.target_relative_path,
            "portable-transfer-authority.json"
        );
        assert_eq!(plan.avatar_entries, 1);
        let mut paths = plan.target_paths();
        paths.sort_unstable();
        assert_eq!(
            paths,
            vec![
                "avatars/ab/1-x.png",
                "lorekeeper.db",
                "portable-transfer-authority.json",
                "prompts/a.md",
                "skills/b.md",
            ]
        );
        assert_eq!(
            restore_storage_root(std::path::Path::new("C:/novels/book")),
            std::path::PathBuf::from("C:/novels/book").join(".lore")
        );
    }

    #[test]
    fn plan_rejects_forbidden_entries_test() {
        for forbidden in [
            "project.json",
            "lorekeeper.db-wal",
            "lorekeeper.db-shm",
            "lorekeeper.db-journal",
            "kb/index-1.hnsw.data",
            "portable-knowledge-source.json",
            ".lore/lorekeeper.db",
            ".lore",
        ] {
            let mut entries = base_entries();
            entries.push(entry(forbidden, PortableEntryDisposition::AuthorContent));
            assert_eq!(
                plan_restore(&manifest(entries)).unwrap_err(),
                "PORTABLE_RESTORE_INVALID",
                "{forbidden} 必须被拒绝"
            );
        }
    }

    #[test]
    fn plan_rejects_shape_drift_test() {
        // 缺库条目
        let entries: Vec<_> = base_entries()
            .into_iter()
            .filter(|entry| entry.disposition != PortableEntryDisposition::PortableDatabase)
            .collect();
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );

        // 缺转移权威条目
        let entries: Vec<_> = base_entries()
            .into_iter()
            .filter(|entry| entry.disposition != PortableEntryDisposition::TransferReceipt)
            .collect();
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );

        // 库条目路径不符
        let mut entries = base_entries();
        entries[0].path = "other.db".to_string();
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );

        // 头像资产不在 avatars/ 下
        let mut entries = base_entries();
        entries[4].path = "assets/1-x.png".to_string();
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );

        // 路径键重复（大小写折叠后同一键）
        let mut entries = base_entries();
        entries.push(entry("PromptS/A.md", PortableEntryDisposition::Prompt));
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );

        // 本项目不产出的处置
        let mut entries = base_entries();
        entries.push(entry(
            "history/x.json",
            PortableEntryDisposition::HistoryProjection,
        ));
        assert_eq!(
            plan_restore(&manifest(entries)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );
    }
}
