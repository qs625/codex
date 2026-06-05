# 客户端 child completion 与 subagent notification item 显示修复

## 任务 brief

用户反馈 root-worker prototype 客户端不显示 child completion 和 subagent notification 的 item。成功标准是：

- `spawn_agent`、`send_message` 等普通 multi-agent tool entry 仍可按既有规则合并展示。
- child completion 与 subagent notification 不再混用普通 `multiAgent` 展示分类。
- child completion 与 subagent notification 在 conversation 中作为独立可见 item 展示，不被普通协作工具分组折叠吞掉。

非目标是不修改 app-server 协议、不重做 multi-agent UI、不改变普通命令和外部工具调用分组行为。

## 现象与根因

app-server 已经能把协作事件送到客户端，客户端也已有 `collabAgentMessage` 与 `collabAgentStatusUpdate` 类型。历史中的 child completion envelope 还会从 `agentMessage` 或 `eventDrivenTool` 解析为 `collabAgentMessage`。

真实问题发生在客户端 conversation 展示聚合层：这些 item 进入 `buildConversationItemEntries` 后都被标记为 `toolCategory: "multiAgent"`。`buildConversationCells` 会把连续同类 tool entry 合并到一个 tool cell，`ToolRow` 折叠摘要只展示第一条和 “and N more”。因此 child completion / subagent notification 实际进入了客户端 state，但在展示上被普通 multi-agent 分组遮蔽，看起来像“没显示”。

上一版“禁止所有 `multiAgent` 合并”的策略不符合需求，因为普通 `spawn_agent`、`send_message` 等 multi-agent tool entry 本来就可以合并。

## 技术设计

在客户端 conversation 构建层引入更细的展示分类：

- 普通协作工具调用继续使用 `toolCategory: "multiAgent"`。
- `collabAgentMessage.operation === "childCompletion"` 使用 `toolCategory: "childCompletion"`。
- `collabAgentStatusUpdate` 使用 `toolCategory: "subagentNotification"`。
- `buildConversationCells` 恢复普通同类 tool entry 合并，但对 `childCompletion` 和 `subagentNotification` 保持逐 item 独立 cell。
- UI 图标与颜色沿用 multi-agent 视觉体系，避免引入新的视觉语言。

这样修复只改变客户端展示分类与聚合规则，不改变 ThreadItem 类型、Electron normalize 逻辑或 app-server 协议。

## 测试设计

`conversation.test.ts` 覆盖三类行为：

- 普通 `spawnAgent` / `sendInput` multi-agent tool entry 仍合并到一个可见 tool cell。
- child completion 与 subagent notification 生成独立 category，并分别成为独立 tool cell。
- 从 event-driven tool / agent message 解析出的 child completion envelope 使用 `childCompletion` category。

## 风险

child completion 和 subagent notification 数量较多时消息流会更长；这是为了保证子任务完成和状态回流可追踪。普通 multi-agent 工具 entry 仍保留分组合并，因此不会把所有协作工具都展开。
