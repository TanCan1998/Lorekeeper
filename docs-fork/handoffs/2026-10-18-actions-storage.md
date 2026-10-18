# 第107次：Actions最短保留策略与历史制品清理

## 用户授权

用户要求尽量缩短Actions制品自动过期策略，并明确同意“全仓库默认及全部上传步骤1天”“立即清理全部已完成运行的Actions制品”。已说明仓库默认同时影响今后日志，旧Electron跨run推广只能在1天内取回依赖。

## 已实施策略

- 仓库默认从90天改为1天，API读回days=1；maximum_allowed_days=90是平台返回的允许配置上限，不伪称修改它。
- 8个工作流11个上传步骤显式1天：Tauri打包/Rust证据、两macOS资格流、Windows资格/诊断、更新E2E、推广包/溯源、Gitleaks。
- 没有新增定时任务/权限、改变触发器、跑产品发布或重打包；Release四安装包与五清单不受影响。
- 遗留工作流契约断言同步；新Node最短策略测试2项通过，根相关3文件34项通过，根typecheck/lint成功，全部YAML解析/11上传步骤及diff核验通过。
- 策略见[Actions存储](../agents/actions-storage.md)。

## 历史清理已完成

确认清单共281个制品，431,516,412字节（约411.53MiB）。只处理这份审批快照，未包含后续新运行产生的制品。逐项再次查run=completed；进行中、未知或已过期对象跳过。

先核验Release ID416413071的九资产、当前标签3bce0445，以及本机最近Rust证据3bce0445/39122557567的validation/log/已校验archive。临时脚本 `.pi/tmp/prune-completed-artifacts.cjs` 与审计 `.pi/tmp/recovery/artifact-cleanup/audit.json` 不入库。清理限频，未修改缓存/运行/现存日志。

- 审批快照 **281项全部删除，0跳过**，合计 **431,516,412字节（411.53MiB）**；逐项所属运行已完成，日志/run记录/cache没有删除请求。
- 清理期间先后出现进程中断及GitHub API EOF，按持久化审计从68、218项断点续跑完成，没有重复启动并发清理或再次删除已记录对象。审计保留中断信息及完成时间。
- 最终Release九个asset ID/名称/字节/摘要、tag/prerelease/publishedAt与清理前一致，标签仍精确指向3bce0445；用户下载不受影响。
- 清理收口时只剩本次策略提交产生的2个小型制品，共 **31,853字节**，未包含在历史审批快照；后续新CI可继续产生短期制品，不承诺固定只有2个。
- 两新制品实际API过期时间均为 **2026-10-19T11:58:14Z**，等于各自run创建时间2026-10-18T11:58:14Z加1天。Rust制品上传较晚，上传至到期不足完整24小时；初次按上传时间±6分钟检查不适用，已按实际run时间基准严格复核，不是放宽保留策略。
- 清理审计与Release读回：`.pi/tmp/recovery/artifact-cleanup/`。仓库用量面板可能延迟刷新，不能把未触碰的cache或Release空间计入释放量。

## 提交与CI

- 策略提交 **a54186141d2604b4670833fab70bb90fee7481cb** 已推送；Commit CI **39125354248**、Gitleaks **39125354263** 成功。
- 本次仅改变Rust工作流上传保留字段，没有产品Rust代码变化；工作流路径触发的独立Rust CI **39125354221** 已完成 **1680/0**（库276.55秒），all-targets成功（36.68秒）。
- artifact **31,526字节**，SHA256 `1be24fb991c89dfa06549faa6fcd79f605a4036a172c07e425b33f5b7642a5de`；同SHA/expectedSHA、run、Cargo.lock `37abf17d07d6cd96b3bc4187827333a9109198e021a5aa8b4a242d422a75907e`、两退出0、state=passed均核验，已及时下载本机 `.pi/tmp/recovery/ci-a5418614/`。最新全量基线更新，累计Rust语义切片0。
- 没有重打包、修改Release、触发前台窗口或真实模型/MCP/业务云调用。版本与公开安装包仍属于3bce0445 / 39122558975，不以CI保留期修改冒充新发布。
