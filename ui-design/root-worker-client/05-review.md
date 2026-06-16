# Review

## 历史 review 记录

Composer Slash 菜单增量设计已完成独立 `@ui-ue-reviewer` 复审，通过。

已处理问题：

- 将 slash menu 验收收窄为把内置命令加入现有 Skills slash 菜单，不重做 composer 或 Skills 行为。
- Skills chip/payload 行为明确保持不变；metadata/短说明仅作为可选增强。
- slash 触发规则沿用当前 `trimStart()` 后首行 `/query`，不扩大到普通文本边界或多行任意位置。
- 内置命令最小 registry 覆盖 `/clear`，`commandId` 对齐当前实现为 `clear`。
- `/clear` 菜单说明修正为真实后果：归档当前会话并新建 root；不再误写为清空 draft。

剩余非阻塞风险：`/clear` 继续无确认立即执行，菜单提高可发现性后仍有误触风险；设计通过真实文案和执行反馈缓解。

## Slash Goal Display review

第一轮结论：未通过，需修正。

已处理问题：

- 补充取消状态模型：`cancelling` 可由 keyed local action pending 或 typed `goal/cancelRequested` lifecycle item 驱动；`cancelFailed` 必须来自 action error result 或 typed lifecycle item，不能解析 assistant text。
- 固定 `/init` 不作为 root-worker builtin command；它来自 system skill discovery。
- 修正原型：Agent Tree 不展示 goal 状态，避免和 canonical `ThreadStatus` 混淆；header cancel 控件改为 icon button 形态。
- 补充响应式折叠规则：先隐藏 budget detail，再截断摘要；cancel icon button 保持固定命中区域，不能被文本挤压。

复审结论：通过，可进入开发。

复审后同步修正：

- 原型中移除 `/goal init` runtime command 表达，保留 `/goal cancel`。
