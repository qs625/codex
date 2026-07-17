# PM Progress

## Current Goal
None

## Active Work
None

## Completed
- commit: 950edd74e
  summary: 合并 `6acc09a` 到主线，修复 completed child 仍让 parent thread 卡在 `WaitChild` / wait on subagent 的问题；`agent_thread_is_active_from_inputs()` 现在优先识别 final agent status，completed child 即使残留 stale active turn 或 event subscription runtime facts，也不会再让 `direct_agent_children_are_active()` 判 active。active/non-final child 的 WaitChild 行为保留。
  validation: owner `rtk cargo test -p codex-agent-runtime control_plan::tests::agent_thread_activity_uses_runtime_facts_in_order -- --nocapture` -> 1 passed；owner `rtk cargo test -p thread-service post_turn_state_stops_waiting_after_child_completion_is_consumed -- --nocapture` -> 1 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_3/reviewer` 通过。PM 合并后同两条 targeted tests、`rtk cargo build -p app-server --bin app-server`、`rtk git diff --check` 均通过。
  residual_risk: 新测试模拟 parent mailbox 已消费 child completion 后的 post-turn 状态，未覆盖真实 `maybe_notify_parent_of_final_status()` trigger-turn 全链路；事实源修复在 runtime active 判定，review 认为风险可接受。
- commit: bef305977
  summary: 合并 `1813355e1` 到主线，修复 `thread/resume` 对 persisted `agent_role` 只恢复 metadata、不重新 apply role config 的问题；resume load config 后会在计算 `instruction_sources` 与恢复 runtime 前重新调用 `apply_role_to_config`，因此 compact/restart 后带 agent type 的 thread 仍会读取对应 `.agent.md` / role instruction files。
  validation: owner `rtk cargo test -p app-server --test all thread_resume_reapplies_stored_agent_role_to_model_context` -> 1 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_2/reviewer` 通过。PM 合并后 `rtk cargo test -p app-server --test all thread_resume_reapplies_stored_agent_role_to_model_context` -> 1 passed；PM `rtk cargo build -p app-server --bin app-server` -> passed；PM `rtk git diff --check` -> passed。
  residual_risk: unknown/unavailable role 的 resume 错误路径未单独加 E2E；实现复用 start 路径 `apply_role_to_config` 的 invalid_request 语义。
- commit: bef305977
  summary: 合并 `32560427f` 到主线，root-worker 左侧 Chat 列表行现在 hover/focus-within 时在右侧显示 icon-only delete button；Chat 行结构改为 shell + select button + delete button，避免 nested button；删除入口走现有 archive IPC，但新增 Chat 专用 handler 只允许删除当前 Chat 列表成员，Project/subagent 右键删除限制保持不变。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 15 passed；owner `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk warning；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev/reviewer` 通过。PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 15 passed；PM `rtk git diff --check` -> passed。
  residual_risk: 未做完整 Electron/Playwright 点击 smoke；组件结构与 review 确认 delete/select 为 sibling buttons，删除点击不会触发行选择。
- commit: cc11fce27f
  summary: 合并 `0e02e9c` 到主线，修复 raw `<subagent_notification>...</subagent_notification>` envelope 泄漏到用户可见 conversation display；新增共享 legacy structured user-input guard，history replay 与 live `CoreTurnItem::UserMessage` projection 复用；live mapper 支持 filtered item 返回 `None`，app-server fanout 跳过发送，避免 panic；普通用户文本提及 marker 和带 skill/富内容的用户消息仍保留。
  validation: owner `rtk cargo test -p app-server-protocol subagent_notification -j1` -> 5 passed；owner `rtk cargo test -p app-server-protocol raw_marker -j1` -> 1 passed；owner `rtk cargo build -p app-server --bin app-server -j1` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_3/reviewer` 通过。PM 合并后 `rtk cargo test -p app-server-protocol subagent_notification -j1` -> 5 passed；PM `rtk cargo test -p app-server-protocol raw_marker -j1` -> 1 passed；PM `rtk cargo build -p app-server --bin app-server -j1` -> passed；PM `rtk git diff --check` -> passed。
  residual_risk: `ItemStarted(raw user)` 未单独加对称测试，但 mapper 同一 match arm 已由完成态路径覆盖；`thread_and_turn` 全文件过滤仍有既有无关失败 `thread_lifecycle_responses_default_missing_optional_fields`，未作为本任务范围修复。
- commit: 0814f4c0d9
  summary: 合并 `c900f1514` 到主线，为后端持久化 `ThreadContextUsage.toolBreakdown`，包含 applyPatch、fileOperations、commands、interAgent、searchMedia、otherTools 的 input/output 估算；`context-usage` 后端按 `ResponseItem` 计算并用 call_id 关联 output；app-server-protocol、rollout/replay fallback、thread_read/thread_resume fixture 与 root-worker 类型/UI 同步，RightPanel 展示后端 Tool I/O Detail 且缺字段/全零不渲染。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test src/lib/contextUsage.test.ts src/components/RightPanel.test.tsx` -> 23 passed；owner `rtk cargo test -p codex-context-usage` -> 6 passed；owner 多条 app-server thread_read/thread_resume/replay targeted tests -> passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_2/reviewer` 多轮通过。PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/lib/contextUsage.test.ts src/components/RightPanel.test.tsx` -> 23 passed；PM `rtk cargo test -p codex-context-usage` -> 6 passed；PM `rtk git diff --check` -> passed。
  residual_risk: breakdown 是估算型诊断视图，不保证与顶层 `toolCalls` 严格数值闭合；inter-agent 保留原顶层分类语义，同时进入独立 breakdown bucket。
- commit: a274097a0e
  summary: 合并 `cc1eba17b` 到主线，root-worker 左侧 Chat header 现在 hover/focus-within 时显示 icon-only `New chat` 按钮；点击后直接构造 cwd-free `mode: "chat"` blank draft 并复用现有 create flow，不打开 New popup、不要求 title/task/cwd；Project New popup 和 Project create 行为不变。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 14 passed；owner `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk warning；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev/reviewer` 两轮通过。PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 14 passed；PM `rtk git diff --check` -> passed。
  residual_risk: 未做完整 Electron/Playwright 视觉 smoke；当前通过 helper shape、App build、review 确认 direct create 会走 chat mode。后端 write-tool visibility 仍是既有残余风险，非本轮范围。
- commit: 3be65d3d
  summary: 合并 `26f8cda3` 到主线，root-worker Chat 现在使用稳定 compat cwd 作为后端 required cwd fallback，但 renderer 将该 cwd 识别为非项目语义；Chat create payload 使用 `permissions: ":read-only"` 且不传 legacy `sandbox`，Project create 继续 `sandbox: "danger-full-access"`；左侧 Chat 区域改为用户截图风格的扁平 conversation list，不再走 collapsible tree node；RightPanel 对 compat cwd 显示无 project cwd 空态，不加载 tmp cwd tree。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx src/lib/thread.test.ts src/components/RightPanel.test.tsx` -> 126 passed；owner `rtk node --test apps/root-worker-prototype/electron/threadConfig.test.cjs` -> 7 passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev/reviewer` 通过。PM 合并后复跑同三项验证均通过。
  residual_risk: 后端 model-visible `apply_patch` 等 host-side 写工具 visibility 尚未收口；本次只保证 Chat thread start 使用 read-only permission profile，exec 类写入受 runtime sandbox 约束。App clear/root-session 重建路径缺少直接组件级测试，但 reviewer 已审查并通过，相关 helper/sidebar/RightPanel 行为已有覆盖。
- commit: 4bc64b1d
  summary: 合并 `654ff4d80` 到主线，修复 raw `InterAgentCommunication` envelope 泄露到可见 conversation 的问题；legacy/live/replay 路径中的 `ResponseItemCompleted(ResponseItem::InterAgentCommunication)` 现在只在有 id 且 operation 非 Unknown 时投影成 typed `CollabAgentMessage` / `CollabAgentStatusUpdate`，其它 raw response item 继续隐藏；模型可见 pending input 语义保持不变。
  validation: owner `rtk cargo test -p app-server-protocol response_item_completed_maps` -> 2 passed；owner `rtk cargo test -p thread-history maps_legacy_response_item_completed` -> 2 passed；owner `rtk cargo test -p app-server-protocol collab` -> 8 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_2/reviewer` 通过。PM 合并后 `rtk cargo test -p app-server-protocol response_item_completed_maps` -> 2 passed；PM `rtk cargo test -p thread-history maps_legacy_response_item_completed` -> 2 passed；PM `rtk cargo test -p app-server-protocol collab` -> 8 passed；PM `rtk cargo build -p app-server --bin app-server` -> passed；PM `rtk git diff --check` -> passed。
  residual_risk: 修复范围刻意限制在 inter-agent legacy completed projection；审计确认 `RawResponseItem` 和普通 `ResponseItemStarted/Completed` 默认仍隐藏，command/event/tool/goal/workflow 已有 structured projection。未做完整 Electron 手工 smoke。空闲 `dev-2` 与 `dev-3` 已 fast-forward 到 `4bc64b1d`。
- commit: 536c0515b
  summary: 合并 `f9ae57cf1` 到主线，修复 `model/list` 因远端 GPT-5.6 catalog 中 `ReasoningEffort` 新值 `max` / `ultra` decode 失败而回退旧 cache/bundled catalog 的问题；协议 enum、effort rank、Bedrock helper、app-server-protocol schema、config schema、TS/Python SDK 类型和 Python examples 均同步支持 `max` / `ultra`。
  validation: owner `rtk cargo test -p protocol reasoning_effort` -> 4 passed；owner `rtk cargo test -p app-server-protocol --test schema_fixtures` -> passed；owner `rtk cargo test -p app-server --test all model_list` -> 11 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_3/reviewer` 多轮通过。PM 合并后 `rtk cargo test -p protocol reasoning_effort` -> 4 passed；PM `rtk cargo test -p app-server-protocol --test schema_fixtures` -> exit 0；PM `rtk cargo test -p app-server --test all model_list` -> 11 passed；PM `rtk cargo build -p app-server --bin app-server` -> passed；PM `rtk git diff --check` -> passed。
  residual_risk: 未更新 bundled offline fallback catalog，完全离线 fallback 仍不包含 GPT-5.6；`config-service` schema fixture 受既有编译错误阻塞，未作为本次有效验证。`dev` 与 `dev-2` 因 active/paused work 暂不同步到 `536c0515b`；空闲 `dev-3` 已 fast-forward 到 `536c0515b`。
- commit: cfef6c781
  summary: 合并 `ea3765633` 到主线，修复 unified exec 后台 command exit pending input：`Output` / `Exit` notification 保持分离；`notify_on=output` 的 exit 只携带最后一次 Output notification 后未通知的 residual output；`notify_on=exit` 的 exit 携带完整有界 transcript；`poll_event` 返回 schema 不扩展。
  validation: owner `rtk cargo test -p command-service unified_exec::async_watcher` -> 5 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev_2/reviewer` 两轮通过；PM 合并后 `rtk cargo test -p command-service unified_exec::async_watcher` -> 5 passed；PM 合并后 `rtk cargo build -p app-server --bin app-server` -> passed；PM `rtk git diff --check` -> passed。空闲 checkout `/Users/bytedance/Projects/my-codex-dev` 与 `/Users/bytedance/Projects/my-codex-dev-2` 已 fast-forward 到 `cfef6c781`；`dev-3` 正在 model/list 修复分支上暂不同步。
  residual_risk: 未补完整 streaming 集成时序测试；当前 helper tests 覆盖核心 full/residual/empty/failure 输出选择语义。
- commit: 3833226ec
  summary: 合并 `d9d0634b4` 到主线，删除 root-worker 右侧 Thread Analysis 中的 `Plan Work` / `Execution Queue` 队列组件；保留 Current Thread Plan、Thread Analysis summary/goal/context/monitor 等内容；`todoItems` 仍作为右侧 rail badge 输入，不再在 Thread Analysis 内渲染 todo cards/filter/Open Project 入口。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test src/components/RightPanel.test.tsx` -> 12/12 passed；owner `rtk git diff --check` -> passed；fixed reviewer `/my_codex/owner_dev/reviewer` 通过；PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/components/RightPanel.test.tsx` -> 12/12 passed；PM `rtk git diff --check` -> passed。空闲 checkout `/Users/bytedance/Projects/my-codex-dev` 与 `/Users/bytedance/Projects/my-codex-dev-3` 已 fast-forward 到 `3833226ec`；`dev-2` 正在 runtime 修复分支上暂不同步。
  residual_risk: 未做完整 Electron 手工 smoke；样式里可能仍有历史 todo/thread-analysis-queue 类名残留，但 TSX 已无入口引用，未作为本次范围清理。
- commit: 37f83178d
  summary: 合并 `e6e8c09a5` 到主线，调整 root-worker New popup 与右侧面板：New popup 删除 Title 输入和 `NewThreadDraft.title`，创建 name 内部用 `taskName` 兜底；默认 taskName 从 cwd basename sanitize 得到，不再追加 hash；project root 的 title/role 两处 helper 改为展示 thread path；右侧移除独立 Todo Board / `todo` view，将 Current Plan 和 Execution Queue 合入 Thread Analysis，历史存储的 `todo` view 回落到 `skills`。
  validation: owner `rtk git diff --check` -> passed；owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx src/lib/thread.test.ts src/components/RightPanel.test.tsx src/lib/rightPanelView.test.ts` -> 129 passed；owner reviewer `/my_codex/owner_dev/reviewer` 两轮通过；PM `rtk git diff --check 88a5227307b76f924e56d0d24e17ba27e62d360e..e6e8c09a5` -> passed；PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx src/lib/thread.test.ts src/components/RightPanel.test.tsx src/lib/rightPanelView.test.ts` -> 129 passed。
  residual_risk: 未跑全量 root-worker 前端测试或 Electron 手工 smoke；Thread Analysis 内容密度增加，后续可按实际使用再微调视觉层级。
- commit: 1e18f87c0
  summary: 合并 `071dc1e09` 到主线，修复 root-scope/no-parent agent init context canonical path 仍显示 `/root`；root-scope agent metadata 在 session spawn 时透传给 initial context，注册尚未 keyed by thread id 时用 creation-time metadata 兜底。root-worker createThread 缺 cwd 时不再默认填 `.codex-home/root_workspace`，避免创建 project thread 后额外出现 `root_workspace` project；同时启用 no-project chat 并修 `/clear` replacement。project sidebar dot 改用 project root thread 自身状态，child active 只进入 badge 计数，不再把 `wait_child` project root 涂成绿色 running。
  validation: owner `rtk pnpm --dir apps/root-worker-prototype test -- src/components/Panels.test.tsx src/lib/thread.test.ts` -> 112 passed；owner `rtk cargo test -p thread-service build_initial_context_uses_root_scope_agent_metadata_path` -> 1 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner reviewer `/my_codex/owner_dev/reviewer` 两轮通过；PM `rtk git diff --check 784c34e47a0c8f9b35824c34117b080d515af564..071dc1e09324e548aeb301db3257a06550c11ffe` -> passed；PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test -- src/components/Panels.test.tsx src/lib/thread.test.ts` -> 112 passed；PM 合并后 `rtk cargo test -p thread-service build_initial_context_uses_root_scope_agent_metadata_path` -> 1 passed；PM 合并后 `rtk cargo build -p app-server --bin app-server` -> passed。
  residual_risk: 未做完整 Electron 手工/Playwright smoke；建议刷新 root-worker 确认 Projects 不再出现 `root_workspace` 分组，且 wait_child project dot 不再显示绿色。no-project `/clear` 已实现但可后续补更细单测。
- commit: 77d082ad8
  summary: 先提交主 checkout 当前现场为 `8e4e91892`，再合并 `82eb5370d` 到主线，修复 root-worker project tree：project first-level subagent 从 depth 1 开始缩进、grandchild 继续递增；普通 selection/status/sidebar rebuild 不再自动展开用户手动折叠的 project；显式选择 child/RightPanel task 入口走 `selectThread()` reveal；project row 复用 tree row 视觉结构并保留 cwd/count/status。
  validation: owner `rtk pnpm install --frozen-lockfile` -> passed；owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 12/12 passed；owner `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk-size warning；owner reviewer `/my_codex/owner_dev_2/reviewer` 两轮通过；PM 合并前 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 12/12 passed；PM 合并前 `rtk git diff --check 82ec71d709359ac47375e6247a6935da686403ed..82eb5370df992481656985571fac1a5be5e0d910` -> passed；PM 合并前 `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk-size warning；PM 合并后 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 12/12 passed；PM 合并后 `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk-size warning。
  residual_risk: 仍缺完整 App 层交互测试直接模拟“折叠 project 后从右侧 task 选择 child 自动 reveal”；RightPanel 现有日期断言失败为既有/独立问题，未纳入本次验证。
- commit: c2f0a9574
  summary: 合并 `b712cefef` 到主线，修复 app-server `thread/start` 在客户端通过 `taskName` 显式提供绝对 root-level agent path 时的解析语义；`taskName` 以 `/` 开头时按 `AgentPath::try_from` 校验并保留 `/owner_dev`，普通 name 仍走 `AgentPath::derive(None, ...)`，role-only 不生成 synthetic path；新增 response、`thread/started` notification、`thread/list` 一致性回归。
  validation: owner 固定 reviewer `/my_codex/owner_dev/reviewer` 两轮通过；owner `rtk cargo test -p app-server parse_thread_start_agent_accepts_root_level_agent_path` -> 1 passed；owner `rtk cargo test -p app-server --test all thread_start_preserves_client_supplied_root_agent_path` -> 1 passed；owner `rtk cargo build -p app-server --bin app-server` -> passed；owner `rtk git diff --check` -> passed；PM 复跑 `rtk cargo test -p app-server parse_thread_start_agent_accepts_root_level_agent_path` -> 1 passed；PM 复跑 `rtk cargo test -p app-server --test all thread_start_preserves_client_supplied_root_agent_path` -> 1 passed；PM `rtk cargo build -p app-server --bin app-server` -> passed。
  residual_risk: `AgentPath::try_from` 会接受当前 contract 下合法的绝对多段路径；如果未来产品要求 `taskName` 只能是一段 root-level name，需要再收紧。`thread/read` 未单独覆盖，但 `thread/list` 已覆盖 stored/listing 表面。
- commit: 26792815c
  summary: 合并 `430e65e71` 到主线，将 root-worker 左侧 `New` popup 改为应用中央 dialog：`NewThreadDialog` 通过 React portal 渲染到 `document.body`，外层 fixed overlay 居中显示，原 `NewThreadPopover` 继续承载表单与创建参数逻辑；增加 Escape、backdrop、Cancel 关闭和窄 viewport 内部滚动。创建 payload 仍沿用 taskName/path name 语义，不提交 canonical `agentPath`。
  validation: owner 固定 reviewer `/root/my_codex_pm/owner_dev/reviewer` 通过；owner `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 10 passed；owner `rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing chunk-size warning；owner `rtk git diff --check` -> passed；owner CSS smoke 确认 `.new-thread-dialog-layer` 为 `position: fixed` 且 dialog centerX 等于 viewport centerX、不在 sidebar 水平范围内。PM `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 10 passed；PM `rtk git diff --check` -> passed。
  residual_risk: 未做完整 Electron/Playwright 点击 Escape/backdrop 自动化；组件测试覆盖结构，CSS smoke 覆盖居中和脱离 sidebar。`/Users/bytedance/Projects/my-codex-dev` 仍停在已合并分支 `fix/new-popup-clipping`，下次派发前需从当前主线重新开分支。
- commit: 8ffc4ccf5
  summary: 修复 compact 后客户端偶发看不到 init context item：定位为 root-worker replacement history hydration/render 层，`contextCompaction` 现在生成 typed compact row 并携带 replacement history 状态与条目；replacement history 中 `role: "developer"` 的 initial context 显示为 `Init Context` context tool row；compact cells 负责归档旧 turn 内容，并丢弃同 turn compact 前内部噪声，避免泄漏到归档计数。
  validation: owner 固定 reviewer `/root/my_codex_pm/owner_main/reviewer` 首轮发现同 turn pre-compact cell 泄漏，返修后复审通过；owner `rtk pnpm test src/lib/conversation.test.ts` -> 38 passed；owner `rtk pnpm test src/components/Conversation.test.tsx` -> 15 passed；owner `rtk git diff --check` -> passed。PM 验收确认实现不靠字符串伪造 init context，不修改后端 model reinject；PM `rtk pnpm --dir apps/root-worker-prototype test src/lib/conversation.test.ts src/components/Conversation.test.tsx src/lib/thread.test.ts` -> 153 passed；PM `rtk git diff --check` -> passed。
  residual_risk: replacement history 中 contextual user fragments 仍按 user message 显示；若未来要完全复刻后端 `InjectedContext` sections，需要后端提供更强 typed replacement display fact。本次只触及 root-worker 前端转换/渲染路径，未运行 app-server build。
- commit: 601b2dd3a
  summary: 合并 `03e1143d9`、`5d7348de6`、`c28bbaa2d` 到主线，完成 root-worker sidebar/agent tree `New` 入口与真实 `thread/start` 参数链路：客户端显示 project path、taskName/path preview、agentType、model/modelProvider、reasoningEffort、serviceTier；客户端只提交 path name/taskName，后端派生 canonical root-level path（无父节点时 `taskName -> /taskName`）；`Thread.agentPath` 作为 typed 返回字段供 root-worker tree 使用；删除 `spawn_agent` 外部 `agent_mode/agentMode` 参数面；扩展 root-level `AgentPath` grammar、registry duplicate protection、root-scope metadata cleanup 和 list scope 隔离。
  validation: owner 侧固定 reviewer 多轮复审通过；PM 侧 `rtk pnpm --dir apps/root-worker-prototype test src/components/Panels.test.tsx` -> 9 passed；`rtk cargo test -p protocol agent_path` -> 7 passed；`rtk cargo test -p app-server parse_thread_start_agent` -> 3 passed；`rtk cargo test -p app-server duplicate_agent_path_create_error_is_invalid_request` -> 1 passed；`rtk cargo test -p thread-service root_scope_agent_path` -> 3 passed；`rtk cargo test -p thread-service root_scope_project_agent_path_is_not_listed_under_root_prefix` -> 1 passed；`rtk cargo test -p thread-service shutdown_all_threads_bounded_submits_shutdown_to_every_thread` -> 1 passed；`rtk cargo build -p app-server --bin app-server` -> passed；`rtk jq empty codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.schemas.json` -> passed；`rtk git diff --check` -> passed.
  residual_risk: duplicate path protection currently covers live registry; if an unarchived persisted thread owns the same path but is not loaded, this merge does not scan and reject it. Root-level project path plus legacy `/root` coexistence should still get one real root-worker smoke run after app-server restart.
- commit: 8f2a272ec
  summary: 修复 compact prompt / replacement history 语义：runtime 不再把 `COMPACT.md` / compact prompt 作为普通 `user` message 注入 compact turn，而是作为 `developer` control item 参与 compact sampling；`compact_turn_final_output()` 以该 control item 为边界提取本次 compact assistant final output。replacement history 继续保留真实 recent user messages 和 assistant continuation seed，但不会把 runtime compact prompt 当成用户输入带入后续 context。
  validation: owner `rtk cargo test -p compact-service replacement_history` -> 2 passed；`rtk cargo test -p thread-service compact_` -> 8 passed；`rtk cargo build -p app-server --bin app-server` -> passed；`rtk git diff --check` -> passed。PM 验收 `git show` 确认改动仅限 `codex-rs/thread-service/src/compact.rs`、`codex-rs/thread-service/src/compact_tests.rs`、`codex-rs/compact-service/src/tests.rs`，实现符合用户要求“user prompt 只是用户输入”。
  residual_risk: 旧历史中已经持久化的 compact user prompt 不会被追溯改写；真实用户手写 checkpoint 文本仍被视为真实用户输入，不做猜测过滤。空闲 dev checkout 尚未同步到 `8f2a272ec`，因为当前均停在 feature 分支现场，后续派发前需同步。
- commit: bc62c5413
  summary: 合并 `06b9b09b1` 到主线，修正 root-worker project sidebar 为单一 `New` 入口 + project/chat 选择菜单；project chat thread 本身作为 project tree root，Project 展开后只渲染该 root 的 subagents，不再额外显示 `PM` / `Project PM` / root row；右侧 Todo action 改为 `Open Project`，设计文档同步为 chat-root 模型。
  validation: PM 侧 `rtk pnpm --dir apps/root-worker-prototype test src/components/RightPanel.test.tsx src/lib/thread.test.ts src/components/Panels.test.tsx src/lib/slashMenu.test.ts` -> 129 passed；`rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing Vite chunk-size warning；`rtk git diff --check dd2f965b5..06b9b09b1` -> passed；fixed reviewer `/root/my_codex_pm/owner_dev/reviewer` 已确认复用规则，`review_project_sidebar_single_entry` 复审通过。
  residual_risk: no-project chat 仍等待后端支持；当前 UI 明确 gated，不用 workspace cwd 伪造。空项目状态仍有输入框和 `Open` CTA 作为 empty-state 辅助入口，但常驻顶部主入口已合并为一个 `New` 按钮。
- commit: 9468eebc8
  summary: 合并 `1f8a1d9bb` 与 `52186fea0` 到主线，实现 root-worker 左侧栏 project PM 模型：左侧根节点改为多个 Project，每个 Project 由一个 PM 管理入口表示，PM 下保留现有 subagent tree；无 project cwd 的 conversations 进入独立 Chat group；同 project 重复 parentless thread 不暴露成多个主 root conversations；New Chat 因后端 no-project thread 语义未确认而明确 gated，不用 workspace cwd 伪造 chat。返修同时清理用户可见 root 文案残留，并将右侧 Todo action 对齐为 Open PM。
  validation: PM 侧 `rtk pnpm --dir apps/root-worker-prototype test src/components/RightPanel.test.tsx src/lib/thread.test.ts src/components/Panels.test.tsx src/lib/slashMenu.test.ts` -> 127 passed；`rtk pnpm --dir apps/root-worker-prototype build` -> passed with existing Vite chunk-size warning；owner `rtk git diff --check` -> passed；independent reviewer passed。owner reported full frontend test has unrelated existing `src/lib/contextUsage.test.ts` failure `3582 !== 19900`.
  residual_risk: Chat mode 仍等待后端明确支持 no-project thread；当前 UI 只 gate/报错，不伪造。未新增 App effect 级集成测试，主要由 pure helper/rendering tests 覆盖；右侧部分内部 prop/legacy helper 名仍可能保留 root 字样但不作为用户概念展示。
- commit: f4a1f73c2
  summary: 合并 `0073579c2` 与 `a1e4eb37` 到主线，拆分 schedule subscription idle 状态为 `waitEventSubscription`，不再把长期/周期 schedule 当成 `WaitCommand` 阻塞 completion；同时删除 parent-side child completion pending/received/dedupe bookkeeping，使 completion 只由 child turn `on_task_finished()` finalization path 投递，followup/status/list/send_event/goal/subscription count change 不再重放旧 completion。
  validation: `rtk cargo test -q -p codex-agent-runtime thread_post_turn` -> 6 passed；`rtk cargo test -q -p thread-service child_completion` -> 8 passed；`rtk cargo test -q -p app-server thread_status` -> 17 + 4 passed；`rtk cargo build -q -p app-server --bin app-server` -> passed；`rtk pnpm --dir apps/root-worker-prototype build` -> passed；`rtk git diff --check` -> passed；owner targeted completion matrix passed；independent reviewer passed.
  residual_risk: `child_completion` filtered test run emits two dead_code warnings for helpers unused under that filter; no functional failure. `maybe_notify_parent_of_final_status_for_current_source` remains as explicit finalization/test helper, but status/read/list production paths no longer call it.
- commit: 8b1ad9614
  summary: 合并 `569a6ba3e` 到主线，`poll_event` started display event 现在携带 `initialTimeoutMs/currentTimeoutMs/hardCapTimeoutMs` metadata，模型可见 arguments 仍保持 `{}`；root-worker 对 in-progress `poll_event` 展示 waiting up to 文案、elapsed/remaining 和 progressbar；并记录 dormant app-server-protocol thread_history tests 未接入的项目事实。
  validation: `rtk cargo test -p thread-service poll_event_ -- --nocapture` -> 5 passed；`rtk cargo test -p app-server limited_replay_keeps_in_progress_poll_event_timeout_metadata -- --nocapture` -> 1 passed；`rtk cargo test -p app-server builtin_poll_event_emits_started_and_completed_thread_items -- --nocapture` -> 1 passed；`rtk cargo test -p thread-history typed_builtin_tool_started_history_keeps_poll_event_timeout_metadata -- --nocapture` -> 1 passed；`rtk pnpm --dir apps/root-worker-prototype test -- src/lib/conversation.test.ts src/components/Conversation.test.tsx` -> 52 passed；`rtk cargo build -p app-server --bin app-server` -> passed；`rtk git diff --check` -> passed
  residual_risk: started metadata 与实际 wait 使用同一 current-window 计算路径但分两次读取，风险较低；`app-server-protocol` dormant thread_history tests 仍是既有技术债，当前有效覆盖在 app-server/thread-history/root-worker 实际消费路径。
- commit: 7cc00da55
  summary: 合并 `ae148b2b4` 到主线，修复 fixed owner/canonical path 的 persisted registry 恢复：同 parent stale generation 会被新 generation 取代，archived/missing metadata child 不会通过 path、thread-id 或 full-tree lazy resume 自动恢复；unknown/null legacy inter-agent JSON envelope 不再裸显成普通 assistant text。
  validation: `rtk cargo test -p thread-service does_not_restore -- --nocapture` -> 4 passed；`rtk cargo test -p thread-service followup_task_by_path_ignores_archived_old_generation -- --nocapture` -> 1 passed；`rtk cargo build -p app-server --bin app-server` -> passed；`rtk git diff --check` -> passed
  residual_risk: 无 metadata 的旧 agent edge 现在按 deleted 处理，不会自动恢复；这是用户明确要求的删除/归档不可恢复语义。显式 archived rollout read/resume 由 owner 侧回归覆盖。
- commit: 860ee00fa
  summary: 合并 `5302c02c5` 及其前置修复到主线，修复 restored completed child 在 `followup_task` 后的 completion 投递语义：恢复 completed child 不会重新投递历史 completion envelope；真实 followup pending input 到达并完成新 turn 后，新的 child completion 仍会投递一次。PM merge 时保留运行时修复，并将回归测试改为关闭自动 turn 调度、等待 pending input 到达后手动完成新 turn，避免 submission 队列竞态。
  validation: `rtk cargo test -p thread-service restored_completed_child_path_resolves_and_receives_followup_after_registry_loss -- --nocapture` -> 1 passed；`rtk cargo test -p thread-service restored_agent_path_resolution_rejects_ambiguous_persisted_duplicates -- --nocapture` -> 1 passed；`rtk cargo build -p app-server --bin app-server` -> passed；`rtk git diff --check` -> passed；owner independent review passed
  residual_risk: 覆盖了 restored completed child 的 canonical followup path 和 ambiguous duplicate path；其他 restored child 重新接收输入的入口若绕过相同 pending-input path，后续仍需按具体 bug 增补测试
- commit: a96d6e921
  summary: 合并 `9674ea252` 到主线，让 root-worker 右侧 Schedules agenda 的每个日期分组可折叠；默认展开，日期 header 是带 `aria-expanded` / `aria-controls` 的 button，折叠状态用组件本地 `Set<dateKey>` 维护，不影响 active schedule list、agenda 数据或 recurrence 计算
  validation: `rtk pnpm --filter @my-codex/root-worker-prototype exec tsx --test src/components/RightPanel.test.tsx` -> 10 passed；`rtk git diff --check`
  residual_risk: 现有测试栈主要用 static markup，未覆盖完整 DOM 层父组件 state 点击交互；已通过组件边界测试覆盖默认展开、toggle 回调和单组折叠不影响其他日期
- commit: 6d3ef1ae2
  summary: 合并 schedule agenda view 到主线；`RightPanel` 的 Schedules 区域新增 `Upcoming` agenda，按日期分组展示未来有限 schedule occurrences，同时保留 active schedule list。实现为 root-worker 纯客户端派生视图，默认最多 20 条、未来 7 天；支持 every_interval、once_after、once_at、every_day_at、every_week_at，并保持 unsubscribe 后 active/agenda 都移除。该完成项包含 `c766afb5b` 合并 `19d6bedd8`，以及 `6d3ef1ae2` 合并 `231d5c195` 的 locale-stable test fix。
  validation: `rtk pnpm --filter @my-codex/root-worker-prototype exec tsx --test src/lib/threadAnalysis.test.ts src/components/RightPanel.test.tsx` -> 25 passed；`rtk git diff --check`；owner 侧 `rtk pnpm --filter @my-codex/root-worker-prototype exec tsc --noEmit` 仍有既有 unrelated TS errors in `App.tsx`, `conversationCompact.ts`, `slashMenu.test.ts`
  residual_risk: daily/weekly 的复杂 DST 边界仍依赖浏览器 Intl；agenda 日期 label 跟随用户默认 locale，测试通过 dateKey 保持稳定；旧历史缺 typed schedule args 时只显示 active fallback，不生成 agenda
- commit: 2b555fe46
  summary: 合并 `fd77a5912` 到主线，优化 root-worker schedule 展示文案；Conversation 和 RightPanel 共用 `scheduleDisplay.ts`，RightPanel 优先使用 typed `arguments.schedule` 显示 `every_interval 6h`，缺 structured args 的旧历史仍 fallback 到原 `schedule_summary`
  validation: `rtk pnpm --filter @my-codex/root-worker-prototype exec tsx --test src/lib/conversation.test.ts src/lib/threadAnalysis.test.ts src/components/RightPanel.test.tsx` -> 59 passed；`rtk rg -n "every 21600000 ms" apps/root-worker-prototype/src/lib apps/root-worker-prototype/src/components` -> only test input/fallback/doesNotMatch；`rtk git diff --check`
  residual_risk: 文案仍采用当前工具式 `every_interval 6h` 风格；后续若做 calendar/agenda 视图，可再统一成更产品化的 `Every 6 hours`
- commit: 51df8798a
  summary: 合并 `74ee8ce60` 到主线，修复 `schedule_subscribe` 成功后 conversation/thread item 不展示、右侧 Schedules 不显示的问题；schedule extension tools 现在只对白名单 `schedule_subscribe` / `schedule_unsubscribe` 发 typed builtin display lifecycle event，root-worker Schedules 面板消费 builtin/eventDriven schedule item 并只在 completed + subscription id / completed unsubscribe true 时增删 active schedule。
  validation: `rtk cargo test -p app-server thread_read_stays_active_while_event_subscription_is_pending -- --nocapture`；`rtk pnpm test src/lib/threadAnalysis.test.ts src/lib/conversation.test.ts src/components/RightPanel.test.tsx`；`rtk cargo build -p app-server --bin app-server`；`rtk git diff --check`；independent review passed
  residual_risk: 未补完整 dispatch/replay 层测试证明 `memories/read` 不会产生 display event；当前由 schedule-only guard 单测和代码审查兜底，风险较低
- commit: 8bd08cbef
  summary: 合并 `b14f41bd4` 到主线，新增 `model_auto_compact_soft_ratio` / `model_auto_compact_hard_ratio`，默认 auto compact usage ratio 从旧 `0.70/0.85` 调整为 `0.80/0.90`；`model_auto_compact_token_limit` 仍保持 token 数语义
  validation: `rtk cargo test -p compact-service soft_compact -- --nocapture`；`rtk cargo test -p compact-service-api -- --nocapture`；`rtk cargo test -p thread-service auto_compact_decision_gate -- --nocapture`；`rtk cargo build -p app-server --bin app-server`；`rtk rg -n "0\\.70|0\\.85" codex-rs/thread-service/src codex-rs/compact-service/src codex-rs/config/src` 无命中；`rtk git diff --check`
  residual_risk: `rtk cargo test -p config-service load_config_reads_auto_compact_ratios -- --nocapture` 被既有 config-service 编译问题阻塞
- commit: 675b750b7
  summary: 合并 `7242b918c` 到主线，修复 reload 后 persisted/listed subagent canonical path 不能被 `followup_task` 复用的问题；resolver live miss 后可按 persisted spawn tree lazy restore 原 child thread 并注册 path metadata，duplicate persisted path 显式 ambiguous
  validation: `rtk cargo test -p thread-service restored_completed_child_path_resolves_and_receives_followup_after_registry_loss -- --nocapture`；`rtk cargo test -p thread-service restored_agent_path_resolution_rejects_ambiguous_persisted_duplicates -- --nocapture`；`rtk cargo test -p thread-service resume_agent_respects_max_threads_limit -- --nocapture`；`rtk cargo build -p app-server --bin app-server`；`rtk git diff --check`
  residual_risk: 只覆盖 canonical path restore/ambiguous/max_threads 的目标路径；更广义 runtime registry 恢复仍需后续按具体问题补测试
- commit: dd09a6b3a
  summary: 将普通 app-server SQLite log DB subscriber 默认 filter 从 `Level::TRACE` 降为 `Level::WARN`，减少 TRACE/DEBUG/INFO 级别事件默认进入本地 log DB 队列和 SQLite batch 写入；stderr `RUST_LOG`、feedback、OTEL、rollout-trace 均未改动
  validation: owner 已通过 `rtk cargo build -p app-server --bin app-server` 和 `rtk git diff --check`，独立 review 通过；PM 侧确认 `codex-rs/app-server/src/lib.rs` 仅一行 `Level::TRACE -> Level::WARN`，`EnvFilter::from_default_env()` 与 OTEL layer 未变，`rtk git diff --check` 通过
  residual_risk: 未新增专门覆盖 log DB 默认 filter 的测试，因改动是 `tracing_subscriber::filter::Targets` 级别常量替换，风险较低
- commit: 8b0a760e6
  summary: 修复主 checkout 当前 `rtk cargo clippy -p app-server --bin app-server --all-targets -- -D warnings` 失败项；包含 clippy 机械修复、删除已无生产调用的 `wait_agent_tool` 兼容 facade 及 helper、将 workflow registry discovery 改用 `TurnContext::discovery_context()`、对仅测试使用 helper 加 `#[cfg(test)]`、对必须保持 scheduler 原子语义的锁内 await 使用局部 `#[expect(..., reason = ...)]`
  validation: owner 已通过 `rtk cargo clippy -p app-server --bin app-server --all-targets -- -D warnings` 和 `rtk git diff --check`；PM 侧复跑 `rtk cargo clippy -p app-server --bin app-server --all-targets -- -D warnings` 输出 `cargo clippy: No issues found`，`rtk git diff --check` 通过；独立 reviewer 多轮通过，覆盖 wait_agent 删除、workflow discovery、scheduler expect、websocket 分支、测试侧 clippy 修复
  residual_risk: `multi_agent.rs` 属于既有脏文件且本任务共同触碰，主要行为风险是删除旧 `wait_agent` 兼容壳，但 reviewer/owner 已确认生产 tool surface 无残留调用，项目长期方向也是等待统一走 `poll_event`
- commit: 6d5a76b65
  summary: 合并 `048c163dc` 到主线，修复 reload/restart 或 live runtime 丢失后 completed subagent 从 `list_agents` / agent tree 缺失的问题；`AgentControl` 现在用 persisted `thread_spawn_edges` 与 thread metadata 补充 registry view，支持从子线程反查 spawn root、按 canonical path 去重并保持 live registry 优先；direct subagent paths 合并 persisted open children，但 active 判断仍走 live runtime，不把 completed child 伪装成 active
  validation: owner 已通过 `rtk cargo test -p thread-service list_agents_restores_completed_child_from_persisted_root_when_registry_is_empty -- --nocapture`；`rtk cargo test -p thread-service direct_subagent_paths_ -- --nocapture`；`rtk cargo test -p thread-service persisted_agent_restore_deduplicates_by_path_with_live_registry_preferred -- --nocapture`；`rtk cargo test -p thread-service list_agents_restores_completed_child_from_persisted_history_when_live_thread_is_gone -- --nocapture`；`rtk cargo build -p app-server --bin app-server`；`rtk git diff --check`。PM merge 后补跑 `rtk cargo test -p thread-service list_agents_restores_completed_child_from_persisted_root_when_registry_is_empty -- --nocapture`、`rtk cargo build -p app-server --bin app-server`、`rtk git diff --check` 均通过。
  residual_risk: `rtk cargo test -p state thread_spawn_edges_track_directional_status -- --nocapture` 仍被 state crate 既有编译问题阻塞（`state/src/log_db.rs` 缺 `StateRuntime` import、memory tests `assert_eq` 宏歧义等），新增 state 层断言未能单独执行；损坏或缺失 persisted metadata 时会安全降级为不列出 pathless persisted child
- commit: bf7e727ad
  summary: 合并 `729efa464` 到主线，线性化 post-turn pending input 调度：mailbox 查询不再隐式 drain，`on_task_finished()` 拆成 prepare/record-leftover/TurnComplete/finalize/后续副作用，goal continuation reserve 后复核 active goal identity，finishing/no-task active turn 下 late user/hook input 转入 next-turn pending
  validation: owner 已通过 `rtk cargo test -p thread-service task_finish -- --nocapture`；`rtk cargo test -p thread-service active_goal_continuation_runs_again_after_no_tool_turn -- --nocapture`；`rtk cargo test -p thread-service pending_request_user_input_does_not_spawn_extra_goal_continuation -- --nocapture`；`rtk cargo test -p thread-service completed_goal_accounts_current_turn_tokens_before_tool_response -- --nocapture`；`rtk cargo test -p thread-service goal_continuation_reservation -- --nocapture`；`rtk cargo test -p thread-service mailbox_queries_do_not_implicitly_drain_incoming_mail -- --nocapture`；`rtk cargo build -p app-server --bin app-server`；`rtk git diff --check`；PM 已按 brief 核对 goal identity 二次校验、finishing late input 排队、TurnComplete 前后顺序
  residual_risk: finishing 窗口并发路径仍复杂，建议后续补更直接的受控并发测试；`rtk cargo test -p model-service resolve_provider_for_selection` 仍有既有缺文件问题，fmt check 也受既有格式差异干扰
- commit: 5a2e47483
  summary: 合并 `1432b8ac2` 到主线，修复 compact/reconstruction 后 `thread/read` live+persisted merge 过度恢复 persisted assistant items 导致 compact 返回/后续 assistant 输出重复可见的问题；merge 现在只回填 persisted `InjectedContext`。同时恢复 disabled project 的 repo-local instruction/workflow init context，并补 canonical containment，避免 instruction/workflow symlink 逃逸和 disabled duplicate workflow 挤掉 enabled workflow
  validation: owner 已跑 `rtk cargo test -p app-server restore_persisted_injected_context_turns -- --nocapture`；`rtk cargo test -p app-server thread_read_after_auto_compaction_preserves_init_context_without_dup_live_assistant_items`；`rtk cargo test -p thread-service instruction_sources_`；`rtk cargo test -p thread-service disabled_project_instruction_files_`；`rtk cargo test -p thread-service build_initial_context_skips_disabled_project_workflow`；`rtk cargo test -p thread-service build_initial_context_keeps_enabled_workflow_when_disabled_project_duplicates_id`；`rtk cargo build -p app-server --bin app-server`
  residual_risk: app-server integration 只锁住 compact 后 init-context item 仍在且 `FINAL_REPLY` 不重复；`instruction_files` 具体 thread item 形态仍主要由 thread-service 单测覆盖。build 仍有既有 warnings，未在本任务内清理
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
