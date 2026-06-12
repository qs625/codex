# 展示 compact 后模型 history

## 任务 brief

root-worker prototype 在 compact 发生后，需要让用户明确看到 compact 边界，并能查看 compact 后模型实际使用的 replacement history。旧 conversation 仍可追溯，但不应继续表现为后续模型的完整活跃上下文。

成功标准：

- conversation timeline 能显示 compact 发生位置。
- compact 边界说明 compact 前历史已被替换为压缩后的模型上下文。
- compact 前内容在视觉和交互上可与后续内容区分。
- 用户能在 compact 边界后直接看到 `replacement_history` 展开的普通聊天/工具列表，其中普通 message、reasoning、function/tool call/output、context compaction 和 unknown item 都有可读展示或 raw fallback。
- 旧历史缺少 `replacement_history` 时不误导用户，只说明该 compact 事件没有可用 replacement history。

非目标：

- 不改变 compact 语义或模型实际使用的 history。
- 不实现 TUI compact history 展示。
- 不做无关聊天 UI 重构。

## 数据流设计

`RolloutItem::Compacted` 已包含 `replacement_history: Option<Vec<ResponseItem>>`，但当前 app-server v2 `ThreadItem::ContextCompaction` 只带 `id`。本改动将 v2 `ContextCompaction` 扩展为：

```text
ContextCompaction {
  id: String,
  replacement_history: Option<Vec<ResponseItem>>,
}
```

wire 上为 `replacementHistory`，没有 replacement history 时为 `null`。实时 `item/started` 还没有 compact 后模型上下文，继续生成 `replacementHistory: null`；实时 `item/completed` 必须携带 Local Compact 刚生成的 replacement history。从 rollout read/resume 重建时，`CompactedItem.replacement_history` 原样进入 thread item。

compact 完成后的 context usage 也必须把 Local Compact summary 计入 `compact` 类别。Local Compact 的 active history 中，summary 以带 `SUMMARY_PREFIX` 的 `user` message 形式存在；usage 分类层需要识别这个前缀并归入 compact，而不是普通 `user_messages`，这样 root-worker 的 context usage ratio 不依赖前端从 replacement history 文本反推。

## UI/UE 方案

conversation 聚合层将 `contextCompaction` 转为专用 compact cell，而不是普通 context tool card。遇到 compact 时，UI timeline 会被重写为：

```text
压缩前历史（默认折叠）
Context compacted 边界
replacement_history 转换出的普通聊天/工具列表
compact 后继续发生的新消息
```

compact cell 包含：

- 标题：`Context compacted`
- 说明：`Earlier conversation was replaced with compacted model context.`
- compact 时间。
- replacement history 数量状态。
- replacement history 数量或不可用状态。

replacement history 不放进 compact 详情 panel，而是复用已有 `ConversationEntry` 概念生成可读条目，并直接进入主 conversation cells：

- `message`：按 role 展示文本内容。
- `reasoning`：展示 summary/content 摘要。
- `function_call`、`custom_tool_call`、`local_shell_call`、`tool_search_call`、`web_search_call`、`image_generation_call`：展示为 tool 条目。
- `function_call_output`、`custom_tool_call_output`、`tool_search_output`：展示为 tool output 条目。
- `compaction`、`context_compaction`：展示为 compact summary/context 条目。
- unknown 或无法细化的 item：展示 type 和 raw JSON，不丢弃。

compact 前 segment 会进入 `Previous conversation` 折叠区域。该区域默认折叠，展开后仍复用普通 message/tool/event renderer，但文案明确说明这些内容已经不再是 compact 后模型的活跃上下文。

空状态：

- `replacementHistory === null`：显示此 compact 事件没有可用 replacement history。
- `replacementHistory.length === 0`：显示 compact 后模型上下文为空。

## 风险

replacement history 是模型实际上下文，可能包含系统、开发者或工具内容。此功能的目标正是让用户检查模型看到的 history，因此客户端不丢弃这些 item；但 UI 会用 compact boundary 明确说明这是模型上下文，不把它混作新聊天消息。
