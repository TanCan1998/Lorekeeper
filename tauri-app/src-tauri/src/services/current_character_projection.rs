//! current_character_projection —— 当前角色投影的可信来源过滤
//! （逐行平移 `electron/services/current-character-projection.ts`，101 行）。
//!
//! 从定稿链（drafts → contents → finalization_outbox →
//! summary_snapshots）重建「当前生效的定稿来源」集合
//! （`currentSources`），再据此逐行过滤 `characters` 的
//! `cs_provenance`：仅当状态字段的来源条目（author / legacy /
//! derived）通过可信判定、且对应 `cs_*` 列为字符串时，保留该列
//! 并把来源条目写入投影后的 `cs_provenance`；否则删除该列。
//!
//! 语义对齐要点：
//! - `JSON.stringify(...)` 的稳定键 → `source_key`（draftId /
//!   chapterNumber 为 JSON 数字，finalizationId / contentHash 为
//!   JSON 字符串）；
//! - `Number.isSafeInteger` → `safe_integer_value`（±2^53-1，
//!   含整数值浮点如 `1.0` 的宽容处理）；
//! - `Object.keys(v).sort().join(',')` → `sorted_keys_join`；
//! - `hash(body)` 在 Rust 侧计算（`text_hash`，UTF-8 SHA-256
//!   小写 hex），对应基线 `node:crypto` 的 `createHash('sha256')`；
//! - 基线为只读投影，无用户可见错误文案；SQL 失败按仓库惯例
//!   以中文 `format!` 上报。

use std::collections::BTreeMap;

use rusqlite::{Connection, Row};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::character_repository::{CharacterStateTextField, FinalizedSourceIdentity};
use crate::services::source_ref::is_finalized_source_identity;

/// 角色状态文本字段（对齐 `src/shared/character-roster.ts` 的
/// `CHARACTER_STATE_TEXT_FIELDS`；Rust 侧无既有等价常量，本文件定义）。
const CHARACTER_STATE_TEXT_FIELDS: [CharacterStateTextField; 6] = [
    CharacterStateTextField::Location,
    CharacterStateTextField::PowerLevel,
    CharacterStateTextField::PhysicalState,
    CharacterStateTextField::MentalState,
    CharacterStateTextField::KeyItems,
    CharacterStateTextField::RecentEvents,
];

/// 当前生效的定稿来源键 → 定稿版本与投影代次
/// （基线 `currentSources`；键为
/// `JSON.stringify([draftId, finalizationId, chapterNumber, contentHash])`）。
type CurrentSources = BTreeMap<String, CurrentSourceEntry>;

/// `currentSources` 的值（基线 `{ draftVersion, projectionGeneration }`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentSourceEntry {
    draft_version: i64,
    projection_generation: i64,
}

/// 投影水印（`continuity_projection_meta` 单行 `main`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectionWatermark {
    generation: i64,
    stale_from_chapter: Option<i64>,
}

/// 读取当前角色投影：逐行过滤 `characters` 的派生状态来源。
///
/// 返回行以 `serde_json::Map` 承载，键为 SQLite 原始列名
/// （snake_case，与基线 `SELECT *` 的行对象逐字一致）；
/// `created_at` / `updated_at` 恒剔除，不可信来源的 `cs_*` 列
/// 删除，`cs_provenance` 在有可信字段时重写为投影后的 JSON 字符串。
pub fn current_character_projection(
    conn: &Connection,
    project_id: &str,
    origin_project_id: Option<&str>,
) -> Result<Vec<Map<String, Value>>, String> {
    let watermark = load_watermark(conn)?;
    let current_sources = load_current_sources(conn, &watermark)?;
    // 基线 `[projectId, originProjectId].filter(Boolean)`：
    // continuityEpoch 允许匹配任一当前项目上下文。
    let mut current_projects: Vec<&str> = vec![project_id];
    if let Some(origin) = origin_project_id {
        current_projects.push(origin);
    }
    let mut stmt = conn
        .prepare("SELECT * FROM characters ORDER BY character_id")
        .map_err(|error| format!("读取角色表失败：{error}"))?;
    let column_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let mut rows = stmt
        .query([])
        .map_err(|error| format!("读取角色表失败：{error}"))?;
    let mut result = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("读取角色表失败：{error}"))?
    {
        let mut record = row_to_map(row, &column_names)?;
        // 基线 `without(raw, ['created_at', 'updated_at'])`。
        record.remove("created_at");
        record.remove("updated_at");
        let provenance = parse_provenance(record.get("cs_provenance").and_then(Value::as_str));
        let mut projected_provenance = Map::new();
        for field in CHARACTER_STATE_TEXT_FIELDS {
            let name = field_name(field);
            let column = state_column(field);
            let mut trusted =
                provenance
                    .get(name)
                    .and_then(Value::as_object)
                    .is_some_and(|entry| {
                        is_trusted_entry(entry, &current_sources, &current_projects, &watermark)
                    });
            // 基线末尾的 `typeof row[csColumn] === 'string'` 守卫。
            trusted = trusted && matches!(record.get(column), Some(Value::String(_)));
            if trusted {
                // 投影 provenance 存的是来源条目本身（基线
                // `projectedProvenance[field] = entry`）。
                projected_provenance
                    .insert(name.to_string(), provenance.get(name).cloned().unwrap());
            } else {
                record.remove(column);
            }
        }
        if !projected_provenance.is_empty() {
            let serialized = serde_json::to_string(&Value::Object(projected_provenance))
                .map_err(|error| format!("序列化投影 provenance 失败：{error}"))?;
            record.insert("cs_provenance".to_string(), Value::String(serialized));
        } else {
            record.remove("cs_provenance");
            record.remove("cs_updated_at_chapter");
        }
        result.push(record);
    }
    Ok(result)
}

/// 展示用字段授权（基线 `currentDerivedCharacterFields`）：
/// 复用同一投影，仅列出可信来源中 `kind === 'derived'` 的字段。
///
/// 与基线一致：每个角色都有条目（无可信字段时为空数组）。
pub fn current_derived_character_fields(
    conn: &Connection,
    project_id: &str,
    origin_project_id: Option<&str>,
) -> Result<BTreeMap<String, Vec<CharacterStateTextField>>, String> {
    let rows = current_character_projection(conn, project_id, origin_project_id)?;
    let mut result = BTreeMap::new();
    for row in rows {
        let provenance = parse_provenance(row.get("cs_provenance").and_then(Value::as_str));
        let fields = CHARACTER_STATE_TEXT_FIELDS
            .iter()
            .copied()
            .filter(|field| {
                provenance
                    .get(field_name(*field))
                    .and_then(Value::as_object)
                    .and_then(|entry| entry.get("kind"))
                    .and_then(Value::as_str)
                    == Some("derived")
            })
            .collect();
        let character_id = row
            .get("character_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        result.insert(character_id, fields);
    }
    Ok(result)
}

fn load_watermark(conn: &Connection) -> Result<ProjectionWatermark, String> {
    conn.query_row(
        "SELECT generation, stale_from_chapter \
         FROM continuity_projection_meta WHERE id = 'main'",
        [],
        |row| {
            Ok(ProjectionWatermark {
                generation: row.get("generation")?,
                stale_from_chapter: row.get("stale_from_chapter")?,
            })
        },
    )
    .map_err(|error| format!("读取投影水印失败：{error}"))
}

/// 重建当前生效的定稿来源集合（基线的大 JOIN + 行级过滤）。
///
/// 注意：基线把 `body === contentSnapshot`、`hash(body) === contentHash`、
/// `Number.isSafeInteger` 与代次区间校验放在 JS 侧过滤，本移植在
/// Rust 侧逐行执行（`text_hash` 替代 `node:crypto`）。
fn load_current_sources(
    conn: &Connection,
    watermark: &ProjectionWatermark,
) -> Result<CurrentSources, String> {
    let mut stmt = conn
        .prepare(
            "SELECT d.id AS draft_id, d.chapter_number, d.version AS draft_version, \
                    c.body, o.finalization_id, o.content_hash, o.content_snapshot, \
                    s.projection_generation \
             FROM drafts d \
             JOIN contents c ON c.id = d.content_id \
             JOIN finalization_outbox o ON o.draft_id = d.id \
             JOIN summary_snapshots s ON s.draft_id = d.id \
                    AND s.chapter_number = d.chapter_number \
                    AND s.source_finalization_id = o.finalization_id \
                    AND s.source_content_hash = o.content_hash \
             WHERE d.status = 'finalized' AND NOT EXISTS ( \
                 SELECT 1 FROM drafts newer \
                 WHERE newer.chapter_number = d.chapter_number \
                   AND newer.status = 'finalized' \
                   AND (newer.version > d.version \
                        OR (newer.version = d.version AND newer.id > d.id)) \
             )",
        )
        .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
    let mut sources = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("重建定稿来源集合失败：{error}"))?
    {
        // 基线 `typeof ... === 'string'` 守卫：非字符串行直接丢弃。
        let body: Option<String> = row
            .get("body")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let Some(body) = body else {
            continue;
        };
        let content_snapshot: Option<String> = row
            .get("content_snapshot")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let Some(content_snapshot) = content_snapshot else {
            continue;
        };
        let finalization_id: String = row
            .get("finalization_id")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let content_hash: String = row
            .get("content_hash")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let draft_id: i64 = row
            .get("draft_id")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let chapter_number: i64 = row
            .get("chapter_number")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let draft_version: i64 = row
            .get("draft_version")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let projection_generation: i64 = row
            .get("projection_generation")
            .map_err(|error| format!("重建定稿来源集合失败：{error}"))?;
        let valid = body == content_snapshot
            && text_hash(&body) == content_hash
            && safe_integer_value(Some(&Value::Number(draft_version.into())))
                .is_some_and(|version| version > 0)
            && safe_integer_value(Some(&Value::Number(projection_generation.into())))
                .is_some_and(|generation| generation >= 0 && generation <= watermark.generation);
        if !valid {
            continue;
        }
        let key = source_key(draft_id, &finalization_id, chapter_number, &content_hash);
        sources.insert(
            key,
            CurrentSourceEntry {
                draft_version,
                projection_generation,
            },
        );
    }
    Ok(sources)
}

/// 基线键：`JSON.stringify([draftId, finalizationId, chapterNumber, contentHash])`
/// （draftId / chapterNumber 为 JSON 数字）。
fn source_key(
    draft_id: i64,
    finalization_id: &str,
    chapter_number: i64,
    content_hash: &str,
) -> String {
    serde_json::to_string(&[
        Value::Number(draft_id.into()),
        Value::String(finalization_id.to_string()),
        Value::Number(chapter_number.into()),
        Value::String(content_hash.to_string()),
    ])
    .unwrap_or_default()
}

/// 基线对每个 `cs_*` provenance 条目的可信判定
/// （`entry` 须为对象；键集按 `Object.keys(v).sort().join(',')` 比对）。
fn is_trusted_entry(
    entry: &Map<String, Value>,
    current_sources: &CurrentSources,
    current_projects: &[&str],
    watermark: &ProjectionWatermark,
) -> bool {
    let keys = sorted_keys_join(entry);
    // 基线 `safeRevision`：revision 未定义，或为 ≥0 的安全整数。
    let safe_revision = entry.get("revision").map_or(true, |revision| {
        safe_integer_value(Some(revision)).is_some_and(|value| value >= 0)
    });
    match entry.get("kind").and_then(Value::as_str) {
        // author：手写来源，键集恰为 chapterNumber,kind[,revision]。
        Some("author") => {
            (keys == "chapterNumber,kind" || keys == "chapterNumber,kind,revision")
                && safe_integer_value(entry.get("chapterNumber"))
                    .is_some_and(|chapter| chapter >= 0)
                && safe_revision
        }
        // legacy：历史来源，键集恰为 kind[,revision]。
        Some("legacy") => (keys == "kind" || keys == "kind,revision") && safe_revision,
        // derived：派生来源，携带完整来源证明链。
        Some("derived") => {
            let valid_keys = keys == "kind,revision,source,sourceOrder";
            let valid_revision =
                safe_integer_value(entry.get("revision")).is_some_and(|revision| revision > 0);
            let Some(source) = entry.get("source").and_then(Value::as_object) else {
                return false;
            };
            // 基线要求 source 为对象且 finalizationId 为字符串，
            // 再经 isFinalizedSourceIdentity 校验——反序列化失败即不可信。
            let Ok(identity) =
                serde_json::from_value::<FinalizedSourceIdentity>(Value::Object(source.clone()))
            else {
                return false;
            };
            if !is_finalized_source_identity(&identity) {
                return false;
            }
            // proof 必须命中当前生效的定稿集合。
            let Some(proof) = current_sources.get(&source_key(
                identity.draft_id,
                &identity.finalization_id,
                identity.chapter_number,
                &identity.content_hash,
            )) else {
                return false;
            };
            let Some(order) = entry.get("sourceOrder").and_then(Value::as_object) else {
                return false;
            };
            // 基线 continuityEpoch 允许匹配当前项目或来源项目上下文。
            let expected_epochs: Vec<String> = current_projects
                .iter()
                .map(|project| format!("{project}:{}", proof.projection_generation))
                .collect();
            let valid_order = sorted_keys_join(order)
                == "authoritativeFinalizationRevision,chapterNumber,continuityEpoch"
                && safe_integer_value(order.get("chapterNumber")) == Some(identity.chapter_number)
                && safe_integer_value(order.get("authoritativeFinalizationRevision"))
                    == Some(proof.draft_version)
                && expected_epochs.iter().any(|epoch| {
                    order
                        .get("continuityEpoch")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == epoch)
                });
            // 基线 stale 条件：水印过期章号已设、来源章号 ≥ 过期章号、
            // 且证明代次严格落后于水印代次（三者同时成立才判过期）。
            let is_stale = watermark.stale_from_chapter.is_some_and(|stale_from| {
                identity.chapter_number >= stale_from
                    && proof.projection_generation < watermark.generation
            });
            valid_keys && valid_revision && valid_order && !is_stale
        }
        _ => false,
    }
}

fn field_name(field: CharacterStateTextField) -> &'static str {
    match field {
        CharacterStateTextField::Location => "location",
        CharacterStateTextField::PowerLevel => "powerLevel",
        CharacterStateTextField::PhysicalState => "physicalState",
        CharacterStateTextField::MentalState => "mentalState",
        CharacterStateTextField::KeyItems => "keyItems",
        CharacterStateTextField::RecentEvents => "recentEvents",
    }
}

/// 基线 `characterStateColumns`：字段名 → `cs_*` 列名。
fn state_column(field: CharacterStateTextField) -> &'static str {
    match field {
        CharacterStateTextField::Location => "cs_location",
        CharacterStateTextField::PowerLevel => "cs_power_level",
        CharacterStateTextField::PhysicalState => "cs_physical_state",
        CharacterStateTextField::MentalState => "cs_mental_state",
        CharacterStateTextField::KeyItems => "cs_key_items",
        CharacterStateTextField::RecentEvents => "cs_recent_events",
    }
}

/// 基线 `Object.keys(v).sort().join(',')`。
fn sorted_keys_join(object: &Map<String, Value>) -> String {
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    keys.join(",")
}

/// 基线 `JSON.parse(String(row.cs_provenance || '{}'))`：
/// 空串 → `{}`；解析失败或非对象（含数组）→ `{}`。
fn parse_provenance(raw: Option<&str>) -> Map<String, Value> {
    let raw = raw.unwrap_or_default();
    let effective = if raw.is_empty() { "{}" } else { raw };
    serde_json::from_str::<Value>(effective)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// 动态行 → `serde_json::Map`（键为 SQLite 原始列名，与基线
/// `SELECT *` 的行对象一致）。
fn row_to_map(row: &Row<'_>, column_names: &[String]) -> Result<Map<String, Value>, String> {
    let mut record = Map::new();
    for (index, column) in column_names.iter().enumerate() {
        let value = row
            .get_ref(index)
            .map_err(|error| format!("读取角色表失败：{error}"))?;
        record.insert(column.clone(), value_to_json(value)?);
    }
    Ok(record)
}

fn value_to_json(value: rusqlite::types::ValueRef<'_>) -> Result<Value, String> {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(number) => Ok(Value::Number(number.into())),
        ValueRef::Real(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .ok_or_else(|| "无效的浮点数值".to_string()),
        ValueRef::Text(text) => Ok(Value::String(String::from_utf8_lossy(text).into_owned())),
        ValueRef::Blob(blob) => Ok(Value::Array(
            blob.iter()
                .map(|byte| Value::Number((*byte as i64).into()))
                .collect(),
        )),
    }
}

/// 基线 `hash(text)`：UTF-8 字节 SHA-256 小写 hex
/// （`node:crypto` 的 `createHash('sha256')`）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 基线 `Number.isSafeInteger`：±2^53-1；整数值浮点（如 `1.0`）
/// 与 JS 一并视为安全整数。
fn safe_integer_value(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_number()?;
    let raw = number.as_i64().or_else(|| {
        number.as_f64().and_then(|float| {
            if float.fract() == 0.0 {
                Some(float as i64)
            } else {
                None
            }
        })
    })?;
    (-9007199254740991..=9007199254740991)
        .contains(&raw)
        .then_some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn seeded_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    /// 种入一章定稿链：contents → drafts → finalization_outbox →
    /// summary_snapshots，返回 (draft_id, finalization_id, content_hash)。
    fn seed_finalized_chapter(
        conn: &Connection,
        chapter_number: i64,
        body: &str,
        version: i64,
        projection_generation: i64,
    ) -> (i64, String, String) {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) \
             VALUES (?1, ?2, 'finalized', ?3)",
            rusqlite::params![chapter_number, version, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        let content_hash = text_hash(body);
        let finalization_id = format!("fin-{chapter_number}");
        conn.execute(
            "INSERT INTO finalization_outbox \
             (finalization_id, draft_id, chapter_number, content_hash, \
              content_revision, content_snapshot, target_file_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                finalization_id,
                draft_id,
                chapter_number,
                content_hash,
                version,
                body,
                format!("chapter-{chapter_number}.md")
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO summary_snapshots \
             (draft_id, chapter_number, source_finalization_id, \
              source_content_hash, projection_generation) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                draft_id,
                chapter_number,
                finalization_id,
                content_hash,
                projection_generation
            ],
        )
        .unwrap();
        (draft_id, finalization_id, content_hash)
    }

    fn set_watermark(conn: &Connection, generation: i64, stale_from: Option<i64>) {
        // create_tables 已播种 id='main' 行（INSERT OR IGNORE）。
        conn.execute(
            "UPDATE continuity_projection_meta SET generation = ?1, stale_from_chapter = ?2 \
             WHERE id = 'main'",
            rusqlite::params![generation, stale_from],
        )
        .unwrap();
    }

    fn insert_character(
        conn: &Connection,
        character_id: &str,
        provenance: &str,
        cs_location: Option<&str>,
        cs_recent_events: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO characters (character_id, cs_provenance, cs_location, cs_recent_events) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![character_id, provenance, cs_location, cs_recent_events],
        )
        .unwrap();
    }

    fn derived_provenance(
        draft_id: i64,
        finalization_id: &str,
        chapter_number: i64,
        content_hash: &str,
        revision: i64,
        draft_version: i64,
        projection_generation: i64,
        project: &str,
    ) -> String {
        let continuity_epoch = format!("{project}:{projection_generation}");
        format!(
            "{{\"kind\":\"derived\",\"revision\":{revision},\
\"source\":{{\"draftId\":{draft_id},\"finalizationId\":\"{finalization_id}\",\
\"chapterNumber\":{chapter_number},\"contentHash\":\"{content_hash}\"}},\
\"sourceOrder\":{{\"authoritativeFinalizationRevision\":{draft_version},\
\"chapterNumber\":{chapter_number},\"continuityEpoch\":\"{continuity_epoch}\"}}}}"
        )
    }

    fn parsed_provenance(row: &Map<String, Value>) -> Map<String, Value> {
        serde_json::from_str::<Value>(row["cs_provenance"].as_str().unwrap())
            .unwrap()
            .as_object()
            .cloned()
            .unwrap()
    }

    #[test]
    fn projection_keeps_trusted_derived_field_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 1, "正文", 3, 1);
        set_watermark(&conn, 1, None);
        let provenance = format!(
            "{{\"location\":{},\"recentEvents\":{{\"kind\":\"author\",\"chapterNumber\":-1}}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                1,
                &content_hash,
                1,
                3,
                1,
                "proj-1"
            )
        );
        insert_character(&conn, "char-1", &provenance, Some("城东"), Some("事件"));

        let rows = current_character_projection(&conn, "proj-1", None).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["character_id"], "char-1");
        assert_eq!(row["cs_location"], "城东", "可信 derived 应保留 cs 列");
        assert!(
            row.get("cs_recent_events").is_none(),
            "author 章号为负应不可信"
        );
        assert!(row.get("created_at").is_none(), "审计列应剔除");
        assert!(row.get("updated_at").is_none());
        // 投影 cs_provenance 是 JSON 字符串，且只含可信字段的来源条目。
        let projected = parsed_provenance(row);
        assert_eq!(projected.len(), 1, "仅 location 通过可信判定");
        let location = projected.get("location").unwrap().as_object().unwrap();
        assert_eq!(location["kind"], "derived");
        assert_eq!(location["revision"], 1);
    }

    #[test]
    fn projection_accepts_origin_project_epoch_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 1, "正文", 2, 1);
        set_watermark(&conn, 1, None);
        // continuityEpoch 用来源项目上下文书写。
        let provenance = format!(
            "{{\"location\":{}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                1,
                &content_hash,
                1,
                2,
                1,
                "origin-9"
            )
        );
        insert_character(&conn, "char-1", &provenance, Some("城东"), None);

        let with_origin = current_character_projection(&conn, "proj-1", Some("origin-9")).unwrap();
        assert_eq!(
            with_origin[0]["cs_location"], "城东",
            "来源项目 epoch 应匹配"
        );

        let without_origin = current_character_projection(&conn, "proj-1", None).unwrap();
        assert!(
            without_origin[0].get("cs_location").is_none(),
            "无 origin 上下文时 epoch 不匹配应不可信"
        );
    }

    #[test]
    fn projection_drops_untrusted_author_field_test() {
        let conn = seeded_db();
        set_watermark(&conn, 1, None);
        // author 来源带多余键 → 键集不合法 → 不可信
        let provenance =
            "{\"location\":{\"kind\":\"author\",\"chapterNumber\":0,\"revision\":2,\"extra\":1}}"
                .to_string();
        insert_character(&conn, "char-1", &provenance, Some("城东"), None);

        let rows = current_character_projection(&conn, "proj-1", None).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.get("cs_location").is_none(), "不可信来源应删 cs 列");
        assert!(
            row.get("cs_provenance").is_none(),
            "无可信字段应删 cs_provenance"
        );
        assert!(row.get("cs_updated_at_chapter").is_none());
        assert!(row.get("created_at").is_none(), "审计列应剔除");
        assert!(row.get("updated_at").is_none());
    }

    #[test]
    fn projection_requires_string_state_column_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 1, "正文", 1, 0);
        set_watermark(&conn, 1, None);
        let provenance = format!(
            "{{\"location\":{}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                1,
                &content_hash,
                1,
                1,
                0,
                "proj-1"
            )
        );
        // 来源可信但 cs_location 为 NULL → 基线 typeof 守卫判不可信。
        insert_character(&conn, "char-1", &provenance, None, None);

        let rows = current_character_projection(&conn, "proj-1", None).unwrap();
        assert!(rows[0].get("cs_location").is_none());
        assert!(rows[0].get("cs_provenance").is_none());
    }

    #[test]
    fn projection_marks_stale_derived_field_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 2, "正文二", 1, 0);
        // 水印代次 2、stale_from_chapter=1：第 2 章的派生来源过期
        //（章号 ≥ 过期章号且证明代次 0 < 水印代次 2）。
        set_watermark(&conn, 2, Some(1));
        let provenance = format!(
            "{{\"location\":{}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                2,
                &content_hash,
                1,
                1,
                0,
                "proj-1"
            )
        );
        insert_character(&conn, "char-1", &provenance, Some("城西"), None);

        let rows = current_character_projection(&conn, "proj-1", None).unwrap();
        let row = &rows[0];
        assert!(row.get("cs_location").is_none(), "stale 派生来源应剔除");
        assert!(row.get("cs_provenance").is_none());
    }

    #[test]
    fn derived_fields_lists_only_trusted_derived_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 1, "正文", 1, 0);
        set_watermark(&conn, 1, None);
        // location 为可信 derived；powerLevel 为可信 author（kind 非
        // derived，不应列入）；recentEvents 不可信。
        let provenance = format!(
            "{{\"location\":{},\
\"powerLevel\":{{\"kind\":\"author\",\"chapterNumber\":0,\"revision\":1}},\
\"recentEvents\":{{\"kind\":\"author\",\"chapterNumber\":-1}}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                1,
                &content_hash,
                1,
                1,
                0,
                "proj-1"
            )
        );
        insert_character(&conn, "char-1", &provenance, Some("城东"), None);
        // 无可信字段的角色也应有条目（空数组，与基线一致）。
        insert_character(
            &conn,
            "char-2",
            "{\"location\":{\"kind\":\"author\",\"chapterNumber\":-1}}",
            None,
            None,
        );

        let fields = current_derived_character_fields(&conn, "proj-1", None).unwrap();
        assert_eq!(
            fields.get("char-1").cloned().unwrap_or_default(),
            vec![CharacterStateTextField::Location]
        );
        assert_eq!(
            fields.get("char-2").cloned().unwrap_or_default(),
            Vec::<CharacterStateTextField>::new(),
            "无可信字段的角色条目应为空数组"
        );
    }

    #[test]
    fn provenance_parsing_and_helpers_test() {
        assert!(parse_provenance(None).is_empty());
        assert!(parse_provenance(Some("")).is_empty());
        assert!(parse_provenance(Some("not-json")).is_empty());
        assert!(
            parse_provenance(Some("[1,2]")).is_empty(),
            "数组应视为无来源"
        );
        assert_eq!(parse_provenance(Some("{}")).len(), 0);
        let map = parse_provenance(Some("{\"location\":1}"));
        assert_eq!(map["location"], 1);
        // 键集排序连接（基线 Object.keys(v).sort().join(',')）
        let mut object = Map::new();
        object.insert("b".to_string(), Value::from(1));
        object.insert("a".to_string(), Value::from(2));
        assert_eq!(sorted_keys_join(&object), "a,b");
        // 稳定键构造与基线 JSON.stringify 数组一致（数字为 JSON 数字）
        assert_eq!(source_key(1, "f1", 2, "hash"), "[1,\"f1\",2,\"hash\"]");
        // 安全整数收敛
        assert_eq!(safe_integer_value(Some(&Value::from(0))), Some(0));
        assert_eq!(
            safe_integer_value(Some(&Value::from(9007199254740992i64))),
            None,
            "超出安全整数范围应拒绝"
        );
        assert_eq!(
            safe_integer_value(Some(&serde_json::json!(1.0))),
            Some(1),
            "整数值浮点与 JS Number.isSafeInteger 一致"
        );
        assert_eq!(safe_integer_value(None), None);
        assert_eq!(safe_integer_value(Some(&Value::from("1"))), None);
        assert_eq!(safe_integer_value(Some(&Value::Null)), None);
        assert_eq!(safe_integer_value(Some(&serde_json::json!(1.5))), None);
        // 基线 hash() 的已知 SHA-256
        assert_eq!(
            text_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn projection_rejects_content_mismatch_test() {
        let conn = seeded_db();
        let (draft_id, finalization_id, content_hash) =
            seed_finalized_chapter(&conn, 1, "正文", 1, 0);
        // 定稿快照与正文不一致 → 不进 currentSources → derived 不可信。
        conn.execute(
            "UPDATE finalization_outbox SET content_snapshot = '被改动的快照' \
             WHERE finalization_id = ?1",
            [&finalization_id],
        )
        .unwrap();
        set_watermark(&conn, 1, None);
        let provenance = format!(
            "{{\"location\":{}}}",
            derived_provenance(
                draft_id,
                &finalization_id,
                1,
                &content_hash,
                1,
                1,
                0,
                "proj-1"
            )
        );
        insert_character(&conn, "char-1", &provenance, Some("城东"), None);

        let rows = current_character_projection(&conn, "proj-1", None).unwrap();
        assert!(rows[0].get("cs_location").is_none());
        assert!(rows[0].get("cs_provenance").is_none());
        // content_hash 也参与键构造，确认其值被使用（非空断言）。
        assert!(!content_hash.is_empty());
    }
}
