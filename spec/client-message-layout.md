# 客户端消息布局调整

## Brief

root-worker prototype 的会话视图需要更接近聊天布局：连续普通 agent/assistant 消息合并为一个可读 cell，user message 在右侧展示。成功标准是用户消息有明确右侧视觉锚点，连续 agent 输出不被拆成多个割裂 cell，同时不改变 app-server v2 协议和 typed `ThreadItem` 消费路径。

## 现状

- `apps/root-worker-prototype/src/lib/conversation.ts` 以 typed `ThreadItem` 为输入，转换为 `ConversationEntry` 后再由 `buildConversationCells` 分组。
- 连续普通 agent message 已由 `shouldMergeConversationEntry` 合并，且会在 `isReplacementHistory` 不同时断开。
- tool/event/multi-agent notification 等非普通消息走 `tool` cell；`childCompletion` 和 `subagentNotification` 是独立通知边界，不参与普通 agent message 合并。
- `MessageRow` 当前没有 role class，user 和 agent 使用同一左侧布局。

## 设计

- 保持数据流不变：继续以 typed `ThreadItem -> ConversationEntry -> ConversationCell` 为 canonical display path，不解析 raw marker 或 assistant JSON。
- `MessageRow` 增加 role 级别 class，让 user row 右对齐，agent row 保持左对齐。
- user row 使用反向 flex 布局，头像在右侧，正文区域右对齐；message stack 最大宽度限制在约 70%，窄屏提升到 90%，避免长文本横向溢出。
- user bubble 使用现有中性色 token 的轻微背景差异，不新增解释性文案。
- 用聚焦测试覆盖：
  - 连续普通 agent message 合并为一个 message cell。
  - 遇到 user、tool 或 standalone notification 时不跨边界合并。
  - user/agent row 输出对应 role class，供 CSS 布局稳定命中。

## 非目标

- 不修改 app-server v2 protocol。
- 不改变 thread/live cache 和 `thread/read` 使用语义。
- 不反解 `<event_command>`、`<subagent_notification>` 或其他 raw marker。
- 不引入虚拟列表、折叠长消息或大范围视觉重构。

## 风险

- 多条长 agent 消息合并后单个 cell 可能更高；当前虚拟列表已有高度测量，后续如果出现阅读负担再考虑折叠或分段导航。
- 右对齐 user bubble 的深色主题对比度需要沿用现有 CSS token，避免局部配色脱节。
