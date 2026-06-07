# 客户端 child completion 与 Agent Status item 显示修复

## 任务 brief

用户反馈 root-worker prototype 客户端里 child completion / Agent Status item 完全不展示。成功标准是：

- 确认后端是否已经把 child completion / status 传给客户端。
- 当客户端收到 `collabAgentStatusUpdate` 时，该 item 能进入 thread state，并在 conversation 中可见。
- 后端只在 child thread 真正不再 active 时向 direct parent 发送 child completion；如果 child turn 结束但仍有 active descendant、active event command subscription、待处理 mailbox/input 或仍在 active turn 中，则暂不发送。
- active 条件解除后，后端会补发一次 child completion，且同一个 child 终态不会重复通知。
- 不为了修复可见性新增协议 item 类型。
- 不把 child completion 和 Agent Status 强行拆成新的产品分类；普通 multi-agent 工具显示规则不做无关重构。

非目标是不修改 TUI、不重做 multi-agent UI、不改变 app-server 协议，不改变 management agent 不向 parent 发送完成通知的约定。

## 探索结论

后端已经传了对应 item。app-server protocol 中 `ChildCompletion` 携带 status 时会映射为 `ThreadItem::CollabAgentStatusUpdate`，`item/completed` 通知也有覆盖该映射的协议测试。

renderer 侧 `conversation.ts` 已经能把 `collabAgentStatusUpdate` 构造成 tool entry。也就是说缺口不是协议类型缺失，也不是 conversation 完全没有渲染分支。

真实断点在 live 通知落 thread state 的目标线程上。`App.tsx` 原来对 `item/started` / `item/completed` 只更新通知参数里的 `threadId`。child completion / Agent Status 的通知可能从 child thread 流过来，但 item 自身的 `recipientThreadId` 是 root；因此 status item 被写到 child thread，当前 root conversation 完全看不到。

后续验证发现后端发送时机也存在问题：`Session::send_event` 原来在 child terminal turn event 刚发出时就立刻转发 completion 给 parent。此时 child 可能只是当前 turn 完成，但它的线程语义仍然 active，例如还有 running grandchild/subagent、注册中的 event command subscription，或者 runtime 尚未清理 active turn。这个时机早于“thread no longer active”，会让 parent 过早看到 child completed。

另一个相关缺口在 Electron 主进程的 snapshot 归一化逻辑。`electron/threadSnapshots.cjs` 会在 thread/read、恢复快照等路径中按语义内容合并 item；它把所有 `collabAgentStatusUpdate` 都当作可语义匹配 item。这样终态 Agent Status，例如两个不同完成事件但内容同为 `/root/worker completed done`，会在进入 renderer 前被合并，和 renderer `thread.ts` 中“终态 status 不做语义合并”的策略不一致。

## 技术设计

最小修复分两层：

- 在 renderer 状态层新增 item notification 目标线程解析：`collabAgentStatusUpdate` 和 legacy `collabAgentMessage.operation === "childCompletion"` 优先写入 `recipientThreadId`，普通 item 仍写入通知线程。
- 目标线程解析只在 child-origin 场景生效，即通知线程等于 item `senderThreadId` 且 `recipientThreadId` 指向另一个线程；否则保留通知线程，避免误投带 recipient 元数据的本线程事件。
- `App.tsx` 处理 `item/started` / `item/completed` 时使用该解析结果更新 thread state，使发给 root 的完成状态能进入 root conversation。
- `collabAgentStatusUpdate` 和 legacy child completion 的 `item/completed` 如果需要新建或更新 synthetic turn，该 turn 直接标记为 completed，并带上 started/completed 时间；这个判断基于 item 语义而不是是否发生重定向，覆盖 child-origin 与 direct-to-recipient 两种到达路径，避免 root conversation 因 synthetic running turn 长期显示 thinking。
- Electron snapshot 归一化层对齐 renderer 的合并策略。
- 运行中的 `collabAgentStatusUpdate` 仍可按语义合并，用于避免同一个非终态状态重复。
- 终态 `completed`、`errored`、`shutdown`、`notFound` 不参与语义合并，保留为独立 item。
- 不改 app-server 协议，不新增 item 类型。
- 不改变现有 conversation UI 结构；收到并保留下来的 status item 继续走已有 tool entry 渲染路径。

后端最小修复：

- `Session::send_event` 不再仅凭 terminal turn event 立即转发 child completion，而是在 agent status 已进入终态后调用统一的 final status 通知检查。
- final status 通知检查先确认 MultiAgentV2、当前 session 是 `ThreadSpawn` child、agent 非 management、状态为 final，并通过 per-session 原子标记保证同一 child completion 只发送一次。
- 发送前检查本 session 是否还有 queued response input 或 pending mailbox input。
- 线程 active 判定复用 `AgentControl` 暴露的 subtree active 查询：同一个子树内任一线程存在 active turn、active event subscription 或非 final lifecycle status，都认为 child completion 仍需等待。
- `Turn` runtime 在清理 `active_turn` 后再次触发 final status 通知检查，用于覆盖 terminal event 发送时仍 active、但 active 条件随后解除的生产路径。
- active event subscription 的 observer 在 active count 归零后通过 `ThreadManager` 触发同一个 final status 通知检查，用于覆盖 event command / schedule subscription 结束后 child thread 变 idle 的生产路径。
- direct parent 丢失、parent path 无法解析或 send 失败时不永久占用“已发送”标记，避免瞬时失败后无法重试。

## 测试设计

新增 renderer 状态层单元测试，覆盖通知来自 child thread、item `recipientThreadId` 指向 root 时，目标线程解析为 root，并且写入 root thread 后能生成可见 conversation tool entry、completed synthetic turn，且 `isThreadThinking` 为 false。

新增 fallback 单元测试，覆盖通知线程不是 sender 时不会被 recipient metadata 误投。

新增 direct-to-recipient 单元测试，覆盖 `item/completed` 已经直接以 root/recipient thread 到达时也创建 completed synthetic turn。

新增 Electron snapshot 单元测试，覆盖同一个 turn 中两个内容相同但 id 不同的终态 `collabAgentStatusUpdate`。期望归一化后两个 item 都保留，证明主进程不会再把 Agent Status completion 合并掉。

保留既有 renderer 测试：`conversation.test.ts` 已覆盖 `collabAgentStatusUpdate` 能构造成可见 conversation cell，`thread.test.ts` 已覆盖 renderer 侧终态 status 不被合并。

新增 core 单元测试，覆盖：

- child 有 pending mailbox/input 时不会发送 completion，等待直接 parent completion watcher 后改为 mailbox message。
- child 有 active event command subscription 时不会发送 completion；active count 清零并通过 thread manager 复检后补发。
- child 有 active grandchild/subagent 时不会发送 completion；grandchild 完成且不再 active 后补发。
- management agent 完成时不通知 parent。

## 风险

终态 status item 数量较多时会保留更多历史项。这符合 child completion / Agent Status 需要可追踪的目标；运行中状态仍按语义去重，避免重复刷新造成噪音。

后端 completion 延后后，parent 看到 child completed 的时间会更接近 thread active 语义，而不是 turn terminal event 语义。风险在于如果某类 active 条件未能正确解除，completion 会被延后；因此测试覆盖了 event subscription 和 descendant 两类容易过早完成的路径。
