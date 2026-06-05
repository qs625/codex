# 客户端 child completion 与 subagent notification item 显示修复

## 任务 brief

用户反馈客户端中 child completion 和 subagent notification 的 item 不显示。成功标准是这些 multi-agent 事件在客户端消息流中作为独立 item 可见，而不是被连续工具调用分组合并后藏进折叠详情。非目标是不修改 app-server 协议、不重做 multi-agent UI 样式、不影响普通命令和外部工具调用的分组展示。

## 现象与根因

app-server 已将 child completion / subagent notification 规范化为客户端可识别的 `collabAgentStatusUpdate`、`collabAgentMessage` 或从 event-driven envelope 派生的 multi-agent tool entry。客户端 `buildConversationItemEntries` 也能生成 `toolCategory: "multiAgent"` 的 entry。

实际不可见点在 `buildConversationCells`：连续 `kind: "tool"` 且 `toolCategory` 相同的 entry 会合并到一个 tool cell。child completion 和 subagent notification 都属于 `multiAgent`，因此会被折叠成同一个 card，列表摘要只显示第一条和 `and N more`，用户看不到每个 notification item。

## 技术设计

在客户端会话构建层做最小修复：

- 保留普通工具同类分组合并行为。
- 对 `toolCategory: "multiAgent"` 禁止合并，保证 spawn / message / child completion / status update 等协作事件各自成为独立 tool cell。
- 不改变 entry 结构、ThreadItem 类型、Electron normalize 逻辑或 app-server 协议。

这样改动面只在展示聚合规则，避免影响事件进入线程 state、历史读取或协议转换。

## 测试设计

新增 `conversation.test.ts` 单元测试，构造连续的 `collabAgentMessage` 和 `collabAgentStatusUpdate`，断言：

- conversation entries 仍是 multi-agent tool entry。
- `buildConversationCells` 生成两个独立 cell。
- child completion cell 的 `toolName` 独立可见。

## 风险

风险是 multi-agent 连续事件数量较多时消息流更长。但这类事件本身是用户需要回溯的协作状态，独立可见优先于折叠密度；普通工具调用仍保持原分组。
