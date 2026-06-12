# 相关模式调研

## 调研结论

本次不需要外部竞品调研。问题属于实时会话 UI 的内部一致性策略，关键不是参考其他产品的视觉模式，而是明确事件流、缓存状态与展示权威来源的关系。

## 可复用设计模式

### Live-first cache

进入 live cache 后，UI 以增量 live payload 作为当前会话的显示事实。snapshot 只负责补齐 cold start，不再参与 loaded thread 的常规切换渲染。

适用原因：

- 避免旧 snapshot 或 history rebuild 覆盖更实时、更结构化的 live items。
- 保留 item lifecycle 的细粒度状态，例如 started、streaming、completed。
- 保持用户切换 thread 时的视觉连续性。

### Snapshot for initialization only

`thread/read` 应被视为初始化或显式恢复机制，而不是每次 thread focus 的刷新机制。

适用场景：

- 首次打开 thread。
- 本地 cache 缺失。
- 用户执行明确的恢复/重新加载动作。
- live subscription 断开且无法通过增量事件补齐。

### Lifecycle metadata separation

turn lifecycle 和 item lifecycle 的 UI 职责不同：

- `turn/started` / `turn/completed`：更新 thread 或 turn 的运行状态、时间、活动标记、结束原因。
- `item/started` / `item/completed`：创建、更新或完成可见内容项。

这能避免 turn snapshot 被误用为 item display payload。

## 风险

- 如果 subscribe/resume 仍携带 snapshot 并进入 reducer merge，可能重新引入 childCompletion 丢失。
- 如果 loaded thread 切换路径复用 cold start 逻辑，可能出现重复 item、顺序错乱或状态回退。
- 如果 reducer 仍保留 raw message fallback，可能绕开 typed `ThreadItem` 的 canonical source。

