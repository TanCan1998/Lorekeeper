//! main_generation_owner —— 主生成 owner（批次 B1-W3 骨架）。
//!
//! 事实源（上游 Electron 基线）：
//! - `electron/services/main-generation-owner.ts`（1401 行）—— owner 服务本体：
//!   `createMainGenerationOwner`（:108）每库一份，捕获 DB 句柄 / 会话 /
//!   模型执行租约；`assertCurrent`（:117-120）入口门禁；
//!   `suspendForProjectClose`（:1390-1399）项目关闭钩子。
//! - `electron/controllers/generation-controller.ts`（266 行）——
//!   per-database owner 缓存 `owners`（:37）+ `ownerSessions`（:38）、
//!   `authorizedOwner` 门控（:86-130）、`onProjectDatabaseBeforeClose`
//!   钩子（:82-85）、`generation:snapshot` / `generation:reasoning` 事件
//!   发射（:117-124）、register 的错误码映射（:160-162）。
//!
//! # 本次落地（骨架级，不接命令层）
//! 1. per-database owner 缓存 [`MainGenerationOwnerCache`]
//!    （`Mutex<HashMap>` 线程安全包装；键 = 归一化项目根路径，
//!    等价基线以 `Database` 对象身份为键——Tauri 侧一项目根 ⇔ 唯一项目库
//!    连接，见 `db/mod.rs` 头注）；
//! 2. [`MainGenerationOwnerCache::authorized_owner`] 门控（逐条对齐基线
//!    `authorizedOwner` 语义）；
//! 3. 错误码映射 [`map_generation_error`]（基线透传正则 + 兜底码）与
//!    MUTATING 前缀 [`mutating_error`]；
//! 4. [`MainGenerationOwner::suspend_for_project_close`] 钩子
//!    （签名 + 缓存清理）；
//! 5. 事件发射接线点 [`emit_generation_snapshot`] /
//!    [`emit_generation_reasoning`]（**TODO(B3)** 才真实发射）。
//!
//! # 刻意偏离（均有上游语义理由）
//! - **缓存键**：基线以 `better-sqlite3` 的 `Database` 对象身份为键；
//!   Tauri 侧 `Connection` 不可哈希，且项目库连接随
//!   `AppState::project_db` 单一生命周期管理，改用
//!   `security::normalized_project_path` 的项目根路径为键，语义等价。
//! - **会话身份**：基线把捕获会话放缓存条目（`entry.session`）并另建
//!   `ownerSessions` WeakMap；本骨架把捕获会话并入 owner（`session`
//!   字段），门禁比对读 `owner.session`，等价。
//! - **订阅者集合**：基线 `Set<WebContents>`（IPC 发送端）；Tauri 事件
//!   经 `tauri::Emitter` 按窗口标签寻址，骨架以 `HashSet<String>`
//!   （窗口标签）占位，B3 收口。
//! - **DB 句柄**：基线 owner 构造时捕获 DB 句柄；Tauri 仓储约定按调用
//!   传入 `&Connection`（`repositories/generation_run_repository.rs`
//!   头注），owner 只捕获会话身份，连接由命令层在 `project_access`
//!   门禁内传入。
//! - **会话再认证**：基线 `deps.assertCurrent()` 在每个 owner 方法入口
//!   重验会话与库身份；Tauri 侧由命令层在缓存访问前经
//!   `security::assert_current_project_context` 等价执行一次
//!   （持守卫期间项目库 / 会话态被同一把锁保护，结构上不会被并发切换）。

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard};

use regex::Regex;
use rusqlite::Connection;

use crate::repositories::generation_run_repository::GenerationRunRepository;
use crate::security::{self, ProjectSessionContext};
use crate::state::ActiveProject;

// ============================================================================
// 错误码（字符串值逐字对齐上游 throw 调用）
// ============================================================================

/// 基线 `authorizedOwner`：当前项目库未打开
/// （generation-controller.ts:89）。
///
/// 结构上不可达：Tauri 侧会话门禁（`security::assert_current_project_context`）
/// 已要求活跃项目存在；常量保留以对齐基线文案。
const GENERATION_DATABASE_NOT_READY: &str = "GENERATION_DATABASE_NOT_READY";
/// 基线 owner `assertCurrent`：owner 已随项目关闭挂起
/// （main-generation-owner.ts:118）。
const GENERATION_OWNER_CLOSED: &str = "GENERATION_OWNER_CLOSED";
/// 基线 owner `assertCurrent` 依赖：owner 存活期间项目库被替换
/// （generation-controller.ts:108）。
///
/// 结构上不可达：Tauri 侧项目库连接随 `AppState::project_db` 单一生命周期
/// 管理，且门禁已认证会话与活跃项目一致；常量保留以对齐基线文案。
#[allow(dead_code)]
const GENERATION_DATABASE_CHANGED: &str = "GENERATION_DATABASE_CHANGED";
/// 基线事务内辅助函数：当前库无 owner 缓存条目
/// （generation-controller.ts:43 等）。
#[allow(dead_code)]
const GENERATION_OWNER_REQUIRED: &str = "GENERATION_OWNER_REQUIRED";
/// 基线 register catch 兜底码（generation-controller.ts:161）。
const GENERATION_REQUEST_FAILED: &str = "GENERATION_REQUEST_FAILED";
/// 基线透传字面量（generation-controller.ts:160）。
const SOURCE_DRAFT_CHANGED: &str = "SOURCE_DRAFT_CHANGED";

/// 基线 register catch 的透传正则（generation-controller.ts:160）：
/// `^(?:GENERATION|ROOT_BUDGET|ARTIFACT|MAIN|CHARACTER|FINALIZED_CHARACTER|LEGACY_ROSTER)_[A-Z_]+$`
static OWNER_ERROR_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:GENERATION|ROOT_BUDGET|ARTIFACT|MAIN|CHARACTER|FINALIZED_CHARACTER|LEGACY_ROSTER)_[A-Z_]+$",
    )
    .expect("owner error code pattern is a valid regex")
});

/// 对齐基线 register catch 的错误码映射（generation-controller.ts:160-162）：
///
/// - [`SOURCE_DRAFT_CHANGED`] 字面量，或匹配 [`OWNER_ERROR_CODE`] 正则的
///   错误码 → **原样透传**；
/// - 其余（含会话守卫的中文文案）→ 一律收敛为 [`GENERATION_REQUEST_FAILED`]。
pub fn map_generation_error(message: &str) -> &str {
    if message == SOURCE_DRAFT_CHANGED || OWNER_ERROR_CODE.is_match(message) {
        message
    } else {
        GENERATION_REQUEST_FAILED
    }
}

/// MUTATING 失败信封前缀（复刻 `commands::db::mutating_error`，保持两栈
/// 前端展示文案逐字一致：基线渲染层把 `throw new Error(code)` 序列化为
/// `Error: <code>`）。
pub fn mutating_error(code: &str) -> String {
    format!("Error: {code}")
}

// ============================================================================
// 主生成 owner
// ============================================================================

/// 主生成 owner（对齐基线 `createMainGenerationOwner` 的闭包状态，
/// main-generation-owner.ts:108-1399）。
///
/// 骨架阶段仅持有会话身份与生成账本仓储（W1）；生成编排方法
/// （`begin` / `execute` / `list` / …）随 B3 批次平移，方法体以
/// `todo!()` 占位。
pub struct MainGenerationOwner {
    /// 基线 `Object.freeze({ ...session })` 捕获的会话身份
    /// （generation-controller.ts:97）——项目生命周期内不变，
    /// 门禁比对（`projectId` / `leaseId`）与 `ownerSessions`
    /// WeakMap 读取均使用它。
    session: ProjectSessionContext,
    /// 生成账本仓储（W1；`repositories/generation_run_repository.rs`）。
    repository: GenerationRunRepository,
    /// 基线 `runLeases: Map<string, string>`（attemptId → leaseId，
    /// main-generation-owner.ts:112）。挂起时逐 lease 关闭并清空。
    run_leases: HashMap<String, String>,
    /// 基线 `closed` 标志（main-generation-owner.ts:116）。
    closed: bool,
}

impl MainGenerationOwner {
    /// 基线 owner 构造的骨架等价物（generation-controller.ts:106-125 的
    /// `createMainGenerationOwner({...})` 调用，裁剪到当前已迁移的依赖面：
    /// 仅会话身份与生成账本仓储；`leases` / `loadModel` / `buildBinding` /
    /// `onSnapshot` / `onReasoning` 等依赖随对应批次接入）。
    fn new(session: ProjectSessionContext) -> Self {
        Self {
            session,
            repository: GenerationRunRepository::default(),
            run_leases: HashMap::new(),
            closed: false,
        }
    }

    /// 对齐基线 `assertCurrent`（main-generation-owner.ts:117-120）：
    /// owner 已挂起 → `GENERATION_OWNER_CLOSED`。
    ///
    /// 基线另外执行 `deps.assertCurrent()`（会话再认证 +
    /// `getProjectDb() !== database` → `GENERATION_DATABASE_CHANGED`）；
    /// Tauri 侧由命令层在缓存访问前经 `security::assert_current_project_context`
    /// 等价完成（见 [`MainGenerationOwnerCache::authorized_owner`]）。
    pub fn assert_current(&self) -> Result<(), String> {
        if self.closed {
            return Err(mutating_error(GENERATION_OWNER_CLOSED));
        }
        Ok(())
    }

    /// 生成账本仓储访问点（B3 迁移生成编排时使用；基线 owner 闭包内
    /// 直接捕获 `repository`）。
    pub fn repository(&self) -> &GenerationRunRepository {
        &self.repository
    }

    /// 基线 `runLeases` 登记点（`runLeases.set(attemptId, leaseId)` 的
    /// 等价物；B3 迁移执行编排时启用）。
    pub fn record_run_lease(&mut self, attempt_id: String, lease_id: String) {
        self.run_leases.insert(attempt_id, lease_id);
    }

    /// 对齐基线 `suspendForProjectClose`（main-generation-owner.ts:1390-1399）：
    /// 项目关闭前挂起 owner——清准备态、关子生成器、挂起运行服务、
    /// 关闭全部在途模型执行租约、置 `closed`。
    ///
    /// 幂等：基线 `if (closed) return` 之后的清理只在首次执行
    /// （:1394-1398）；`preparations` / 子生成器的清理（:1391-1393）
    /// 在对应服务迁移后接入。
    pub fn suspend_for_project_close(&mut self) {
        // TODO(B3+)：`preparations.clear()`——草稿上下文准备缓存
        //   （main-generation-owner.ts:1391；`prepareDraftContext` 迁移后接入）。
        // TODO(B3+)：`finalizedCharacters.close()` / `reviewRevisions.close()`
        //   （main-generation-owner.ts:1392-1393；子生成器迁移后接入）。
        if self.closed {
            return;
        }
        // TODO(B3+)：`service.suspendForProjectClose()`——生成运行服务挂起
        //   （main-generation-owner.ts:1395；`generation-run-service.ts` 迁移后接入）。
        self.closed = true;
        // TODO(B3+)：基线逐个 `deps.leases.close(leaseId)` 关闭模型执行租约
        //   （main-generation-owner.ts:1397；`llm::lease` 注册表接线后接入）。
        self.run_leases.clear();
    }

    // -----------------------------------------------------------------
    // B3 迁移占位：生成编排方法（方法体平移自 main-generation-owner.ts
    // 的同名闭包；契约类型随 `src/shared/generation-owner-contract.ts`
    // 迁移落地后补齐签名）。
    // -----------------------------------------------------------------

    /// 基线 `begin`（选择 → 建运行 → 返回运行视图）。
    pub fn begin(&self, _conn: &Connection) -> Result<serde_json::Value, String> {
        self.assert_current()?;
        todo!("B3: 迁移自 electron/services/main-generation-owner.ts 的 begin")
    }

    /// 基线 `execute`（分发 → 流式回执）。
    pub fn execute(&self, _conn: &Connection) -> Result<serde_json::Value, String> {
        self.assert_current()?;
        todo!("B3: 迁移自 electron/services/main-generation-owner.ts 的 execute")
    }

    /// 基线 `list`（按创建时间倒序列出当前项目的生成运行视图）。
    pub fn list(&self, _conn: &Connection) -> Result<serde_json::Value, String> {
        self.assert_current()?;
        todo!("B3: 迁移自 electron/services/main-generation-owner.ts 的 list")
    }
}

// ============================================================================
// per-database owner 缓存
// ============================================================================

/// 缓存条目（对齐基线 `{ owner, session, subscribers }`，
/// generation-controller.ts:37）。
struct OwnerCacheEntry {
    owner: MainGenerationOwner,
    /// 基线 `subscribers: Set<WebContents>`。Tauri 侧事件经
    /// `tauri::Emitter` 按窗口标签寻址，骨架以窗口标签集合占位
    /// （**TODO(B3)**：真实订阅登记 + `emit_to` 发射）。
    #[allow(dead_code)]
    subscribers: HashSet<String>,
}

/// per-database owner 缓存（对齐基线 `owners` Map，
/// generation-controller.ts:37）。
///
/// - 键：归一化项目根路径（`security::normalized_project_path`），等价
///   基线以 `Database` 对象身份为键（Tauri 侧一项目根 ⇔ 唯一项目库
///   连接，`db/mod.rs` 头注「一个进程同一时刻只持有一个项目库连接」）；
/// - 线程安全包装：`Mutex`（单写入者模型，与 `state.rs` 各注册表一致）。
#[derive(Default)]
pub struct MainGenerationOwnerCache {
    entries: Mutex<HashMap<String, OwnerCacheEntry>>,
}

/// [`MainGenerationOwnerCache::authorized_owner`] 返回的 owner 守卫：
/// 持有缓存锁直至 drop，保证「门禁 → owner 方法」临界区内项目库 /
/// 会话态不被并发切换（基线单线程事件循环的等价物）。
///
/// ⚠️ 持守卫期间**不得**调用 [`MainGenerationOwnerCache::suspend_for_project_close`]
/// （同一把锁，会自死锁）；基线的挂起时机是项目关闭，发生在任何
/// 命令临界区之外。
pub struct OwnerGuard<'a> {
    entries: MutexGuard<'a, HashMap<String, OwnerCacheEntry>>,
    key: String,
}

impl<'a> OwnerGuard<'a> {
    /// 当前 owner（可变）。owner 方法应在持守卫时调用。
    pub fn owner(&mut self) -> &mut MainGenerationOwner {
        let entry = self
            .entries
            .get_mut(&self.key)
            .expect("owner entry is held by the guard");
        &mut entry.owner
    }

    /// 捕获的会话身份（基线 `ownerSessions` WeakMap 的等价读取点，
    /// generation-controller.ts:38 / :126）。
    pub fn session(&self) -> &ProjectSessionContext {
        let entry = self
            .entries
            .get(&self.key)
            .expect("owner entry is held by the guard");
        &entry.owner.session
    }
}

impl MainGenerationOwnerCache {
    /// 新建空缓存（等价基线模块级 `const owners = new Map()` 的初始态）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 缓存条目数（诊断 / 测试用；基线无等价物）。
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .ok()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// 缓存是否为空（诊断 / 测试用；基线无等价物）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 对齐基线 `authorizedOwner` 门控（generation-controller.ts:86-130）。
    ///
    /// 语义（逐条对齐）：
    /// 1. 会话再认证：`security::assert_current_project_context`（基线
    ///    `projectAccess.assertCurrentProjectContext`）；失败按基线
    ///    register catch 映射为 [`GENERATION_REQUEST_FAILED`]（基线中文
    ///    守卫文案不匹配透传正则，generation-controller.ts:160-162）；
    /// 2. 当前项目库必须已打开，否则 [`GENERATION_DATABASE_NOT_READY`]
    ///    （基线 `getProjectDb()` 为 null，generation-controller.ts:88-90）；
    /// 3. 缓存命中但 `projectId` / `leaseId` 与缓存会话不一致 → 先挂起
    ///    旧 owner 再逐出（基线 generation-controller.ts:91-95）；
    /// 4. 未命中 → 以**冻结的会话副本**创建 owner 并入缓存（基线
    ///    `Object.freeze({ ...session })`，generation-controller.ts:97）；
    /// 5. 基线末步 `entry.subscribers.add(event.sender)`
    ///    （generation-controller.ts:129）的 Tauri 等价物为 **TODO(B3)**：
    ///    命令层把当前窗口标签登记进 [`OwnerCacheEntry::subscribers`]。
    ///
    /// 返回 [`OwnerGuard`]（持锁）；失败返回带 `Error: ` 前缀的基线错误码
    /// （MUTATING 信封，见 [`mutating_error`]）。
    pub fn authorized_owner(
        &self,
        session: &ProjectSessionContext,
        active: Option<&ActiveProject>,
    ) -> Result<OwnerGuard<'_>, String> {
        // 1) 会话再认证（基线 projectAccess.assertCurrentProjectContext）
        security::assert_current_project_context(session, active)
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        // 2) 当前项目库（基线 getProjectDb()；Tauri 侧活跃会话即库已打开）
        let Some(active) = active else {
            return Err(mutating_error(GENERATION_DATABASE_NOT_READY));
        };
        let key = security::normalized_project_path(&active.root_path);
        // 锁中毒（持有锁的线程 panic）：按基线兜底码收敛（同 register catch）
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(_) => return Err(mutating_error(GENERATION_REQUEST_FAILED)),
        };
        // 3) 会话身份不一致 → 挂起并逐出（基线 generation-controller.ts:91-95）
        let stale_session = entries.get(&key).is_some_and(|entry| {
            entry.owner.session.project_id != session.project_id
                || entry.owner.session.lease_id != session.lease_id
        });
        if stale_session {
            let mut stale = entries
                .remove(&key)
                .expect("stale entry was just found above");
            stale.owner.suspend_for_project_close();
        }
        // 4) 未命中 → 以冻结会话副本创建（基线 Object.freeze({ ...session })）
        entries
            .entry(key.clone())
            .or_insert_with(|| OwnerCacheEntry {
                owner: MainGenerationOwner::new(session.clone()),
                subscribers: HashSet::new(),
            });
        // 5) TODO(B3)：基线在此登记 `event.sender` 到 subscribers。
        Ok(OwnerGuard { entries, key })
    }

    /// 基线 `onProjectDatabaseBeforeClose` 钩子（generation-controller.ts:82-85）：
    /// 项目库关闭前挂起其 owner 并逐出缓存。
    ///
    /// 命令层在关闭项目库（`AppState::project_db` 置空）**前**调用。
    pub fn suspend_for_project_close(&self, project_root: &str) {
        let key = security::normalized_project_path(project_root);
        if let Some(mut entries) = self.entries.lock().ok() {
            if let Some(mut entry) = entries.remove(&key) {
                entry.owner.suspend_for_project_close();
                // TODO(B3)：基线同时随条目丢弃 `subscribers`
                // （WebContents 集合随窗口销毁）。
            }
        }
    }
}

// ============================================================================
// 事件发射接线点（**TODO(B3)**：真实发射）
// ============================================================================

/// `generation:snapshot` 事件发射接线点。
///
/// 基线：owner 构造时注入 `onSnapshot`（generation-controller.ts:117-120），
/// 生成运行服务每步经 `subscriber.send('generation:snapshot', snapshot)`
/// 向全部订阅窗口广播 `MainGenerationSnapshot`（契约见
/// `src/shared/generation-owner-contract.ts` 的 `GenerationOwnerChannels`）。
///
/// Tauri 等价物（**TODO(B3)**）：`AppHandle` + `tauri::Emitter::emit_to`
/// 按 [`OwnerCacheEntry::subscribers`] 的窗口标签发射。services 层保持
/// 无 Tauri 依赖（`services/mod.rs` 约定），故此处仅留接线点，
/// 骨架阶段不发射。
#[allow(dead_code)]
fn emit_generation_snapshot(_subscribers: &HashSet<String>, _snapshot: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:snapshot", snapshot) }`
}

/// `generation:reasoning` 事件发射接线点（基线 generation-controller.ts:121-124；
/// 载荷 `{ projectId, epoch, rootActionId, runId, attemptId, text }`，
/// 见 `MainGenerationOwnerDependencies::onReasoning`）。
///
/// Tauri 等价物（**TODO(B3)**）：同上，经 `emit_to` 发射
/// `generation:reasoning`。
#[allow(dead_code)]
fn emit_generation_reasoning(_subscribers: &HashSet<String>, _event: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:reasoning", event) }`
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(project_id: &str, lease_id: &str) -> ProjectSessionContext {
        ProjectSessionContext {
            project_id: project_id.to_string(),
            lease_id: lease_id.to_string(),
            project_path: r"F:\novel\demo".to_string(),
        }
    }

    fn active() -> ActiveProject {
        ActiveProject {
            project_id: "project-1".to_string(),
            lease_id: "lease-1".to_string(),
            root_path: r"F:\novel\demo".to_string(),
        }
    }

    #[test]
    fn error_mapping_passes_generation_codes_through() {
        assert_eq!(
            map_generation_error("GENERATION_EPOCH_STALE"),
            "GENERATION_EPOCH_STALE"
        );
        assert_eq!(
            map_generation_error("ROOT_BUDGET_EXHAUSTED"),
            "ROOT_BUDGET_EXHAUSTED"
        );
        assert_eq!(
            map_generation_error("SOURCE_DRAFT_CHANGED"),
            "SOURCE_DRAFT_CHANGED"
        );
        assert_eq!(
            map_generation_error("ARTIFACT_INTEGRITY_FAILED"),
            "ARTIFACT_INTEGRITY_FAILED"
        );
        assert_eq!(
            map_generation_error("FINALIZED_CHARACTER_STATE_INVALID"),
            "FINALIZED_CHARACTER_STATE_INVALID"
        );
    }

    #[test]
    fn error_mapping_falls_back_to_request_failed() {
        // 基线会话守卫的中文文案不匹配透传正则 → 兜底码
        assert_eq!(
            map_generation_error("项目租约已失效，已拒绝操作"),
            GENERATION_REQUEST_FAILED
        );
        assert_eq!(
            map_generation_error("unexpected panic"),
            GENERATION_REQUEST_FAILED
        );
        // 前缀匹配但缺少 `_CODE` 段 → 不透传
        assert_eq!(
            map_generation_error("GENERATION"),
            GENERATION_REQUEST_FAILED
        );
        // 小写 → 不透传
        assert_eq!(
            map_generation_error("generation_epoch_stale"),
            GENERATION_REQUEST_FAILED
        );
    }

    #[test]
    fn gate_creates_owner_once_per_session() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("first gate");
            guard.owner().assert_current().expect("owner is open");
        }
        // 同一会话再次进门控：命中缓存，owner 不重建（仍开放）
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("second gate");
            guard.owner().assert_current().expect("owner still open");
            assert_eq!(guard.session().lease_id, "lease-1");
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn gate_suspends_and_recreates_on_lease_change() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("first gate");
        // 租约变更（新会话）→ 旧 owner 挂起并逐出，新 owner 重建
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-2"), Some(&active))
                .expect("second gate");
            guard.owner().assert_current().expect("new owner is open");
            assert_eq!(guard.session().lease_id, "lease-2");
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn gate_rejects_cross_session_and_missing_project() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        // 会话与活跃项目不一致 → 基线中文守卫文案经映射收敛为 GENERATION_REQUEST_FAILED
        let error =
            match cache.authorized_owner(&session("other-project", "lease-1"), Some(&active)) {
                Ok(_) => panic!("cross-project session must be rejected"),
                Err(error) => error,
            };
        assert_eq!(error, mutating_error(GENERATION_REQUEST_FAILED));
        // 无活跃项目（基线 getCurrentProjectPath() 为 null）→ 同上兜底
        let error = match cache.authorized_owner(&session("project-1", "lease-1"), None) {
            Ok(_) => panic!("missing active project must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, mutating_error(GENERATION_REQUEST_FAILED));
        assert!(cache.is_empty());
    }

    #[test]
    fn owner_close_is_gated() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("gate");
            guard.owner().suspend_for_project_close();
            let error = guard.owner().assert_current().expect_err("owner suspended");
            assert_eq!(error, mutating_error(GENERATION_OWNER_CLOSED));
            // 幂等：二次挂起不 panic
            guard.owner().suspend_for_project_close();
        }
    }

    #[test]
    fn suspend_hook_evicts_entry() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("gate");
        cache.suspend_for_project_close(r"F:\novel\demo");
        assert!(cache.is_empty());
        // 逐出后重新进门控 → 重建开放的新 owner
        let mut guard = cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("re-gate");
        guard
            .owner()
            .assert_current()
            .expect("rebuilt owner is open");
    }
}
