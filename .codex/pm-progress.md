# PM Progress

## Current Goal
收口 `on_task_finished()` 与 pending input 的并发模型：最后 task 结束时必须在同一把调度锁内完成 thread 状态提交和后续走向判定，而不是只在取 pending input 时短暂持锁。

## Active Work
- id: on-task-finished-atomic-thread-state
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/on-task-finished-atomic-thread-state
  task_type: bugfix
  depends_on: 无
  files: codex-rs/thread-service/src/tasks/mod.rs, codex-rs/thread-service/src/session/pending_input.rs, codex-rs/thread-service/src/turn_state.rs, codex-rs/thread-service/src/mailbox.rs, codex-rs/thread-service/src/session/tests/context_and_history.rs, .codex/pending-input-post-turn-design.md, .codex/memory/project-understanding.md
  base_commit: 4d8e542a5
  pending_sync_from_main:
  status: merged
  objective: 让 `on_task_finished()` 在最后 task 收尾时持有完整调度锁来提交 thread 状态，原子决定 pending input、follow-up turn、goal continuation、final-status 等后续走向；继续弱化/移除 mailbox current-turn/next-turn 归属语义
  last_update: 2026-07-10
  next_action: 已 merge 到主线；同步空闲 checkout 后继续 planned bug
  blockers: 无
  validation: owner 已跑 `rtk cargo test -p thread-service task_finish_restarts_turn_for_leftover_pending_user_input`；`rtk cargo test -p thread-service compact_task_continues_pending_input_with_regularized_metadata`；`rtk cargo test -p thread-service prepend_pending_input_keeps_older_tail_ahead_of_newer_input`；`rtk cargo test -p thread-service task_finish_prioritizes_thread_pending_work_without_losing_leftover_input`；`rtk cargo test -p thread-service queue_only_mailbox_mail_waits_for_next_turn_after_answer_boundary`；`rtk cargo test -p thread-service trigger_turn_mailbox_mail_waits_for_next_turn_after_answer_boundary`；`rtk cargo build -p app-server --bin app-server`
  commit: 144f240eb（dev-2）；103d3e7b7（main merge）
- id: restore-persisted-agent-registry
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/restore-persisted-agent-registry
  task_type: bugfix
  depends_on: on-task-finished-atomic-thread-state 已 merge 到主线 `103d3e7b7`
  files: codex-rs/thread-service/**, codex-rs/thread-history/**, codex-rs/thread-store/**, codex-rs/app-server/**, apps/root-worker-prototype/**, 相关 list_agents / reload / persisted history / agent tree 状态一致性测试
  base_commit: 103d3e7b7
  pending_sync_from_main: 等待派发 checkout 同步到 `103d3e7b7`
  status: planned
  objective: 让 `list_agents` 重启后仍能看到持久化历史中的已完成 agent thread；runtime 初始化或 thread 恢复时应从持久化恢复 agent registry，并保证 conversation 顶部 `Waiting on Subagent` 状态与 agent tree 中可见子 agent 一致
  last_update: 2026-07-10
  next_action: 选择空闲 checkout 派发；修复点应在 persisted thread/agent registry 恢复语义，并让 app-server/root-worker 的 thread snapshot 与 agent tree 数据源一致，不是只改 `list_agents` 查询表面
  blockers: 无
  validation:
  commit:
- id: root-worker-header-wait-status-dot
  owner: /root/project_pm/owner_dev
  checkout: /Users/bytedance/Projects/my-codex-dev
  branch: fix/root-worker-header-wait-status-dot
  task_type: bugfix
  depends_on: 无
  files: apps/root-worker-prototype/src/styles.css, apps/root-worker-prototype/src/lib/thread.ts, apps/root-worker-prototype/src/components/Panels.tsx, 相关 thread / panel 测试
  base_commit: daab7b0fa
  pending_sync_from_main:
  status: in_progress
  objective: 修复 conversation 顶部状态点与状态文案不一致：`waitChild` 已显示 `Waiting on Subagent`，但 `.status-dot` 缺少 `waiting-subagent` / `waiting-eventtool` 样式导致视觉仍是灰色 complete/inactive；应与 agent tree 的等待状态颜色一致
  last_update: 2026-07-10
  next_action: owner_dev 实现并提交；PM 验收顶部 status dot 与 tree waiting 状态视觉一致
  blockers: 无
  validation:
  commit:
- id: compact-replacement-history-final-output
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/compact-replacement-history-final-output
  task_type: bugfix
  depends_on: poll-event-live-thread-item-missing 已 merge；compact continuation 修复已 merge
  files: codex-rs/compact-service/**, codex-rs/thread-service/**, codex-rs/app-server/**, apps/root-worker-prototype/**, 必要时相关 compaction / conversation 测试
  base_commit: 3ade42e397
  pending_sync_from_main:
  status: merged
  objective: 回退 compact 的公开 turn 语义，只保留 `COMPACT.md` prompt 与 replacement history 注入；compact 最后一条输出应进入 replacement history，客户端不再展示 compact turn
  last_update: 2026-07-10
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p compact-service`；`rtk cargo test -p thread-service compact_final_output`；`rtk pnpm --dir apps/root-worker-prototype test src/lib/conversation.test.ts`；`rtk cargo test -p app-server thread_compact_start_triggers_compaction_and_returns_empty_response`；`rtk cargo build -p app-server --bin app-server`
  commit: 17bb4bb3a（dev）；2c8ecb39e2（main merge）
- id: on-task-finished-leftover-pending-input
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/on-task-finished-leftover-pending-input-v2
  task_type: bugfix
  depends_on: 无
  files: codex-rs/thread-service/src/tasks/mod.rs, codex-rs/thread-service/src/tasks/compact.rs, codex-rs/thread-service/src/session/tests/context_and_history.rs, 必要时 codex-rs/thread-service/src/session/turn.rs / codex-rs/thread-service/src/compact.rs 用于确认 continuation 语义
  base_commit: 6553aaea7
  pending_sync_from_main:
  status: merged
  objective: 重新定位并修复 compact 结束后 thread 停机的问题；先确认 compact 应继续当前 turn 还是进入统一收尾，再在正确层级修复 leftover pending input / continuation 断链
  last_update: 2026-07-09
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p thread-service compact_task_continues_pending_input_with_regularized_metadata`；`rtk cargo test -p thread-service task_finish_restarts_turn_for_leftover_pending_user_input`；`rtk cargo build -p app-server --bin app-server`；已确认 `rtk cargo test -p thread-service task_finish_emits_turn_item_lifecycle_for_leftover_pending_user_input` 在主线同样失败，属于既有测试基线问题而非本次回归
  commit: 9faeaebbd0c9f94e8fbef4c96d447e5b0874f610（dev）；3ade42e397（main merge）
- id: poll-event-live-thread-item-missing
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/poll-event-live-thread-item-missing
  task_type: bugfix
  depends_on: 无
  files: codex-rs/thread-service/**, codex-rs/app-server-protocol/**, apps/root-worker-prototype/**, 必要时相关 thread item / conversation 测试
  base_commit: c82a07700
  pending_sync_from_main:
  status: merged
  objective: 查明并修复 `poll_event` 实际执行阻塞等待时，客户端仍不显示对应 live thread item 的链路缺口
  last_update: 2026-07-09
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p app-server builtin_poll_event_emits_started_and_completed_thread_items`；`rtk cargo test -p app-server builtin_poll_event_failure_emits_completed_failed_thread_item`；`rtk cargo build -p app-server --bin app-server`
  commit: 40576a385（dev）；6553aaea71（main merge）
- id: reload-live-command-backend-root-fix
  owner: /root/project_pm/owner_dev
  checkout: /Users/bytedance/Projects/my-codex-dev
  branch: fix/reload-live-command-backend-root-fix
  task_type: bugfix
  depends_on: reload-live-command-residue 已 merge；后续应评估是否回收该前端兜底
  files: codex-rs/app-server-protocol/**, codex-rs/thread-history/**, codex-rs/app-server/**, 必要时 apps/root-worker-prototype/** 仅用于删除/收紧前端兜底与补测试
  base_commit: 6f03f8854
  pending_sync_from_main:
  status: merged
  objective: 找出并修复 reload/read 路径仍会把已完成 command 恢复为 stale running item 的后端根因；在后端事实修好后，移除或最小化前端 live-command 兜底逻辑
  last_update: 2026-07-09
  next_action: 已 merge 到主线；`dev-2` / `dev-3` 已同步到 `c82a07700`
  blockers: 无
  validation: `rtk cargo test -p app-server thread_turns_list_uses_only_in_progress_live_turn_snapshots`；`rtk cargo test -p app-server populate_thread_turns_from_history_keeps_persisted_completed_command_when_no_live_turn`；`rtk cargo test -p app-server thread_resume_and_read_interrupt_incomplete_rollout_turn_when_thread_is_idle`；`rtk cargo build -p app-server --bin app-server`
  commit: 88568729a（dev）；c82a07700（main merge）
- id: reload-live-command-residue
  owner: /root/project_pm/owner_dev
  checkout: /Users/bytedance/Projects/my-codex-dev
  branch: fix/reload-live-command-residue
  task_type: bugfix
  depends_on: 无
  files: apps/root-worker-prototype/**, 如需定位 reload 数据来源可只读检查 codex-rs/app-server-protocol/** 与相关测试
  base_commit: c32bf3efd9f42f28306b0e6c4fe208811846cfe3
  pending_sync_from_main:
  status: merged
  objective: 修复客户端重启后 `Live Commands` 错误展示一批已完成 command 为 `Running` 的问题，优先排查 reload snapshot / merge 保留逻辑，而不是新增前端文案补丁
  last_update: 2026-07-09
  next_action: 已 merge 到主线；`dev-2` / `dev-3` 已同步到 `6f03f8854`
  blockers: 无
  validation: `rtk pnpm --dir apps/root-worker-prototype test src/lib/threadAnalysis.test.ts src/components/RightPanel.test.tsx`
  commit: 38e340fc0（dev）；6f03f8854（main merge）
- id: init-context-workflow-instructions
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/init-context-workflow-instructions
  task_type: bugfix
  depends_on: 无
  files: codex-rs/thread-service/**, codex-rs/app-server/**, 相关 thread start / init context 测试
  base_commit: a0aaf3d06ca0d39bf91729e6a5989a3d0b30d272
  pending_sync_from_main:
  status: merged
  objective: 让 init context 正确包含 config path 下可发现的 workflow init context，以及 config file 中配置的 instruction_files
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p app-server --test all thread_start_initial_context_includes_project_workflows_and_instruction_files_without_primary_environment -- --exact`；`rtk cargo build -p app-server --bin app-server`
  commit: 3335c130e34140da7a118cc1bf21b91824c28509
- id: poll-event-thread-item-visibility
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/poll-event-thread-item-visibility
  task_type: bugfix
  depends_on: 无
  files: codex-rs/app-server-protocol/**, codex-rs/thread-history/**, codex-rs/tool-service/**, apps/root-worker-prototype/**, 相关 thread item / conversation 测试
  base_commit: a0aaf3d06ca0d39bf91729e6a5989a3d0b30d272
  pending_sync_from_main:
  status: merged
  objective: 让 poll_event 在客户端 thread items 中明确可见；同时将 command output / exit event 的客户端展示从 command id 改为具体命令，并提供合适的文案/分类与回归覆盖
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p app-server-protocol builtin_tool_call_completed_display_event_maps_to_thread_item`；其余 `rollout` / `tool-service` / 前端测试受既有基线问题或本地依赖缺失阻断
  commit: fbb90be4e

- id: unified-poll-event-waiting
  owner: /root/project_pm/owner_dev_2
  checkout: /Users/bytedance/Projects/my-codex-dev-2
  branch: fix/unified-poll-event-waiting
  task_type: bugfix
  depends_on: poll-event-thread-item-visibility merge 到主线
  files: codex-rs/tool-service/**, codex-rs/thread-service/**, codex-rs/command-service/**, app-server protocol / tests, 相关 tool spec / prompt / runtime 测试
  base_commit: eee237bdf2dc6a410d397b6a466caa045192e294
  pending_sync_from_main:
  status: merged
  objective: 删除 `wait_agent` / `command_wait`，统一由 `poll_event` 承担 turn 内等待并在同一 turn 继续执行；event 来源信息通过 pending input 暴露，runtime 不维护硬性等待目标
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p command-service-api unified_exec_error -- --nocapture`；`rtk cargo test -p thread-service poll_event_wakes_for_command_exit_notification -- --exact --nocapture`；`rtk cargo build -p app-server --bin app-server`
  commit: 646fa4f5a

- id: child-completion-thread-status
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/child-completion-thread-status
  task_type: bugfix
  depends_on: unified-poll-event-waiting 已 merge 到主线
  files: codex-rs/agent-runtime/**, codex-rs/thread-service/**, 相关 child completion / thread status / poll_event 测试
  base_commit: 050342997320e0cb7327fa79b39554feecf90dd0
  pending_sync_from_main:
  status: merged
  objective: 调整 child completion 与 thread status 的关系，让 child thread status 只反映 child 自己的本地 turn 生命周期；去掉 parent-side direct child completion bookkeeping 对 `WaitChild` / child status 的直接驱动
  last_update: 2026-07-07
  next_action: 已 merge 到主线；空闲时同步 checkout
  blockers: 无
  validation: `rtk cargo test -p thread-service control_tests::post_turn_state_waits_for_active_direct_child_without_active_goal`；`rtk cargo test -p thread-service control_tests::pending_child_completion_bookkeeping_does_not_trigger_wait_child`；`rtk cargo test -p thread-service session::tests::context_and_history::turn_start_consumes_child_completion_before_parent_visible_complete`；`rtk cargo test -p thread-service session::tests::context_and_history::clearing_stale_child_completion_preserves_non_completion_messages`；`rtk cargo test -p thread-service session::tests::context_and_history::aborting_turn_clears_pending_child_completion_tracking_from_turn_state`
  commit: 4baba77cb

- id: exec-command-thread-item-reload
  owner: /root/project_pm/owner_dev_3
  checkout: /Users/bytedance/Projects/my-codex-dev-3
  branch: fix/exec-command-thread-item-reload
  task_type: bugfix
  depends_on: child-completion-thread-status 已 merge 到主线
  files: codex-rs/rollout/**, codex-rs/app-server-protocol/**, codex-rs/app-server/**, codex-rs/thread-service/**, codex-rs/thread-store/**, apps/root-worker-prototype/**, 相关 reload/history/thread item / list_agents 测试
  base_commit: 1b5f3a3961b6113bf6da403724614e0d97545e2d
  pending_sync_from_main:
  status: merged
  objective: 修复重启客户端后普通 `exec_command` thread item 仍不可见的问题，并保留已完成 agent 在 reload 后仍可列出；同时核对 `poll_event` 对应 thread item 的 live / reload 可见性，确认真实 typed item / reload/read 缺口，而不只停留在 agent command replay
  last_update: 2026-07-08
  next_action: 已 merge 到主线；空闲时同步 dev-3 checkout
  blockers: 无
  validation: `rtk cargo test -p thread-history typed_builtin_tool_history_rebuilds_thread_item`；`rtk cargo test -p app-server limited_replay_keeps_poll_event_builtin_tool_items`；`rtk cargo test -p app-server limited_replay_truncates_large_agent_command_execution_output`；`rtk cargo test -p app-server thread_read_after_restart_keeps_unified_exec_command_execution_items`；`rtk cargo build -p app-server --bin app-server`；`rtk cargo test -p rollout limited_mode_sanitizes_unified_exec_command_end_output -- --exact` 仍被仓库既有无关编译问题阻塞
  commit: cdc4896c1（dev-3）；33e930678（main merge）

## Completed
- commit: 36aed5f41
  summary: 合并 `0f837f012` 到主线，让 compact turn 构造 prompt 时不再携带 model-visible tools；`COMPACT.md` prompt、replacement history final output 与“客户端不展示 compact turn”行为保持不变；按用户要求停止 `owner_dev_3`，由 `owner_dev_2` 独占完成该任务
  validation: `rtk cargo test -p thread-service compact_turn_hides_model_visible_tools_without_affecting_regular_turns`；`rtk cargo test -p thread-service compact_final_output`；`rtk cargo build -p app-server --bin app-server`；独立 reviewer 结论为“通过/可继续”
  residual_risk: 当前回归主要锁住 `thread-service` prompt build 分支；仍缺一条更完整的 compact 调用链测试，直接覆盖“compact 无 tools + replacement history final output 保留 + compact turn 继续隐藏”的组合语义；`dev-2` 已 fast-forward 到 `36aed5f41`，`dev` 与 `dev-3` 因各自未提交脏改暂未同步
- commit: c82a07700
  summary: 合并 `88568729a` 到主线，修复 reload/read 路径把 finished fallback turn 当成 live turn merge 回 persisted history 的问题；`thread/resume` 与 `thread/turns/list` 现在都只接受 `active_in_progress_turn_snapshot()`，不再把已完成 command 盖回成 stale running residue
  validation: `rtk cargo test -p app-server thread_turns_list_uses_only_in_progress_live_turn_snapshots`；`rtk cargo test -p app-server populate_thread_turns_from_history_keeps_persisted_completed_command_when_no_live_turn`；`rtk cargo test -p app-server thread_resume_and_read_interrupt_incomplete_rollout_turn_when_thread_is_idle`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: 当前回归已锁住核心 merge 误用，但更细的 listener/resume 排队时序仍主要靠现有 suite 近邻覆盖；若后续再改该链路，建议补一条更贴近真实接口时序的回归
- commit: 6f03f8854
  summary: 合并 `38e340fc0` 到主线，修复客户端重启后的 reload 路径中 `Live Commands` 错把 stale running `commandExecution` residue 当作 live command 的问题；仅在线程顶层状态仍可能承载真实 live command（`active.running`、`idle.waitCommand`、`idle.waitChild`）时才展示
  validation: `rtk pnpm --dir apps/root-worker-prototype test src/lib/threadAnalysis.test.ts src/components/RightPanel.test.tsx`
  residual_risk: `waitingOnUserInput` 未单列一条对称测试，但与 `waitingOnApproval` 共享 `activeFlags.includes("running")` gate；若后续该状态也出现类似 residue，可再补一条更显式回归
- commit: c32bf3efd9f42f28306b0e6c4fe208811846cfe3
  summary: 将 compact 后的 replacement history 收缩为仅保留 `initial_context` 与最近最多两条真实 user message；不再把 memory 文件正文复制成 `Memory checkpoint: ...` user messages；compact persisted/UI 事实链路保持不变
  validation: `rtk cargo test -p compact-service replacement_history`；`rtk cargo test -p app-server thread_compact_start_triggers_compaction_and_returns_empty_response`
  residual_risk: `auto_compaction_local_emits_started_and_completed_items` 在等待 compact lifecycle notification 时超时，未覆盖到本次新语义；当前以更稳定的手动 compact 集成用例兜底
- commit: 4cb22849494c39acf76f35a3ca19c3acbfca2346
  summary: 收口 `pending input` / `on_task_finished()` 调度临界区：pending input 路由与 active turn 检查走统一原子区；post-turn 收尾区分线程级 pending work 与 leftover pending input；仅 `Accepted` leftover 会重启 follow-up turn，纯 `Blocked` leftover 不会误启空 turn
  validation: owner/reviewer 已确认 `NextTurn` 下 late mailbox mail 不再扩展当前 turn、mailbox preempt 路径恢复、leftover pending input 仅在 `inspect_pending_input(...)` 返回 `Accepted` 时触发 follow-up turn；相关回归位于 `codex-rs/thread-service/src/session/tests/context_and_history.rs`
  residual_risk: progress file 此前长期滞后，缺少一条集中记录的 owner 命令级验证；若后续继续加固，优先补两条建议测试：`Blocked` leftover 不启动 follow-up turn，以及“线程级 pending work 与 accepted leftover 同时存在时优先走 pending work 分支”
- commit: 33e930678
  summary: 合并 `cdc4896c1` 到主线，修复普通 `exec_command` reload/read 丢 completed 态的问题；对进入 `Limited` 的 `ExecCommandEnd` 统一做有界 sanitize；补齐 `thread-history` 对 builtin tool call 的事件分派，使 `poll_event` thread item 在 `thread/read` / reload 路径恢复可见
  validation: `rtk cargo test -p thread-history typed_builtin_tool_history_rebuilds_thread_item`；`rtk cargo test -p app-server limited_replay_keeps_poll_event_builtin_tool_items`；`rtk cargo test -p app-server limited_replay_truncates_large_agent_command_execution_output`；`rtk cargo test -p app-server thread_read_after_restart_keeps_unified_exec_command_execution_items`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: `rtk cargo test -p rollout limited_mode_sanitizes_unified_exec_command_end_output -- --exact` 仍被仓库现存无关编译问题阻塞；如果后续还要加固，可再补一条“只有 `BuiltinToolCallStarted`、没有 completed 时 reload 仍保留 `InProgress`”测试
- commit: 951f010cd611
  summary: 合并 `cce24f0d7` 到主线，补齐 reload 路径对 agent `exec_command` thread item 的 `Limited` 持久化恢复，并让 `list_agents` 在 live thread 不存在时回退到 persisted completed agent 状态
  validation: `rtk cargo test -p thread-service list_agents_restores_completed_child_from_persisted_history_when_live_thread_is_gone`；`rtk cargo test -p app-server limited_replay_keeps_agent_command_execution_items_visible_after_reload`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: 当前补的是 persisted replay 与 completed-agent fallback；仍建议后续补一条更完整的 app-server reload/integration 测试，串联真实重启后的 thread read/list_agents 行为
- commit: 61d5d4e18
  summary: 合并 `4baba77cb` 到主线，修正 child completion / `WaitChild` 状态语义，使 direct child 本地 active 状态与 pending completion bookkeeping 解耦
  validation: merge 级集成；沿用 owner 已提交验证结果与 reviewer 通过结论
  residual_risk: 仍缺一条更完整的 integration-style 生命周期测试，串联 `spawn_agent -> parent completion -> child completion envelope -> parent wakeup`
- commit: 1dc9c8cba9ae8c446bf8d803dff1486198a75acb
  summary: 合并 `646fa4f5a` 到主线，删除 `wait_agent` / `command_wait`，统一等待入口为 `poll_event`，并让 command output/exit 与 child completion 复用同一 pending-input 唤醒链路
  validation: merge 级集成；沿用 owner 已提交验证结果与 reviewer 通过结论
  residual_risk: `codex-analytics` crate 仍有既有测试基线问题，导致无法补跑一条目标测试；本次改动只显式补齐 `BuiltinToolCall` 穷举覆盖并保持原有 analytics 语义
- commit: eee237bdf2dc6a410d397b6a466caa045192e294
  summary: 合并 `3335c130e34140da7a118cc1bf21b91824c28509`（init context workflow/instruction files 修复）与 `fbb90be4e`（poll_event thread items 可见性与 command 文案）到主线
  validation: merge 级集成；沿用各 owner 已提交验证结果
  residual_risk: `poll-event-thread-item-visibility` 仍受 rollout/tool-service 既有测试问题与前端本地依赖缺失影响，尚无该分支上的完整全链路回归
- commit: ad4dd4c1247552e9f21fda28fda9391f12e5c433
  summary: 合并 compact 展示与 reinject 修复到主线：compact row 可保留 hydrated archived history、compact turn display items 在 reload 路径保持可见、`.codex/memory/current-work.md` 默认忽略；并同步所有 dev checkout 到最新主线
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/conversation.test.ts`；`rtk cargo test -p app-server-protocol preserves_compaction_turn_display_items_alongside_compaction_marker`；`rtk cargo test -p thread-service process_compacted_history_reinjects_full_initial_context`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: reviewer 仅保留一条轻量残余风险：若未来 `readCompactHistory()` 真实返回形态再次变化，仍建议在真实 compact/reload 流程下手工观察一次 UI
- commit: 1fb47b58b6467ed815a7450e6feb9b1b2b9419ca
  summary: 合并 unified `poll_event` runtime/tooling 实现与架构收口返修到主线；统一 thread wait primitive、thread-scoped backoff，并将 `command_wait` / `wait_agent` 收敛为兼容壳路径
  validation: `rtk cargo test -p thread-service poll_event_`；`rtk cargo test -p command-service`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: 仍缺一条 `tool-service` 侧回归测试，直接覆盖 `command_wait` started/finished 事件顺序与 `try_finish_now()` 命中后的 shared backoff reset
- commit: 9cfeac59c5e8
  summary: 合并 `58faae6` stale child completion 修复与 `f1d874c` 右侧面板 git/files 视图功能到主线，并将空闲 `dev-2`/`dev-3` checkout fast-forward 到最新集成基线
  validation: merge 级集成；未新增额外验证，沿用各 owner 已提交验证结果
  residual_risk: `right-panel-git-files` 仍保留 owner 提交时已有的前端依赖缺失验证缺口；本次只完成集成与同步
- commit: n/a
  summary: 新增 unified `poll_event` 设计文档，明确 turn 内等待应统一为“新的 thread input 唤醒”，不引入独立 event buffer，backoff 改为 thread-scoped runtime state
  validation: 文本级核对；设计结论已与用户对齐
  residual_risk: 尚未落实现代码；现有部分外部事件路径可能只有 display event、缺少模型可消费 input，后续实现前需补齐契约
- commit: 44c701abcdc3
  summary: 修复 root-worker prototype 中 reload thread 后 tool-like item 显示异常，补齐 builtin tool、event-command、schedule 的前端显示与回归覆盖
  validation: `rtk pnpm --dir apps/root-worker-prototype exec tsx --test --test-name-pattern 'renders event command subscriptions and output events|renders event command exit signals in event summaries|builds visible entries for empty reasoning and builtin schedule tools|mergeThreadSnapshot preserves restored event-driven tool calls with distinct ids|counts event command subscriptions and events in tool usage' src/lib/conversation.test.ts src/lib/thread.test.ts src/lib/contextUsage.test.ts` 通过；`rtk pnpm --dir apps/root-worker-prototype build` 通过
  residual_risk: 相关全文件测试中仍有一条既有 `contextUsage` 断言失败，owner 评估为与本修复无关；本次只锁定新增回归路径
- commit: n/a
  summary: `compact-memory-runtime` 不再继续推进；用户确认该需求已结束，无需 owner 派发或后续集成
  validation: 用户口头确认，无新增实现或验收动作
  residual_risk: 若后续再次开启同主题需求，需要重新建立 active work 与验收范围
- commit: 56adcd785113ff951598794db5e279476d49b7cd
  summary: compact item 默认改为按需加载历史，按 compact 轮次分组展示，折叠后丢弃已加载详情
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/compactHistoryRequest.test.ts src/lib/conversation.test.ts src/components/Conversation.test.tsx src/lib/conversationVirtualization.test.ts src/lib/conversationSearch.test.ts`；`rtk pnpm --dir apps/root-worker-prototype build`
  residual_risk: 仍缺一个更贴近 `App.tsx` 异步状态流的竞态测试，以及展开后搜索/焦点联动测试
- commit: 9c3e13d71
  summary: `list_agents` 返回仍注册且可读取状态的 completed agent，并允许其 canonical path 继续复用
  validation: `rtk cargo build -p app-server --bin app-server`
  residual_risk: `thread-service` 现存无关测试编译问题仍未清理，新增回归未能在目标 crate 全量通过
- commit: d9746c6aabc8e6835ba862d9aab6764b1ca011ce
  summary: 删除 UI/UE agent 定义并移除 `ui-design/`、`spec/` 目录
  validation: 文本级核对与目录删除确认
  residual_risk: 可能还有仓库外部引用尚未清理
- commit: 15c66dd
  summary: compact prompt 默认来源切换到 `cwd/.codex/compact/COMPACT.md` 优先、`CODEX_HOME/compact/COMPACT.md` 回退，并解锁 `config-service` 最小测试链
  validation: `rtk cargo test -p config-service compact_prompt`；`rtk cargo test -p config-service config_loader_tests`
  residual_risk: 可选增强是再补一条同时设置 `compact_prompt` 和 `experimental_compact_prompt_file` 的优先级测试
- commit: e90db46
  summary: live command 仅展示 in-progress command，并修复点击后 conversation 被重复拉回导致无法滚动的问题
  validation: `rtk pnpm --dir apps/root-worker-prototype test -- src/lib/threadAnalysis.test.ts`；`rtk pnpm --dir apps/root-worker-prototype test -- src/components/Conversation.test.tsx src/components/RightPanel.test.tsx src/lib/threadAnalysis.test.ts`
  residual_risk: 仍缺少更贴近真实 DOM 副作用的滚动交互测试

## Known Issues
- 原 `.codex/pm-progress.md` 中记录的 thread 重构上下文已过期，且引用的 `spec/` 文档已删除；后续若继续推进该主题，需要重新建立可执行拆分计划。
