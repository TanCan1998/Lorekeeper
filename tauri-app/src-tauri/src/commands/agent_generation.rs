//! 批次 G4-10c：agent 生成命令 —— `agent-generation:*` 8 频道。
//!
//! 平移自 `electron/controllers/generation-controller.ts:217-230`
//! 的 8 个注册项（全部 arity=1：request 对象 + 末位
//! projectSession）；owner 接线见
//! `services/main_generation_owner.rs`「批次 G4-10c」节
//! （8 个 `agent_*` 委托方法，对齐 `admit_workflow` 先例）。
//!
//! 命令名对齐前端 `ipc-client.ts` 的 `toCommandName`
//! （`channel.replace(/[:-]/g, '_')`），入参按频道契约
//! `src/shared/agent-generation.ts` 末节
//! `AgentGenerationChannels` 登记。
//!
//! # 门禁（基线 `authorizedOwner`，generation-controller.ts:86-130）
//! 所有命令先经 `MainGenerationOwnerCache::authorized_owner` 会话
//! 再认证 + 活跃项目库校验，再进入 owner 方法（owner 方法内部以
//! `assert_current` 作二级门禁，失败收敛为 `GENERATION_OWNER_CLOSED`）。
//! 会话缺失时按基线 register 守卫（controller:217-230：末位参数
//! 必须为合法 `ProjectSessionContext` 否则
//! `GENERATION_PROJECT_SESSION_REQUIRED`）的等价守卫文案拒绝
//! （对齐 `commands/generation.rs` / `commands/character_proposal.rs`
//! 的 `authorized_gate` 先例）。
//!
//! # 基线 controller 层门禁的承载
//! register 统一门禁（controller:217-230）与错误映射
//! （正则 `^(?:GENERATION|ROOT_BUDGET|ARTIFACT|MAIN|CHARACTER|
//! FINALIZED_CHARACTER|LEGACY_ROSTER)_[A-Z_]+$` 透传，其余折叠
//! `GENERATION_REQUEST_FAILED`）在命令层承载：错误经
//! `mutating_error(map_generation_error(&e))` 折叠。
//!
//! # 本波占位频道
//! `agent-generation:resume` / `agent-generation:round` 的 owner
//! 主体（`AgentGeneration::resume` / `execute_round`）依赖分发核心
//! 与 LLM 双阶段执行循环迁移（后续波次），当前 fail-closed 于
//! `GENERATION_AGENT_RESUME_UNMIGRATED` /
//! `GENERATION_AGENT_ROUND_UNMIGRATED`（迁移占位码、非基线码，
//! 匹配透传正则，经错误映射原样透传）。

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::security::ProjectSessionContext;
use crate::services::main_generation_owner::{
    map_generation_error, mutating_error, MainGenerationOwnerCache,
};
use crate::state::AppState;

/// 基线 `authorizedOwner` 门禁的命令层入口（对齐
/// `commands/character_proposal.rs` 的同型门禁）：会话再认证 +
/// 活跃项目库校验，返回持锁的 owner 守卫。会话缺失时按基线
/// register 守卫（controller:217-230）拒绝。
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

/// 域结果 → IPC 返回值：typed → `Value` 序列化 + 错误折叠
/// （基线 register catch 的错误映射正则透传 / 折叠
/// `GENERATION_REQUEST_FAILED`，再经 `mutating_error` 加
/// MUTATING 失败信封前缀）。
fn domain_result<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result
        .map_err(|error| mutating_error(map_generation_error(&error)))
        .and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
        })
}

/// `agent-generation:begin` —— 幂等恢复入口：按 nonce 派生
/// session key，命中且 context / modelId 一致则返回既有
/// recovery，否则开第 0 轮 run（基线
/// `owner.agents.begin(request)`，controller:217）。
#[tauri::command]
pub async fn agent_generation_begin(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_begin(conn, &request)))
}

/// `agent-generation:read` —— 纯读：session 全轮次重建 +
/// nextRound 计算 + sourceStatus（基线
/// `owner.agents.read(request.handle)`，controller:218）。
#[tauri::command]
pub async fn agent_generation_read(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 取 request.handle（controller:218）。
    let handle = request.get("handle").cloned().unwrap_or(Value::Null);
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_read(conn, &handle)))
}

/// `agent-generation:resume` —— 恢复执行：已完成轮次走控制
/// 纪元写入，否则 host.resume（LLM 续跑）（基线
/// `owner.agents.resume(request.handle)`，controller:226）。
///
/// ⚠️ 本波占位：owner 主体依赖分发核心迁移，当前
/// fail-closed 于 `GENERATION_AGENT_RESUME_UNMIGRATED`
/// （迁移占位码、非基线码，经错误映射透传）。
#[tauri::command]
pub async fn agent_generation_resume(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let handle = request.get("handle").cloned().unwrap_or(Value::Null);
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_resume(conn, &handle)))
}

/// `agent-generation:round` —— 执行指定轮次（0..7）：构造
/// messages 后 host.execute 派发 LLM（基线
/// `owner.agents.executeRound(request)`，controller:227）。
///
/// ⚠️ 本波占位：owner 主体依赖 LLM 双阶段执行循环迁移，
/// 当前 fail-closed 于 `GENERATION_AGENT_ROUND_UNMIGRATED`
/// （迁移占位码、非基线码，经错误映射透传）。
#[tauri::command]
pub async fn agent_generation_round(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_round(conn, &request)))
}

/// `agent-generation:claim-tool` —— 作者认领工具调用：
/// pending→running（confirmed）或 declined；返回
/// `{ action, execute }`（基线 `owner.agents.claimTool(request)`，
/// controller:228）。
#[tauri::command]
pub async fn agent_generation_claim_tool(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_claim_tool(conn, &request)))
}

/// `agent-generation:finish-tool` —— 工具结果回执写入
/// （幂等重放 / 冲突检测）（基线
/// `owner.agents.finishTool(request)`，controller:229）。
#[tauri::command]
pub async fn agent_generation_finish_tool(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_finish_tool(conn, &request)))
}

/// `agent-generation:register-workflow` —— `start_workflow`
/// 工具确认后注册子生成工作流（基线
/// `owner.agents.registerWorkflow(request.ref)`，
/// controller:230 register-workflow 项）。
#[tauri::command]
pub async fn agent_generation_register_workflow(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 取 request.ref（controller:230
    // register-workflow 项）。
    let ref_ = request.get("ref").cloned().unwrap_or(Value::Null);
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_register_workflow(conn, &ref_)))
}

/// `agent-generation:commit-domain-tool` —— 在捕获的事务内
/// 提交 `propose_novel_config` / `propose_chapter_blueprint`
/// 域效应（基线 `owner.agents.commitDomainTool(request.ref)`，
/// controller:230 commit-domain-tool 项）。
#[tauri::command]
pub async fn agent_generation_commit_domain_tool(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    // 基线 handler 取 request.ref（controller:230
    // commit-domain-tool 项）。
    let ref_ = request.get("ref").cloned().unwrap_or(Value::Null);
    let state = state.inner();
    let cache = &state.generation_owners;
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| domain_result(owner.agent_commit_domain_tool(conn, &ref_)))
}
