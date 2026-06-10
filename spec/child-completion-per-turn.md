# Child Completion Active 边沿通知

## Brief

子线程完成后会通过 `ChildCompletion` 通知直接父线程。现有问题是 completion 去重绑定在整个 child session 生命周期上：同一个 child 第一次完成后，父线程后续通过 `followup_task` 唤醒该 child 执行新 work，第二次完成不会再通知父线程。

成功标准：

- child 第一次完成时按原时机通知 parent。
- 同一个 child 被 parent followup 唤醒并完成后，可以再次通知 parent。
- 同一轮 active->inactive 边沿只通知一次，重复 final status 或重复 retry 不重复发送。
- 当前 thread 派发给 direct child 的 work 未收到 child completion 前，当前 thread 仍视为 active，不能提前向自己的 parent 发送 completion。

非目标：

- 不改变 root-worker 客户端 UI。
- 不改变 collab item 展示协议。
- 不改变 parent mailbox 的消息 envelope。

## 技术设计

不把 completion 改成“每个 turn 都发送”。发送时机仍由原来的 final notification 路径控制：当前 thread final，且没有 pending mailbox、queued next-turn input、active event/subtree work 时，才可能向 direct parent 发送 `ChildCompletion`。

去重改为 active 边沿触发：

- `parent_child_completion_active` 记录上一次 completion 判断中该 thread 是否仍 active。
- 所有新 turn 的共享创建路径把该 latch 置为 `true`。
- completion 判断发现仍 active 时也保持 `true`。
- completion 判断发现 inactive 时，只有 `true -> false` 的 compare-exchange 成功才发送 completion；重复 retry 因 latch 已经是 `false` 不会重复发送。
- 发送失败时恢复为 `true`，允许后续 retry。

active 判断还包含 direct child completion 屏障：

- 当前 thread 对会产生 `ChildCompletion` 的 direct child 执行 `spawn_agent` 或 `followup_task` 时，在投递给 child 前按 child 增加 outstanding 计数；投递失败时回滚一个计数。
- management child 和 legacy 非 MultiAgentV2 child 不会产生 `ChildCompletion`，因此不登记 outstanding。
- 当前 thread 接收 direct child 的 `ChildCompletion` 时，先完成 mailbox/live item 入账，再递减一个 outstanding 计数；`close_agent` 会清空该 direct child 的所有 outstanding 计数。
- outstanding direct child 非空时，当前 thread 仍视为 active，不能向自己的 parent 发送 completion。
- 通过真正的 `ChildCompletion` mailbox 清理 outstanding 时，后续 trigger-turn 会重新走 turn 结束链路，由现有 final-status gate 完成判定；不在 mailbox 链路里的失败/close 清理路径，因为不会自然起后续 turn，需要在清掉最后一个 outstanding blocker 后立即补一次同样的 final-status 判定。

递归场景下，每个 thread 只维护自己的 direct child outstanding 计数表；孙子节点由它自己的 parent 维护。

## 风险

关键风险是 direct child completion 的清除时序。清除必须发生在 parent thread 已经完成 `ChildCompletion` 入账之后，否则 parent 可能在尚未真正收到 child completion 的边界上提前向自己的 parent 发送 completion。
