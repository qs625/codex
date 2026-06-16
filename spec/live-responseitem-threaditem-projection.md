# Live ResponseItem 到 ThreadItem 投影修复

> 状态：已废弃。本文保留为历史记录，当前实现以
> [EventMsg 作为 ThreadItem 展示源](eventmsg-threaditem-display-source.md) 为准。

## 当前结论

早期方案尝试让 `ResponseItem` 同时承担模型上下文、rollout history 和 UI display
投影来源。这个方向已经放弃。

当前边界是：

- `ResponseItem` 只用于模型交互、context manager/provider history、compact、
  guardian 和模型可见工具输出。
- `EventMsg` 是 runtime/UI display source。
- `ThreadItem` 只从 display-capable `EventMsg` 投影生成。
- `RolloutItem::ResponseItem`、`RawResponseItem`、`ResponseItemStarted/Completed`
  不再用于 thread/read 或 live display 重建。
- 公开 `TurnItem -> ThreadItem` adapter 已删除；旧 `ItemStarted/ItemCompleted(TurnItem)`
  只能在 app-server protocol 边界通过私有 EventMsg projector 适配。

## 后续规则

新增会话展示语义时，不要扩展 `ResponseItem` display projector，也不要解析
assistant 文本、raw marker 或旧 rollout response item。需要 UI 可见的事实必须新增
dedicated typed `EventMsg`，再在 `event_item_projection.rs` 中投影为 `ThreadItem`。

如果同一个事实还需要模型可见，使用双写 helper：写入 model-visible
`ResponseItem`，同时 emit display-capable `EventMsg`。两条路径在边界上保持分离。
