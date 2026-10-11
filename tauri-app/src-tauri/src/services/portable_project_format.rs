//! 批次 G4-1：便携项目格式常量（自 `src/shared/project-format.ts` 平移）。
//!
//! 上游事实源：`src/shared/project-format.ts` ——
//! `export const CANONICAL_PROJECT_DIRECTORY = '.ai-novel'`。
//! crate 内此前无等价常量（grep 全 crate 无 `CANONICAL_PROJECT_DIRECTORY` /
//! `.ai-novel` 命中），故本波新建此模块作为唯一 Rust 单源；
//! `portable_runtime_freeze` 等同波模块从本模块导入，
//! 禁止各自硬编码（避免与上游漂移）。
//!
//! 纯逻辑、无 Tauri 依赖，由 commands 层按需调用。

/// 规范项目目录名（上游 `CANONICAL_PROJECT_DIRECTORY`，值 `.ai-novel`）。
/// 项目存储根 = `<项目根>/.ai-novel`（L3 双栈隔离：Tauri 专用 `.lore/` 为
/// 另一层目录，见 `AGENTS.md`；本常量是**便携项目归档**内的目录名约定）。
pub const CANONICAL_PROJECT_DIRECTORY: &str = ".ai-novel";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_project_directory_matches_upstream() {
        assert_eq!(CANONICAL_PROJECT_DIRECTORY, ".ai-novel");
    }
}
