//! 批次 G4-9a：角色提案命令 —— `character-proposal:*` 6 频道
//! + `character-identity:read` 1 频道（共 7 频道）。
//!
//! 平移自 `electron/controllers/generation-controller.ts:181-190` 的
//! 7 个注册项，owner 接线见
//! `electron/services/main-generation-owner.ts:880-896`（`characters`
//! 依赖构造）与 `:1255-1266`（`approveCharacterProposal`）；Tauri 侧
//! owner 方法见 `services/main_generation_owner.rs` 末段「批次 G4-9a」
//! 节（`character_proposal_service` 依赖构造 + 7 个 owner 方法）。
//!
//! 命令名对齐前端 `ipc-client.ts` 的 `toCommandName`
//! （`channel.replace(/[:-]/g, '_')`），入参按频道契约
//! `src/shared/character-proposal.ts` 末节
//! `CharacterProposalChannels` 登记。
//!
//! # 门禁（基线 `authorizedOwner`，generation-controller.ts:86-130）
//! 所有命令先经 `MainGenerationOwnerCache::authorized_owner` 会话
//! 再认证 + 活跃项目库校验，再进入 owner 方法（owner 方法内部以
//! `assert_current` 作二级门禁，失败收敛为 `GENERATION_OWNER_CLOSED`）。
//!
//! # 基线 controller 层门禁的承载
//! `character-proposal:stage` 的 admission 门禁
//! （controller:182 `request.source?.kind === 'legacy-roster-generation'`
//! → `GENERATION_LEGACY_ADMISSION_REQUIRED`）在命令层承载（对齐基线
//! register 顺序：门禁先于 owner 方法调用）。

use serde_json::Value;
use tauri::State;

use crate::security::ProjectSessionContext;
use crate::services::main_generation_owner::{
    map_generation_error, mutating_error, MainGenerationOwnerCache,
};
use crate::state::AppState;

/// 基线 `authorizedOwner` 门禁的命令层入口（对齐
/// `commands/generation.rs` 的同型门禁）：会话再认证 + 活跃项目库
/// 校验，返回持锁的 owner 守卫。会话缺失时按基线 register catch 前的
/// 守卫文案拒绝。
fn authorized_gate<'a>(
    cache: &'a MainGenerationOwnerCache,
    state: &'a AppState,
    project_session: Option<&'a ProjectSessionContext>,
) -> Result<crate::services::main_generation_owner::OwnerGuard<'a>, String> {
    let Some(session) = project_session else {
        return Err("缺少项目会话上下文，已拒绝操作".to_string());
    };
    let active = state.active_project_snapshot();
    cache.authorized_owner(session, active.as_ref())
}

/// `character-proposal:stage` —— 阶段提交角色提案
/// （基线 `owner.characterProposals.stage(request.source)`，
/// controller:181-184）。
///
/// admission 门禁（基线 controller:182）：`source.kind ===
/// 'legacy-roster-generation'` → `GENERATION_LEGACY_ADMISSION_REQUIRED`
/// （旧名册生成提案须经 `legacy-roster:stage` 专用通道提交）。
#[tauri::command]
pub async fn character_proposal_stage(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 接收整个 request 对象，取 request.source
    // （controller:181-184：`request.source?.kind === 'legacy-roster-generation'`
    // → GENERATION_LEGACY_ADMISSION_REQUIRED；`stage(request.source)`）。
    // Tauri 语义：位置参数 {source} 整体映射为命名参数 request
    // （blueprint-character-sync.test.ts 断言 named = {request: {source}, projectSession}）。
    let source = request.get("source").cloned().unwrap_or(Value::Null);
    if source.get("kind").and_then(Value::as_str) == Some("legacy-roster-generation") {
        return Err(mutating_error(map_generation_error(
            "GENERATION_LEGACY_ADMISSION_REQUIRED",
        )));
    }
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.stage_character_proposal(conn, &source))
}

/// `character-proposal:read` —— 读取角色提案批次
/// （基线 `owner.characterProposals.read(request.proposalBatchId)`，
/// controller:185）。
#[tauri::command]
pub async fn character_proposal_read(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 接收整个 request 对象，取 request.proposalBatchId
    // （controller:185：`owner.characterProposals.read(request.proposalBatchId)`）。
    // Tauri 语义：位置参数 {proposalBatchId} 整体映射为命名参数 request
    // （与 stage 的 blueprint-character-sync.test.ts 断言约定一致）。
    let proposal_batch_id = request
        .get("proposalBatchId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.read_character_proposal(conn, &proposal_batch_id))
}

/// `character-proposal:list-pending-finalized` —— 列挂起定稿的角色提案
/// 摘要（基线 `owner.characterProposals.listPendingFinalized()`，
/// controller:186）。
#[tauri::command]
pub async fn character_proposal_list_pending_finalized(
    state: State<'_, AppState>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.list_pending_finalized_character_proposals(conn))
}

/// `character-proposal:read-pending-finalized` —— 读取挂起定稿的角色提案
/// 批次（基线 `owner.characterProposals.readPendingFinalized(request.proposalBatchId)`，
/// controller:187）。
#[tauri::command]
pub async fn character_proposal_read_pending_finalized(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 接收整个 request 对象，取 request.proposalBatchId
    // （controller:187：`readPendingFinalized(request.proposalBatchId)`）。
    // Tauri 语义：位置参数 {proposalBatchId} 整体映射为命名参数 request
    // （与 stage 的 blueprint-character-sync.test.ts 断言约定一致）。
    let proposal_batch_id = request
        .get("proposalBatchId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| {
        owner.read_pending_finalized_character_proposal(conn, &proposal_batch_id)
    })
}

/// `character-proposal:approve` —— 批准（采纳）角色提案
/// （基线 `owner.approveCharacterProposal(request)`，controller:188 →
/// main-generation-owner.ts:1255-1266）。
///
/// 幂等重放（已批准 → 直接重放决策校验）→ handle 四分支解析 →
/// `agents.withChildEffect` 效应（主体 fail-closed 占位
/// `GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`，agent-generation 域
/// 迁移波次活化）；import 来源（无 handle）直接执行。
/// finalized-generation 来源的证明函数（`proveFinalizedCharacterGeneration`）
/// 未迁移，fail-closed 于 `CHARACTER_PROPOSAL_PROOF_PORT_PENDING`
/// （迁移占位码）。详见 owner 方法头注的刻意偏离说明。
#[tauri::command]
pub async fn character_proposal_approve(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.approve_character_proposal(conn, &request))
}

/// `character-proposal:cancel` —— 取消角色提案
/// （基线 `owner.characterProposals.cancel(request)`，controller:189）。
#[tauri::command]
pub async fn character_proposal_cancel(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.cancel_character_proposal(conn, &request))
}

/// `character-identity:read` —— 角色身份快照
/// （基线 `owner.characterProposals.identitySnapshot()`，controller:190）。
#[tauri::command]
pub async fn character_identity_read(
    state: State<'_, AppState>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.character_identity_snapshot(conn))
}
