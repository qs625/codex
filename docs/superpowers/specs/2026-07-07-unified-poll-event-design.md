# Unified `poll_event` 设计

## 背景

当前 thread 内等待外部事件主要依赖两套专用工具：

- `wait_agent`
- `command_wait`

它们的问题不是 post-turn thread state，而是 turn 内 tool call 的等待语义：

- `wait_agent` 在一次等待窗口内主要只监听 child status / child mailbox 更新。
- `command_wait` 在一次等待窗口内主要只监听 command session 的 future notification。
- 如果当前 turn 正在等待某一类事件，其他外部输入虽然可能已经进入 thread，但不一定能立刻中断这次等待并唤醒模型。

这和目标体验不一致。对于模型来说，更自然的语义不是“我只等某一种事件源”，而是“当 thread 上出现新的可消费输入时唤醒我，我再决定怎么处理”。

## 目标

- 为 turn 内等待引入统一 `poll_event` primitive。
- 让 user input 和各种 runtime external event 都能唤醒当前等待。
- 不新增独立 event buffer，不复制 event payload。
- timeout/backoff 由 runtime 内置管理，且为 thread-scoped。
- 保留 `wait_agent` / `command_wait` 作为兼容 wrapper，逐步退化为 hint。

## 非目标

- 本次不修改 `ThreadPostTurnState` 的语义；它仍只负责 turn 结束后的 thread scheduler state。
- 本次不改变 display-capable `EventMsg` -> `ThreadItem` projector 体系。
- 本次不把 pending input / mailbox / schedule / command event 的持久化路径整体重构成新协议。
- 本次不要求一次返回多个事件，也不要求在 `poll_event` 结果里承载完整 payload。

## 原则

### 1. `poll_event` 等待的是“新的 thread input”

`poll_event` 的职责不是从独立 event queue 里取数据，而是等待：

- 新的 user input
- 新的 child completion / child communication
- 新的 command notification
- 新的 schedule trigger
- 新的 event command output
- 未来其他进入 thread 正常消费通道的外部输入

一旦这些输入已经成功进入 thread 的正常输入链路，`poll_event` 就可以结束等待并返回。

### 2. 不新增独立 event buffer

外部事件仍然沿用现有主路径：

- 正常进入 pending input / mailbox / typed event 注入链
- 正常被 display、history、compact、replay 消费

`poll_event` 不应维护一套脱离主路径的独立 buffer。否则会出现两套语义：

- 一套由 `poll_event` 消费
- 一套由 turn input 消费

这会导致重复、乱序和 context/display 解释分叉。

### 3. `poll_event` 不承载 payload

模型在下一轮会从正常输入里看到完整事件内容，因此 `poll_event` 返回值只需要表达：

- 本次是被新输入唤醒，还是超时返回
- 可选地提示一个 best-effort source
- 当前 wait window 是多少

不需要在 `poll_event` 结果里重复携带完整 payload。

### 4. user input 必须能唤醒

`poll_event` 不仅服务 runtime event，也必须服务用户输入。

否则 runtime 会继续存在两套唤醒模型：

- 某些输入能打断 wait
- 某些输入只能走别的路径

统一语义应为：任何新的可消费 thread input 都可以唤醒。

### 5. backoff 是 thread-scoped runtime state

timeout/backoff 属于等待机制本身，不应由模型负责记忆或调整。

默认策略：

- 每个 thread 维护一个统一 wait backoff state。
- 任意 `poll_event` 超时，推进同一个 backoff window。
- 任意新输入成功唤醒，reset backoff。
- 不按 `child:<id>` / `command:<id>` 等 waiting key 细分。

这更符合“thread 当前整体处于等待外部世界”的语义，也避免模型切换等待对象时出现反直觉 timeout。

## API 草案

```ts
poll_event({
  interests?: ("user" | "child" | "command" | "schedule" | "event_command")[],
  timeout_ms?: number
})
```

说明：

- `interests` 是可选 hint，不是硬过滤器。
- `timeout_ms` 是本次请求的可选覆盖值；默认 window 由 runtime 的 thread-scoped backoff policy 决定。

返回：

```json
{
  "status": "event" | "timeout",
  "source": "user" | "child" | "command" | "schedule" | "event_command" | "unknown" | null,
  "wait_timeout_ms": 30000
}
```

说明：

- `source` 是 best-effort hint，不要求和后续 input item 一一精确映射。
- 真正事件内容仍在正常 input 通道中消费。

## Runtime 行为

### 调用时

1. `poll_event` 先检查 thread 当前是否已经存在新的可消费 pending input。
2. 如果已经存在，则立即返回 `status = "event"`。
3. 如果不存在，则按当前 thread wait window 挂起等待。

### 等待中

runtime 监听“新的 thread input 已成功进入正常消费通道”这一事实，而不是只监听某一个专用子系统。

可触发唤醒的来源至少包括：

- user input 注入 active turn / next turn
- child completion 或其他 canonical typed inter-agent input
- command notification 进入 thread 的正常输入链路
- schedule trigger 进入 thread 的正常输入链路
- event command output 进入 thread 的正常输入链路

### 返回时

- 命中新输入：返回 `status = "event"`，reset thread-scoped backoff
- 超时：返回 `status = "timeout"`，advance thread-scoped backoff

## 与现有工具的关系

### `wait_agent`

保留为兼容 wrapper，但不再独占 child 事件源。

语义上等价于：

- 调用 `poll_event`
- 声明当前更关心 `child`
- 返回值文案保持 `wait_agent` 兼容形态

### `command_wait`

同样保留为兼容 wrapper，但不再独占 command 事件源。

语义上等价于：

- 调用 `poll_event`
- 声明当前更关心 `command`
- 返回值文案保持 `command_wait` 兼容形态

### `interests`

`interests` 只用于：

- planner / tool hint
- runtime 日志与 display 文案
- 将来如需做 source 优先级提示时的参考

`interests` 不应用于屏蔽 thread 上已经到达的新输入。

## Why Not

### 为什么不是单一 `wait_agent`

`wait_agent` 要求模型先预测“接下来最值得等的是 child”，但模型常常并不需要在等待前就做出这种硬绑定。

### 为什么不是单一 `command_wait`

`command_wait` 只覆盖 command session，不解决 child / schedule / user input 的统一唤醒。

### 为什么不是独立 event buffer

独立 buffer 会制造第二套消费语义，并与 pending input / history / replay / display 分叉。

### 为什么不在 `poll_event` 返回 payload

payload 已经会在正常 input 链路里出现；再次返回只会增加重复和 context 噪音。

### 为什么不一次返回多个 event

`poll_event` 只解决“何时唤醒”，不承担批量 drain。模型下一轮会在正常输入里消费实际内容，不需要 wait 结果再重复承载多条事件。

## 实现顺序

1. 在 runtime 内抽象“新的 thread input 到达”检测点。
2. 新增 `poll_event` tool spec 和最小 runtime 实现。
3. 让 `wait_agent` / `command_wait` 复用统一等待内核。
4. 收敛现有 `wait_agent_backoff` / `command_wait_backoff` 到 thread-scoped wait policy。
5. 更新 planner / builtin awaiter 提示词与 UI 文案。

## 风险

- 当前各类外部事件进入“正常消费通道”的时机并不完全统一，落地时需要先梳理哪些路径是 active-turn 注入、哪些路径是 next-turn pending input、哪些只是 display event。
- 如果某些事件今天只有 display 而没有模型可消费输入，`poll_event` 的统一语义会倒逼这些路径补齐 typed input 契约。
- `source` 只是 hint；如果后续产品强依赖精确 source 分类，可能需要单独补一个稳定 source taxonomy。
