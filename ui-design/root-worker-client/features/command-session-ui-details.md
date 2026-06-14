# Command Session UI Details

## 设计结论

Command Session 不新增独立页面。保留现有 conversation command cell + RightPanel Live Commands 结构，但补齐信息层级、状态规则和定位行为。

## 信息层级

Command cell 必须展示：
- command：完整命令，header 截断，details 展开显示完整文本。
- cwd：完整路径，header 可 trim。
- status：running / waiting / completed / failed / declined。
- duration：running 时显示已运行时长或 `Running`，完成后显示 duration。
- exit code：完成后展示；0 使用 neutral/success，非 0 使用 error。
- output 摘要：最后非空输出行，加截断说明；完整 output 不默认铺满页面。
- session 参数：initial wait、notify on、yield time alias、tty、max output tokens、sandbox/approval 参数。

如果 typed payload 暂时没有 session 参数，UI 不应从 raw message 反解；开发应先扩展 typed `ThreadItem.commandExecution` 或投影层。

## Live Commands 规则

- running / in progress：显示。
- waiting output / waiting exit：显示，状态文案分别为 `Waiting: output`、`Waiting: exit`。
- successful completed：自动从 Live Commands 移除。
- failed completed：保留为近期失败，文案 `Exit N`；建议保留到用户切换线程、刷新 thread analysis 或后续明确 dismiss。不要和 running 计数混淆。
- declined / approval failed：保留为近期失败/blocked，文案明确原因。
- 空态：`No live commands.`。
- 加载态：`Loading command activity...`。

## 点击定位

- RightPanel row 通过 `ThreadItem.id` 定位 command cell。
- 滚动到目标后高亮 1600ms 到 2400ms。
- 不展开 details，不改变 composer 焦点，不打断用户正在输入的 draft。
- 如果 conversation 虚拟列表尚未挂载目标，先通过 item id 找到 cell index，再滚动虚拟列表；禁止用 command 文本匹配。
- 如果目标不存在，row 进入 disabled/不可定位状态并显示 `Not in local view`。

## Notification Event

Output/exit notification 必须作为独立 typed conversation event 显示：
- `Command output notification`：展示关联 command、cwd、notify_on 值、收到时间、chunk 摘要。
- `Command exit notification`：展示关联 command、exit code、duration、收到时间。
- notification event 提供返回 command 的关联动作，使用同一个 typed command id。
- command cell 的 live tail 只表达当前聚合输出摘要；notification event 表达“为什么此时唤醒/通知模型或用户”。

RightPanel row 的 latest line 优先使用 latest typed notification 摘要；没有 notification 时退回 latest output tail。

## 可访问性

- LiveCommandRow 必须可键盘访问，Enter/Space 触发定位。
- status 不能只靠颜色表达，要有文本。
- 高亮目标需要非颜色线索，且不要造成布局位移。
- 长 command/cwd 使用 title 或 details 展开可读，不截断关键信息。
- notification event 可由屏幕阅读器读出其类型、关联 command 和状态。

## 原型资产

本次不需要高保真原型。已保留完整 Electron baseline 截图：

![baseline](../assets/baseline-command-session-2026-06-14.png)

## 开发 handoff

实现入口：
- `apps/root-worker-prototype/src/lib/conversation.ts`
- `apps/root-worker-prototype/src/components/Conversation.tsx`
- `apps/root-worker-prototype/src/lib/threadAnalysis.ts`
- `apps/root-worker-prototype/src/components/RightPanel.tsx`
- `apps/root-worker-prototype/src/components/ConversationVirtualList.tsx`
- typed protocol / projector 中的 `ThreadItem.commandExecution`

测试建议：
- conversation details 包含 session 参数。
- Live Commands running/waiting 显示，success completed 消失，failed 保留。
- RightPanel row 点击调用定位 handler。
- notification event 独立显示并能关联 command cell。
