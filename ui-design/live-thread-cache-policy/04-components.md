# 组件与开发 Handoff

## ThreadStore / Live Cache

职责：

- 维护每个 thread 的 loaded 状态、items、turn metadata 和 subscription metadata。
- 判断 thread focus 时走 cold start 还是 loaded cache。

状态：

- `missing`：无本地状态。
- `initializing`：正在 `thread/read`。
- `loadedLive`：已有 live items，可直接展示。
- `recovering`：显式恢复或本地状态损坏。

关键规则：

- `loadedLive` thread 切换时不得触发 `thread/read`。
- `subscribe` / `resume` 不得覆盖 `items` 或 `turns`。

## ThreadView

职责：

- 渲染当前 thread 的 typed `ThreadItem` 列表。
- 展示 loading、empty、error、connection 状态。

关键规则：

- 内容渲染只消费 typed `ThreadItem`。
- 不从 message text、marker、raw `ResponseItem` 或 JSON envelope 解析可见内容。
- loaded thread 切换不显示 full loading，不清空内容区。

## LiveReducer

职责：

- 处理 app-server v2 live notification。
- 将 lifecycle event 应用到明确的 state 分区。

事件规则：

- `item/started`：创建或更新可见 item。
- `item/completed`：完成可见 item，是显示内容最终 payload。
- `turn/started`：只更新 turn lifecycle metadata。
- `turn/completed`：只更新 turn lifecycle metadata，不合并 snapshot items。
- `thread/read` response：仅在 cold start、缺失本地 thread 或显式恢复时初始化。

## SubscriptionController

职责：

- 建立、恢复、重连 live subscription。
- 维护连接状态和 cursor。

关键规则：

- subscription 成功不代表内容刷新。
- resume 返回的 thread metadata 可更新 metadata，但不能覆盖 live `ThreadItem`。

## 验收用例

1. 打开本地不存在的 thread：调用 `thread/read`，初始化后显示历史 items。
2. thread A 收到 childCompletion，切到 thread B，再切回 A：childCompletion 仍存在。
3. loaded thread 切换期间没有 `thread/read` 请求。
4. `turn/completed` 到达时，不删除或替换已经由 `item/completed` 写入的 items。
5. subscribe/resume 后，内容区不闪烁、不清空、不重复 item。
6. subscription 失败时，已有 items 保留，只展示连接异常。

## 开发风险

- reducer 中如果仍有 snapshot merge helper，需限制调用入口。
- 如果 focus thread action 内部无条件 fetch/read，需要按 thread cache 状态分支。
- 如果 turn completed payload 仍被当作 authoritative content，需要拆分 metadata 更新与 item 更新。
- 测试应覆盖切换 thread 后 childCompletion 保留的回归路径。

