# Pending Input And Post-Turn Design

## Goal
- 给 `pending input` 写入与 `on_task_finished()` 收尾定义一个最小、闭合的并发模型。
- 保证异步 event 不丢失，且不会因为旧 turn 收尾 race 而错过下一轮 turn。
- 不引入新的持久 `PostTurn` 状态，也不让 mailbox 再承载“属于当前 turn 还是下一个 turn”的语义。

## Non-Goals
- 不精确记录某条 input 最终由“当前 turn”还是“下一轮 turn”消费。
- 不为 command / child / goal 增加新的并行状态机。
- 不要求 event arrival path 自己理解 turn 生命周期细节。

## Design Summary
- mailbox 只保留“线程级异步输入入口”的职责，不再区分 current-turn / next-turn delivery。
- 所有 pending input 写入路径与 `on_task_finished()` 的最后收尾，共用 `active_turn` 这把同步锁来决定：
  - 当前有 active turn：根据 turn-local gating 决定写入当前 `turn_state.pending_input`，还是继续留在线程级 mailbox
  - 当前 idle：把输入保留在线程级 pending 容器，并在锁外启动新 turn
- `on_task_finished()` 在清理最后一个 task 时，也必须获取同一把锁，原子完成：
  - 取走当前 turn leftover pending input
  - 检查线程级 pending queue / mailbox 是否已有待处理工作
  - 清掉旧 active turn
  - 产出锁外副作用所需的 next-step 决议
- 如果收尾时发现仍有 pending input，则跳过 goal continuation / final-status completion 这类“线程已空闲”的后续逻辑，转而让下一轮 turn 启动。

## Why Change The Current Model
- 现在的 `MailboxDeliveryPhase::CurrentTurn/NextTurn` 会把“mailbox 是否被当前 turn drain”编码成 turn 内部相位，导致：
  - event 到达路径需要理解 answer boundary
  - 旧 turn 结束后没有统一的自动补拉起保证
  - mailbox 中有 pending work，但 `active_turn` 清掉后未必立即触发下一轮
- 这次收敛的方向是把 correctness 收口到一个事实：
  - 只要某条 input 已经进入线程，就必须在同一把锁保护下，最终落到“当前 turn 可见”或“下一轮必起”两者之一

## Core Invariants
- `active_turn` 只表示“当前是否有 turn 持有执行权”，不再隐含 mailbox delivery phase。
- 每条 pending input 在任一时刻只能属于一个位置：
  - 当前 active turn 的 `turn_state.pending_input`
  - 线程级 pending queue
- mailbox 如果保留实现形态，也只应是线程级 pending queue 的承载，不再表达 current-turn / next-turn 归属。
- `on_task_finished()` 完成最后一个 task 后，若观察到任何 pending input，就不能把线程当成真正 idle。
- goal / child / command 的 post-turn 判断优先级低于“已有 pending input，必须继续起下一轮”。

## Proposed Model

### 1. Unified Input Routing
- 所有异步输入都先进入统一的线程级入口。
- 入口在写入时获取 `active_turn` 锁。
- 在锁内只做判定，不直接启动 turn：
  - 若存在 active turn：根据 turn-local gating 把输入写入该 turn 的 `turn_state.pending_input` 或线程级 mailbox
  - 若不存在 active turn：把输入放到线程级 pending queue，并设置 `should_start_turn = true`
- 解锁后，若 `should_start_turn = true`，调用 `maybe_start_turn_for_pending_work()`

### 2. Mailbox Semantics
- mailbox 不再区分“当前 turn 收”还是“下一轮收”。
- turn 仍可保留一个更直接的 turn-local gating，例如 `accepts_async_input_for_current_turn`：
  - `true`：late async input 仍可并入当前 turn
  - `false`：late async input 留在线程级 mailbox，等待下一轮
- 若保留 mailbox：
  - 它只是线程级 pending queue 的一种实现
  - `trigger_turn` 仍可保留为 wakeup hint
  - 但不再有 `MailboxDeliveryPhase::CurrentTurn/NextTurn`
- 若后续要继续简化，mailbox 与 `idle_pending_input` 可以进一步合并成单一线程级 pending queue。

### 3. Turn Finish Fallback
- `on_task_finished()` 在移除 task 后，如果这是最后一个 task，必须再次拿 `active_turn` 锁完成最后收尾。
- 锁内需要做的不是直接执行后续逻辑，而是提交一个明确的决议：
  - 取走当前 turn 的 leftover `turn_state.pending_input`
  - 检查线程级 pending queue / mailbox 是否已有待处理工作
  - 清掉旧 `active_turn`
  - 返回 `pending input + has_thread_pending_work` 这种锁外可消费的 next-step 决议
- 解锁后：
  - 先 inspect / record leftover pending input
  - 若 `has_thread_pending_work = true`，优先调用 `maybe_start_turn_for_pending_work()`，并跳过 goal continuation / final completion 逻辑
  - 否则仅在 leftover 中存在 accepted input 时启动 follow-up turn
  - 若两者都没有，才继续 goal / parent-final-status / wait-command 判断

### 4. Ordering Rules
- 推荐顺序：
  1. 输入到达，获取 `active_turn` 锁
  2. 锁内决定写当前 turn 还是线程级 pending queue
  3. 若线程原本 idle，解锁后启动新 turn
  4. task 正常运行，期间继续消费 `turn_state.pending_input`
  5. 最后一个 task 结束时，`on_task_finished()` 再次获取同一把锁
  6. 锁内检查“当前 turn + 线程级 queue”是否还有 pending input，并清理旧 `active_turn`
  7. 若仍有 pending input，解锁后立刻补拉起下一轮
  8. 仅当确实没有 pending input 时，才执行 goal continuation / final-status completion

## Important Constraint
- 不要在持有 `active_turn` 锁时直接 `spawn_task` 或启动新 turn。
- 锁内只做状态判定和容器写入；是否启动下一轮由锁外布尔标记决定。
- 否则会把 turn 启动、事件发送、甚至后续回调重新带回同一个临界区，放大死锁和重入风险。

## Behavioral Consequence
- 这套设计接受一个实现变化：
  - 系统不再依赖 mailbox delivery phase 来强制 late async input 的归属
  - 如果要保留“answer boundary 之后不再扩展当前 turn”的产品语义，应通过 turn 内一个更直接的 `accepts_async_input` 判定位来做，而不是让 mailbox 承担 turn 归属语义

## Required Code Changes
- 删除 `MailboxDeliveryPhase::CurrentTurn/NextTurn` 对 pending-input 消费路径的控制。
- 给 async input / mailbox input 提供统一 helper：
  - 锁内根据 turn-local gating 决定写 active turn 或线程级 queue
  - 锁外决定是否起 turn
- 重写 `on_task_finished()` 的最后收尾：
  - 在最后一个 task 结束时获取同一把锁
  - 检查 pending input 作为兜底
  - 若存在 pending work，则优先拉起下一轮，而不是先走 idle 语义
- `maybe_start_turn_for_pending_work_with_sub_id()` 继续只在 idle 时起 turn，但要依赖上述兜底逻辑保证“旧 turn 挡住的启动请求”会在 finish 后被补回。

## Validation
- 最少应覆盖这 4 类测试：
  1. 输入到达时 thread active，锁内写入当前 `turn_state.pending_input`，当前 turn 或 finish fallback 能继续处理。
  2. 输入到达时 thread idle，写入线程级 queue，并自动拉起新 turn。
  3. `on_task_finished()` 期间先拿到锁，event 随后到达；event 在锁外看到 idle 后成功拉起下一轮。
  4. event 先拿到锁但 thread 仍 active，启动请求被挡住；随后 `on_task_finished()` 兜底发现 pending input，并补拉起下一轮。

## Open Questions
- mailbox 是否还值得保留为独立结构，还是直接并入线程级 pending queue。
- `trigger_turn` 是继续留在 `PendingInputItem` 上，还是改成单独的 wakeup bit。
- 目前已采用 turn-local `accepts_async_input_for_current_turn` gating；若后续继续收敛，这个状态是否还能进一步下沉到 task-level runtime。
