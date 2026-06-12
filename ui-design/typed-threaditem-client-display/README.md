# Typed ThreadItem 客户端展示收口

## Brief

root-worker prototype 展示 app-server v2 thread/live 内容时，应以 typed `ThreadItem` 为唯一展示输入。旧的 `agentMessage.text`、`eventDrivenTool.text`、compact replacement raw `ResponseItem.message` 中的 marker 或 JSON envelope 不能再被客户端反解成 display item。

## 设计结论

- typed `eventDrivenTool`、`eventDrivenToolCall`、`eventCommandCall`、`eventCommandEvent` 继续按现有 tool/event entry 展示。
- typed `collabAgentToolCall`、`collabAgentMessage`、`collabAgentStatusUpdate` 继续按现有 multi-agent / subagent system tool entry 展示。
- 普通 `agentMessage` 始终作为 assistant message 文本展示；如果上游仍漏出 legacy structured marker/envelope，状态层应过滤，不能转换成 typed 展示项。
- compact replacement history 只作为 archived model-context 检查内容，不作为 live/thread canonical display projection 来源。

## 开发 Handoff

- 移除或停用 `thread.ts`、`threadSnapshots.cjs` 中从 `<event_driven_tool>` marker 重建 typed display item 的逻辑。
- 移除 `conversation.ts` 中从 `agentMessage.text`、`eventDrivenTool.text`、compact replacement raw `ResponseItem.message` 解析 childCompletion envelope 的 UI projection。
- 保留所有 typed `ThreadItem -> ConversationEntry` 分支。
- 对旧 raw structured message 采用过滤策略：不展示 raw child-completion/subagent/event-command marker，也不在客户端反解为 typed item。

## Review

最终 UI/UE review 通过。无需 mockup 或截图资产：本任务收紧 projection contract，不改变视觉组件或布局；验收依赖 unit tests / projection tests。
