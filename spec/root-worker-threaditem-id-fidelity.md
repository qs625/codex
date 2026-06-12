# root-worker ThreadItem id 保真

## Brief

root-worker prototype 展示 app-server v2 thread/live 内容时，typed `ThreadItem` 已经是展示层 canonical payload。客户端不能再通过 `agentMessage.text`、raw marker、legacy JSON envelope、semantic key 或内容相似度判断 item 是否重复。成功标准是同一个 `ThreadItem.id` 的 started/completed 生命周期更新可以合并；不同 `ThreadItem.id` 必须保留为不同 item，并至少生成一个 `ConversationEntry`。

## 设计

- state 写入层只按 `item.id` 查找已有 item。同 id 使用 `mergeThreadItem` 保留时间戳和更完整的 agent text；不同 id 直接追加。
- snapshot normalization 只合并同 turn 内同 id item；跨 turn snapshot/read 与 live/restored 合并时，只用 id 判断已有 item 是否已被新 snapshot 覆盖。
- Electron snapshot helper 与 renderer state helper 保持同一规则，避免冷启动/持久化恢复和 live reducer 产生不同展示结果。
- `ThreadItem -> ConversationEntry` 不返回空数组：empty reasoning 显示 reasoning fallback，未显式支持的 typed item 显示 unsupported fallback。
- `ConversationEntry -> ConversationCell` 只做视觉分组，不再根据 standalone notification 内容 key 丢弃 replacement/live entries。

## 非目标

- 不修改 app-server v2 API shape。
- 不改变 typed `ThreadItem` / v2 payload 作为展示源的协议约定。
- 不修复后端可能双发 childCompletion 的源头；本次只保证客户端不靠内容去重掩盖或丢 typed item。
- 不改变连续普通 agent message、普通 tool cell 等允许的 UI 分组，只保证 entries 保留。

## 风险

- 后端继续发送不同 id 但内容相同的 item 时，客户端会如实展示重复内容。这是 typed payload 保真的预期结果；如果重复本身是 bug，应在 projector 或后端事件源修复。
- `appendAgentDelta` 现在不再压制 marker-like 文本。如果服务端仍发送 legacy raw marker delta，客户端会按普通 agent message 展示，直到上游改为只发送 typed item。
