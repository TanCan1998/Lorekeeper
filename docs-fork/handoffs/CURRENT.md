# 当前交接入口

- 最新：[2026-10-18-tauri-migration-status.md](./2026-10-18-tauri-migration-status.md)，第 **96** 次。
- 代码 **561e91d0**，master，已推送；当前文档不改变Rust代码指纹。
- 最新核验云端全量：**561e91d0 / 39094795752，1682/0，all-targets 36.92秒无告警**；完整SHA、独立退出码、lock及state=passed已核对，Rust语义切片累计0。
- 恢复副本作者内容可继续编辑；审稿→作者确认→修稿→合并→复核已跑通合成SQLite集成链。执行历史仍冻结、无可继续候选；未新增GUI验收。
- 旧handle读/恢复精确拒绝PORTABLE_RUNTIME_FROZEN；草稿依赖中和为[]，普通项目和恢复副本的当前空值/坏JSON仍拒绝。
- 最新定向：review_revision 70/0、portable 87/0，格式/diff通过。可空R列扫描已记录，不代表全部JSON消费路径验收。
- #25–29完成，#30读面收口进行中，#31最终验收待办；下一步见快照。
- 保留完整master与双栈参照，按需使用本机稀疏检出开关；不派子代理、不删除target、不扩展真实业务/发布授权。
