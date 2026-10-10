//! CharacterIdentityRepository —— m02 角色身份六表仓储（批次 B2）。
//!
//! 平移自上游 `electron/repositories/character-roster-repository.ts` 的
//! identity 域（`commitCharacterIdentities` / `readCharacterIdentitySnapshot` /
//! `hasCharacterIdentitySchema`），覆盖 m02 六表：
//! `character_identity_origins` / `character_aliases` /
//! `character_identity_proposals` / `character_identity_approvals` /
//! `character_relationships` / `character_identity_meta`。
//! 其中 `character_identity_origins` 仅由 M02 迁移写入，本仓储只读不写。
//!
//! 与基线的一致性：
//! - 错误码逐字对齐（`CHARACTER_ID_SCHEMA_REQUIRED` / `CHARACTER_APPROVAL_REQUIRED` /
//!   `CHARACTER_APPROVAL_NONCE_CONFLICT` …）；「Error: 」前缀由命令层
//!   `mutating_error` 补齐（同 `character_roster_repository`）。
//! - 请求体保持 `serde_json::Value`（而非强类型）：校验与错误码与基线
//!   `commitCharacterIdentities` 逐条对齐（同 roster commit 的 `normalize_request` 模式）。
//! - `payload_hash = sha256(请求规范 JSON)` 仅作本栈幂等重放判定，
//!   不要求与 Node 逐字节一致（双栈隔离约定）。
//!
//! 投影刷新（`refreshCharacterIdentityProjection` 等）写 `character_roster_meta`
//! 与 `project_core`，属角色名单仓储域，留在 `character_roster_repository`。

use std::collections::HashSet;
use std::sync::LazyLock;

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::project_access::random_uuid_v4;

/// 静态身份字段白名单（对齐基线 `allowed` 集合 / `CharacterStaticFields`）。
const ALLOWED_FIELDS: [&str; 11] = [
    "name",
    "role",
    "gender",
    "age",
    "appearance",
    "personality",
    "background",
    "abilities",
    "motivation",
    "arc",
    "notes",
];

/// 泛化旧数据提案的 `source_key` 形状：仅旧快照关系与旧蓝图角色提案可在此解析，
/// 不可改写不可变的定稿快照或版本化提案信封。
static LEGACY_PROPOSAL_SOURCE_KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^legacy:(?:characters:\d+:relationships:\d+|blueprints:\d+:characters:\d+)$",
    )
    .expect("legacy proposal source key regex is valid")
});

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ===== 共享类型（对齐 `src/shared/character-identity.ts` / `source-ref.ts`） =====

/// 来源引用（对齐 `SourceRef`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub project_id: String,
    pub epoch: String,
    pub source_id: String,
    pub revision: i64,
    /// 未修改 UTF-8 字节的 SHA-256；永不归一化作者正文。
    pub content_hash: String,
    /// 编辑器偏移（UTF-16 码元，end 排他）。
    pub span: Option<SourceSpan>,
}

/// 来源引用跨度（对齐 `SourceRef.span`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpan {
    pub start: i64,
    pub end: i64,
    pub unit: String,
}

/// 角色静态来源（对齐 `CharacterStaticProvenance`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CharacterStaticProvenance {
    Legacy {
        #[serde(rename = "sourceKey")]
        source_key: String,
    },
    Author {
        source: SourceRef,
    },
    Generated {
        source: SourceRef,
        #[serde(rename = "modelRevision")]
        model_revision: String,
    },
    Derived {
        source: SourceRef,
        #[serde(rename = "modelRevision")]
        model_revision: String,
    },
}

/// 身份提交动作（对齐 `CharacterApproval.action`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterIdentityAction {
    AuthorEdit,
    AdoptGenerated,
    AdoptImport,
    ConfirmIdentity,
}

/// 主进程显式作者动作收据（对齐 `CharacterApproval`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterApproval {
    pub operation_id: String,
    pub expected_revision: i64,
    pub source: CharacterStaticProvenance,
    /// 主进程显式作者动作，永不从生成文本推断。
    pub action: CharacterIdentityAction,
}

/// 本次提交新建的角色（对齐收据 `created` 项）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityCreated {
    pub selection_key: String,
    pub character_id: String,
}

/// 身份提交收据（对齐 `CharacterIdentityCommitReceipt`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityCommitReceipt {
    pub approval: CharacterApproval,
    pub operation_id: String,
    pub revision: i64,
    pub created: Vec<CharacterIdentityCreated>,
    pub idempotent: bool,
}

// ===== 快照行类型（readCharacterIdentitySnapshot 的原始行投影） =====

/// 身份域快照行集合（对齐基线 `readCharacterIdentitySnapshot` 的返回形状）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentitySnapshotRows {
    pub revision: i64,
    pub characters: Vec<CharacterIdentityCharacterRow>,
    pub aliases: Vec<CharacterIdentityAliasRow>,
    pub relationships: Vec<CharacterIdentityRelationshipRow>,
    pub proposals: Vec<CharacterIdentityProposalRow>,
}

/// `characters` 行（身份域投影：静态字段 + 静态来源 + 身份水位）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityCharacterRow {
    pub character_id: String,
    pub name: String,
    pub role: Option<String>,
    pub gender: Option<String>,
    pub age: Option<String>,
    pub appearance: Option<String>,
    pub personality: Option<String>,
    pub background: Option<String>,
    pub abilities: Option<String>,
    pub motivation: Option<String>,
    pub arc: Option<String>,
    pub notes: Option<String>,
    /// 静态来源 JSON（`CharacterStaticProvenance` 的序列化形态）。
    pub static_provenance: String,
    pub identity_revision: i64,
    pub retired: bool,
}

/// `character_aliases` 行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityAliasRow {
    pub character_id: String,
    pub name: String,
    /// 基线约定：写入时携带审批 operationId。
    pub source_key: String,
    /// 基线约定：写入时携带身份 revision。
    pub valid_from: i64,
    pub valid_through: Option<i64>,
}

/// `character_relationships` 行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityRelationshipRow {
    pub relationship_id: String,
    pub source_character_id: String,
    pub target_character_id: String,
    pub relation: String,
    pub source_display_snapshot: String,
    pub target_display_snapshot: String,
    /// 归因来源 JSON（含 `approvalId`）。
    pub provenance_json: String,
    pub approval_id: String,
}

/// `character_identity_proposals` 行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIdentityProposalRow {
    pub proposal_id: String,
    pub owner_character_id: Option<String>,
    pub source_key: String,
    pub source_hash: String,
    pub raw_value: String,
    pub candidate_ids_json: String,
    pub resolved_character_id: Option<String>,
    pub approval_id: Option<String>,
}

/// 事务内读取的角色卡投影（基线 `active()` 的使用面）。
struct ActiveCharacter {
    name: String,
    static_provenance: String,
}

/// 提案读取投影（resolution 校验面）。
struct IdentityProposalRef {
    resolved_character_id: Option<String>,
    owner_character_id: Option<String>,
    raw_value: String,
    source_key: String,
}

// ===== 入参读取辅助（对齐基线的动态校验） =====

fn text_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// 基线 `nonempty`：字符串且 trim 后非空。
fn nonempty_text(value: &Value, key: &str) -> bool {
    text_field(value, key).is_some_and(|text| !text.trim().is_empty())
}

fn integer_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

/// 基线 `isContentHash`：`/^[a-f0-9]{64}$/`。
fn is_content_hash(value: Option<&str>) -> bool {
    value.is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// 基线 `validateFields`：键必须在白名单、值为字符串、name 非空。
fn validate_identity_fields(fields: &Value) -> Result<(), String> {
    let entries = fields
        .as_object()
        .ok_or_else(|| "CHARACTER_FIELDS_INVALID".to_string())?;
    for (key, value) in entries {
        if !ALLOWED_FIELDS.contains(&key.as_str())
            || !value.is_string()
            || (key == "name" && value.as_str().unwrap_or_default().trim().is_empty())
        {
            return Err("CHARACTER_FIELDS_INVALID".to_string());
        }
    }
    Ok(())
}

/// 基线 `unique`：无空值且无重复。
fn unique_ids(ids: &[String]) -> Result<(), String> {
    if ids.iter().any(|id| id.is_empty()) || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err("CHARACTER_ID_DUPLICATE".to_string());
    }
    Ok(())
}

/// 基线 `active`：取未退役角色卡；不存在或已退役 → `CHARACTER_ID_UNKNOWN`。
fn active_character(tx: &Transaction<'_>, character_id: &str) -> Result<ActiveCharacter, String> {
    tx.query_row(
        "SELECT name,static_provenance FROM characters WHERE character_id=?1 AND retired=0",
        [character_id],
        |row| {
            Ok(ActiveCharacter {
                name: row.get(0)?,
                static_provenance: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "CHARACTER_ID_UNKNOWN".to_string())
}

/// 基线 `endpoint`：id 与 selectionKey 恰有其一；id 优先，selectionKey 查本次创建。
fn relationship_endpoint(
    relationship: &Value,
    id_key: &str,
    selection_key: &str,
    created: &[CharacterIdentityCreated],
) -> Result<String, String> {
    let id = text_field(relationship, id_key);
    let selection = text_field(relationship, selection_key);
    // 基线 `Boolean(id) === Boolean(selectionKey)`：空串与缺失同等视之。
    let has_id = id.is_some_and(|value| !value.is_empty());
    let has_selection = selection.is_some_and(|value| !value.is_empty());
    if has_id == has_selection {
        return Err("CHARACTER_RELATIONSHIP_ID_REQUIRED".to_string());
    }
    match id {
        // 基线 `id ?? …`：空串 id 不是 nullish，仍落入 `!resolved` 拒绝。
        Some(id) if id.is_empty() => Err("CHARACTER_RELATIONSHIP_ID_REQUIRED".to_string()),
        Some(id) => Ok(id.to_string()),
        None => created
            .iter()
            .find(|item| item.selection_key == selection.unwrap_or_default())
            .map(|item| item.character_id.clone())
            .ok_or_else(|| "CHARACTER_RELATIONSHIP_ID_REQUIRED".to_string()),
    }
}

/// 基线 `hasCharacterIdentitySchema`：m02 六表是否已就位。
pub fn has_character_identity_schema(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='character_identity_meta'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|row| row.is_some())
    .unwrap_or(false)
}

/// 基线 `commitCharacterIdentities` —— 角色身份的唯一提交 seam。
///
/// 外部输入保持 `serde_json::Value`（即 `CharacterIdentityCommitRequest` 的序列化
/// 形态）：校验与错误码与基线逐条对齐。`authorize` 对齐基线的主进程授权回调
/// （命令层在调用前完成等价授权校验）。
pub fn commit_character_identities(
    conn: &Connection,
    request: &Value,
    authorize: impl Fn(&Value) -> bool,
) -> Result<CharacterIdentityCommitReceipt, String> {
    if !has_character_identity_schema(conn) {
        return Err("CHARACTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let encoded = request.to_string();
    let digest = sha256_hex(&encoded);
    // 基线把整个提交包在单个事务里；错误提前返回时 `tx` drop 即回滚。
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let receipt = commit_in_transaction(&tx, request, &encoded, &digest, authorize)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(receipt)
}

/// 事务体内的提交逻辑。错误提前返回时 `tx` drop 即回滚，与基线
/// `db.transaction()` 语义一致。
fn commit_in_transaction(
    tx: &Transaction<'_>,
    request: &Value,
    encoded: &str,
    digest: &str,
    authorize: impl Fn(&Value) -> bool,
) -> Result<CharacterIdentityCommitReceipt, String> {
    if !authorize(request) {
        return Err("CHARACTER_APPROVAL_REQUIRED".to_string());
    }
    // 基线在授权回调后重序列化请求以检测回调内的篡改；Rust 的不可变借用使该
    // 防护恒真，保留检查点以对齐上游语义。
    if request.to_string() != encoded {
        return Err("CHARACTER_APPROVAL_MUTATED".to_string());
    }

    let approval = &request["approval"];
    let operation_id = text_field(approval, "operationId").unwrap_or_default();

    // 幂等重放：同一 operationId 且请求未变，直接返回历史收据。
    let previous: Option<(String, String)> = tx
        .query_row(
            "SELECT payload_hash,receipt_json FROM character_identity_approvals WHERE operation_id=?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((previous_hash, receipt_json)) = previous {
        if previous_hash != digest {
            return Err("CHARACTER_APPROVAL_NONCE_CONFLICT".to_string());
        }
        let mut receipt: CharacterIdentityCommitReceipt =
            serde_json::from_str(&receipt_json).map_err(|error| error.to_string())?;
        receipt.idempotent = true;
        return Ok(receipt);
    }

    let current: i64 = tx
        .query_row(
            "SELECT revision FROM character_identity_meta WHERE id='main'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if integer_field(approval, "expectedRevision") != Some(current) {
        return Err("CHARACTER_ID_REVISION_CONFLICT".to_string());
    }

    // 审批结构校验：operationId / source 非空、action 在白名单内。
    let action = text_field(approval, "action").unwrap_or_default();
    if operation_id.is_empty()
        || !approval["source"].is_object()
        || !matches!(
            action,
            "author-edit" | "adopt-generated" | "adopt-import" | "confirm-identity"
        )
    {
        return Err("CHARACTER_APPROVAL_INVALID".to_string());
    }

    // 来源形态校验：action 决定允许的 provenance kind（legacy 永不合法）。
    let source = &approval["source"];
    let kind = text_field(source, "kind").unwrap_or_default();
    let allowed_kinds: &[&str] = match action {
        "author-edit" => &["author"],
        "adopt-generated" => &["generated"],
        "adopt-import" => &["generated", "derived"],
        "confirm-identity" => &["author"],
        _ => &[],
    };
    if kind == "legacy" || !allowed_kinds.contains(&kind) {
        return Err("CHARACTER_PROVENANCE_INVALID".to_string());
    }
    // 引用完整性：projectId/epoch/sourceId 非空、revision 为 ≥0 的安全整数、
    // contentHash 为 SHA-256 形态；generated/derived 另要求 modelRevision 同形。
    let reference = &source["source"];
    if !nonempty_text(reference, "projectId")
        || !nonempty_text(reference, "epoch")
        || !nonempty_text(reference, "sourceId")
        || !integer_field(reference, "revision")
            .is_some_and(|revision| (0..=9007199254740991).contains(&revision))
        || !is_content_hash(text_field(reference, "contentHash"))
        || (matches!(kind, "generated" | "derived")
            && !is_content_hash(text_field(source, "modelRevision")))
    {
        return Err("CHARACTER_PROVENANCE_INVALID".to_string());
    }

    let revision = current + 1;
    // 归因来源：把 approvalId 合并进静态来源，随字段级 provenance 与关系落库。
    let mut attributed_source = source.clone();
    attributed_source["approvalId"] = Value::from(operation_id);
    let source_json = attributed_source.to_string();

    if request["resolutions"]
        .as_array()
        .is_some_and(|resolutions| !resolutions.is_empty())
        && action != "confirm-identity"
    {
        return Err("CHARACTER_ID_CONFIRMATION_REQUIRED".to_string());
    }

    // 三类 ID 的唯一性前置校验（基线在事务开头一次性完成）。
    let changes: Vec<(String, Value)> = request["changes"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        text_field(item, "characterId")
                            .unwrap_or_default()
                            .to_string(),
                        item["fields"].clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let creations: Vec<Value> = request["creations"].as_array().cloned().unwrap_or_default();
    let retire_ids: Vec<String> = request["retireIds"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    let change_ids: Vec<String> = changes.iter().map(|(id, _)| id.clone()).collect();
    let creation_keys: Vec<String> = creations
        .iter()
        .map(|creation| {
            text_field(creation, "selectionKey")
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    unique_ids(&change_ids)?;
    unique_ids(&creation_keys)?;
    unique_ids(&retire_ids)?;
    if retire_ids.iter().any(|id| change_ids.contains(id)) {
        return Err("CHARACTER_ID_CONFLICT".to_string());
    }

    let mut created: Vec<CharacterIdentityCreated> = Vec::new();
    for creation in &creations {
        let fields = &creation["fields"];
        validate_identity_fields(fields)?;
        let name = text_field(fields, "name")
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "CHARACTER_FIELDS_INVALID".to_string())?;
        let character_id = random_uuid_v4();
        let keys: Vec<String> = fields
            .as_object()
            .map(|entries| entries.keys().cloned().collect())
            .unwrap_or_default();
        // 键已通过白名单校验，拼接列名无注入面。
        let columns = keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; keys.len()].join(",");
        let sql = format!(
            "INSERT INTO characters(character_id,static_provenance,identity_revision,{columns}) VALUES(?,?,?,{placeholders})"
        );
        let field_values: Vec<String> = keys
            .iter()
            .map(|key| {
                fields[key.as_str()]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        let mut bindings: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(keys.len() + 3);
        bindings.push(&character_id);
        bindings.push(&source_json);
        bindings.push(&revision);
        bindings.extend(
            field_values
                .iter()
                .map(|value| value as &dyn rusqlite::ToSql),
        );
        tx.execute(&sql, bindings.as_slice())
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO character_aliases VALUES(?,?,?,?,NULL)",
            rusqlite::params![character_id, name, operation_id, revision],
        )
        .map_err(|error| error.to_string())?;
        created.push(CharacterIdentityCreated {
            selection_key: text_field(creation, "selectionKey")
                .unwrap_or_default()
                .to_string(),
            character_id,
        });
    }

    for (change_character_id, fields) in &changes {
        let row = active_character(tx, change_character_id)?;
        validate_identity_fields(fields)?;
        let keys: Vec<String> = fields
            .as_object()
            .map(|entries| entries.keys().cloned().collect())
            .unwrap_or_default();
        if !keys.is_empty() {
            // 字段级来源合并：采用生成文本永不重标记未触碰的作者字段；
            // 基线 `{ ...prior }` 展开：prior 非对象时静默降级为空对象。
            let mut provenance: serde_json::Map<String, Value> =
                serde_json::from_str::<Value>(&row.static_provenance)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
            let mut merged_fields: serde_json::Map<String, Value> = provenance
                .get("fields")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for key in &keys {
                merged_fields.insert(key.clone(), attributed_source.clone());
            }
            provenance.insert("fields".to_string(), Value::Object(merged_fields));
            let assignments = keys
                .iter()
                .map(|key| format!("{key}=?"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "UPDATE characters SET {assignments},static_provenance=?,identity_revision=? WHERE character_id=?"
            );
            let field_values: Vec<String> = keys
                .iter()
                .map(|key| {
                    fields[key.as_str()]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                })
                .collect();
            let provenance_json = Value::Object(provenance).to_string();
            let mut bindings: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(keys.len() + 3);
            bindings.extend(
                field_values
                    .iter()
                    .map(|value| value as &dyn rusqlite::ToSql),
            );
            bindings.push(&provenance_json);
            bindings.push(&revision);
            bindings.push(change_character_id);
            tx.execute(&sql, bindings.as_slice())
                .map_err(|error| error.to_string())?;
        }
        // 改名：当前别名封存（valid_through=当前 revision），新别名自本 revision 生效。
        if let Some(new_name) = text_field(fields, "name") {
            if new_name != row.name {
                tx.execute(
                    "UPDATE character_aliases SET valid_through=? WHERE character_id=? AND valid_through IS NULL",
                    rusqlite::params![current, change_character_id],
                )
                .map_err(|error| error.to_string())?;
                tx.execute(
                    "INSERT INTO character_aliases VALUES(?,?,?,?,NULL)",
                    rusqlite::params![change_character_id, new_name, operation_id, revision],
                )
                .map_err(|error| error.to_string())?;
            }
        }
    }

    for retire_id in &retire_ids {
        active_character(tx, retire_id)?;
        tx.execute(
            "UPDATE characters SET retired=1,identity_revision=? WHERE character_id=?",
            rusqlite::params![revision, retire_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "UPDATE character_aliases SET valid_through=? WHERE character_id=? AND valid_through IS NULL",
            rusqlite::params![current, retire_id],
        )
        .map_err(|error| error.to_string())?;
    }

    let receipt = CharacterIdentityCommitReceipt {
        approval: serde_json::from_value(request["approval"].clone())
            .map_err(|error| error.to_string())?,
        operation_id: operation_id.to_string(),
        revision,
        created: created.clone(),
        idempotent: false,
    };
    let receipt_json = serde_json::to_string(&receipt).map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO character_identity_approvals VALUES(?,?,?)",
        rusqlite::params![operation_id, digest, receipt_json],
    )
    .map_err(|error| error.to_string())?;

    let mut relation_keys: HashSet<String> = HashSet::new();
    for relationship in request["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let from_id = relationship_endpoint(
            &relationship,
            "sourceCharacterId",
            "sourceSelectionKey",
            &created,
        )?;
        let to_id = relationship_endpoint(
            &relationship,
            "targetCharacterId",
            "targetSelectionKey",
            &created,
        )?;
        let from = active_character(tx, &from_id)?;
        let to = active_character(tx, &to_id)?;
        let relation = text_field(&relationship, "relation").unwrap_or_default();
        let relation_key = serde_json::json!([from_id, to_id, relation]).to_string();
        if relation.trim().is_empty() || from_id == to_id || relation_keys.contains(&relation_key) {
            return Err("CHARACTER_RELATIONSHIP_INVALID".to_string());
        }
        relation_keys.insert(relation_key);
        tx.execute(
            "INSERT INTO character_relationships VALUES(?,?,?,?,?,?,?,?)",
            rusqlite::params![
                random_uuid_v4(),
                from_id,
                to_id,
                relation,
                from.name,
                to.name,
                source_json,
                operation_id
            ],
        )
        .map_err(|error| error.to_string())?;
    }

    for resolution in request["resolutions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let target_character_id = text_field(&resolution, "characterId").unwrap_or_default();
        let proposal_id = text_field(&resolution, "proposalId").unwrap_or_default();
        active_character(tx, target_character_id)?;
        let proposal: Option<IdentityProposalRef> = tx
            .query_row(
                "SELECT resolved_character_id,owner_character_id,raw_value,source_key FROM character_identity_proposals WHERE proposal_id=?1",
                [proposal_id],
                |row| {
                    Ok(IdentityProposalRef {
                        resolved_character_id: row.get(0)?,
                        owner_character_id: row.get(1)?,
                        raw_value: row.get(2)?,
                        source_key: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let proposal = proposal.ok_or_else(|| "CHARACTER_PROPOSAL_CONFLICT".to_string())?;
        if proposal.resolved_character_id.is_some() {
            return Err("CHARACTER_PROPOSAL_CONFLICT".to_string());
        }
        if !LEGACY_PROPOSAL_SOURCE_KEY.is_match(&proposal.source_key) {
            return Err("CHARACTER_PROPOSAL_KIND_INVALID".to_string());
        }
        tx.execute(
            "UPDATE character_identity_proposals SET resolved_character_id=?,approval_id=? WHERE proposal_id=?",
            rusqlite::params![target_character_id, operation_id, proposal_id],
        )
        .map_err(|error| error.to_string())?;
        if let Some(owner_character_id) = proposal.owner_character_id {
            // raw_value 解析出非空 relation 时，同步建立 owner → 目标 的关系。
            let relation: Option<String> = serde_json::from_str::<Value>(&proposal.raw_value)
                .ok()
                .and_then(|parsed| {
                    parsed
                        .get("relation")
                        .and_then(Value::as_str)
                        .filter(|relation| !relation.trim().is_empty())
                        .map(str::to_string)
                });
            if let Some(relation) = relation {
                // 基线先校验两端 active，再拒绝自环（错误优先级对齐）。
                let from = active_character(tx, &owner_character_id)?;
                let to = active_character(tx, target_character_id)?;
                if owner_character_id == target_character_id {
                    return Err("CHARACTER_RELATIONSHIP_INVALID".to_string());
                }
                tx.execute(
                    "INSERT INTO character_relationships VALUES(?,?,?,?,?,?,?,?)",
                    rusqlite::params![
                        random_uuid_v4(),
                        owner_character_id,
                        target_character_id,
                        relation,
                        from.name,
                        to.name,
                        source_json,
                        operation_id
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
        }
    }

    tx.execute(
        "UPDATE character_identity_meta SET revision=? WHERE id='main'",
        rusqlite::params![revision],
    )
    .map_err(|error| error.to_string())?;
    Ok(receipt)
}

/// 基线 `readCharacterIdentitySnapshot` —— 读取身份域原始行快照。
///
/// 返回原始行（对齐基线 `SELECT *` 的列投影）；服务层再组装为
/// `CharacterIdentitySnapshot`（`character-identity:read` 契约）。
pub fn read_character_identity_snapshot(
    conn: &Connection,
) -> Result<CharacterIdentitySnapshotRows, String> {
    if !has_character_identity_schema(conn) {
        return Err("CHARACTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let revision = conn
        .query_row(
            "SELECT revision FROM character_identity_meta WHERE id='main'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let characters = conn
        .prepare("SELECT character_id,name,role,gender,age,appearance,personality,background,abilities,motivation,arc,notes,static_provenance,identity_revision,retired FROM characters ORDER BY character_id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(CharacterIdentityCharacterRow {
                character_id: row.get(0)?,
                name: row.get(1)?,
                role: row.get(2)?,
                gender: row.get(3)?,
                age: row.get(4)?,
                appearance: row.get(5)?,
                personality: row.get(6)?,
                background: row.get(7)?,
                abilities: row.get(8)?,
                motivation: row.get(9)?,
                arc: row.get(10)?,
                notes: row.get(11)?,
                static_provenance: row.get(12)?,
                identity_revision: row.get(13)?,
                retired: row.get::<_, i64>(14)? != 0,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let aliases = conn
        .prepare(
            "SELECT character_id,name,source_key,valid_from,valid_through FROM character_aliases ORDER BY character_id,name,valid_from",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(CharacterIdentityAliasRow {
                character_id: row.get(0)?,
                name: row.get(1)?,
                source_key: row.get(2)?,
                valid_from: row.get(3)?,
                valid_through: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let relationships = conn
        .prepare("SELECT relationship_id,source_character_id,target_character_id,relation,source_display_snapshot,target_display_snapshot,provenance_json,approval_id FROM character_relationships ORDER BY relationship_id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(CharacterIdentityRelationshipRow {
                relationship_id: row.get(0)?,
                source_character_id: row.get(1)?,
                target_character_id: row.get(2)?,
                relation: row.get(3)?,
                source_display_snapshot: row.get(4)?,
                target_display_snapshot: row.get(5)?,
                provenance_json: row.get(6)?,
                approval_id: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let proposals = conn
        .prepare("SELECT proposal_id,owner_character_id,source_key,source_hash,raw_value,candidate_ids_json,resolved_character_id,approval_id FROM character_identity_proposals ORDER BY proposal_id")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(CharacterIdentityProposalRow {
                proposal_id: row.get(0)?,
                owner_character_id: row.get(1)?,
                source_key: row.get(2)?,
                source_hash: row.get(3)?,
                raw_value: row.get(4)?,
                candidate_ids_json: row.get(5)?,
                resolved_character_id: row.get(6)?,
                approval_id: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(CharacterIdentitySnapshotRows {
        revision,
        characters,
        aliases,
        relationships,
        proposals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// m02 形态的 characters 表 + 六表 DDL（逐字取自
    /// `electron/migrations/m02-character-identity.ts` 的 `M02_CHARACTER_AUXILIARY_SQL`）。
    fn setup_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE characters (
              character_id TEXT PRIMARY KEY, legacy_key TEXT,
              static_provenance TEXT NOT NULL DEFAULT '{}',
              identity_revision INTEGER NOT NULL DEFAULT 0,
              retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0,1)),
              name TEXT, role TEXT DEFAULT 'supporting', gender TEXT DEFAULT '',
              age TEXT DEFAULT '', appearance TEXT DEFAULT '', personality TEXT DEFAULT '',
              background TEXT DEFAULT '', abilities TEXT DEFAULT '', motivation TEXT DEFAULT '',
              relationships TEXT DEFAULT '', arc TEXT DEFAULT '', notes TEXT DEFAULT ''
            );
            CREATE TABLE character_identity_origins (
              character_id TEXT PRIMARY KEY REFERENCES characters(character_id), source_key TEXT NOT NULL UNIQUE,
              original_row_json TEXT NOT NULL, original_hash TEXT NOT NULL
            );
            CREATE TABLE character_aliases (
              character_id TEXT NOT NULL REFERENCES characters(character_id), name TEXT NOT NULL,
              source_key TEXT NOT NULL, valid_from INTEGER NOT NULL, valid_through INTEGER,
              PRIMARY KEY(character_id,name,source_key,valid_from)
            );
            CREATE TABLE character_identity_proposals (
              proposal_id TEXT PRIMARY KEY, owner_character_id TEXT REFERENCES characters(character_id),
              source_key TEXT NOT NULL, source_hash TEXT NOT NULL, raw_value TEXT NOT NULL, candidate_ids_json TEXT NOT NULL,
              resolved_character_id TEXT REFERENCES characters(character_id), approval_id TEXT REFERENCES character_identity_approvals(operation_id)
            );
            CREATE TABLE character_identity_approvals (
              operation_id TEXT PRIMARY KEY, payload_hash TEXT NOT NULL, receipt_json TEXT NOT NULL
            );
            CREATE TABLE character_relationships (
              relationship_id TEXT PRIMARY KEY,
              source_character_id TEXT NOT NULL REFERENCES characters(character_id),
              target_character_id TEXT NOT NULL REFERENCES characters(character_id),
              relation TEXT NOT NULL, source_display_snapshot TEXT NOT NULL, target_display_snapshot TEXT NOT NULL,
              provenance_json TEXT NOT NULL, approval_id TEXT NOT NULL REFERENCES character_identity_approvals(operation_id)
            );
            CREATE TABLE character_identity_meta (
              id TEXT PRIMARY KEY CHECK(id='main'), revision INTEGER NOT NULL, legacy_count INTEGER NOT NULL
            );
            INSERT INTO character_identity_meta VALUES('main',0,0);
            "#,
        )
        .unwrap();
        conn
    }

    fn author_source() -> Value {
        json!({
            "kind": "author",
            "source": {
                "projectId": "proj-1",
                "epoch": "epoch-1",
                "sourceId": "src-1",
                "revision": 0,
                "contentHash": "a".repeat(64)
            }
        })
    }

    fn sample_request(operation_id: &str, expected_revision: i64) -> Value {
        json!({
            "approval": {
                "operationId": operation_id,
                "expectedRevision": expected_revision,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [
                { "selectionKey": "sel-1", "fields": { "name": "Alice", "role": "protagonist" } }
            ],
            "retireIds": [],
            "relationships": [],
            "resolutions": []
        })
    }

    #[test]
    fn commit_requires_identity_schema() {
        let conn = Connection::open_in_memory().unwrap();
        let error = commit_character_identities(&conn, &sample_request("op-1", 0), |_| true)
            .expect_err("missing schema must be rejected");
        assert_eq!(error, "CHARACTER_ID_SCHEMA_REQUIRED");
        assert!(read_character_identity_snapshot(&conn).is_err());
    }

    #[test]
    fn commit_requires_authorization() {
        let conn = setup_connection();
        let error = commit_character_identities(&conn, &sample_request("op-1", 0), |_| false)
            .expect_err("unauthorized commit must be rejected");
        assert_eq!(error, "CHARACTER_APPROVAL_REQUIRED");
    }

    #[test]
    fn commit_rejects_revision_conflict() {
        let conn = setup_connection();
        let error = commit_character_identities(&conn, &sample_request("op-1", 7), |_| true)
            .expect_err("stale revision must be rejected");
        assert_eq!(error, "CHARACTER_ID_REVISION_CONFLICT");
    }

    #[test]
    fn commit_creates_character_and_replays_idempotently() {
        let conn = setup_connection();
        let request = sample_request("op-1", 0);
        let receipt = commit_character_identities(&conn, &request, |_| true).unwrap();
        assert_eq!(receipt.revision, 1);
        assert_eq!(receipt.operation_id, "op-1");
        assert!(!receipt.idempotent);
        assert_eq!(receipt.created.len(), 1);
        assert_eq!(receipt.created[0].selection_key, "sel-1");
        assert!(!receipt.created[0].character_id.is_empty());

        // 幂等重放：同一请求返回历史收据且不再推进 revision。
        let replay = commit_character_identities(&conn, &request, |_| true).unwrap();
        assert!(replay.idempotent);
        assert_eq!(
            replay.created[0].character_id,
            receipt.created[0].character_id
        );

        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.characters.len(), 1);
        assert_eq!(snapshot.characters[0].name, "Alice");
        assert_eq!(snapshot.characters[0].role.as_deref(), Some("protagonist"));
        assert_eq!(snapshot.characters[0].identity_revision, 1);
        assert!(!snapshot.characters[0].retired);
        // 落库的静态来源携带归因 approvalId。
        let provenance: Value =
            serde_json::from_str(&snapshot.characters[0].static_provenance).unwrap();
        assert_eq!(provenance["kind"], json!("author"));
        assert_eq!(provenance["approvalId"], json!("op-1"));
        assert_eq!(snapshot.aliases.len(), 1);
        assert_eq!(snapshot.aliases[0].name, "Alice");
        assert_eq!(snapshot.aliases[0].source_key, "op-1");
        assert_eq!(snapshot.aliases[0].valid_from, 1);
        assert_eq!(snapshot.aliases[0].valid_through, None);
    }

    #[test]
    fn commit_rotates_alias_on_rename_and_retires_character() {
        let conn = setup_connection();
        let created = commit_character_identities(&conn, &sample_request("op-1", 0), |_| true)
            .unwrap()
            .created[0]
            .character_id
            .clone();

        // 改名：旧别名封存，新别名自新 revision 生效。
        let rename = json!({
            "approval": {
                "operationId": "op-2",
                "expectedRevision": 1,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [ { "characterId": created, "fields": { "name": "Alicia" } } ],
            "creations": [],
            "retireIds": [],
            "relationships": [],
            "resolutions": []
        });
        commit_character_identities(&conn, &rename, |_| true).unwrap();
        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert_eq!(snapshot.revision, 2);
        assert_eq!(snapshot.characters[0].name, "Alicia");
        assert_eq!(snapshot.characters[0].identity_revision, 2);
        assert_eq!(snapshot.aliases.len(), 2);
        let sealed = snapshot
            .aliases
            .iter()
            .find(|alias| alias.name == "Alice")
            .unwrap();
        assert_eq!(sealed.valid_through, Some(1));
        let current_alias = snapshot
            .aliases
            .iter()
            .find(|alias| alias.name == "Alicia")
            .unwrap();
        assert_eq!(current_alias.valid_from, 2);
        assert_eq!(current_alias.valid_through, None);

        // 退役：角色卡 retired=1，当前别名封存。
        let retire = json!({
            "approval": {
                "operationId": "op-3",
                "expectedRevision": 2,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [],
            "retireIds": [ created ],
            "relationships": [],
            "resolutions": []
        });
        commit_character_identities(&conn, &retire, |_| true).unwrap();
        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert_eq!(snapshot.revision, 3);
        assert!(snapshot.characters[0].retired);
        let sealed_first = snapshot
            .aliases
            .iter()
            .find(|alias| alias.name == "Alice")
            .unwrap();
        assert_eq!(sealed_first.valid_through, Some(1));
        let sealed_second = snapshot
            .aliases
            .iter()
            .find(|alias| alias.name == "Alicia")
            .unwrap();
        assert_eq!(sealed_second.valid_through, Some(2));
    }

    #[test]
    fn commit_inserts_relationships_by_selection_key() {
        let conn = setup_connection();
        let request = json!({
            "approval": {
                "operationId": "op-1",
                "expectedRevision": 0,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [
                { "selectionKey": "sel-1", "fields": { "name": "Alice" } },
                { "selectionKey": "sel-2", "fields": { "name": "Bob" } }
            ],
            "retireIds": [],
            "relationships": [
                { "sourceSelectionKey": "sel-1", "targetSelectionKey": "sel-2", "relation": "sibling" }
            ],
            "resolutions": []
        });
        commit_character_identities(&conn, &request, |_| true).unwrap();
        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert_eq!(snapshot.relationships.len(), 1);
        let relationship = &snapshot.relationships[0];
        assert_eq!(relationship.relation, "sibling");
        assert_eq!(relationship.source_display_snapshot, "Alice");
        assert_eq!(relationship.target_display_snapshot, "Bob");
        assert_eq!(relationship.approval_id, "op-1");
        assert!(!relationship.relationship_id.is_empty());
        assert!(!relationship.provenance_json.is_empty());
    }

    #[test]
    fn commit_rejects_duplicate_relationship_and_rolls_back() {
        let conn = setup_connection();
        let request = json!({
            "approval": {
                "operationId": "op-1",
                "expectedRevision": 0,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [
                { "selectionKey": "sel-1", "fields": { "name": "Alice" } },
                { "selectionKey": "sel-2", "fields": { "name": "Bob" } }
            ],
            "retireIds": [],
            "relationships": [
                { "sourceSelectionKey": "sel-1", "targetSelectionKey": "sel-2", "relation": "sibling" },
                { "sourceSelectionKey": "sel-1", "targetSelectionKey": "sel-2", "relation": "sibling" }
            ],
            "resolutions": []
        });
        let error = commit_character_identities(&conn, &request, |_| true)
            .expect_err("duplicate relationship key must be rejected");
        assert_eq!(error, "CHARACTER_RELATIONSHIP_INVALID");
        // 事务回滚：关系之前的创建也一并撤销。
        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert!(snapshot.characters.is_empty());
        assert!(snapshot.relationships.is_empty());
    }

    #[test]
    fn commit_resolves_legacy_proposal_and_links_owner_relationship() {
        let conn = setup_connection();
        let alice_id = commit_character_identities(&conn, &sample_request("op-1", 0), |_| true)
            .unwrap()
            .created[0]
            .character_id
            .clone();
        let create_bob = json!({
            "approval": {
                "operationId": "op-2",
                "expectedRevision": 1,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [ { "selectionKey": "sel-2", "fields": { "name": "Bob" } } ],
            "retireIds": [],
            "relationships": [],
            "resolutions": []
        });
        let bob_id = commit_character_identities(&conn, &create_bob, |_| true)
            .unwrap()
            .created[0]
            .character_id
            .clone();

        // 直接落一个旧蓝图提案：owner 为 Alice，raw_value 携带 relation。
        conn.execute(
            "INSERT INTO character_identity_proposals VALUES(?,?,?,?,?,?,NULL,NULL)",
            rusqlite::params![
                "proposal-1",
                alice_id,
                "legacy:blueprints:1:characters:0",
                "hash",
                r#"{"relation":"mentor"}"#,
                "[]"
            ],
        )
        .unwrap();

        let resolve = json!({
            "approval": {
                "operationId": "op-3",
                "expectedRevision": 2,
                "source": author_source(),
                "action": "confirm-identity"
            },
            "changes": [],
            "creations": [],
            "retireIds": [],
            "relationships": [],
            "resolutions": [ { "proposalId": "proposal-1", "characterId": bob_id } ]
        });
        commit_character_identities(&conn, &resolve, |_| true).unwrap();
        let snapshot = read_character_identity_snapshot(&conn).unwrap();
        assert_eq!(snapshot.revision, 3);
        let proposal = snapshot
            .proposals
            .iter()
            .find(|p| p.proposal_id == "proposal-1")
            .unwrap();
        assert_eq!(
            proposal.resolved_character_id.as_deref(),
            Some(bob_id.as_str())
        );
        assert_eq!(proposal.approval_id.as_deref(), Some("op-3"));
        // owner → 目标 的关系随解析一并建立。
        let relationship = snapshot
            .relationships
            .iter()
            .find(|item| item.relation == "mentor")
            .unwrap();
        assert_eq!(relationship.source_display_snapshot, "Alice");
        assert_eq!(relationship.target_display_snapshot, "Bob");

        // 未知提案 → CONFLICT；非泛化旧数据来源 → KIND_INVALID。
        let unknown = json!({
            "approval": {
                "operationId": "op-4",
                "expectedRevision": 3,
                "source": author_source(),
                "action": "confirm-identity"
            },
            "changes": [],
            "creations": [],
            "retireIds": [],
            "relationships": [],
            "resolutions": [ { "proposalId": "proposal-404", "characterId": bob_id } ]
        });
        let error = commit_character_identities(&conn, &unknown, |_| true)
            .expect_err("unknown proposal must be rejected");
        assert_eq!(error, "CHARACTER_PROPOSAL_CONFLICT");
    }

    #[test]
    fn commit_rejects_duplicate_and_conflicting_ids() {
        let conn = setup_connection();
        let duplicate_creations = json!({
            "approval": {
                "operationId": "op-1",
                "expectedRevision": 0,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [
                { "selectionKey": "sel-1", "fields": { "name": "Alice" } },
                { "selectionKey": "sel-1", "fields": { "name": "Alicia" } }
            ],
            "retireIds": [],
            "relationships": [],
            "resolutions": []
        });
        let error = commit_character_identities(&conn, &duplicate_creations, |_| true)
            .expect_err("duplicate selectionKey must be rejected");
        assert_eq!(error, "CHARACTER_ID_DUPLICATE");

        let duplicate_fields = json!({
            "approval": {
                "operationId": "op-1",
                "expectedRevision": 0,
                "source": author_source(),
                "action": "author-edit"
            },
            "changes": [],
            "creations": [ { "selectionKey": "sel-1", "fields": { "name": "Alice", "bogus": "x" } } ],
            "retireIds": [],
            "relationships": [],
            "resolutions": []
        });
        let error = commit_character_identities(&conn, &duplicate_fields, |_| true)
            .expect_err("non-whitelisted field must be rejected");
        assert_eq!(error, "CHARACTER_FIELDS_INVALID");
    }
}
