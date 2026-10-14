//! services —— 领域服务的纯 Rust 逻辑层（批次 B5 起）。
//!
//! 首批成员：`avatar_image`（自 `electron/services/avatar-image.ts` 移植的头像压缩）。
//! B1-W3 起：`main_generation_owner`（主生成 owner 骨架——per-database
//! 缓存 + authorizedOwner 门控 + 错误映射 + 项目关闭挂起钩子）。
//! B1 起：`character_proposal`（角色提案服务——stage / read / approve /
//! cancel / evidence / 身份快照；prove 注入式，五分支证明结构含待移植叶函数桩）。
//! 约定：纯逻辑、无 Tauri 依赖，由 commands 层按需调用。

pub mod avatar_image;
pub mod character_proposal;
pub mod main_generation_owner;

// ── 批次 G4-1：生成采集器与纯逻辑面（移植自 electron/services/）──
// 依赖关系：source_ref 为共享内核（SourceRef/FrozenInputFingerprint 等）；
// 各采集器（import/graph/legacy-roster/portable/current-character-projection）
// 与纯逻辑层（plot_tree/narrative_thread/legacy_roster）互依于 services 内部，
// 由 commands 层按需调用。
pub mod current_character_projection;
pub mod graph_generation;
pub mod graph_generation_source;
pub mod import_generation;
pub mod import_generation_source;
pub mod legacy_roster_generation;
pub mod legacy_roster_generation_context;
pub mod legacy_roster_generation_pure;
pub mod legacy_roster_source;
pub mod narrative_thread_generation_pure;
pub mod plot_tree_generation_pure;
pub mod portable_current_authority;
pub mod portable_project_format;
pub mod portable_runtime_freeze;
pub mod portable_transfer_authority;
pub mod source_ref;

// ── 批次 G4-2：提示词渲染与三采集器（agent / editor-inline / finalization）──
// ── 批次 G4-10c：agent 域工具效应（平移自
// electron/services/agent-domain-tool-effect.ts）──
pub mod agent_domain_tool_effect;
// ── 批次 G4-10a：agent-generation 域（AgentGeneration 状态机服务层，
// 平移自 electron/services/agent-generation.ts；命令注册属子波次 C）──
pub mod agent_generation;
pub mod agent_generation_context;
pub mod editor_inline_generation;
pub mod finalization_generation_source;
pub mod generation_draft_effects; // 批次 G4-5：批量草稿效应读面（generation:* 读面依赖）
pub mod generation_source_binding; // 批次 G4-6a：生成源绑定服务层（平移自 electron/services/generation-source-binding.ts）
pub mod generation_task;
pub mod prompt_system_role;
pub mod render_prompt;

// ── 批次 G4-3b：审稿修稿服务层（移植自 electron/services/review-revision-*.ts）──
// 依赖关系：review_revision_context 为纯捕获层（仓储只读 + 共享纯函数私有平移）；
// review_revision_generation 依赖前者与生成账本仓储，由 commands 层按需调用。
pub mod review_revision_context;
pub mod review_revision_generation;

// ── 批次 G4-B3-1a：预算规划器与预算失败诊断（移植自
// src/services/generation/task-budget-planner.ts 与
// prompt-budget-failure.ts）──
// 依赖关系：prompt_budget_failure 依赖 task_budget_planner
// 的决策类型；GenerationReasoningStage 复用 llm::reasoning。
pub mod prompt_budget_failure;
pub mod task_budget_planner;

// ── 批次 G4-B3-1b：蓝图批次成本规划（1:1 移植自
// src/services/workflows/blueprint-batch-policy.ts）──
// 纯逻辑面：为 B3-1c 计划构造器供给成本包络数学，
// 本波不接 owner 调用点。
pub mod blueprint_batch_policy;
pub mod main_generation_plan;

// ── 批次 G4-B3-2b：Agent 响应协议解析（1:1 移植自
// src/shared/agent-response-protocol.ts）+ 生成运行服务
// （1:1 移植自 electron/services/generation-run-service.ts）──
// 依赖关系：agent_response_protocol 为纯函数解析器；
// generation_run_service 依赖前者（agentToolNames 路径）
// 与 generation_run_repository，dispatch 经
// GenerationDispatcher trait 依赖注入（B3-3 经
// llm::openai/gemini 进程内流式核心实现）。
pub mod agent_response_protocol;
pub mod generation_run_service;

// B3-3：从 owner 的 dispatchProvider 抽出的进程内适配器。
pub mod main_generation_dispatcher;

pub mod builtin_writing_skills;
pub(crate) mod main_generation_sources;
