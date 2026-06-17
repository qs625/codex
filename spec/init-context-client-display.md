# Init Context 客户端展示修复

## Brief

用户在 root-worker 客户端中新建会话后看不到 Init Context item。成功标准是：

- 后端启动时生成的 Init Context 作为 typed display item 出现在新会话 Conversation 中。
- `thread/read` / reload 后仍能从 persisted `EventMsg` replay 出同一个 `ThreadItem::InjectedContext`。
- item 内容包含 Agent file / developer instructions，而不是只显示 AGENTS.md。
- 不引入 raw marker、assistant JSON envelope、display-only `ResponseItem` 或 legacy parsing。

非目标：

- 不改变普通 `thread/started` notification 的空 turns 契约。
- 不重构 thread status、Conversation 分组或其它 display item family。

## 断点

core 已经在初始化上下文记录时发出 `EventMsg::ItemCompleted(TurnItem::InjectedContext)`，app-server-protocol 也能通过共享 `EventMsg -> ThreadItem` projector replay 成 `ThreadItem::InjectedContext`，root-worker 在拿到该 typed item 后也会生成 `ConversationEntry`。

缺口有两层：

- `thread/start` 创建 core thread 时 Init Context 事件已经发出，但 app-server listener 随后才 attach，发起客户端会错过 live `item/completed`。
- limited persistence 原先不保存 `ItemCompleted(InjectedContext)`，因此 `thread/read` / reload 和 start response 都无法从 canonical `EventMsg -> ThreadItem` replay 链路恢复该 display item。

## 设计

- rollout limited persistence 保存 `ItemCompleted(InjectedContext)`，让 Init Context 成为可 replay 的 typed display event。
- `thread/start` 在 listener attach 后，从 thread store 读取 rollout history 并 replay `ThreadItem::InjectedContext`。
- 复用现有 `build_api_turns_from_rollout_items` / `EventMsg -> ThreadItem` replay 链路，只保留 `ThreadItem::InjectedContext`。
- start response 的 `thread.turns` 填入这批初始 Init Context item，支持 createThread 立即展示。
- 对发起连接额外发送 typed `item/completed` notification，补齐启动期间错过的 live event。
- `thread_started_notification` 继续清空 turns，避免改变已有 notification 契约和 fork/start 测试预期。

## 风险

- 启动路径新增一次 thread store 读取；读取失败或没有 Init Context 时降级为空，不影响 `thread/start` 成功。persistent thread 的正常路径依赖 limited persistence 保存 Init Context event。
- 补发 notification 只针对发起连接和 `InjectedContext`，避免广播重复 item 或扩大其它历史 item 的 live replay 行为。
