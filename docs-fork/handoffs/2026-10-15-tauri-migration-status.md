# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第55次）

本轮代码：`4c9364c4`，master。普通迁移由主会话完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册249 → **覆盖248**，剩余42 | `4c9364c4`，check:channels | 2026-10-15 | `.pi/tmp/b92-channels.log` |
| Rust全量 | **1415 passed / 0 failed，56.63s** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log` |
| Rust定向 | **74 passed / 0 failed** | `4c9364c4`，cargo test --lib kb | 2026-10-15 | `.pi/tmp/b92-kb-final.log` |
| 全目标检查 | 退出0，无告警 | `4c9364c4`，cargo check --all-targets | 2026-10-15 | `.pi/tmp/b92-all-targets.log` |
| 前端定向 | 副本/会话传输13项、IPC结构体合同1项通过 | `4c9364c4`，3文件 | 2026-10-15 | `.pi/tmp/b92-transport.log`、`b92-ipc-struct.log` |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `4c9364c4` | 2026-10-15 | `.pi/tmp/b92-{typecheck,lint}.log`、终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B92 知识库文档副本

- 接通 `kb:read/save/reindex-document-copy`，Rust命令、TS参数映射、迁移清单与传输合同齐全；覆盖从245增加至248。
- 完整原文独立存储于 `.lore/knowledge-copies/<sha256(docId)>.json`。普通项目知识库导入写副本与标记；旧的仅分块文档明确不可用，不拼凑原文、不读取旧栈目录。
- 保存采用内容哈希乐观校验；只修改副本，保留旧块并标脏。脏、缺失、损坏副本在文本/向量检索中排除，重建后恢复。
- 项目级 source gate 覆盖副本、相关导入/删除/清空/回填/检索。排队后及数据库锁内重新验证项目租约和连接归属，不嵌套获取 gate。
- 重建保持文档ID、章节元数据和导入时间，本地更新块/FTS，不发模型请求；撤销旧向量代际激活，成功后清理旧物理向量。
- 数据安全：先复现再修复 stable-id 替换失败丢旧索引；插入保存点、替换事务统一回滚。重建提交前只写脏标记、提交后才写 current；提交失败恢复旧副本，提交后落盘失败仍为 stale，可重试，避免进程退出窗口误报已更新。
- Windows 使用已有 Win32 API 查询真实硬链接数，同句柄读取，查询失败拒绝；拒绝非普通文件/符号链接与非法副本JSON。无新依赖、schema或权限配置。
- 测试覆盖：往返、旧库不可用、空副本、哈希冲突、并发保存、排队后关闭会话、导入失败清理、同名替换、副本写失败/数据库提交失败/提交后落盘失败恢复。证据与范围见表。

## 后续工作与未解除边界

1. 下一候选切片：`project:overview-current` / `project:peek-overview`；先核对渲染契约、只读探测边界与旧栈隔离，不自动扩大外部文件授权。
2. Agent/draft 尚有组合核对：在途收据×关闭重开×多会话导航、unknown result×父根生命周期。仅已登记的 `generate_draft → chapter-draft` 开放，其余工作流/特殊域继续关闭。
3. 启动门禁已在`74ea2200`接通并持久化`~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom`仍返回`CUSTOM_SKIN_UNAVAILABLE`，前端`SKIN_SNAPSHOT_NOT_READY`分类待补。
4. 剩余42频道：cloud-backup8、finalization-generation5、finalized-character5、graph-generation5、project5、character-avatar4、dialog4、editor-inline4、import-generation2。归档/zip、头像schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
5. 前端B91全量的7个失败文件与`f15a1b71`完全一致：export-service-integration 1、narrative-thread-candidate-generator 2、plot-tree-generator 1、finalize-chapter.blueprint-entities 1、finalize-postprocess-character.integration 15、mutation-failure-boundary 63、refine-draft.command 40；B92仅跑相关合同，不宣称这些遗留失败已解除。
6. 浏览器最近整仓9文件失败、319通过/38失败；启动页专门filter曾报无匹配文件，不能记为已验收。此前38个失败附件和4个空截图目录已授权清理，不重复列为待清理事项。

工作区、实际HEAD及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根`src/`、`electron/`、`docs/`及根依赖配置/lockfile未改。最近全量基线为本轮1415/0，之后Rust切片累计0；本轮秘密扫描无发现。
