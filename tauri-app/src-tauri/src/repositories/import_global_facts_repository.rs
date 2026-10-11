//! import_global_facts —— 全局事实导入台账（批次 G3a；B1-W-IGF 移植 m02 身份分支）。
//!
//! 平移自 `electron/repositories/import-global-facts-repository.ts`：
//! 配置 / 非角色架构 / 角色名单事实的原子导入接缝。
//!
//! # 刻意偏离（对齐 G3 开工清单）
//!
//! - `import_global_fact_operations` 台账表**懒建**（对齐基线 `ensureLedger()`
//!   的 `CREATE TABLE IF NOT EXISTS`），不改 `db/schema.rs`——G schema 已获批
//!   且刻意固定 9 表，不新增（D2）。
//! - 收据 JSON 的键序由 serde 结构声明序决定；哈希只在 Tauri 内部自洽
//!   （存 → 回读比对），不要求与 Node 逐字节一致（D3）。
//!   身份分支的 `payloadHash` / `sourceHash` 经 serde_json::Value 往返
//!   规范化键序后计算，与 `character_proposal::prove_character_proposal`
//!   的 `text_hash(to_string(value))` 校验同构（D3 的内部自洽前提）。
//!
//! # 身份分支（m02）
//!
//! `has_character_identity_schema` 为真时走身份分支：revision 守卫 →
//! 项目清单探测 → 角色提议信封落库（`CharacterProposalService::stage`，
//! 语句并入调用方事务）→ 回读证据深比对。legacy 收据（`roster`）与
//! 身份收据（`characterProposal` + `proposalSource`）在收据类型上互斥，
//! 与上游判别联合一致。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::import::parsing::sha256_hex;
use crate::project_access::{self, ProjectProbe};
use crate::repositories::character_repository::has_character_identity_schema;
use crate::repositories::character_roster_repository as character_roster;
use crate::repositories::generation_run_repository::GenerationRunRepository;
use crate::repositories::project_core_repository as project_core;
use crate::services::character_proposal::{
    find_character_proposal_evidence, prove_character_proposal, CharacterProposalProof,
    CharacterProposalService,
};

/// 核心文本字段（对齐 `CORE_TEXT_FIELDS`，共 13 个；序列化键为 camelCase）
const CORE_TEXT_FIELDS: [&str; 13] = [
    "genre",
    "subGenre",
    "targetAudience",
    "plotStructure",
    "narrativePov",
    "goldenFinger",
    "globalGuidance",
    "coreOutline",
    "worldSetting",
    "protagonistProfile",
    "premise",
    "worldbuilding",
    "synopsis",
];

const PLOT_STRUCTURES: [&str; 6] = [
    "three_act",
    "heros_journey",
    "save_the_cat",
    "kishotenketsu",
    "multi_thread",
    "freeform",
];

const NARRATIVE_POVS: [&str; 4] = [
    "third_limited",
    "first_person",
    "third_omniscient",
    "multi_pov",
];

/// 对齐契约 `ImportGlobalFactsCore`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsCore {
    pub genre: String,
    pub sub_genre: String,
    pub target_audience: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub golden_finger: String,
    pub global_guidance: String,
    pub core_outline: String,
    pub world_setting: String,
    pub protagonist_profile: String,
    pub premise: String,
    pub worldbuilding: String,
    pub synopsis: String,
}

/// 对齐契约 `ImportGlobalFactsRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsRequest {
    pub operation_id: String,
    pub expected_roster_revision: i64,
    pub core: ImportGlobalFactsCore,
    pub character_entries: Vec<character_roster::CharacterRosterEntry>,
}

/// 身份分支的角色提议阶段证据（对齐契约 `CharacterProposalStageEvidence`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterProposalStageEvidence {
    pub proposal_batch_id: String,
    pub source_hash: String,
}

/// 对齐契约 `ImportGlobalFactsReceipt`：`roster`（legacy 分支）与
/// `characterProposal` + `proposalSource`（身份分支）互斥，随分支落库。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGlobalFactsReceipt {
    pub operation_id: String,
    pub payload_hash: String,
    pub idempotent: bool,
    pub core: ImportGlobalFactsCore,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster: Option<character_roster::CharacterRosterCommitReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_proposal: Option<CharacterProposalStageEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_source: Option<ImportGlobalFactsRequest>,
}

/// 台账行（对齐基线 `OperationRow`）
struct OperationRow {
    operation_id: String,
    payload_hash: String,
    receipt_json: String,
}

/// 对齐 `normalizedRequest`：trim + 枚举 / 范围校验，返回规范化请求。
fn normalized_request(
    candidate: ImportGlobalFactsRequest,
) -> Result<ImportGlobalFactsRequest, String> {
    let operation_id = candidate.operation_id.trim();
    if operation_id.is_empty() || operation_id.len() > 160 {
        return Err("导入全局事实 operationId 无效".to_string());
    }
    if candidate.expected_roster_revision < 0 {
        return Err("导入全局事实角色 revision 无效".to_string());
    }
    let mut core = candidate.core;
    for field in CORE_TEXT_FIELDS {
        // 基线按 camelCase 键取值；此处按字段映射逐项 trim 校验。
        let value = match field {
            "genre" => &mut core.genre,
            "subGenre" => &mut core.sub_genre,
            "targetAudience" => &mut core.target_audience,
            "plotStructure" => &mut core.plot_structure,
            "narrativePov" => &mut core.narrative_pov,
            "goldenFinger" => &mut core.golden_finger,
            "globalGuidance" => &mut core.global_guidance,
            "coreOutline" => &mut core.core_outline,
            "worldSetting" => &mut core.world_setting,
            "protagonistProfile" => &mut core.protagonist_profile,
            "premise" => &mut core.premise,
            "worldbuilding" => &mut core.worldbuilding,
            "synopsis" => &mut core.synopsis,
            _ => unreachable!("CORE_TEXT_FIELDS 越界"),
        };
        if value.trim().is_empty() {
            return Err(format!("导入全局事实字段 {field} 无效"));
        }
        *value = value.trim().to_string();
    }
    if !PLOT_STRUCTURES.contains(&core.plot_structure.as_str()) {
        return Err("导入全局事实字段 plotStructure 无效".to_string());
    }
    if !NARRATIVE_POVS.contains(&core.narrative_pov.as_str()) {
        return Err("导入全局事实字段 narrativePov 无效".to_string());
    }
    if core.total_chapters < 1 {
        return Err("导入全局事实总章数无效".to_string());
    }
    if core.words_per_chapter < 1 {
        return Err("导入全局事实章节字数无效".to_string());
    }
    if candidate.character_entries.is_empty() {
        return Err("导入全局事实角色名单不能为空".to_string());
    }
    // 逐条校验（对齐上游）：姓名非空；角色定位由类型化 `CharacterRole`
    // 枚举结构性保证 ∈ 四个合法值；关系的 target / relation 非空。
    for entry in &candidate.character_entries {
        if entry.name.trim().is_empty()
            || entry.relationships.iter().any(|relation| {
                relation.target.trim().is_empty() || relation.relation.trim().is_empty()
            })
        {
            return Err("导入角色提议格式无效".to_string());
        }
    }
    Ok(ImportGlobalFactsRequest {
        operation_id: operation_id.to_string(),
        expected_roster_revision: candidate.expected_roster_revision,
        core,
        character_entries: candidate.character_entries,
    })
}

/// 对齐 `hashRequest`（`sha256(JSON.stringify(request))`）。
///
/// 键序经 `serde_json::Value` 往返规范化（BTreeMap 键序），与
/// `prove_character_proposal` 对 `proposalSource` 的
/// `text_hash(to_string(value))` 校验、以及收据回读后的再序列化
/// 三者同构——这是身份路径「存 → prove 校验 → 回读比对」自洽的前提（D3）。
fn hash_request(request: &ImportGlobalFactsRequest) -> Result<String, String> {
    let normalized = serde_json::to_value(request)
        .map_err(|error| format!("全局事实请求序列化失败：{error}"))?;
    let json = serde_json::to_string(&normalized)
        .map_err(|error| format!("全局事实请求序列化失败：{error}"))?;
    Ok(sha256_hex(&json))
}

/// 对齐 `coreSnapshot`：当前项目主台账的核心事实投影。
fn core_snapshot(conn: &Connection) -> Result<ImportGlobalFactsCore, String> {
    let current = project_core::get(conn)?.ok_or_else(|| "项目主台账未初始化".to_string())?;
    Ok(ImportGlobalFactsCore {
        genre: current.genre,
        sub_genre: current.sub_genre,
        target_audience: current.target_audience,
        total_chapters: current.total_chapters,
        words_per_chapter: current.words_per_chapter,
        plot_structure: current.plot_structure,
        narrative_pov: current.narrative_pov,
        golden_finger: current.golden_finger,
        global_guidance: current.global_guidance,
        core_outline: current.core_outline,
        world_setting: current.world_setting,
        protagonist_profile: current.protagonist_profile,
        premise: current.premise,
        worldbuilding: current.worldbuilding,
        synopsis: current.synopsis,
    })
}

/// 对齐 `parseReceipt`：回读收据并核对身份绑定；身份收据额外校验
/// `proposalSource` 的载荷哈希（即「存的是这条请求」）。
fn parse_receipt(row: &OperationRow) -> Result<ImportGlobalFactsReceipt, String> {
    let parsed: ImportGlobalFactsReceipt = serde_json::from_str(&row.receipt_json)
        .map_err(|_| "导入全局事实收据损坏，已拒绝重放".to_string())?;
    if parsed.operation_id != row.operation_id || parsed.payload_hash != row.payload_hash {
        return Err("导入全局事实收据损坏，已拒绝重放".to_string());
    }
    if let Some(proposal_source) = &parsed.proposal_source {
        if hash_request(proposal_source)? != row.payload_hash {
            return Err("导入全局事实收据损坏，已拒绝重放".to_string());
        }
    }
    Ok(parsed)
}

/// 对齐 `ensureLedger`（D2：懒建表，不动 `db/schema.rs`）。
fn ensure_ledger(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS import_global_fact_operations (
            operation_id TEXT PRIMARY KEY,
            payload_hash TEXT NOT NULL,
            receipt_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .map_err(|error| format!("创建全局事实台账失败：{error}"))
}

fn read_operation_row(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<OperationRow>, String> {
    conn.query_row(
        "SELECT operation_id, payload_hash, receipt_json
         FROM import_global_fact_operations WHERE operation_id = ?1",
        [operation_id],
        |row| {
            Ok(OperationRow {
                operation_id: row.get(0)?,
                payload_hash: row.get(1)?,
                receipt_json: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取全局事实台账失败：{error}"))
}

/// 事务体内的提交逻辑（批次 G3a 抽取；B1-W-IGF 增补身份分支）：供
/// [`ImportGlobalFactsRepository::commit`] 与
/// `import_run_repository::commit_effect_receipt` 的跨仓原子事务复用。
///
/// `session` = `(project_id, project_path)`：身份分支用 `project_path`
/// 探测项目清单（对齐上游 `getCurrentProjectPath()` + `probeExistingProject`）；
/// `None` 时身份分支按上游语义报「导入项目身份缺失」。
/// 错误返回时 `tx` drop 即回滚，与基线 `db.transaction()` 语义一致。
pub(crate) fn commit_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    candidate: ImportGlobalFactsRequest,
    session: Option<(&str, &str)>,
) -> Result<ImportGlobalFactsReceipt, String> {
    let request = normalized_request(candidate)?;
    let payload_hash = hash_request(&request)?;
    if let Some(existing) = read_operation_row(tx, &request.operation_id)? {
        if existing.payload_hash != payload_hash {
            return Err("导入全局事实 operationId 已绑定不同载荷".to_string());
        }
        let stored = parse_receipt(&existing)?;
        assert_facts_unchanged(tx, &stored)?;
        if stored.character_proposal.is_some() {
            // 身份收据：提议已落库，原样幂等 ACK（对齐上游不重写）。
            let mut replay = stored;
            replay.idempotent = true;
            return Ok(replay);
        }
        // legacy 收据：刷新名单 revision / 快照后幂等 ACK。
        let current_roster = character_roster::read(tx)?;
        let Some(mut roster) = stored.roster.clone() else {
            return Err("导入全局事实收据损坏，已拒绝重放".to_string());
        };
        roster.revision = current_roster.revision;
        roster.snapshot = current_roster;
        let mut replay = stored;
        replay.idempotent = true;
        replay.roster = Some(roster);
        return Ok(replay);
    }

    if has_character_identity_schema(tx) {
        return commit_identity_branch(tx, &request, &payload_hash, session);
    }

    // legacy 分支：主台账更新 + 角色名单提交 + 收据落库。
    let core_json = serde_json::to_value(&request.core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    let core_map = core_json
        .as_object()
        .cloned()
        .ok_or_else(|| "导入全局事实配置无效".to_string())?;
    project_core::update(tx, &core_map)?;
    let roster_payload = serde_json::json!({
        "operationId": format!("{}:roster", request.operation_id),
        "expectedRevision": request.expected_roster_revision,
        "schemaVersion": character_roster::CHARACTER_ROSTER_SCHEMA_VERSION,
        "intent": "novel_import",
        "entries": request.character_entries,
    });
    // 基线在 db.transaction() 内嵌套调用 CharacterRosterRepository.commit；
    // rusqlite 不支持嵌套 BEGIN，故复用抽取出的事务体（G3a）。
    let roster = character_roster::commit_in_transaction(tx, &roster_payload, None)?;
    let receipt = ImportGlobalFactsReceipt {
        operation_id: request.operation_id.clone(),
        payload_hash: payload_hash.clone(),
        idempotent: false,
        core: core_snapshot(tx)?,
        roster: Some(roster),
        character_proposal: None,
        proposal_source: None,
    };
    tx.execute(
        "INSERT INTO import_global_fact_operations (operation_id, payload_hash, receipt_json)
         VALUES (?1, ?2, ?3)",
        [
            &request.operation_id,
            &payload_hash,
            &serde_json::to_string(&receipt)
                .map_err(|error| format!("全局事实收据序列化失败：{error}"))?,
        ],
    )
    .map_err(|error| format!("写入全局事实台账失败：{error}"))?;
    Ok(receipt)
}

/// 身份分支（对齐上游 m02 `hasCharacterIdentitySchema` 路径）：
/// revision 守卫 → 项目清单探测 → 核心更新 → 收据落库 →
/// 角色提议 staging（语句并入本事务）→ 证据回读深比对。
///
/// 注意顺序：prove 需要读台账收据，故 INSERT 必须先于 `stage`。
fn commit_identity_branch(
    tx: &rusqlite::Transaction<'_>,
    request: &ImportGlobalFactsRequest,
    payload_hash: &str,
    session: Option<(&str, &str)>,
) -> Result<ImportGlobalFactsReceipt, String> {
    let revision = tx
        .query_row(
            "SELECT revision FROM character_roster_meta WHERE id='main'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取角色名单 revision 失败：{error}"))?;
    if revision != Some(request.expected_roster_revision) {
        return Err("角色名单 revision 已过期，已拒绝导入".to_string());
    }
    let Some((_session_project_id, project_path)) = session else {
        return Err("导入项目身份缺失".to_string());
    };
    let probe = project_access::probe_existing_project(project_path, None)?;
    let ProjectProbe::Manifest(trusted) = probe else {
        return Err("导入项目身份未迁移".to_string());
    };
    let project_id = trusted.project_id.clone();
    let source = serde_json::json!({
        "kind": "import",
        "operationId": request.operation_id,
    });
    // 条目载荷经 Value 规范化键序后参与哈希（与 prove 侧同构，见 D3）。
    let entries_value = serde_json::to_value(&request.character_entries)
        .map_err(|error| format!("全局事实请求序列化失败：{error}"))?;
    let character_proposal = CharacterProposalStageEvidence {
        proposal_batch_id: format!(
            "cpb:{}",
            sha256_hex(
                &serde_json::to_string(&serde_json::json!([project_id, source]))
                    .map_err(|error| format!("全局事实请求序列化失败：{error}"))?,
            ),
        ),
        source_hash: sha256_hex(
            &serde_json::to_string(&serde_json::json!([payload_hash, entries_value]))
                .map_err(|error| format!("全局事实请求序列化失败：{error}"))?,
        ),
    };
    let core_json = serde_json::to_value(&request.core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    let core_map = core_json
        .as_object()
        .cloned()
        .ok_or_else(|| "导入全局事实配置无效".to_string())?;
    project_core::update(tx, &core_map)?;
    let receipt = ImportGlobalFactsReceipt {
        operation_id: request.operation_id.clone(),
        payload_hash: payload_hash.to_string(),
        idempotent: false,
        core: core_snapshot(tx)?,
        roster: None,
        character_proposal: Some(character_proposal.clone()),
        proposal_source: Some(request.clone()),
    };
    tx.execute(
        "INSERT INTO import_global_fact_operations (operation_id, payload_hash, receipt_json)
         VALUES (?1, ?2, ?3)",
        [
            &request.operation_id,
            &receipt.payload_hash,
            &serde_json::to_string(&receipt)
                .map_err(|error| format!("全局事实收据序列化失败：{error}"))?,
        ],
    )
    .map_err(|error| format!("写入全局事实台账失败：{error}"))?;
    // prove 闭包与上游构造器注入一致：`proveCharacterProposal(db, runs,
    // projectId, candidate, forWrite, assertSources, version)`。
    let service = CharacterProposalService::new(
        &tx,
        &project_id,
        |candidate: &serde_json::Value,
         for_write: bool,
         architecture_derivation_version: u8|
         -> Result<CharacterProposalProof, String> {
            prove_character_proposal(
                &tx,
                &GenerationRunRepository::default(),
                &project_id,
                candidate,
                for_write,
                |_assert_sources: &serde_json::Value, _expected_source_ids: Option<&[i64]>| Ok(()),
                architecture_derivation_version,
            )
        },
    );
    service.stage(&source)?;
    let evidence = service.evidence(&character_proposal.proposal_batch_id)?;
    let expected_evidence = serde_json::to_value(&character_proposal)
        .map_err(|error| format!("全局事实收据序列化失败：{error}"))?;
    if evidence != expected_evidence {
        return Err("导入角色提议回读失败".to_string());
    }
    Ok(receipt)
}

/// 对齐 `assertCurrentReceipt`：当前事实必须与收据一致。
/// 身份收据比对提议证据（`findCharacterProposalEvidence` 深比对）；
/// legacy 收据比对**姓名版本** factHash（身份 schema 升级后公开
/// factHash 会从姓名版本变成 ID 版本，历史回执只能与保留的姓名版本
/// 比较，否则一次合法的幂等 ACK 重试会被误报）。
fn assert_facts_unchanged(
    conn: &Connection,
    stored: &ImportGlobalFactsReceipt,
) -> Result<(), String> {
    let current_core = core_snapshot(conn)?;
    let current_core_json = serde_json::to_value(&current_core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    let stored_core_json = serde_json::to_value(&stored.core)
        .map_err(|error| format!("全局事实核心序列化失败：{error}"))?;
    if current_core_json != stored_core_json {
        return Err("导入全局事实已被后续修改，不能将历史操作冒充为当前事实".to_string());
    }
    if let Some(character_proposal) = &stored.character_proposal {
        let evidence = find_character_proposal_evidence(
            conn,
            &serde_json::json!({
                "kind": "import",
                "operationId": stored.operation_id,
            }),
            |project_id: &str, candidate: &serde_json::Value| {
                prove_character_proposal(
                    conn,
                    &GenerationRunRepository::default(),
                    project_id,
                    candidate,
                    false,
                    |_assert_sources: &serde_json::Value, _expected_source_ids: Option<&[i64]>| {
                        Ok(())
                    },
                    2,
                )
            },
        )?;
        let stored_evidence = serde_json::to_value(character_proposal)
            .map_err(|error| format!("全局事实收据序列化失败：{error}"))?;
        if evidence != stored_evidence {
            return Err("导入角色提议回执已失效".to_string());
        }
        return Ok(());
    }
    let roster = stored
        .roster
        .as_ref()
        .ok_or_else(|| "导入全局事实收据损坏，已拒绝重放".to_string())?;
    let current_roster = character_roster::read(conn)?;
    if current_roster.name_only_fact_hash != roster.snapshot.fact_hash {
        return Err("导入角色事实已被后续修改".to_string());
    }
    Ok(())
}

/** Atomic import seam for config, non-character architecture and roster facts. */
pub struct ImportGlobalFactsRepository;

impl ImportGlobalFactsRepository {
    /// 历史 ACK：只证明保存过的操作，不要求当前事实一致
    /// （对齐 `readHistoricalCommittedOperation`；台账表未建时返回 `None`）。
    pub fn read_historical_committed_operation(
        conn: &Connection,
        operation_id: &str,
    ) -> Result<Option<ImportGlobalFactsReceipt>, String> {
        if operation_id.trim().is_empty() {
            return Err("导入全局事实 operationId 无效".to_string());
        }
        let table_exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table'
                 AND name='import_global_fact_operations'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("读取全局事实台账失败：{error}"))?;
        if table_exists.is_none() {
            return Ok(None);
        }
        let Some(row) = read_operation_row(conn, operation_id)? else {
            return Ok(None);
        };
        parse_receipt(&row).map(Some)
    }

    /// 只读权威证据：已提交导入操作的回读（对齐 `getCommittedOperation`）。
    pub fn get_committed_operation(
        conn: &Connection,
        operation_id: &str,
    ) -> Result<Option<ImportGlobalFactsReceipt>, String> {
        if operation_id.trim().is_empty() {
            return Err("导入全局事实 operationId 无效".to_string());
        }
        ensure_ledger(conn)?;
        let Some(row) = read_operation_row(conn, operation_id)? else {
            return Ok(None);
        };
        let stored = parse_receipt(&row)?;
        assert_facts_unchanged(conn, &stored)?;
        Ok(Some(stored))
    }

    /// 对齐 `commit`：事务内幂等重放或「主台账更新 + 角色名单 / 提议提交」。
    ///
    /// `session` = `(project_id, project_path)`，见 [`commit_in_transaction`]。
    pub fn commit(
        conn: &Connection,
        candidate: ImportGlobalFactsRequest,
        session: Option<(&str, &str)>,
    ) -> Result<ImportGlobalFactsReceipt, String> {
        ensure_ledger(conn)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("开启事务失败：{error}"))?;
        let receipt = commit_in_transaction(&tx, candidate, session)?;
        tx.commit()
            .map_err(|error| format!("提交事务失败：{error}"))?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::character_roster_repository::CharacterRosterEntry;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 与 `project_access::write_manifest` 同构的合法清单 projectId（v4 UUID）。
    const TEST_PROJECT_ID: &str = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";

    /// 内存库 + 主台账 / 角色名单 schema（`create_tables` 已含身份 schema）。
    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        crate::db::schema::create_tables(&conn).expect("schema");
        crate::db::schema::migrate_character_roster_schema(&conn).expect("角色名单 schema");
        // 基线语义：全局事实提交要求主台账已初始化（项目打开时 init）
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("初始化主台账");
        conn
    }

    /// 模拟 M02 前项目：身份 schema 尚未建立（`hasCharacterIdentitySchema`
    /// 为假），强制 commit 走 legacy 名单分支。
    fn legacy_conn() -> Connection {
        let conn = memory_conn();
        conn.execute_batch(
            "DROP TABLE character_identity_meta;
             DROP TABLE character_identity_origins;
             DROP TABLE character_identity_proposals;
             DROP TABLE character_identity_approvals;
             DROP TABLE character_aliases;
             DROP TABLE character_relationships;",
        )
        .expect("降级身份 schema");
        conn
    }

    /// 带合法清单的临时项目目录（`probe_existing_project` 返回 Manifest）。
    struct TestProject {
        root: PathBuf,
        root_display: String,
    }

    impl TestProject {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "lorekeeper-igf-{label}-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).expect("创建测试项目目录");
            let lore = root.join(".lore");
            std::fs::create_dir_all(&lore).expect("创建 .lore 目录");
            let manifest = serde_json::json!({
                "schemaVersion": 1,
                "kind": "ai-novel-project",
                "projectId": TEST_PROJECT_ID,
                "createdAt": "2026-10-06T12:00:00.000Z",
            });
            std::fs::write(
                lore.join("project.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&manifest).expect("清单序列化")
                ),
            )
            .expect("写入项目清单");
            Self {
                root_display: root.to_string_lossy().to_string(),
                root,
            }
        }

        /// 会话透传 `(project_id, project_path)`；身份分支的 projectId
        /// 以清单探测为准（对齐上游 `probeExistingProject().projectId`）。
        fn session(&self) -> Option<(&str, &str)> {
            Some((TEST_PROJECT_ID, self.root_display.as_str()))
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn core(genre: &str) -> ImportGlobalFactsCore {
        ImportGlobalFactsCore {
            genre: genre.to_string(),
            sub_genre: "都市".to_string(),
            target_audience: "general".to_string(),
            total_chapters: 100,
            words_per_chapter: 3000,
            plot_structure: "three_act".to_string(),
            narrative_pov: "third_limited".to_string(),
            golden_finger: "金手指".to_string(),
            global_guidance: "全局指导".to_string(),
            core_outline: "核心大纲".to_string(),
            world_setting: "世界观".to_string(),
            protagonist_profile: "主角".to_string(),
            premise: "前提".to_string(),
            worldbuilding: "世界构建".to_string(),
            synopsis: "情节大纲".to_string(),
        }
    }

    /// 经 serde 构造条目（`CharacterRosterEntry` 无 `Default`，
    /// 其余字段均为 `#[serde(default)]`）。
    fn entry(name: &str) -> CharacterRosterEntry {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "role": "protagonist",
        }))
        .expect("角色条目构造")
    }

    fn request(genre: &str, operation_id: &str) -> ImportGlobalFactsRequest {
        ImportGlobalFactsRequest {
            operation_id: operation_id.to_string(),
            expected_roster_revision: 0,
            core: core(genre),
            character_entries: vec![entry("角色一")],
        }
    }

    // ===== 身份分支（上游 import-global-facts-repository.test.ts 移植） =====

    /// 上游 `request()` 的角色卡构造（全字段，relationships 为空数组）。
    fn import_card(name: &str, role: &str) -> CharacterRosterEntry {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "role": role,
            "gender": "未知",
            "age": "18",
            "appearance": "明确",
            "personality": "明确",
            "background": "明确",
            "abilities": "明确",
            "motivation": "明确",
            "relationships": [],
            "arc": "明确",
            "notes": "待确认",
        }))
        .expect("角色条目构造")
    }

    fn import_core() -> ImportGlobalFactsCore {
        ImportGlobalFactsCore {
            genre: "现实".to_string(),
            sub_genre: "讽刺".to_string(),
            target_audience: "通用".to_string(),
            total_chapters: 9,
            words_per_chapter: 2500,
            plot_structure: "three_act".to_string(),
            narrative_pov: "third_limited".to_string(),
            golden_finger: "无".to_string(),
            global_guidance: "克制".to_string(),
            core_outline: "阿Q由自尊走向幻灭".to_string(),
            world_setting: "辛亥前后的江南乡村".to_string(),
            protagonist_profile: "阿Q，贫困而善于精神胜利".to_string(),
            premise: "个人与社会冲突".to_string(),
            worldbuilding: "未庄".to_string(),
            synopsis: "阿Q的命运".to_string(),
        }
    }

    fn import_request(operation_id: &str) -> ImportGlobalFactsRequest {
        ImportGlobalFactsRequest {
            operation_id: operation_id.to_string(),
            expected_roster_revision: 0,
            core: import_core(),
            character_entries: vec![
                import_card("阿Q", "protagonist"),
                import_card("吴妈", "supporting"),
            ],
        }
    }

    fn commit_identity(
        conn: &Connection,
        project: &TestProject,
        request: ImportGlobalFactsRequest,
    ) -> Result<ImportGlobalFactsReceipt, String> {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let receipt = commit_in_transaction(&tx, request, project.session())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(receipt)
    }

    #[test]
    fn identity_commit_stages_source_preserving_proposal_test() {
        let project = TestProject::new("identity-commit");
        let conn = memory_conn();
        let receipt = commit_identity(&conn, &project, import_request("import-global-run-1"))
            .expect("身份分支提交");

        assert_eq!(receipt.operation_id, "import-global-run-1");
        assert!(!receipt.idempotent);
        assert!(receipt.roster.is_none());
        let proposal = receipt.character_proposal.expect("身份收据含提议证据");
        assert!(proposal.proposal_batch_id.starts_with("cpb:"));
        assert_eq!(proposal.proposal_batch_id.len(), 4 + 64);
        assert_eq!(proposal.source_hash.len(), 64);
        assert!(proposal.source_hash.chars().all(|c| c.is_ascii_hexdigit()));
        // proposalSource 完整回显请求
        let source = receipt.proposal_source.expect("身份收据含提议来源");
        assert_eq!(source.operation_id, "import-global-run-1");
        assert_eq!(source.expected_roster_revision, 0);
        assert_eq!(source.character_entries.len(), 2);
        assert_eq!(source.character_entries[0].name, "阿Q");
        // core 已更新
        let current = project_core::get(&conn)
            .expect("主台账读回")
            .expect("主台账已初始化");
        assert_eq!(current.genre, "现实");
        assert_eq!(current.premise, "个人与社会冲突");
        assert_eq!(current.synopsis, "阿Q的命运");
        assert_eq!(current.core_outline, "阿Q由自尊走向幻灭");
        assert_eq!(current.world_setting, "辛亥前后的江南乡村");
        assert_eq!(current.protagonist_profile, "阿Q，贫困而善于精神胜利");
        assert!(current.characters_arch.is_empty());
        // 名单不受影响
        let roster = character_roster::read(&conn).expect("名单读回");
        assert_eq!(roster.revision, 0);
        assert!(matches!(
            roster.status,
            character_roster::CharacterRosterStatus::Empty
        ));
        // 角色未落库，提议信封已落库
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
            .expect("角色计数");
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM character_identity_proposals WHERE source_key LIKE 'character-proposal-v1:%'",
                [],
                |row| row.get(0),
            )
            .expect("提议计数");
        assert_eq!(count, 1);
    }

    #[test]
    fn identity_commit_rolls_back_when_proposal_staging_fails_test() {
        let project = TestProject::new("identity-rollback");
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TRIGGER reject_imported_roster
             BEFORE INSERT ON character_identity_proposals
             BEGIN SELECT RAISE(ABORT, 'injected roster failure'); END;",
        )
        .expect("注入失败触发器");
        let tx = conn.unchecked_transaction().expect("事务");
        let error = commit_in_transaction(
            &tx,
            import_request("import-global-run-1"),
            project.session(),
        )
        .expect_err("注入的落库失败应传播");
        assert!(error.contains("injected roster failure"));
        tx.rollback().expect("回滚");
        // core 与台账保持原状
        let current = project_core::get(&conn)
            .expect("主台账读回")
            .expect("主台账已初始化");
        assert!(current.genre.is_empty());
        let roster = character_roster::read(&conn).expect("名单读回");
        assert_eq!(roster.revision, 0);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM import_global_fact_operations",
                [],
                |row| row.get(0),
            )
            .expect("台账计数");
        assert_eq!(count, 0);
    }

    #[test]
    fn identity_commit_replays_without_rewriting_facts_test() {
        let project = TestProject::new("identity-replay");
        let conn = memory_conn();
        let first = commit_identity(&conn, &project, import_request("import-global-run-1"))
            .expect("首次提交");
        let replay = commit_identity(&conn, &project, import_request("import-global-run-1"))
            .expect("幂等重放");

        let mut expected = first;
        expected.idempotent = true;
        assert_eq!(replay, expected);
        let roster = character_roster::read(&conn).expect("名单读回");
        assert_eq!(roster.revision, 0);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM character_identity_proposals",
                [],
                |row| row.get(0),
            )
            .expect("提议计数");
        assert_eq!(count, 1);
    }

    #[test]
    fn identity_commit_keeps_same_name_imports_distinct_test() {
        let project = TestProject::new("identity-distinct");
        let conn = memory_conn();
        let first_input = import_request("import-global-run-1");
        let mut second_input = import_request("import-other-source");
        second_input.character_entries[0].background = "另一份原稿中的同名者".to_string();
        let first = commit_identity(&conn, &project, first_input).expect("首次提交");
        let second = commit_identity(&conn, &project, second_input).expect("二次提交");

        let first_proposal = first.character_proposal.expect("首次提议证据");
        let second_proposal = second.character_proposal.expect("二次提议证据");
        assert_ne!(
            first_proposal.proposal_batch_id,
            second_proposal.proposal_batch_id
        );
        let envelopes: Vec<serde_json::Value> = conn
            .prepare(
                "SELECT raw_value FROM character_identity_proposals WHERE source_key LIKE 'character-proposal-v1:%'",
            )
            .expect("查询")
            .query_map([], |row| {
                let raw: String = row.get(0)?;
                Ok(serde_json::from_str(&raw).expect("信封解析"))
            })
            .expect("查询")
            .collect::<Result<Vec<_>, _>>()
            .expect("信封收集");
        assert_eq!(envelopes.len(), 2);
        let items: Vec<&serde_json::Value> = envelopes
            .iter()
            .map(|envelope| &envelope["batch"]["items"][0])
            .collect();
        assert_eq!(items[0]["fields"]["name"], serde_json::json!("阿Q"));
        assert_eq!(items[1]["fields"]["name"], serde_json::json!("阿Q"));
        let first_source_id = items[0]["sourceId"].as_str().expect("sourceId");
        let second_source_id = items[1]["sourceId"].as_str().expect("sourceId");
        assert_ne!(first_source_id, second_source_id);
        for item in &items {
            assert_eq!(
                item["resolution"]["status"],
                serde_json::json!("unresolved")
            );
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
            .expect("角色计数");
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM character_relationships", [], |row| {
                row.get(0)
            })
            .expect("关系计数");
        assert_eq!(count, 0);
    }

    #[test]
    fn identity_commit_retains_inferred_provenance_test() {
        let project = TestProject::new("identity-provenance");
        let conn = memory_conn();
        let mut input = import_request("import-global-run-1");
        input.character_entries[0].current_state = Some(
            serde_json::from_value(serde_json::json!({
                "location": "原稿推断的位置",
                "powerLevel": "",
                "physicalState": "",
                "mentalState": "",
                "keyItems": "",
                "recentEvents": "",
                "updatedAtChapter": 2,
            }))
            .expect("动态状态构造"),
        );
        let receipt = commit_identity(&conn, &project, input).expect("提交");
        let batch_id = receipt
            .character_proposal
            .expect("提议证据")
            .proposal_batch_id;
        let raw: String = conn
            .query_row(
                "SELECT raw_value FROM character_identity_proposals WHERE proposal_id=?1",
                [batch_id],
                |row| row.get(0),
            )
            .expect("信封读回");
        let envelope: serde_json::Value = serde_json::from_str(&raw).expect("信封解析");
        assert!(envelope["proof"]["provenance"].is_null());
        let item = &envelope["batch"]["items"][0];
        assert_eq!(
            item["rawValue"]["currentState"]["location"],
            serde_json::json!("原稿推断的位置")
        );
        assert!(item["fields"].get("currentState").is_none());
        assert!(item["fields"].get("provenance").is_none());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM character_identity_approvals",
                [],
                |row| row.get(0),
            )
            .expect("审批计数");
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
            .expect("角色计数");
        assert_eq!(count, 0);
    }

    // ===== legacy 收据（上游 import-global-facts-legacy-receipt.test.ts 移植） =====

    /// 身份 schema 项目 + 两条同名角色 + 身份投影刷新
    /// （上游 beforeEach 的 fixture 形态）。
    fn identity_projection_conn() -> Connection {
        let conn = memory_conn();
        conn.execute(
            "INSERT INTO characters(character_id,name,role)
             VALUES('同名甲','沈砺','supporting'),('同名乙','沈砺','supporting')",
            [],
        )
        .expect("同名角色写入");
        let tx = conn.unchecked_transaction().expect("事务");
        character_roster::refresh_character_identity_projection(&tx).expect("身份投影刷新");
        tx.commit().expect("提交");
        conn
    }

    /// 历史回执的 roster 哈希是姓名版本，正是旧 legacy commit 写入的那个值。
    fn seed_legacy_receipt(conn: &Connection, operation_id: &str, roster_fact_hash: &str) {
        // 先触发 ensureLedger（上游先读一次缺失操作）
        ImportGlobalFactsRepository::get_committed_operation(conn, "seed-probe-missing")
            .expect("缺失操作只读返回");
        let snapshot = character_roster::read(conn).expect("名单读回");
        let mut snapshot_with_hash = snapshot.clone();
        snapshot_with_hash.fact_hash = roster_fact_hash.to_string();
        let receipt = ImportGlobalFactsReceipt {
            operation_id: operation_id.to_string(),
            payload_hash: "b".repeat(64),
            idempotent: false,
            core: core_snapshot(conn).expect("核心快照"),
            roster: Some(character_roster::CharacterRosterCommitReceipt {
                operation_id: format!("{operation_id}:roster"),
                payload_hash: "c".repeat(64),
                revision: snapshot.revision,
                idempotent: false,
                snapshot: snapshot_with_hash,
            }),
            character_proposal: None,
            proposal_source: None,
        };
        conn.execute(
            "INSERT INTO import_global_fact_operations(operation_id,payload_hash,receipt_json)
             VALUES(?1,?2,?3)",
            rusqlite::params![
                receipt.operation_id,
                receipt.payload_hash,
                serde_json::to_string(&receipt).expect("收据序列化")
            ],
        )
        .expect("种子收据写入");
    }

    #[test]
    fn legacy_receipt_public_hash_distinct_from_name_only_hash_test() {
        let conn = identity_projection_conn();
        let snapshot = character_roster::read(&conn).expect("名单读回");
        // 身份 schema 项目里两者必须真的不同，否则本用例的区分就没有意义。
        assert_ne!(snapshot.fact_hash, snapshot.name_only_fact_hash);
        assert_eq!(snapshot.name_only_fact_hash.len(), 64);
        assert!(snapshot
            .name_only_fact_hash
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn legacy_receipt_validates_idempotent_ack_against_name_only_hash_test() {
        let conn = identity_projection_conn();
        let name_only = character_roster::read(&conn)
            .expect("名单读回")
            .name_only_fact_hash;
        seed_legacy_receipt(&conn, "legacy-import-1", &name_only);
        let stored = ImportGlobalFactsRepository::get_committed_operation(&conn, "legacy-import-1")
            .expect("历史回执读回")
            .expect("种子收据存在");
        assert_eq!(stored.operation_id, "legacy-import-1");
    }

    #[test]
    fn legacy_receipt_refuses_receipt_whose_facts_changed_test() {
        let conn = identity_projection_conn();
        seed_legacy_receipt(&conn, "legacy-import-2", &"d".repeat(64));
        let error = ImportGlobalFactsRepository::get_committed_operation(&conn, "legacy-import-2")
            .expect_err("事实已变的收据应拒绝");
        assert!(error.contains("导入角色事实已被后续修改"));
    }

    // ===== Rust 侧补充（收据防篡改 / 历史 ACK 语义） =====

    #[test]
    fn tampered_proposal_source_rejected_test() {
        let project = TestProject::new("tamper");
        let conn = memory_conn();
        let receipt =
            commit_identity(&conn, &project, import_request("import-global-run-1")).expect("提交");
        // 篡改收据中的 proposalSource（operationId 替换 → 载荷哈希不再匹配）
        let mut stored: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&receipt).expect("收据序列化"))
                .expect("收据解析");
        stored["proposalSource"]["operationId"] = serde_json::json!("tampered-operation");
        conn.execute(
            "UPDATE import_global_fact_operations SET receipt_json=?1 WHERE operation_id=?2",
            rusqlite::params![stored.to_string(), receipt.operation_id],
        )
        .expect("收据篡改");
        let error =
            ImportGlobalFactsRepository::get_committed_operation(&conn, &receipt.operation_id)
                .expect_err("篡改收据应拒绝");
        assert!(error.contains("导入全局事实收据损坏"));
    }

    #[test]
    fn historical_ack_survives_current_facts_change_test() {
        let project = TestProject::new("historical");
        let conn = memory_conn();
        // 台账表尚未建立时，历史 ACK 直接返回 None（对齐上游表存在性检查）
        assert!(
            ImportGlobalFactsRepository::read_historical_committed_operation(
                &conn,
                "import-global-run-1"
            )
            .expect("未建表返回 None")
            .is_none()
        );
        let receipt =
            commit_identity(&conn, &project, import_request("import-global-run-1")).expect("提交");
        // 后续修改主台账 → 权威读面拒绝，但历史 ACK 仍可读
        let mut map = serde_json::Map::new();
        map.insert("genre".to_string(), serde_json::json!("玄幻"));
        project_core::update(&conn, &map).expect("主台账更新");
        let historical = ImportGlobalFactsRepository::read_historical_committed_operation(
            &conn,
            &receipt.operation_id,
        )
        .expect("历史 ACK")
        .expect("已提交操作应可读回");
        assert_eq!(historical.operation_id, "import-global-run-1");
        assert!(!historical.idempotent);
        assert_eq!(historical.core.genre, "现实");
        let error =
            ImportGlobalFactsRepository::get_committed_operation(&conn, &receipt.operation_id)
                .expect_err("当前事实已变，权威读面应拒绝");
        assert!(error.contains("导入全局事实已被后续修改"));
    }

    // ===== legacy 分支（G3a 既有用例，改造为模拟 M02 前项目） =====

    #[test]
    fn normalized_request_trims_and_rejects_invalid_test() {
        let mut bad = request("  ", "op-1");
        assert!(normalized_request(bad.clone()).is_err());
        bad.core.total_chapters = 0;
        assert!(normalized_request(bad).is_err());
        let mut bad_pov = request("仙侠", "op-1");
        bad_pov.core.narrative_pov = "unknown".to_string();
        assert!(normalized_request(bad_pov).is_err());
        let mut bad_revision = request("仙侠", "op-1");
        bad_revision.expected_roster_revision = -1;
        assert!(normalized_request(bad_revision).is_err());
        // 逐条校验：姓名空白 / 关系 target 空白 → 「导入角色提议格式无效」
        let mut bad_name = request("仙侠", "op-entries");
        bad_name.character_entries[0].name = "  ".to_string();
        assert!(normalized_request(bad_name).is_err());
        let mut bad_relation = request("仙侠", "op-relation");
        bad_relation.character_entries[0].relationships =
            vec![character_roster::CharacterRosterRelationship {
                target: "  ".to_string(),
                relation: "同门".to_string(),
            }];
        assert!(normalized_request(bad_relation).is_err());
        let trimmed = normalized_request(request("  仙侠  ", "  op-1  ")).unwrap();
        assert_eq!(trimmed.core.genre, "仙侠");
        assert_eq!(trimmed.operation_id, "op-1");
    }

    #[test]
    fn commit_then_get_committed_operation_roundtrip_test() {
        let conn = legacy_conn();
        let receipt =
            ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1"), None).unwrap();
        assert!(!receipt.idempotent);
        assert_eq!(receipt.core.genre, "仙侠");
        assert_eq!(
            receipt.roster.as_ref().expect("legacy 收据含名单").revision,
            1
        );

        let read_back = ImportGlobalFactsRepository::get_committed_operation(&conn, "op-1")
            .unwrap()
            .expect("已提交操作应可读回");
        assert_eq!(read_back.payload_hash, receipt.payload_hash);
        assert_eq!(read_back.core.genre, "仙侠");

        // 幂等重放：同一载荷返回 idempotent 收据
        let replay =
            ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1"), None).unwrap();
        assert!(replay.idempotent);
        assert_eq!(
            replay.roster.as_ref().expect("legacy 收据含名单").revision,
            1
        );
    }

    #[test]
    fn commit_rejects_conflicting_payload_for_same_operation_test() {
        let conn = legacy_conn();
        ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1"), None).unwrap();
        let conflicting = ImportGlobalFactsRepository::commit(&conn, request("玄幻", "op-1"), None);
        assert!(conflicting.is_err());
        assert!(conflicting
            .unwrap_err()
            .contains("operationId 已绑定不同载荷"));
    }

    #[test]
    fn get_committed_operation_rejects_stale_evidence_test() {
        let conn = legacy_conn();
        ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-1"), None).unwrap();
        // 后续修改主台账 → 历史操作不得冒充当前事实
        let mut map = serde_json::Map::new();
        map.insert("genre".to_string(), serde_json::json!("玄幻"));
        map.insert(
            "plotStructure".to_string(),
            serde_json::json!("heros_journey"),
        );
        project_core::update(&conn, &map).unwrap();
        let stale = ImportGlobalFactsRepository::get_committed_operation(&conn, "op-1");
        assert!(stale.is_err());
        assert!(stale.unwrap_err().contains("已被后续修改"));
    }

    #[test]
    fn get_committed_operation_missing_returns_none_test() {
        let conn = legacy_conn();
        assert!(
            ImportGlobalFactsRepository::get_committed_operation(&conn, "missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn commit_validates_core_enums_test() {
        let conn = legacy_conn();
        let mut bad = request("仙侠", "op-enum");
        bad.core.plot_structure = "unknown".to_string();
        assert!(ImportGlobalFactsRepository::commit(&conn, bad, None).is_err());
        let mut bad_pov = request("仙侠", "op-enum");
        bad_pov.core.narrative_pov = "unknown".to_string();
        assert!(ImportGlobalFactsRepository::commit(&conn, bad_pov, None).is_err());
    }

    #[test]
    fn commit_rejects_empty_operation_id_test() {
        let conn = legacy_conn();
        assert!(ImportGlobalFactsRepository::commit(&conn, request("仙侠", "  "), None).is_err());
        assert!(ImportGlobalFactsRepository::get_committed_operation(&conn, "  ").is_err());
    }

    #[test]
    fn commit_creates_roster_with_bound_operation_id_test() {
        let conn = legacy_conn();
        let receipt =
            ImportGlobalFactsRepository::commit(&conn, request("仙侠", "op-roster"), None).unwrap();
        // roster 提交操作 ID 绑定为 `${operationId}:roster`，且名单条目已落地
        let roster = receipt.roster.as_ref().expect("legacy 收据含名单");
        assert_eq!(roster.operation_id, "op-roster:roster");
        assert_eq!(roster.snapshot.entries.len(), 1);
        assert_eq!(roster.snapshot.entries[0].name, "角色一");
    }
}
