# Lorekeeper 旧版副本导入：输入核查与待确认边界

## 已确定范围

用户本轮选择：**仅考虑旧版 Lorekeeper，先核实实际旧格式**。
因此不读取 Electron 的 `.vela/vela.db`、LanceDB，也不把 Tauri 在 L3 隔离之前
曾使用的同名 `.vela` 自动认作 Lorekeeper 输入。

此页是源码与提交记录核查，不是对真实用户旧库的兼容性认证；未扫描用户项目或小说数据。

## 可核实的旧形态

1. `project_access.rs::probe_existing_project` 已有 Lorekeeper 旧库指纹识别：
   在本项目 `.lore/lorekeeper.db` 中检查必要表和 project_core 列，允许没有 project.json。
   现有 `adopt_legacy_project` 会在源目录写清单，**不能直接复用于副本导入**。
2. `db/schema.rs::migrate_project_core_legacy_columns` 已有作者配置字段的无损补列，
   包括从 synopsis/worldbuilding 继承 core_outline/world_setting；不会把角色派生文本复制成作者档案。
3. `migrate_character_roster_schema` 已有 provenance/meta 补列及旧卡片/旧图谱归档状态；
   旧图谱仅归档，不擅自解析成新作者事实。
4. 历史上还存在新增表的差异：
   m01生成账本（285d2284）、m02角色身份（641a12f6）、m03审稿（41940a16）、
   m05头像资产（d48e10ca）、便携源列集对账（ee9b0aa9）。
   这些提交证明结构曾有变化，**不等于当前 create_tables 能升级所有早期列集**。
5. `db/schema.rs::create_tables` 除建表/补列还围栏导入运行租约；
   因此不得在源库上调用，不得把“只读副本导入”退化为原地收养。

## 建议实施范围（尚未批准迁移）

- 源与目标均来自独立显式授权；源库和原始资产只读捕获，包含已提交 WAL 内容，
  捕获前后核对文件身份/大小/时间/哈希，拒绝变动与链接/越界路径。
- 仅在自有隔离副本上复用已有补表/补列及租约围栏；不新增 schema 迁移规则。
- 升级后逐表/字段与便携最终列集严格对账，无法无损识别的形态返回 blocked，
  不靠空表、丢字段或删数据凑通过。
- 有有效清单时保留原始来源 projectId；无清单时是否分配临时来源 UUID 需要明确确认，
  不能把新目标身份冒充历史来源身份。定稿收据、章节号与正文哈希保持不变。
- 最终复用已验证的脱敏、冻结证据、转移权威与无覆盖恢复安装服务，
  输出新的 .lore 项目；不会切换活动项目或修改源文件。

## 下一确认

是否允许在**隔离副本**上运行现有补表/补列迁移并按最终列集核验，
对无清单的已识别旧库生成明确标记的临时来源身份？
没有此确认，project:import-legacy-copy 仍不登记为迁移完成。
