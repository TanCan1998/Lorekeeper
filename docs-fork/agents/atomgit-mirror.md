# GitHub 主仓库与 AtomGit 镜像

## 仓库和下载

- 主仓库：https://github.com/TanCan1998/Lorekeeper
- 镜像仓库：https://atomgit.com/TanCan1998/Lorekeeper
- 下载表格：[已公开的 1.0.0-beta](../releases/1.0.0-beta.md)。中国大陆用户推荐 AtomGit，保留 GitHub 源。
- 两端共用相同 README。文件、图片和文档使用相对路径，分别由当前托管平台解析到当前仓库；Release 是平台路由，不把它当作仓库文件相对路径。
- Release 和附件使用明确的两套绝对 URL；不在镜像仓库单独编辑 README，不用脚本做平台替换。

## 中文首页 README 的选择

2026-10-19 实际网页核验：AtomGit 中文首页请求 `readme_file`（`language=zh`）后选择 `README_zh.md`，即使默认分支为 `master`、`README.md` 已完整同步，也会显示旧的短跳转页。不能只检查指定文件 API 一致就宣布首页完整。

`README.md` 是唯一编辑源，`README_zh.md` 是逐字节相同的完整展示副本，两平台都提交同样内容；不是只在 AtomGit 修改，也不按平台改写正文。每次修改 README 或合并上游后执行：

```powershell
node tauri-app/scripts/sync-readme.mjs
node tauri-app/scripts/sync-readme.mjs --check
```

这两个命令只处理 README，不构建、不测试应用。验收要同时检查中文首页实际渲染内容和所选文件，不能只读 `README.md`。

## 2026-10-21 beta.1 同步结果

- 代码与标签同步成功；Release 与九个附件最终通过匿名 SHA256 读回，说明含双源表格。
- 首次发布作业与第一次镜像重试均在单请求 600 秒上限超时（GitHub runner 跨境上传 AtomGit 存储）；按协议只补缺失附件后重跑镜像成功，未覆盖已有文件。旧流程已整体改为平台拉取 + 本机附件上传，不再依赖 GitHub 跨境上传。
- 中文首页只读抓取未出现「本 Fork 的目的」段落与新版本链接，首页完整渲染仍未验收。

## 代码镜像：由 AtomGit 平台拉取

2026-10-21 用户决定：**代码与标签由 AtomGit 平台从 GitHub 拉取，GitHub 不再推送同步**。

- 已删除 `.github/workflows/atomgit-mirror.yml` 与 `tauri-app/scripts/sync-atomgit.mjs`；GitHub 侧没有推送代码或标签到 AtomGit 的工作流。
- 在 AtomGit 仓库的镜像/同步设置里，把镜像源指向 `https://github.com/TanCan1998/Lorekeeper`，选择 `master` 与 `lorekeeper-v*` 标签；按平台能力设置定时或手动拉取。平台拉取是只读镜像，不会向 GitHub 回写。
- **已于 2026-10-21 启用并实测**：Pull 镜像 `mirroring_enabled=true`、`force=false`、`number_of_failures=0`；手动“立即同步”后 AtomGit `master` = GitHub 当时 HEAD `ccdb3664`，标签 `lorekeeper-v1.0.0-beta.1` 指向发布源码 `f01b027b`。后续提交由平台按自己的间隔拉取。
- 手动“立即同步”有冷却：连续操作会返回 `BAD_REQUEST`（提示需等待约 117 分钟）。需要立刻核对时先等平台拉取或冷却结束，不要反复点击。
- 写配置没有公开 API：启用与“立即同步”通过仓库设置页完成（`/setting/mirror` → Pull）。本仓库脚本只读校验 SHA 与附件。
- 附件不进入 Git，平台拉取不会带来安装包；Release 资产仍按下节单独上传。

## 令牌

GitHub 仓库 Secret：`ATOMGIT_ACCESS_TOKEN`，从本机用户环境变量同名项通过标准输入交给 GitHub CLI 加密保存。

令牌不写远端 URL、Git 配置、源文件、日志或提交；不导出本机全部环境变量。应限制为此镜像仓库所需权限，失效后在 Secret 中轮换。不将本机令牌发送给下载代理或附件存储域名。

## 发布：GitHub 打包 → 平台拉取代码 → 单独上传附件

1. 打包流仍手动触发，沿用锁定源码、同 SHA Rust 全量、四个安装包和五个校验文件的门禁；发布作业只负责 GitHub 草稿、核验与公开，不再调用任何 AtomGit 同步。
2. GitHub 公开后，确认 AtomGit 平台已拉取到同一标签与提交 SHA；拉取不到时先在平台手动同步，不重跑打包。
3. 代码就绪后在本机或可信环境单独上传附件（不编译、不打包、不重新压缩）：
   ```powershell
   cd tauri-app
   $env:ATOMGIT_ACCESS_TOKEN = '<token>'
   $env:MIRROR_RELEASE_TAG = 'lorekeeper-v1.0.0-beta.1'
   $env:GITHUB_REPOSITORY = 'TanCan1998/Lorekeeper'
   pnpm mirror:release
   ```
   工具从 GitHub 下载已公开附件，校验发布清单、同 SHA Rust 记录、两端标签 SHA 与原始字节摘要，只补缺失附件，不覆盖已有文件。
4. 上传后匿名流式读回全部九附件的大小和 SHA256，再写回双源下载表格并读回确认；国内源在前，保留预览身份与限制。
5. 两平台无法原子公开：一头失败保留现状，不删除另一端、不覆写冲突附件、不把半完成状态登记为双端成功。
6. AtomGit 不提供已确认的草稿能力；它的 Release 接口也没有与 GitHub `make_latest` 等价的独立指针，不宣称两端 Latest 完全等价。

## 资源和验收边界

本轮用户只授权镜像配置、文档和已公开 beta 的说明更新；不构建、不重压缩、不发布 beta.1，也不解除安装器与真实原生验收门禁。仅脚本语法、YAML 和轻量差异检查不等于运行回归通过。镜像传输工作流不代替现有 CI。

参考：[AtomGit 镜像说明](https://docs.gitcode.com/v1-docs/docs/repo/mirrors)、[Release API](https://docs.gitcode.com/docs/apis/post-api-v-5-repos-owner-repo-releases)、[GitHub README 相对链接](https://docs.github.com/articles/relative-links-in-readmes)。
