# Actions存储与最短保留策略

2026-10-18用户明确同意：仓库默认制品/日志保留期及所有制品上传步骤统一为 **1天**，并立即清理已完成运行的历史Actions制品。

## 自动过期

- 仓库设置 `actions/permissions/artifact-and-log-retention` 的 `days=1`，已API写入并读回。
- 当前8条含上传步骤的工作流、共11个 `actions/upload-artifact` 步骤均显式 `retention-days: 1`，包括Tauri打包/Rust证据与遗留Electron资格/诊断/推广/Gitleaks制品。
- 1天是GitHub原生制品保留的最短天数；使用平台自动过期，不增加定时删除工作流或扩大GITHUB_TOKEN权限。实际API到期时间以所属run创建时间加1天计算，上传发生在其后，因此不保证上传后仍有完整24小时。
- 默认设置同时影响今后运行日志；用户已确认。更改保留期不追溯既有制品，因此历史存量另行清理。
- 若从旧引用运行旧工作流，须重新核查保留期；日常按master当前策略执行，不能假设旧标签文件已被改写。

## 发布和验证边界

- Release assets不属于Actions临时artifact。公开发布的四安装包与五清单继续保留，不受1天策略或本次清理影响。
- 同run跨job传递仍正常，但必须在产物过期前完成；跨run手动资格推广也须在1天内操作。若依赖已过期，重新执行完整资格/打包流程，不伪造或替换收据。
- 重要Rust验证证据须及时下载到本机隔离证据目录；保留SHA、run、artifact摘要和退出码。云端过期后不声称原日志仍可下载。
- 本次历史清理只删除审批清单中的Actions制品；逐项再次核验所属run为completed，无法证明则跳过。保留Release、缓存、运行记录及现存日志；不自动删除进行中运行制品。
- 清理前核验当前Release九资产与本机已保存的最近Rust全量证据，清理后再核验Release资产/标签未变。
- 删除后的API计数和计费/存储用量面板可能存在刷新延迟，不能把缓存或Release空间算作本次释放量。

## 自检

```powershell
node --test tauri-app/scripts/artifact-retention.node-test.mjs
```

此测试扫描全部工作流上传步骤，拒绝缺省、14/90天、动态保留值和重复配置。修改遗留工作流时同步相关契约断言，纯保留期变化不代表产品Rust或GUI重新验收。
