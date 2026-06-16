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

## Goal Command Actions review

第一轮结论：未通过，需修正。

本轮待审范围：

- `features/goal-command-actions.md`
- `assets/goal-command-actions-prototype.svg`
- `00-brief.md`、`01-research.md`、`02-ue-flow.md`、`03-information-architecture.md`、`04-components.md`
- `components/goal-state.md`

已处理问题：

- 修正 `/goal p` 场景：空 subquery 时 `/goal <objective>` 第一；subquery 命中保留 subcommand 前缀时，优先选中对应 subcommand；SVG prototype 改为高亮 `/goal pause`。
- 定稿无参数 goal action 的触发模型：slash menu 的 Enter、Tab、鼠标点击只补全 command，不执行副作用；用户再次 Enter 后执行完整 `/goal pause`、`/goal resume`、`/goal cancel` 或 `/goal clear`。
- 统一 paused 状态：GoalStrip 至少展示 Resume 作为 primary action，Cancel 作为 secondary/overflow；GoalDetailPanel 展示 Resume 与 Cancel，并由 backend capability 控制 disabled reason。
- 补充 composer action feedback 的 `role=status` / `aria-live=polite` 要求。
- 补充 `/clear goal` 只作为搜索 alias 命中 `/goal cancel`，不改变 `/clear` 自身 root session clear 语义。

复审结论：通过，可进入开发。

复审后同步修正：

- 将 `components/goal-state.md` 的可访问性要求从“错误反馈使用 live region”扩展为“所有 action feedback 使用 `role=status` 或 `aria-live=polite`”。

## Goal ThreadItem Display review

第一轮结论：未通过，需修正。

本轮待审范围：

- `features/goal-threaditem-display.md`
- `assets/goal-threaditem-display-prototype.svg`
- `00-brief.md`、`01-research.md`、`02-ue-flow.md`、`03-information-architecture.md`、`04-components.md`

已处理问题：

- 补充 `GoalLifecycleEventCell` 的响应式收缩规则：badge 不压缩，title 单行截断，time 可下移到 meta 或隐藏到 `aria-label` / tooltip，objective preview 两行 clamp。
- 补充虚拟列表 handoff：实现两行 goal event 时需要更新 `conversationVirtualization` 高度估算或确保测量稳定修正，并覆盖搜索跳转、RightPanel recent event 跳转和 compact archive 内展示。
- 补充 RightPanel Recent event 的 keyboard 语义：button/link、Tab 聚焦、Enter/Space 触发、focus ring 和跳转后的 `aria-live=polite` 状态反馈。
- 补充 internal/bulk `get_goal` 不生成 visible conversation item 的噪音验收场景。
- 原型补充 responsive rules callout，并移除容易造成行尾重叠误解的长时间文案。

复审结论：通过，可进入开发。
