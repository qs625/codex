# Conversation 搜索

## Brief

root-worker 客户端需要在当前 conversation 区域内搜索关键字。用户输入关键词后，客户端应在当前 `ConversationCell` 列表中查找可见文本和详情文本，显示当前匹配序号与总数，并支持上一条/下一条导航。导航时滚动到对应 cell，并给匹配 cell 清晰的高亮或选中状态。关键词为空时清空搜索状态。

非目标：

- 不改变 `ThreadItem -> ConversationEntry -> ConversationCell` 展示链路。
- 不改 `ThreadItem.id` 合并规则，也不按文本、状态或 legacy marker 对 item 做去重。
- 不从 raw marker、assistant message JSON 或 legacy envelope 解析搜索内容。
- 不做逐字文本替换高亮；当前版本以 cell 级命中和当前命中高亮为最小可验收行为。

## 设计

搜索状态维护在 `ConversationPanel` 层，并由 `conversationCells` 派生匹配结果：

- `conversationSearchQuery` 保存用户输入。
- `buildConversationSearchResults(cells, query)` 返回按当前展示顺序排列的匹配结果。
- 每个结果携带 `cellId`、`entryId`、`matchIndex` 和文本来源，导航时通过现有 `ConversationVirtualList.focusedItem` 定位到 entry 所在 cell。
- `activeSearchIndex` 表示当前命中；当 query 或 cells 变化时，优先保留原 active match 的 entry/source/range，不能保留时落到第一条结果。

搜索文本来源：

- `entry.text`
- `entry.toolName`
- `entry.toolStatus`
- `entry.toolDetails`
- `entry.attachments` 的 label/url/path
- compact/archive 中的 replacement history 与 archived cells 递归文本

UI：

- 搜索控件放在 conversation header actions 中，与现有 thread action 相邻。
- 展开后显示输入框、`当前 / 总数`、上一条、下一条、清空按钮。
- `Cmd+F` / `Ctrl+F` 聚焦搜索框；`Enter` 下一条；`Shift+Enter` 上一条；`Esc` 清空，空输入时收起。
- 无结果时显示 `0 / 0`，导航按钮 disabled。

## 风险

- 详情文本命中时，若 tool detail 未展开，当前版本只能把对应 cell 定位和高亮；后续可扩展为自动展开命中详情。
- 超长 conversation 每次 query 变化会同步扫描全部 cells；当前用 `useMemo` 限定重算范围，后续如有性能问题再引入索引。

## 验证

root-worker prototype 使用 `tsx --test` 执行前端 `node:test` 用例。本功能新增：

- `conversationSearch.test.ts`：覆盖搜索结果计算、详情/附件/replacement/archive 文本、不同 id 不去重、上一条/下一条循环导航。
- `Conversation.test.tsx`：覆盖虚拟列表 row 的 search match/current 高亮 class。
