# 第107次：Actions最短保留策略与历史制品清理

## 用户授权

用户要求尽量缩短Actions制品自动过期策略，并明确同意“全仓库默认及全部上传步骤1天”“立即清理全部已完成运行的Actions制品”。已说明仓库默认同时影响今后日志，旧Electron跨run推广只能在1天内取回依赖。

## 已实施策略

- 仓库默认从90天改为1天，API读回days=1；maximum_allowed_days=90是平台返回的允许配置上限，不伪称修改它。
- 8个工作流11个上传步骤显式1天：Tauri打包/Rust证据、两macOS资格流、Windows资格/诊断、更新E2E、推广包/溯源、Gitleaks。
- 没有新增定时任务/权限、改变触发器、跑产品发布或重打包；Release四安装包与五清单不受影响。
- 遗留工作流契约断言同步；新Node最短策略测试2项通过，根相关3文件34项通过，全部YAML解析/11上传步骤核验通过。根typecheck/lint及对应提交CI待登记。
- 策略见[Actions存储](../agents/actions-storage.md)。

## 历史清理在飞

确认清单共281个制品，431,516,412字节（约411.53MiB）。只处理这份审批快照，未包含后续新运行产生的制品。逐项再次查run=completed；进行中、未知或已过期对象跳过。

先核验Release ID416413071的九资产、当前标签3bce0445，以及本机最近Rust证据3bce0445/39122557567的validation/log/已校验archive。临时脚本 `.pi/tmp/prune-completed-artifacts.cjs` 与审计 `.pi/tmp/recovery/artifact-cleanup/audit.json` 不入库。清理限频，未修改缓存/运行/现存日志。

当前清理与提交CI进行中；最终删除数量/字节、跳过原因、剩余制品与Release读回及对应SHA验证结果待收口，不把“新保留策略”说成已追溯删除旧制品。
