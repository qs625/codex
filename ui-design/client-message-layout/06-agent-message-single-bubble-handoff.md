# 连续 agent message 单 bubble handoff

## 设计结论

连续普通 `agentMessage` 已经在 `ConversationEntry -> ConversationCell` 层被归入同一个 cell 时，视觉上也必须表现为一个 message bubble，而不是同一个 cell 内的多个独立 bubble。

换句话说，本需求的完成标准不是“virtual list 少了一行”，而是：

- 一个连续 agent message block 只有一个左侧头像、一个 header、一个外层 bubble surface。
- 多条 `ConversationEntry` 在该 surface 内按顺序作为 segment 展示。
- segment 之间保留轻量边界，例如 1px divider、紧凑 meta 或局部 spacing。
- 每个 segment 仍保留自己的文本、附件、streaming、error 和 hover/focus 操作上下文。

## 不合并的边界

以下内容继续打断单 bubble：

- 任意 user message；user 消息仍右对齐，且不和 agent 输出合并。
- tool / event / compact / archive cell。
- typed child completion / subagent notification；它们属于执行轨迹或协作通知，不伪装成普通 assistant 文本。
- 需要保留明确身份区分的不同 agent/assistant 作者。

## 推荐 DOM 语义

```text
article.message-row.message-row-agent[data-grouped="true"]
  div.message-avatar.agent
  div.message-main
    div.message-head
    div.message-cell-surface
      section.message-segment[data-entry-id="agent-1"]
      section.message-segment[data-entry-id="agent-2"]
```

不推荐：

```text
div.message-stack
  div.message-bubble
  div.message-bubble
```

这种结构虽然在同一个 `ConversationCell` 里，但用户看到的仍是多个独立气泡，会继续制造重复边框、重复背景和分段噪音。

## 视觉和交互要求

- 外层 surface 使用统一背景、边框、圆角和阴影；内部 segment 不再使用完整 bubble 样式。
- 第一段和后续段的文本阅读顺序不变，附件仍跟随所属 segment。
- 复制操作默认作用于当前 segment；如果后续要支持复制整个 grouped cell，应作为单独操作添加。
- streaming 时在当前 segment 内增量更新，不新增临时 bubble。
- grouped cell 的高度必须由外层 `article` 自然撑开，避免影响 virtual list 测量。
- 屏幕阅读器需要能感知 segment 边界，可使用隐藏文本或 `aria-label` 表达 “assistant message 1 of 2”。

## 原型和 baseline 说明

本次是对已完成 `client-message-layout` 设计的 handoff 澄清，没有改变页面结构、布局方向、视觉风格或状态模型，因此不新增原型图。继续引用既有视觉 handoff mock：

- [prototype-message-cell-layout.png](assets/prototype-message-cell-layout.png)

既有 brief 已记录 Electron baseline 自动化失败和 Vite renderer fallback 空白截图。实现 PR 中仍需要在 Electron 可用后补采真实 baseline / after 截图，验证 grouped agent cell 是否从多个 bubble 收敛为单个 surface。

## 开发验收

- `MessageRow` 对 grouped agent entries 渲染一个外层 surface。
- 单条 agent message 与 grouped agent message 的布局宽度、头像、header 位置保持一致。
- user message 右对齐不回归。
- tool、childCompletion、subagentNotification 不进入 message segment。
- 组件测试应覆盖 grouped agent entries 的 markup，避免再次出现一个 cell 内多个 `.message-bubble` 的回归。
