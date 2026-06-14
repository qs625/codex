# Review

## 当前状态

Composer Slash 菜单增量设计已完成独立 `@ui-ue-reviewer` 复审，通过。

## Composer Slash 菜单 review 记录

第一轮结论：未通过，需收敛范围。

已处理问题：
- 将本次验收收窄为把内置命令加入现有 Skills slash 菜单，不重做 composer 或 Skills 行为。
- Skills chip/payload 行为明确保持不变；metadata/短说明仅作为可选增强。
- slash 触发规则沿用当前 `trimStart()` 后首行 `/query`，不扩大到普通文本边界或多行任意位置。
- 内置命令最小 registry 只覆盖 `/clear`，`commandId` 对齐当前实现为 `clear`。
- `/clear` 菜单说明修正为真实后果：归档当前会话并新建 root；不再误写为清空 draft。
- 原型移除 `/compact` 残留，并将空态标注为状态示例。

通过结论：
- 当前设计可作为开发 handoff。
- 剩余非阻塞风险：`/clear` 继续无确认立即执行，菜单提高可发现性后仍有误触风险；本次通过真实文案和执行反馈缓解。
- 后续如果引入 `/compact` 或参数命令，需要新增 command registry、参数输入和执行反馈设计，不能从本次 `/clear` 规则直接推断。

晚到复审补充项已处理：
- `commandId` 已从设计假设的 `clear-current-root-session` 对齐到当前实现已有的 `clear`，避免文档和代码出现两套命名。
- Skills 分组说明已改为可选 metadata，不要求本次补 app-server metadata 透传。
- `/clear` 候选可见说明仍要求改成“归档当前会话并新建 root”，这是开发验收点；本次设计不直接修改实现代码。
