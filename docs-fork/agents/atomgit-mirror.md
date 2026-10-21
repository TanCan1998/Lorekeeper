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
- 首次发布作业与第一次镜像重试均在单请求 600 秒上限超时（GitHub runner 跨境上传 AtomGit 存储）；按协议只补缺失附件后重跑镜像成功，未覆盖已有文件。超时上限未调整，后续大包发布需关注。
- 中文首页只读抓取未出现「本 Fork 的目的」段落与新版本链接，首页完整渲染仍未验收。

## 代码自动同步

工作流：`.github/workflows/atomgit-mirror.yml`。

- GitHub 的分支推送、LoreKeeper 标签推送、Release 公开或编辑触发；也可手动执行。
- 只从 GitHub 抓取分支和标签，向 AtomGit 普通推送；不编译、不打包、不启用 AtomGit 自身的构建流水线。
- 不强推、不自动删除远端引用。遇到 AtomGit 独立提交、标签冲突或非快进更新会失败，保留两端内容并人工处理。
- GitHub 删除的分支不会自动删掉 AtomGit 分支，历史重写也不自动传播。这是防破坏同步，不是带删除能力的 `git push --mirror`。
- 当前使用 GitHub 推送式同步，不声称 AtomGit 平台的定时拉取开关已开启。
- 各工作流从可信主分支加载镜像脚本，不运行外部 PR 代码，不把 Secret 暴露给 Fork PR。

## 令牌

GitHub 仓库 Secret：`ATOMGIT_ACCESS_TOKEN`，从本机用户环境变量同名项通过标准输入交给 GitHub CLI 加密保存。

令牌不写远端 URL、Git 配置、源文件、日志或提交；不导出本机全部环境变量。应限制为此镜像仓库所需权限，失效后在 Secret 中轮换。不将本机令牌发送给下载代理或附件存储域名。

## 发布和恢复

1. 原打包流继续手动触发，沿用锁定源码、同 SHA Rust 全量、四个安装包和五个校验文件的门禁。
2. GitHub 草稿与资产核验、公开逻辑不变；发布作业显式调用代码同步与 Release 镜像，避免 `GITHUB_TOKEN` 创建的事件不触发后续工作流。
3. 已公开 Release 也可以由镜像工作流处理：手动填 `release_tag`，不再次打包。没有新标签或新版本授权。
4. 检查原始发布清单、同 SHA 独立 Rust 成功记录、GitHub 原始附件 digest 和两端标签 SHA；仅原样传输，不重新压缩或签名。
5. AtomGit 不提供已确认的草稿能力，传输期间明确显示未完成。上传后匿名流式读回全部九附件的大小和 SHA256，不仅检查附件列表。
6. 校验通过后，两端更新为相同双源表格，并读回确认。国内源排在前面；保留版本限制、签名说明及预览身份。
7. 两平台无法原子公开：一端失败，任务失败并保留现状，不删除另一端、不覆写冲突附件、不把半完成状态登记为双端成功。
8. 重试只补缺失附件；已有附件必须读回核验，字节不符就失败，不自动替换。修复后重跑镜像工作流，不重跑打包/发布新标签流程。
9. AtomGit 的 Release 状态接口与 GitHub 独立 `make_latest` 选项不同；保留预发布属性，不宣称两端 Latest 指针完全等价或自动更新分发已迁移。

## 资源和验收边界

本轮用户只授权镜像配置、文档和已公开 beta 的说明更新；不构建、不重压缩、不发布 beta.1，也不解除安装器与真实原生验收门禁。仅脚本语法、YAML 和轻量差异检查不等于运行回归通过。镜像传输工作流不代替现有 CI。

参考：[AtomGit 镜像说明](https://docs.gitcode.com/v1-docs/docs/repo/mirrors)、[Release API](https://docs.gitcode.com/docs/apis/post-api-v-5-repos-owner-repo-releases)、[GitHub README 相对链接](https://docs.github.com/articles/relative-links-in-readmes)。
