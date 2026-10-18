# 当前交接入口

- 最新：[2026-10-18-tauri-migration-status.md](./2026-10-18-tauri-migration-status.md)，第 **102** 次。
- 代码 **0f0880d2**，master，已推送；文档不改变Rust代码指纹。
- 最新已核验全量：**0f0880d2 / 39105179211，1692/0，库223.16秒，all-targets36.80秒无告警**；前片b5850fbe/39102768655独立1690/0。产物expectedSha/testedSha、两退出0、lock哈希、state=passed与日志均核对，语义切片累计0。
- **#30可登记关闭**：真实导出→原生保存对话框→原生文件夹选择→正式恢复新身份→副本真实编辑保存→项目/应用重开→旧lease拒绝、定稿不变、头像渲染全部通过；导出2231534字节SHA256匹配，源项目与lease未变，33/33历史run冻结。前台授权仅限本次验收，已失效。
- **#31首版范围可声明**：290契约频道全部有处置（289功能迁移 + 1明确不支持 `project:import-legacy-copy`），未迁移0，事件7，版本三处1.1.0。
- 本轮按rust-lib-ux审查198个Rust文件；仅A1/A2两项建议修改并已落地：UpdateService、McpManager改内部Arc内核+共享Clone，AppState与命令层去掉Arc包装，新增两条克隆句柄回归。B类技术债与C类契约例外已记录，未改IPC/schema/依赖/权限。
- 不能推出真实模型/MCP/业务云/发布就绪；macOS/Linux未认证；未穷尽所有JSON消费者与交错。`update:download`/`quit-and-install`仍诚实返回NOT_READY。
- 不派子代理、不删target、不改双栈保护目录与根配置。
