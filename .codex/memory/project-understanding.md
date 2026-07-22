# Project Understanding

## Stable Working Rules
- 所有 shell 命令必须以 `rtk` 开头。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 当前项目的 PM / owner / reviewer 协作规则以 `.codex/agents/project-pm.agent.md` 及对应 owner agent 定义为准。

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
- external CLI agent 的协作协议由后端 bridge 在启动 context 中注入 JSON tool schema/call/result 约定；provider raw stdout/JSON 只在后端 adapter 解析，UI 不解析 raw external JSON。
- external tools 与 internal tools 可共享 AgentControl / InterAgentCommunication / pending input / completion 事实源，但 model-visible tool 名称和 schema 必须分离。
- 等待模型的长期方向已确定为统一收口到 `poll_event`；`wait_agent` / `command_wait` 应删除，不再作为独立 tool surface 保留。
- `poll_event` 需要支持在同一个 turn 内等待并在 event 到达后继续执行；runtime 不应维护会影响状态机的硬性等待目标，event 应直接作为 pending input 注入当前 turn，并携带来源信息供模型判断。
- `poll_event` 对模型暴露的是空参数对象；默认 `initial_timeout_ms` / `hard_cap_timeout_ms` 由 thread runtime 从 `TurnContext.config.multi_agent_v2` 注入，再结合 thread-scoped backoff 计算每次调用的 `current_timeout_ms`。
- command output / exit、child completion、inter-agent completion 应复用同一套 pending-input 唤醒链路；不要再为某一类等待事件维护平行 wait API。
- parent-side child completion bookkeeping 只用于 completion envelope 的投递、去重和清理，不应定义 child 当前是否 active；`WaitChild` / `IdleWaitChild` 只应由 direct child thread 的本地 active 状态驱动。
- thread init context 中的 workflow discovery 依赖 `TurnContext::discovery_context()`，其 project workflows 来自 `config.config_layer_stack` 中各 project layer 的 `.codex/workflows`。
- session 初始化阶段的 `instruction_files` 应按本地 config 路径读取，不应依赖 primary execution environment 是否存在。

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
- 当前 provider capability：Claude CLI 可通过 `claude -p --input-format stream-json --output-format stream-json --verbose` 作为持续 stdin/stdout session transport；Codex 有 `codex app-server --listen ...` server/session transport，OpenCode 有 HTTP OpenAPI `opencode serve` 和 ACP JSON-RPC `opencode acp`，但在专用 adapter 实现前不能把 `codex_cli` / `opencode` 暴露为 supported external provider，也不能用 `codex exec` / `opencode run` one-shot 冒充 resume。
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
