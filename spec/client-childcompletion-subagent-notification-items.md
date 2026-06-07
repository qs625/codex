# 客户端 child completion 与 Agent Status item 显示修复

## 任务 brief

用户反馈 root-worker prototype 客户端里 child completion / Agent Status item 完全不展示。成功标准是：

- 确认后端是否已经把 child completion / status 传给客户端。
- 当客户端收到 `collabAgentStatusUpdate` 时，该 item 能进入 thread state，并在 conversation 中可见。
- 不为了修复可见性新增协议 item 类型。
- 不把 child completion 和 Agent Status 强行拆成新的产品分类；普通 multi-agent 工具显示规则不做无关重构。

非目标是不修改 TUI、不重做 multi-agent UI、不改变 app-server 协议。

## 探索结论

后端已经传了对应 item。app-server protocol 中 `ChildCompletion` 携带 status 时会映射为 `ThreadItem::CollabAgentStatusUpdate`，`item/completed` 通知也有覆盖该映射的协议测试。

renderer 侧 `conversation.ts` 已经能把 `collabAgentStatusUpdate` 构造成 tool entry。也就是说缺口不是协议类型缺失，也不是 conversation 完全没有渲染分支。

真实断点在 live 通知落 thread state 的目标线程上。`App.tsx` 原来对 `item/started` / `item/completed` 只更新通知参数里的 `threadId`。child completion / Agent Status 的通知可能从 child thread 流过来，但 item 自身的 `recipientThreadId` 是 root；因此 status item 被写到 child thread，当前 root conversation 完全看不到。

另一个相关缺口在 Electron 主进程的 snapshot 归一化逻辑。`electron/threadSnapshots.cjs` 会在 thread/read、恢复快照等路径中按语义内容合并 item；它把所有 `collabAgentStatusUpdate` 都当作可语义匹配 item。这样终态 Agent Status，例如两个不同完成事件但内容同为 `/root/worker completed done`，会在进入 renderer 前被合并，和 renderer `thread.ts` 中“终态 status 不做语义合并”的策略不一致。

## 技术设计

最小修复分两层：

- 在 renderer 状态层新增 item notification 目标线程解析：`collabAgentStatusUpdate` 和 legacy `collabAgentMessage.operation === "childCompletion"` 优先写入 `recipientThreadId`，普通 item 仍写入通知线程。
- `App.tsx` 处理 `item/started` / `item/completed` 时使用该解析结果更新 thread state，使发给 root 的完成状态能进入 root conversation。
- Electron snapshot 归一化层对齐 renderer 的合并策略。
- 运行中的 `collabAgentStatusUpdate` 仍可按语义合并，用于避免同一个非终态状态重复。
- 终态 `completed`、`errored`、`shutdown`、`notFound` 不参与语义合并，保留为独立 item。
- 不改 app-server 协议，不新增 item 类型。
- 不改变现有 conversation UI 结构；收到并保留下来的 status item 继续走已有 tool entry 渲染路径。

## 测试设计

新增 renderer 状态层单元测试，覆盖通知来自 child thread、item `recipientThreadId` 指向 root 时，目标线程解析为 root，并且写入 root thread 后能生成可见 conversation tool entry。

新增 Electron snapshot 单元测试，覆盖同一个 turn 中两个内容相同但 id 不同的终态 `collabAgentStatusUpdate`。期望归一化后两个 item 都保留，证明主进程不会再把 Agent Status completion 合并掉。

保留既有 renderer 测试：`conversation.test.ts` 已覆盖 `collabAgentStatusUpdate` 能构造成可见 conversation cell，`thread.test.ts` 已覆盖 renderer 侧终态 status 不被合并。

## 风险

终态 status item 数量较多时会保留更多历史项。这符合 child completion / Agent Status 需要可追踪的目标；运行中状态仍按语义去重，避免重复刷新造成噪音。
