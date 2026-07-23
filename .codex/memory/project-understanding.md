# Project Understanding

## Stable Working Rules
- 所有 shell 命令必须以 `rtk` 开头。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 当前项目的 PM / owner / reviewer 协作规则以 `.codex/agents/project-pm.agent.md` 及对应 owner agent 定义为准。
- 我们自己的 agent/runtime 产品名定为 Morpheus；外部官方 Codex provider 仍称 `codex_cli` / external Codex CLI provider。
- 代码 crate、模块、变量名默认使用语义名，除非明确表达产品本身语义，否则不要带 Morpheus/Codex 等产品名。

## System Model
- 这个项目的核心不是单一 CLI，而是一套围绕 thread、agent、tool call、event replay 和客户端展示组织起来的运行时系统。
- owner 需要长期区分 3 类东西：
  - live runtime state：当前 turn、agent、command、wait 状态等即时运行态
  - persisted event/history：供 reload、history replay、UI 恢复使用的持久化事实
  - model-visible context：提供给模型推理的 history / response items / compact context
- 许多问题都来自把这 3 层混在一起；排查时先判断故障发生在 live path、persisted replay，还是 model context。

## Stable Architecture Rules
- app-server / root-worker conversation display 走 typed `EventMsg -> ThreadItem` 路径。
- `ResponseItem` 主要用于模型交互、模型可见 history/context，不应用作 display-only 展示源。
- 不要从 raw marker、assistant JSON envelope 或 legacy 解析路径反解客户端展示项。
- external code agents 使用独立 external tool surface（如 `spawn_external_agent`、`followup_external_task`、`list_external_agents`），不通过内部/native `spawn_agent` 的 provider 参数暴露给 native 模型。
- 命名边界：Morpheus 指本项目自己的 agent/runtime 产品；`codex_cli` 指外部官方 Codex CLI/server provider。不要用 “Codex agent” 泛指 Morpheus/native agent，避免和 external Codex provider 混淆。
- external CLI agent 的协作协议由后端 bridge 在启动 context 中注入 JSON tool schema/call/result 约定；provider raw stdout/JSON 只在后端 adapter 解析，UI 不解析 raw external JSON。
- external tools 与 internal tools 可共享 AgentControl / InterAgentCommunication / pending input / completion 事实源，但 model-visible tool 名称和 schema 必须分离。
- `thread/read` 和 `thread/turns/list` 的 live persisted history 读取已迁到 `LiveThreadHistoryRuntime` / `AppServerLiveThreadHistoryRuntime`。
- listener/event-stream 入口已迁到 `LiveThreadListenerRuntime` / `LiveThreadListenerHandle`；idle-unload shutdown/removal、listener lifecycle live `AgentStatus` read 和 TurnComplete post-turn `ThreadRuntimeStatus` read 已迁到 `ThreadLifecycleRuntime`，running resume usage replay 已迁到 `LiveThreadUsageRuntime` / `AppServerLiveThreadUsageRuntime`，running resume goal effects/idle continuation 已迁到 `LiveThreadGoalRuntime` / `AppServerLiveThreadGoalRuntime`。listener handle 不再暴露 shutdown/wait、token/context usage、goal resume/continue effects、`AgentStatus` copied read 或 `ThreadRuntimeStatus` copied read。旧 `LiveThreadRegistry` / `AppServerLiveThreadRegistry` facade 已删除。后续不要通过恢复 broad registry 来获取 live handle；需要新能力时应继续拆窄 runtime 或挂到明确的 provider-neutral runtime 边界。
- Memory consolidation startup/shutdown/status/token usage 已从 broad `AppServerLiveThreadHandle` 迁到 memory-specific `AppServerMemoryConsolidationThreadHandle`；memory code 只需要 submit user input、agent status、wait terminated、token usage 和 shutdown，不应访问 config/read/history/context/goal/listener 能力。
- 等待模型的长期方向已确定为统一收口到 `poll_event`；`wait_agent` / `command_wait` 应删除，不再作为独立 tool surface 保留。
- `poll_event` 需要支持在同一个 turn 内等待并在 event 到达后继续执行；runtime 不应维护会影响状态机的硬性等待目标，event 应直接作为 pending input 注入当前 turn，并携带来源信息供模型判断。
- `poll_event` 对模型暴露的是空参数对象；默认 `initial_timeout_ms` / `hard_cap_timeout_ms` 由 thread runtime 从 `TurnContext.config.multi_agent_v2` 注入，再结合 thread-scoped backoff 计算每次调用的 `current_timeout_ms`。
- command output / exit、child completion、inter-agent completion 应复用同一套 pending-input 唤醒链路；不要再为某一类等待事件维护平行 wait API。
- parent-side child completion bookkeeping 只用于 completion envelope 的投递、去重和清理，不应定义 child 当前是否 active；`WaitChild` / `IdleWaitChild` 只应由 direct child thread 的本地 active 状态驱动。
- thread init context 中的 workflow discovery 依赖 `TurnContext::discovery_context()`，其 project workflows 来自 `config.config_layer_stack` 中各 project layer 的 `.codex/workflows`。
- session 初始化阶段的 `instruction_files` 应按本地 config 路径读取，不应依赖 primary execution environment 是否存在。
- ThreadProvider runtime API split 的当前边界是：`ThreadServiceApi` 作为兼容 facade，组合 `ThreadLifecycleRuntime`、`ThreadCollaborationRuntime` 和 `ThreadEventRuntime`；`NativeAgentRuntime` 承载 Morpheus-only `spawn_agent` / `followup_task` / `close_agent` / `list_agents`。后续新增 provider/thread 能力时优先挂到窄 trait 或 provider-neutral handle，不要继续扩大旧 facade。
- `ThreadLifecycleRuntime` 当前已承载第一批真实 provider-neutral lifecycle 方法：`shutdown_all_threads_bounded`、`shutdown_live_thread`、`remove_live_thread`、status read/subscribe、`subscribe_thread_created`、`active_event_subscriptions`，并由 app-server thread processor/listener idle-unload 和 thread-service `AgentControl` 直接依赖；root start/resume/fork 仍保留在 app-server-local 过渡 trait 中，因为这些请求仍携带完整 `Config`、dynamic tool、environment selection 等 native/app-server 结构。不要把 shell、MCP、approval、dynamic tool、agent job、完整 `ThreadTurnCapability` / `Session` 塞进 ThreadProvider 或 external provider adapter。
- `ThreadCollaborationRuntime` 目前仍保留 native/external model-visible tool surface 分离；这只是兼容入口，不代表 external provider 可以实现 native `agent_type` / role / model 语义。
- MCP refresh 已从旧 `LiveThreadRegistry` 过渡路径迁到 provider-neutral live inspection/command surfaces：live thread ids、config refresh snapshot 走 inspection runtime，`Op::RefreshMcpServers` submit 走 command runtime；app-server 只保留 MCP server planning 和 latest config rebuild helper。
- Feedback upload 的 feedback-only runtime reads 已从旧 `LiveThreadRegistry` blanket impl 迁到 `LiveThreadFeedbackRuntime` / `AppServerLiveThreadFeedbackRuntime`：subtree ids、guardian rollout path、session source 都是 copied/derived metadata，不需要暴露 concrete live handle；feedback live rollout path lookup 仍走 inspection runtime。
- Thread goal processor 的 external goal prepare/apply side effects 已从旧 `LiveThreadRegistry` blanket impl 迁到 `LiveThreadGoalRuntime` / `AppServerLiveThreadGoalRuntime`；app-server 仍拥有 persisted goal mutation 和 response/ordered notification 顺序，live runtime 只承接 prepare/apply 副作用；旧 `LiveThreadRegistry` facade 已删除对应 goal prepare/apply methods。
- Thread processor 的 out-of-band elicitation pause counter increment/decrement 已从 `AppServerLiveThreadRegistry` 迁到 `LiveThreadElicitationRuntime` / `AppServerLiveThreadElicitationRuntime`；app-server request path 只拿返回 count 生成 `paused: count > 0`，底层 0->1 / 1->0 pause transition 仍由 live thread runtime 拥有；旧 `LiveThreadRegistry` facade 已删除对应 counter methods，后续 caller 不应重新依赖旧 registry surface。
- Listener skill watch path resolution 已从旧 `AppServerLiveThreadRegistry` / `LiveThreadRegistry` 迁到 `LiveThreadSkillWatchRuntime` / `AppServerLiveThreadSkillWatchRuntime`；listener 只获取 copied `Vec<SkillWatchPath>`，resolution 失败仍 warn + empty fallback，watch registration/unload timing 不变；旧 registry facade 已删除对应 `thread_skill_watch_paths` methods。
- Thread read/listing 的 copied token/context usage reads 已从旧 `AppServerLiveThreadRegistry` / `LiveThreadRegistry` 迁到 `LiveThreadUsageRuntime` / `AppServerLiveThreadUsageRuntime`；history load 和 persisted/live merge 仍留在原路径，usage runtime 只返回 copied `TokenUsageInfo` / `ThreadContextUsage`，旧 registry facade 已删除对应 usage read methods。
- Listener idle-unload 的 shutdown/removal 已迁到 provider-neutral `ThreadLifecycleRuntime::shutdown_live_thread` / `remove_live_thread`；listener full handle/event stream 不再暴露 shutdown/wait，old registry facade 已删除对应 removal method。
- Bespoke event handling 的 `CollabCloseEnd` receiver loaded check 已从旧 `AppServerLiveThreadRegistry` / `LiveThreadRegistry::is_thread_loaded` 迁到 `LiveThreadInspectionRuntime::is_live_thread_loaded`；watch cleanup 和 notification ordering 不变，old registry facade 已删除对应 loaded-check method。
- 旧 `LiveThreadRegistry` 的 unused `shutdown_thread_and_wait` / `wait_thread_until_terminated` facade methods 已删除；真实 shutdown command 现在走 `ThreadLifecycleRuntime::shutdown_live_thread`，per-handle wait/shutdown 仍保留在 `LiveThreadHandle`，listener/event-stream 和 memory consolidation 已拆到各自窄 handle。
- Turn context override pre-submit validation 已从旧 `LiveThreadRegistry::validate_thread_turn_context_overrides` 迁到 `LiveThreadTurnRuntime` / `AppServerLiveThreadTurnRuntime`；validation 仍只在 `has_any_overrides` 时发生，并且仍在 user input enqueue 前失败，真正 override 应用和 input 排队顺序不变。旧 registry facade 已删除该 validation method，app-server-local `TurnProcessorRuntime` leftover bucket 也已删除。
- File subscription append 已从旧 `LiveThreadRegistry::append_thread_conversation_item` 迁到 `LiveThreadConversationRuntime::append_live_thread_conversation_item`；该 runtime 保留旧 `append_message` 行为，即 subscription item 作为 async input 进入 live thread 并可触发 pending work。不要用 `inject_thread_conversation_items` 替代这一路径，因为它只记录 model-visible history/rollout，不等价于 append async input。旧 registry facade 已删除对应 append method。
- 旧 `LiveThreadRegistry` / `AppServerLiveThreadRegistry` compatibility facade 已删除；已迁到窄 runtime 的 copied reads、commands、status、feedback metadata、turn validation、conversation append、history read、usage、skill-watch、goal、elicitation、listener/event-stream 等能力不应再通过 broad registry 暴露。
- app-server thread processor 的 broad `ThreadProcessorThreadRuntime` 已删除；native root start/resume/fork creation 和 environment default/validation 现收口到 thread-service crate 内的 native-only `NativeThreadCreationRuntime` / `NativeThreadEnvironmentRuntime`。这些 trait 仍携带完整 `Config`、`InitialHistory`、dynamic tools、environment selections、agent metadata 等 native/app-server DTO，不是最终 provider-neutral `ThreadProvider` contract；不要把它们伪装成 external provider root start support。
- turn request 的 environment selection validation 已复用 `NativeThreadEnvironmentRuntime`；`TurnProcessorRuntime` 已删除，不再承载 environment validation、live config read 或其他 native-only turn 能力。
- `thread/inject_items` 的 live conversation injection 已迁到独立 `LiveThreadConversationInjectionRuntime` / `AppServerLiveThreadConversationInjectionRuntime`。Injection 语义是直接记录 prebuilt `ResponseItem` 到 live conversation history，不 enqueue async input、不触发 pending work；不要与 subscription append 的 `LiveThreadConversationRuntime::append_live_thread_conversation_item` 混用。
- `turn/steer` 的 live input capability 已迁到 thread-service crate native module 的 `NativeThreadSteerRuntime` 和 app-server object-safe `AppServerLiveThreadSteerRuntime`。它保留 `ThreadService::steer_thread_input` / `SteerInputError` typed active-turn semantics，不应改成普通 `LiveThreadCommandRuntime::submit_live_thread_op_with_trace(Op::UserInput...)`，也不应放入 provider-neutral `thread-service-api` 以泄漏 native steer error DTO。
- detached review 的 current-history fork 和 metadata-only stored read 已迁到 thread-service crate native module 的 `NativeDetachedReviewRuntime`。它仍依赖 native `Config`、`ForkSnapshot::Interrupted`、current-history fork 和 `StoredThread` DTO，不是 provider-neutral lifecycle API；app-server 仍负责 listener attach、watch upsert、`ThreadStarted` notification 和 review turn submit orchestration。
- turn memory startup 使用的 live full `Config` read 已迁到 thread-service crate native module 的 `NativeMemoryStartupConfigRuntime`。它继续返回完整 `Arc<Config>` 给 `AppServerMemoryStartupAdapter` / `build_memory_startup_settings`，不能用 copied `ThreadConfigSnapshot` 替代；`ThreadConfigSnapshot` 仍只服务 app-server response/session-source assembly 等 copied metadata 语义。
- live thread agent/runtime status read / subscribe 已提升到 provider-neutral `ThreadLifecycleRuntime`；app-server thread/turn processors 和 thread-service `AgentControl` 不再依赖 `AppServerLiveThreadStatusRuntime` / `LiveThreadStatusRuntime`。agent status read 语义仍是 external live record 优先、否则 native live thread status；runtime status read 对 native 保留 post-turn wait semantics，对 external live record 只做 `PendingInit`/`Running` -> `Active`、terminal/interrupted/not-found -> `Complete` 的粗粒度映射；subscribe 支持 native live thread watch 和 external live record watch，但不代表 reload/list_agents 或 root start provider routing 已完成。旧 `LiveThreadStatusRuntime` compatibility surface 已删除。
- thread-created/status-changed internal event 现在可携带权威 `AgentStatus` payload；app-server 收到 payload 时优先映射成 `ThreadLifecycleStatus`，不再依赖 live status read 才能发送 terminal status notification。external close 会在 terminal Shutdown notification/persistence 后清理 external live record；外发 `ThreadStatusChangedNotification` shape 未变，reload/list_agents 和 root provider routing 仍未完成。
- app-server archive 前和 native agent cleanup 的 per-thread live shutdown 已提升到 provider-neutral `ThreadLifecycleRuntime`；app-server thread processor 和 `AgentControl` 不再依赖 `AppServerLiveThreadShutdownRuntime` / `LiveThreadShutdownRuntime`。底层 shutdown 语义仍复用旧逻辑：rollout materialize/flush、status/action decision、必要时提交 `Op::Shutdown`、native found thread wait-until-terminated。旧 `LiveThreadShutdownRuntime` compatibility surface 已删除。
- app-server archive/listener teardown 与 native agent cleanup 的 live-thread removal 已提升到 provider-neutral `ThreadLifecycleRuntime`；app-server command wrapper 不再暴露 `remove_live_thread`。removal bool 语义是本次调用是否删除 native live thread 或 external live record，并且只在实际删除 native thread 时释放 uncounted metadata。旧 `LiveThreadCommandRuntime::remove_live_thread` compatibility method 已删除。

## Owner-Level Design Understandings
- conversation display 的真实设计意图是“事件先 typed 化，再展示”；任何 UI 修复都应先补齐 event/replay/typed item 链路，而不是在前端做字符串补丁。
- 判断“某条信息为什么丢了”时，要先问它是否进入了正确的持久化层，而不是只看 live path 下是否一度可见。
- 对稳定的 thread item，长期正确方向应是“live 可见 == reload 可恢复”；如果某类 item 只在 live 可见、reload 缺失，默认视为持久化或 replay 设计不完整，而不是产品预期。
- 但 live 与 reload 不是同一条实现链：live thread item 主要走 `tool-service/app-server-protocol event_mapping -> server notification -> root-worker thread state`，reload/thread_read 主要走 `rollout Limited -> thread-history/app-server-protocol replay -> thread snapshot`。排查缺 item 时先判断是哪一条链路在丢。
- thread reload / history replay 默认只消费 rollout `Limited` 视图；因此“重启后能否恢复”本质上是持久化策略问题，不只是 replay 代码问题。
- `commandExecution` 的历史重建依赖 thread history 对 `ExecCommandBegin` / `ExecCommandEnd` 的重放；如果 rollout limited 缺少所需事件，重启后就无法恢复 `exec_command` thread item。
- 对 `thread/read` / `thread/resume` / `thread/turns/list` 这类“persisted history + live turn merge”路径，只有仍处于 in-progress 的 live turn snapshot 才能覆盖 persisted history；带 fallback 语义的“最后一个 turn 快照”不能当作 live turn merge 回去，否则会把已完成 command 或其他 finished item 重新盖回成 stale live residue。
- `list_agents` 当前依赖 runtime 的 `registered_agents()` 索引和 `live_thread_agent_status(...)`，不是直接从持久化 thread store 列 completed agent；reload 后如果未把已完成 agent 恢复到 runtime 注册表，`list_agents` 就会缺失它们。
- owner 在处理“看起来像前端问题”的缺失、重复、状态错乱时，应默认检查 `rollout policy -> thread history replay -> runtime registration -> renderer` 这条链，而不是直接改 renderer。

## Stable Module Boundaries
- `thread-service` 负责 runtime、session、agent control、tool runtime 和大部分“系统现在怎么跑”的语义。
- `app-server` 负责对外请求处理、thread start、history 组装，以及把底层运行时能力暴露给客户端。
- `rollout` 负责定义哪些 `EventMsg` 会被持久化到哪些视图；它决定 reload 时理论上“有什么可恢复”。
- `app-server-protocol` 负责把 persisted event/history replay 成 typed thread items；它决定客户端“如何恢复显示”。
- `config` 负责运行时配置来源与覆盖优先级，包括 compact prompt、project/home config layer 等。
- `workflow` 负责 workflow runtime bridge；其等待语义也必须服从统一的 `poll_event` 设计。
- `apps/root-worker-prototype` 负责客户端展示、交互和 renderer 状态，但它不应成为修复后端事实缺失的补丁层。

## Key Module Map
- `codex-rs/thread-service/`
  - 负责线程运行时、compact、session、agent control 等核心后端逻辑。
  - `src/session/codex_runtime.rs` 负责 session spawn/init；这里会装载 user instructions。
  - `src/agents_md.rs` 负责 `instruction_files` 与 user instructions 的拼装。
  - `src/session/turn_context.rs` 负责 workflow discovery context 的 project/home roots。
  - `src/tools/` 与 turn wait runtime 相关模块是统一等待语义的主要收口位置；后续等待行为应围绕 `poll_event` 组织，而不是新增平行 wait tool。
- `codex-rs/config/`
  - 负责运行时配置加载，包括 compact prompt 的读取优先级。
- `codex-rs/app-server/`
  - `src/request_processors/thread_processor/ops.rs` 负责 `thread/start` 的 config 派生、thread 创建与初始响应。
  - `src/request_processors.rs` 的 `build_api_turns_from_rollout_items()` 只按 `EventPersistenceMode::Limited` 重建 API thread history。
  - `tests/suite/thread_start.rs` 是 thread/start init context 与 instruction sources 的端到端回归测试位置。
- `codex-rs/thread-history/`
  - 是 `app-server thread/read` 与部分 reload 恢复的真实 history builder。
  - 如果 `app-server-protocol` 的 replay 测试是绿的，但 `app-server thread/read` 仍丢 item，优先检查这里的 `handle_event()` 是否漏分派对应 display event。
- `codex-rs/rollout/`
  - `src/policy.rs` 定义哪些 `EventMsg` 会进入 rollout `Limited` / `Extended` 持久化；reload 丢 item 时先检查这里。
- `codex-rs/app-server-protocol/`
  - `src/protocol/thread_history/tool_events.rs` 负责把 persisted event replay 成 typed thread items；`exec_command` 的恢复链路在这里。
  - 它不是 `app-server thread/read` 的唯一事实来源；与 `thread-history` 之间存在行为漂移风险。
  - `src/protocol/event_mapping.rs` 负责 live event 到 typed thread-item notification 的映射；`BuiltinToolCallStarted/Completed -> ThreadItem::BuiltinToolCall` 这类 live display 缺口要先查这里，而不是 `thread-history`。
  - `src/protocol/thread_history/tests/` 当前未接入 `app-server-protocol` crate 测试树；直接接入会暴露既有缺失 import/type 的编译问题。不要把这里的新增用例当作有效验证，除非先修复并接入整套测试。
- `codex-rs/workflow/`
  - workflow runtime bridge 的 agent wait 语义也应统一走 `poll_event`，不要继续依赖独立 `wait_agent` API。
- `apps/root-worker-prototype/`
  - 负责 root-worker prototype 客户端、thread 展示、compact UI 与 renderer 状态。
  - `src/lib/conversation.ts` 已支持 `ThreadItem::BuiltinToolCall` 的 `poll_event` 展示文案；若 live UI 仍缺失，要先怀疑 server notification / 运行实例版本，而不是前端 summary 缺口。
  - `poll_event` 的主文案当前只展示 `sourceHint` / `timeout` / `failed` 等输出态，不会在 summary 文案里额外展开 arguments；而且该 tool 的 arguments 按设计本来就是 `{}`。

## Persistent State And Recovery
- thread/history 的 reload 恢复能力由两段共同决定：
  - rollout policy 是否把需要的事件写进可被 reload 消费的持久化视图
  - thread history / protocol 是否能把这些事件重放成正确的 typed item
- 因此 reload 类 bug 的长期排查顺序应优先是：
  - 事件是否发出
  - 事件是否被 `Limited` 或其他实际消费的视图持久化
  - replay 是否覆盖该事件
  - runtime 或 renderer 是否正确消费 replay 结果
- 不能假设 `Extended` 中有事件就足够；如果 reload 路径只读 `Limited`，那对恢复语义来说 `Extended` 等于不存在。
- 进入 `Limited` 的恢复型事件仍要保持 payload 有界；否则 `thread/read` / reload 会把大输出原样带回客户端。对 `ExecCommandEnd` 这类事件，持久化 sanitize 与展示恢复语义需要一起考虑。
- `UnifiedExecStartup` / `Agent` 来源的 `ExecCommandBegin` 与 `ExecCommandEnd` 应进入 `Limited` 以支持 reload；`UserShell` / `UnifiedExecInteraction` 不应借此进入可恢复展示路径。

## Runtime And Agent Lifecycle
- agent 是否可见、是否 active、是否 complete，不能只从某一个 bookkeeping 字段推断；需要区分本地 active 状态、completion 投递状态，以及 reload 后是否重新注册到 runtime 索引。
- child completion 相关逻辑的长期目标是“完成态事件投递正确”而不是“靠 bookkeeping 假装 child 仍 active”。
- `list_agents` 这类查询面向的是 runtime 可见集合；如果需求是“重启后仍能列出已完成 agent”，通常要检查恢复后的 runtime 注册语义，而不是只改查询接口。
- external agent 的长期目标是成为 backend thread/provider execution mode：thread lifecycle、followup/pending input、tool loop、close/abort、parent completion 应与 native thread 对齐，差异只应留在 model IO / provider transport adapter。
- external runtime 不应再是纯 live-only registry：external spawn 应创建 persisted thread-store thread 与 thread-spawn edge，external input / assistant output / tool-result / terminal status 应进入可 replay 的 bounded rollout history。reload 后 completed external 应可通过 persisted metadata/list_agents 恢复；running external 若没有可重连 provider session，应明确收口为 interrupted，不能静默丢失或伪装 active。
- external durable default list / agent-reference recovery 基于 Open thread-spawn edge：Open + terminal Completed 的 external agent reload 后可列出，Open + 无可重连 live process 的 external agent reload 后列为 Interrupted；显式 close 后的 Closed + Shutdown external agent 不应进入默认 `list_agents`，也不应由 `resolve_agent_reference` 从 Closed edge 恢复。
- cold resume/fork 后的 goal continuation 已迁到 `LiveThreadGoalRuntime` / `AppServerLiveThreadGoalRuntime`；app-server 的 `ThreadGoalRequestProcessor` 先按 Goals feature gate 发 persisted goal snapshot，再通过 goal runtime best-effort continue。`AppServerLiveThreadHandle` 不再暴露 `continue_active_goal_if_idle`。
- running resume 的 goal resume effects 和 post-replay idle continuation 已迁到 `LiveThreadGoalRuntime` / `AppServerLiveThreadGoalRuntime` by thread id；app-server 仍保持 attach 后先 apply resume effects、response/usage replay/goal snapshot/request replay 后再 best-effort continue 的顺序。listener handle 不再暴露 goal resume/continue methods。
- cold resume/fork 与 running resume 的 token/context usage replay 已迁到 `LiveThreadUsageRuntime` / `AppServerLiveThreadUsageRuntime`；`context_usage_replay` 通过 runtime-backed usage source 按 thread id 读取 usage。`AppServerLiveThreadHandle` 和 listener handle 不再暴露 token/context usage。
- app-server response assembly 的 session/config copied reads 已迁到 `LiveThreadInspectionRuntime` / `AppServerLiveThreadInspectionRuntime`；detached review read-thread assembly 已改到 `NativeDetachedReviewRuntime` / store read 路径并保留 `include_archived=true` / `include_history=false` 语义；app-server transitional `AppServerLiveThreadHandle` 已删除。`ThreadProcessorCreatedThread` 当前只保留 startup telemetry，不再继承 full handle。
- `LiveThreadInspectionRuntime` 的 `list_live_thread_ids` / `is_live_thread_loaded` / `live_thread_info` 已补齐 external live record 可见性，和既有 external snapshot/config snapshot 特判保持一致；external live record removal primitive 和 close 后 live record cleanup 也已补齐，但这不代表 reload/list_agents 或 root start provider routing 已完成。
- listener generation、running resume response、rollback response 和 permissions request 缺 cwd fallback 所需的 session/config copied reads 已迁到 `LiveThreadInspectionRuntime` / `AppServerLiveThreadInspectionRuntime` by thread id；rollback response 的 stored-thread read 已迁到 `LiveThreadHistoryRuntime` / `AppServerLiveThreadHistoryRuntime` by thread id；bespoke approval / elicitation / user-input / permissions responses 和 dynamic tool responses 的 listener submit 已迁到 `LiveThreadCommandRuntime` / `AppServerLiveThreadCommandRuntime` by thread id。listener handle 不再暴露 `session_configured` / `config_snapshot` / submit / read-thread，只保留 event stream。
- native creation runtime extraction 后，app-server 只把 `thread_service::NewThread` 投影成本地 response assembly 所需的 `thread_id`、telemetry-only created-thread handle 和 `SessionConfiguredEvent`；root start/resume/fork 仍未 provider-neutral routing，external root `thread/start` 仍应拒绝。
- 当前 provider capability：Claude CLI 可通过 `claude -p --input-format stream-json --output-format stream-json --verbose` 作为持续 stdin/stdout session transport；OpenCode 已通过 `opencode serve --port 0 --hostname 127.0.0.1 --print-logs` + HTTP `/session`、`/session/{sessionID}/prompt_async`、`/event` SSE 实现同 session resume adapter；Codex 有 `codex app-server --listen ...` server/session transport，但 thread-service 不能直接依赖反向依赖它的 app-server-client，仍需要专门 remote JSON-RPC adapter 后才能暴露 `codex_cli`。
- 当前实现里不存在持久的 `PostTurn` 调度状态；turn 收尾更接近 `active_turn` 锁保护下的一段临界区。`thread_post_turn_state()` 会在 `active_turn.is_some()` 或存在 pending turn input 时直接视为 `ThreadActive`，然后才退化到 goal / child / command 的后续判断。
- 因此后续修 turn 生命周期竞态时，首先要区分“当前 turn 的 pending_input”和“thread 级 mailbox / queued next-turn input”，以及它们各自是在持锁的哪个阶段被检查或搬运；不要把“有 active turn”误当成“当前 turn 一定还能消费新事件”。
- active turn 下的 async input 先在 `active_turn` 调度锁保护下判定：若 `TurnState.accepts_async_input_for_current_turn()` 为真则并入 `turn_state.pending_input`，否则继续留在线程级 mailbox，等待后续 turn 消费。
- `on_task_finished()` 收尾时要区分两类“后续还有事可做”的来源：线程级 pending work 与 leftover `pending_input`。只有 leftover 输入经 `inspect_pending_input(...)` 判成 `Accepted` 时才允许直接拉起 follow-up turn；纯 `Blocked` leftover 不应启动空 turn，而应继续走 goal continuation / parent final-status 语义。
- `on_task_finished()` 现在应视为唯一的 post-turn 状态提交点：最后一个 task 结束时，会在 `active_turn` 调度锁内原子取走 leftover pending input、检查线程级 pending work、清除旧 active turn，并生成锁外副作用要执行的 next-step 决议。
- `MailboxDeliveryPhase` 已删除；取而代之的是 `TurnState.accepts_async_input_for_current_turn()` 这类更直接的 turn-local gating。mailbox 仍只承载线程级 pending input，但“final answer 后的 late async input 不再扩展当前 turn”的边界仍需保留。

## Compact Understanding
- compact prompt 支持 workspace 级 `.codex/compact/COMPACT.md` 与 `CODEX_HOME/compact/COMPACT.md`。
- 如果没有自定义 compact prompt，运行时仍会回退到内置 compact prompt。
- compact prompt 仍是独立的 compact-phase 输入来源；即使收紧 compact 的公开 turn 语义，也不能回退到删除或绕过 `COMPACT.md`。
- root-worker prototype 当前对 compact history 采用按需加载，而不是默认常驻保存。
- compact 的 replacement history 应尽量最小化：只保留 initial context 与最近真实 user messages，不再把 `.codex/memory/*.md` 正文复制成 `Memory checkpoint: ...` user messages 塞回 conversation history。
- 当前主线已进一步收紧 compact 语义：
  - replacement history 现在会追加“当前 compact turn 的最后一条 assistant 输出”，作为后续 continuation seed
  - root-worker 主会话不再把 compact turn / compact row 当作公开对话展示
- compact 继续复用普通 `run_sampling_request()` / `build_prompt()` 链，但会在 compact 调用点显式覆盖为空的 `TurnToolInputs`，从而对模型隐藏全部 model-visible tools；这条限制只适用于 compact，不应误伤普通 turn 的 tool visibility。
- memory 文件在 compact 后仍通过 `instruction_files` / init context 注入后续模型上下文；`CompactedItem.replacement_history` / `ContextCompaction.replacementHistory` 负责的是最小模型可见 history 种子与 persisted/UI compact 事实，不承载整份 memory markdown 的重复副本。
- compact final output 的提取必须限定在“当前 compact turn、最后一次 compact prompt 之后”的 assistant 输出；不能从整段历史扫描最后 assistant message，否则会误吸上一轮普通回复。

## Validation Defaults
- 默认只做最小必要验证，不默认运行全量 `cargo test`、广域 `just fix`、snapshot、schema 或 lockfile workflow。
- 涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时，默认在 `codex-rs/` 下运行 `cargo build -p app-server --bin app-server`。
- 只有确实改到 CLI/TUI 或 CLI app-server 包装时，才增加 `cargo build -p codex-cli`。

## Rejected Paths
- 不要把 display 修复建立在 raw marker、assistant JSON envelope 或 legacy 解析路径上。
- 不要把 `dev` checkout 的改动文件手工复制回主仓库代替 merge。
