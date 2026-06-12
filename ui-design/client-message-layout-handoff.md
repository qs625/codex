# 客户端消息布局 UE/UX Handoff

## 目标

root-worker prototype 的会话视图需要把连续普通 agent/assistant 输出显示为同一组消息 cell，并把 user message 放到右侧，形成清晰的双向聊天布局。

## 布局规则

- agent/assistant 消息保持左侧展示，头像在左，连续普通消息合并在同一个 cell 内。
- 合并后的 agent cell 内部保留轻微垂直间距，让多段输出仍可区分。
- user message 右侧展示，头像在右，正文内容右对齐。
- user bubble 最大宽度建议为桌面约 68%-72%，移动端约 86%-90%，避免短消息横跨整行，也避免长消息溢出。

## 合并边界

- 合并以 typed `ThreadItem` 转换后的 semantic entry 为准。
- 只合并连续普通 `agentMessage`。
- 遇到 user message、tool/event/schedule、childCompletion、subagentNotification、错误或状态类 item 时断开；当前客户端按已展平的 typed entry 序列合并，不把 turn 本身作为额外断点。
- 不从 raw marker、assistant text JSON 或 legacy envelope 反解展示项。

## 响应式与可读性

- 桌面端 user row 右侧留出明确边界，agent row 维持现有左侧扫描节奏。
- 窄屏下 user bubble 可放宽到 90% 左右，但仍保持右对齐和头像右侧锚点。
- 长单词、代码块和附件应继续由现有 markdown/attachment 样式处理，不新增遮挡或横向覆盖。

## 开发 Handoff

- 在 `MessageRow` 上增加 role class 或 data attribute，例如 `message-row-user` / `message-row-agent`。
- CSS 对 `.message-row-user` 使用反向 flex、右对齐的 `.message-main` 和 `.message-stack`。
- 保留 agent 默认布局，避免影响 tool、compact、archive rows。
- 不新增解释性 in-app 文案。

## 剩余 UX 风险

- 多条长 agent 消息合并后 cell 变高，未来可能需要折叠或分段导航。
- 深色主题或高对比模式如后续引入，需要重新检查 user bubble 背景与文本对比度。
