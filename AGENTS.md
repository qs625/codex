# Rust/codex-rs

In the codex-rs folder where the rust code lives:

- Crate names are prefixed with `codex-`. For example, the `core` folder's crate is named `codex-core`
- When using format! and you can inline variables into {}, always do that.
- Install any commands the repo relies on (for example `just`, `rg`, or `cargo-insta`) if they aren't already available before running instructions here.
- Never add or modify any code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR`.
  - You operate in a sandbox where `CODEX_SANDBOX_NETWORK_DISABLED=1` will be set whenever you use the `shell` tool. Any existing code that uses `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` was authored with this fact in mind. It is often used to early exit out of tests that the author knew you would not be able to run given your sandbox limitations.
  - Similarly, when you spawn a process using Seatbelt (`/usr/bin/sandbox-exec`), `CODEX_SANDBOX=seatbelt` will be set on the child process. Integration tests that want to run Seatbelt themselves cannot be run under Seatbelt, so checks for `CODEX_SANDBOX=seatbelt` are also often used to early exit out of tests, as appropriate.
- Always collapse if statements per https://rust-lang.github.io/rust-clippy/master/index.html#collapsible_if
- Always inline format! args when possible per https://rust-lang.github.io/rust-clippy/master/index.html#uninlined_format_args
- Use method references over closures when possible per https://rust-lang.github.io/rust-clippy/master/index.html#redundant_closure_for_method_calls
- Avoid bool or ambiguous `Option` parameters that force callers to write hard-to-read code such as `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other idiomatic Rust API shapes when they keep the callsite self-documenting.
- When you cannot make that API change and still need a small positional-literal callsite in Rust, follow the `argument_comment_lint` convention:
  - Use an exact `/*param_name*/` comment before opaque literal arguments such as `None`, booleans, and numeric literals when passing them by position.
  - Do not add these comments for string or char literals unless the comment adds real clarity; those literals are intentionally exempt from the lint.
  - The parameter name in the comment must exactly match the callee signature.
- If local lint verification is specifically needed, run `just argument-comment-lint`. This is powered by Bazel, so running it the first time can be slow if Bazel is not warmed up, though incremental invocations should take <15s. Most of the time, it is best to update the PR and let CI take responsibility for checking this. Note CI checks all three platforms, which the local run does not.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Newly added traits should include doc comments that explain their role and how implementations are expected to use them.
- Discourage both `#[async_trait]` and `#[allow(async_fn_in_trait)]` in Rust traits.
  - Prefer native RPITIT trait methods with explicit `Send` bounds on the returned future, as in `3c7f013f9735` / `#16630`.
  - Preferred trait shape:
    `fn foo(&self, ...) -> impl std::future::Future<Output = T> + Send;`
  - Implementations may still use `async fn foo(&self, ...) -> T` when they satisfy that contract.
  - Do not use `#[allow(async_fn_in_trait)]` as a shortcut around spelling the future contract explicitly.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- 如果 Rust async/integration 测试在默认 test harness 线程上因为 future 或 core-heavy mock 路径过大触发 stack overflow，不要要求调用者手工设置 `RUST_MIN_STACK` 作为唯一修复。优先把该测试改成普通 `#[test]` wrapper，用 `std::thread::Builder::stack_size(...)` 启动明确大栈线程，并在该线程内创建 `tokio::runtime::Builder` 后 `block_on` 原 async 测试主体；这样 `cargo test -p <crate>` 默认路径稳定且不依赖外部环境变量。只有确认是无限递归或真实 bug 时才按逻辑 bug 修复。
- Do not add general product or user-facing documentation to the `docs/` folder. The official Codex documentation lives elsewhere. The exception is app-server API documentation, which is covered by the app-server guidance below.
- Prefer private modules and explicitly exported public crate API.
- If you change `ConfigToml` or nested config types, update `codex-rs/core/config.schema.json` when needed; use `just write-config-schema` when regenerating it.
- When working with MCP tool calls, prefer using `codex-rs/codex-mcp/src/mcp_connection_manager.rs` to handle mutation of tools and tool calls. Aim to minimize the footprint of changes and leverage existing abstractions rather than plumbing code through multiple levels of function calls.
- 对话/线程展示相关的结构化语义以 typed `EventMsg` 为 runtime/UI display source；`ResponseItem` 只作为模型交互、context manager/provider history、compact、guardian 和模型可见工具输出的 source；`ThreadItem` 应通过共享 `EventMsg -> ThreadItem` projector 统一生成。新增或修改 event-command、schedule、collab、goal、command session 这类展示项时，必须新增 display-capable `EventMsg` variant，并复用 `codex-rs/app-server-protocol/src/protocol/event_item_projection.rs` 的边界；不要新增 display-only `ResponseItem` variant，不要新增 raw response item 展示分支，也不要从 message marker 文本、assistant message JSON、`RolloutItem::ResponseItem`、`RawResponseItem`、`ResponseItemStarted/Completed`、live `ResponseItem::FunctionCall` / `FunctionCallOutput` 或旧 `TurnItem -> ThreadItem` adapter 重建展示。provider 请求侧为了 wire/model 输入需要保留 marker 包装时，只能作为单向 formatting，不得作为展示或 history 重建的解析来源。schedule subscribe/unsubscribe 仍属于 typed projection，不要作为旧 generic event-driven 兼容路径移除。
- 工具执行完成后的纯历史记录路径应尽早 canonicalize 为 typed `ResponseItem`；pending user-hook 路径使用 `PendingInputItem::HookInspectable(ResponseItem)` 表达“需要 hook 检查的对话项”。`ResponseInputItem` 只保留在 Responses API request 输入和 client/request 输入适配层，不要作为工具输出、pending history 或 hook history 的核心中转类型继续扩散。
- active turn 注入需要经过 pending/user prompt hook 检查的输入时，使用语义明确的 `inject_hook_inspectable_items`；直接写入 model-visible history 的 typed `ResponseItem` 使用 `inject_conversation_items` 或 `record_conversation_items`。不要新增模糊的 `inject_response_items` 入口。
- `record_conversation_items` 只负责 typed `ResponseItem` 写入 history/rollout/context usage，不得顺带发送 live `RawResponseItem`；需要同时写模型上下文和客户端可见 live item 时，使用 `record_model_items_and_emit_display_events` 这类 dual-write helper，由 helper 记录 model-visible `ResponseItem` 并 emit display-capable `EventMsg`，再通过 shared projector 生成 `ThreadItem`。
- 业务代码不得直接通过 `send_event_raw` 发 conversation display item（例如 collab/child-completion、event-command、schedule、command session 这类展示项）。新增或修复展示语义时，应先形成 typed `EventMsg` display event；确实也需要模型可见时再通过 helper 双写 `ResponseItem`。`send_event_raw` 仅保留给非 conversation display 的 runtime/control 事件或已有 legacy adapter 边界。
- live `item/started` 和 `item/completed` payload 在 app-server v2 边界以 typed `ThreadItem` 为 canonical display payload；core `ItemStarted/ItemCompleted(TurnItem)` 只能由 `event_item_projection.rs` 作为 `EventMsg` lifecycle payload 显式投影，不再保留公开 `TurnItem -> ThreadItem` adapter。thread/read replay 只从 persisted `EventMsg` 生成展示，不从 `RolloutItem::ResponseItem` 或 `RawResponseItem` 回放旧展示。
- root-worker prototype、SDK 示例或其他非 TUI 客户端展示 app-server v2 thread/live 内容时，只能消费 typed `ThreadItem` / v2 payload；不要从 `agentMessage.text`、`eventDrivenTool.text`、compact replacement raw `ResponseItem`、`<event_driven_tool>`、`<event_command>`、`<subagent_notification>` 或 inter-agent JSON envelope 反解 display item。旧 raw structured message 不再作为 conversation display 兼容输入；需要展示的事实必须由后端发出 typed `ThreadItem`。
- root-worker composer slash 菜单中，能表达为 Skill 的命令应来自 Skills discovery；例如 `/init` 由 embedded system skill 提供，不要作为 root-worker builtin command 硬编码。只有依赖 runtime/thread state 或客户端本地动作的命令（例如 `/clear`、`/goal <objective|pause|resume|cancel|clear>`）才放入 root-worker builtin slash command registry，并且执行时不得作为普通 user message 发送给模型；`/cancel-goal` 只能作为兼容别名，不作为主展示命令。
- root-worker composer slash 菜单中的动态候选必须来自后端 discovery，不要硬编码具体 id。需要当前 turn/runtime bridge 的动作应由模型在 turn 内调用对应 tool，客户端不得直接绕过边界。
- root-worker 展示进度类内容时只消费 typed `ThreadItem` / typed display event 投影结果；相关归属 metadata 只能消费后端提供的 typed metadata，不得从 progress item、tool output、runner output、assistant 文本或 legacy envelope 反推。
- command session 的 output/exit notification 必须作为独立 typed display-capable `EventMsg` 并投影为 `ThreadItem`，并通过 typed command item id 关联原 `CommandExecution`；`command_wait` 和 `command_write_stdin` 的等待/写 stdin 行为也必须记录为独立 typed display event，确实需要模型可见时再双写 `ResponseItem`。`command_wait` 每次只等待当前 backoff window，timeout 后返回 running 并推进同 command 的下一次窗口；output/exit 或 completed 命中时 reset。`CommandWait.wait_timeout_ms` 必须展示本次 current window，不能展示 hard cap。`ExecCommandOutputDelta` 只用于更新 command cell live tail，不得作为 raw marker、assistant 文本或按 output 内容反解出的 conversation event。
- `command_wait` 调用开始时必须发 typed started lifecycle item，等待结束后用同一个 typed item id 发 completed lifecycle item；started/completed payload 都必须使用同一次 current wait window，不能用 hard cap、默认 250ms 或重新生成的 id。
- root-worker Agent Tree 的主状态必须消费后端 canonical `ThreadStatus` / `thread/status/changed`，例如 `Active.activeFlags` 中的 `running`、`waitingOnApproval`、`waitingOnUserInput`，以及 `Idle.reason` 中的 `waitChild`、`waitCommand`；不要从 turn/items/raw marker/legacy JSON envelope 或 children 递归自行推导 running、waiting 或 complete。
- Go/Goal post-turn 流程必须按 `ThreadActive -> GoContextContinuation / ThreadIdle(WaitChild | WaitCommand) / ThreadCompletion` 状态模型推进；`ThreadActive` 只表示当前 thread turn 正在运行或 pending input 正在启动新 turn，`ThreadIdle(WaitChild)` 表示无 turn 运行且没有 active goal continuation，但 direct child 尚未 parent-visible complete，`ThreadIdle(WaitCommand)` 表示无 turn 运行且没有 active goal continuation，但仍等待 command/event-command 外部唤醒。Goal `Active` 且无 pending input 时立即注入 `<goal_context>` 并启动 continuation，不因 incomplete direct child、running command 或 event subscription 被阻止；Goal `Complete` / `Paused` / `BudgetLimited` / 不存在时才按 wait child / wait command / completion 推进。不要用 grandchild/descendant recursive scan 替代 direct-child completion 协议，也不要只用当前 turn complete 近似判断；root-worker 也不得递归 children 推导父 thread 状态，父状态必须来自后端 canonical `ThreadStatus`。
- thread lifecycle 完成顺序固定为 pending input -> active goal continuation -> incomplete direct child -> wait command -> complete；child completion 只对 direct parent 生效，递归等待由各级 child completion 协议逐层传递。普通 non-management subagent 本地完成后必须向 parent 投递 typed child completion pending input，并唤醒 parent turn；parent 启动 turn 消费该 pending input 时才把 direct child 标记为 parent-visible complete。`list_agents` / status read 能看到 child lifecycle `Completed` 不等于 parent-visible complete；读到 final lifecycle status 时只能触发幂等 typed delivery recheck，不得用 raw marker、assistant text 或 list 结果反解/伪造 completion。`agent_mode = management` 不参与 parent completion delivery，可在本地完成条件满足时直接 complete。
- goal 创建、更新、暂停、恢复、预算耗尽和完成的 conversation 展示必须走 dedicated typed display lifecycle；primary path 是 `EventMsg::ThreadGoalUpdateCompleted -> ThreadItem::ThreadGoalUpdate`。`ResponseItem::ThreadGoalUpdate` 只用于模型上下文双写，不得通过 `EventMsg::ResponseItemCompleted` 或 history replay 投影为 `ThreadItem`。`thread/goal/updated` / `thread/goal/cleared` 仍只表达当前 goal state，不可从 goal tool output JSON、assistant 文本、`<goal_context>` 或 legacy marker 反解会话展示项。
- 外部工作唤醒必须采用 typed runtime event + active state evaluator + goal scheduler 的分层模型：runtime event 表达“发生了什么”（child completed/failed/interrupted、command output/exit/stdin、schedule fired 等），active state 表达“现在能不能继续”（local turn、active child、active command/event tool、queued input、pending external event），goal 只表达“thread idle 后是否继续”。不要让 goal 轮询 child/command 状态，也不要把长期 subagent/command 等待改成阻塞 turn；外部事实变化必须写入 typed `EventMsg`/`ThreadItem`，需要模型可见时再双写 `ResponseItem`，并唤醒 scheduler。`wait_agent` 和 `command_wait` 只是显式短窗口等待/用户可见等待动作，不是系统调度主机制。subagent 异常、丢失或中断必须作为 typed child lifecycle event 传回 parent，不能静默依赖 parent goal continuation 猜测。
- root-worker prototype 的 `ThreadItem` 写入、snapshot normalization 和 pending/live 合并只能按 `ThreadItem.id` 判断同一个 item；不同 id 必须作为不同 item 保留，不得根据 text/content/status/semantic key/raw marker/legacy JSON envelope 合并或丢弃。每个 typed `ThreadItem` 至少生成一个 `ConversationEntry`；`ConversationCell` 只能做视觉分组，不能丢 entry。
- root-worker live 模式下，已经进入本地 live cache 的 thread 在切换展示时只能使用持续接收的 live `ThreadItem`，不要触发 `thread/read` 或用 snapshot/history rebuild 对 item 做 destructive/non-destructive merge；`thread/read` 仅用于 cold start、缺失本地 thread 或显式恢复路径。
- root-worker 已初始化 thread 的 live `turn/started` / `turn/completed` 只能更新 turn lifecycle metadata，不得把通知中的 `turn.items` 当作 snapshot 覆盖本地 items；conversation item 内容必须通过 typed `item/started` / `item/completed` 或 agent delta 增量进入 cache。
- root-worker prototype 会话消息布局应继续以 typed `ThreadItem -> ConversationEntry -> ConversationCell` 为展示链路；连续普通 agent message 需要保持为一个 message cell、一个外层 agent bubble，内部可用 segment 展示多条 entry；user message 右对齐展示。新增展示分组或布局逻辑时，不要跨 user/tool/event/schedule、childCompletion/subagentNotification、replacement history 等语义边界合并。
- root-worker prototype 的 conversation 搜索、过滤、定位或高亮能力只能基于已投影出的 `ConversationEntry` / `ConversationCell` 派生；搜索结果不得参与 `ThreadItem.id` 合并或去重，不得从 raw marker、assistant message JSON、legacy envelope 或 agent text 中反解 display item。
- MultiAgent 运行时只保留 V2 工具和 typed child completion 路径；不要重新引入 V1 `send_input`/`resume_agent` 工具、legacy completion watcher、raw `inject_user_message_without_turn` child completion fallback，或通过配置在 V1/V2 之间切换。
- MultiAgent V2 的 `wait_agent` 只能等待 canonical typed subagent 更新：调用开始必须先非消费式检查 parent pending input/mailbox 中已有的 typed `InterAgentCommunication` / child completion / status，然后再通过 status watch 与 mailbox sequence notify 进入 runtime backoff；不得 drain mailbox，不得从 raw marker、assistant text 或 legacy JSON envelope 反解唤醒条件。`features.multi_agent_v2.default_wait_timeout_ms` 表示 initial window，`max_wait_timeout_ms` 表示 hard cap。每次 `wait_agent` 调用只等待当前 backoff window，timeout 后返回给模型并推进同 sender/receiver target 的下一次窗口；pending mailbox、status/final 或 child completion 等相关事件命中时 reset。`CollabWaitingBegin/End.timeout_ms` 必须展示本次 current window，不能展示 hard cap。
- Compact 当前用户可见和默认路径是 Local Compact：手动 `/compact`、`thread/compact/start` 和自动 context-limit compact 都应走 `codex-rs/core/src/compact.rs` 的本地 summarization 流程，并把 `CompactedItem.replacement_history` 持久化到 rollout 以便 thread/history 和 app-server 展示检查；compact 完成的 live `item/completed` 也必须携带同一份 replacement history，已 live/loaded 的 root-worker thread 不得依赖 `thread/read` 回填。Local Compact summary 在 active history 中是带 `SUMMARY_PREFIX` 的 user message，context usage 分类必须把它计入 compact 类别，不能当作普通 user message 导致 compact ratio 丢失。`compact_remote.rs` / `compact_remote_v2.rs` 只保留为未路由的历史兼容实现，不要重新接到默认入口、用户触发入口或模型请求 beta header。
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), update `MODULE.bazel.lock`
  when needed. Use `just bazel-lock-update` and `just bazel-lock-check` when the change requires
  Bazel lockfile validation, but these are not part of the default validation set.
- Bazel does not automatically make source-tree files available to compile-time Rust file access. If
  you add `include_str!`, `include_bytes!`, `sqlx::migrate!`, or similar build-time file or
  directory reads, update the crate's `BUILD.bazel` (`compile_data`, `build_script_data`, or test
  data) or Bazel may fail even when Cargo passes.
- Do not create small helper methods that are referenced only once.
- Avoid large modules:
  - Prefer adding new modules instead of growing existing ones.
  - Target Rust modules under 500 LoC, excluding tests.
  - If a file exceeds roughly 800 LoC, add new functionality in a new module instead of extending
    the existing file unless there is a strong documented reason not to.
  - This rule applies especially to high-touch files that already attract unrelated changes, such
    as `codex-rs/tui/src/app.rs`, `codex-rs/tui/src/bottom_pane/chat_composer.rs`,
    `codex-rs/tui/src/bottom_pane/footer.rs`, `codex-rs/tui/src/chatwidget.rs`,
    `codex-rs/tui/src/bottom_pane/mod.rs`, and similarly central orchestration modules.
  - When extracting code from a large module, move the related tests and module/type docs toward
    the new implementation so the invariants stay close to the code that owns them.
  - Avoid adding new standalone methods to `codex-rs/tui/src/chatwidget.rs` unless the change is
    trivial; prefer new modules/files and keep `chatwidget.rs` focused on orchestration.
- When running Rust commands (e.g. `just fix` or `cargo test`) be patient with the command and never try to kill them using the PID. Rust lock can make the execution slow, this is expected.
- 验证 Rust 编译和测试时，不要为当前 checkout 配置 `TARGET_DIR` 指向其他 checkout，也不要把 `codex-rs/target` 做成跨 checkout 的符号链接；每个开发目录使用自己的默认 target 目录，保持独立测试和独立编译。
- 如果多个 Rust 测试或构建命令出现文件锁竞争，使用 `exec_command` 启动命令并通过 `command_wait` 等待通知；不要通过反复轮询、sleep 循环或持续检查进程状态来等待锁释放。
- Rust/Cargo/`just` 长时间验证命令一旦使用 `command_wait` 等待完成事件，当前验证流程必须进入静默等待：不要查询该命令状态、不要查看日志、不要启动替代测试、不要重复验证同一结果。
- 同一 checkout 内同一时间只允许一个会竞争 Rust target 或 Cargo 文件锁的长命令运行；在它完成前，不要连续启动新的 `cargo test`、`cargo check`、`cargo build`、`just fix` 或其他 Rust 验证命令。不同 checkout 可以独立编译和测试，但不要共享 target 或依赖目录。可以继续处理不依赖该命令结果、且不竞争 Rust/Cargo 资源的前端、文档或只读设计工作。

Rust 代码变更完成后，默认串行执行两类验证：

1. 修改模块的单元测试或最小 crate 测试。例如改 `codex-rs/tui` 时，include `cargo test -p codex-tui`；更窄的单测命令可用时优先用更窄命令。
2. 验证与入口匹配的 binary 能编译：只涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时，include `cargo build -p codex-app-server --bin codex-app-server` from `codex-rs`；只有确实改到 CLI/TUI 或 CLI app-server 子命令包装时，才 include `cargo build -p codex-cli` from `codex-rs`.

Do not run full workspace `cargo test`, `just test`, broad `just fix`, or snapshot/schema/lockfile commands by default in every checkout. Add those commands only when the change specifically requires them or the user asks for broader validation.

## The `codex-core` crate

Over time, the `codex-core` crate (defined in `codex-rs/core/`) has become bloated because it is the largest crate, so it is often easier to add something new to `codex-core` rather than refactor out the library code you need so your new code neither takes a dependency on, nor contributes to the size of, `codex-core`.

To that end: **resist adding code to codex-core**!

Particularly when introducing a new concept/feature/API, before adding to `codex-core`, consider whether:

- There is an existing crate other than `codex-core` that is an appropriate place for your new code to live.
- It is time to introduce a new crate to the Cargo workspace for your new functionality. Refactor existing code as necessary to make this happen.
- 不依赖 `codex-core` 内部实现的稳定 MultiAgent runtime 基础层，例如 agent registry 和 status helper，应放在 `codex-rs/agent-runtime`（`codex-agent-runtime`）。`AgentControl`、session/turn loop、绑定 `PendingInputItem` 的 mailbox 状态、tool dispatch adapter 等偏编排的代码，在边界被明确拆分前继续留在 `codex-core`。
- 不依赖 `codex-core` 的工具定义、工具配置、tool discovery 模型、Responses API tool
  shape 转换、MCP/dynamic tool 适配、code-mode tool spec 兼容层等 host-side 纯类型和
  helper，应优先放在 `codex-rs/tools`（`codex-tools`）。纯 tool planning，例如
  agent tool pattern 过滤、hosted model tool specs、namespace 合并、code-mode exec prompt
  plan，也属于 `codex-tools` 边界。`Session`、`TurnContext`、hooks、approval、
  telemetry、真实 tool handler 执行、`dispatch_any` 和 turn loop 编排继续留在 `codex-core`，
  除非先拆出稳定共享接口。
- code-mode 的描述层、工具定义、tool namespace description、exec/wait public tool name、
  exec prompt builder、pragma parser 和 JSON schema 到 TypeScript 的渲染属于轻量 API 边界，
  owner crate 是 `codex-rs/code-mode-api`（`codex-code-mode-api`）。`codex-tools` 这类只做
  tool spec/planning 的 crate 应依赖 `codex-code-mode-api`，不得为了生成 code-mode tool
  description 或 nested tool definition 直接依赖带 V8 runtime 的 `codex-code-mode`。
  `codex-rollout-trace` 这类只需要 code-mode public tool name、runtime response DTO 或 trace
  serialization 的 crate 也应依赖 `codex-code-mode-api`，不应直接依赖 runtime implementation。
  `codex-code-mode-api` 还承载 `CodeModeRuntimeService`、`CodeModeRuntimeFactory`、
  `CodeModeTurnHost` 和 execute/wait request/outcome 这些 runtime trait/DTO；`codex-core`
  和后续 session runtime 只能依赖这些 trait/DTO。V8-backed `codex-code-mode` 是 runtime
  implementation crate，由 app-server/CLI/TUI 这类组合根通过 constructor injection 注入，
  不要在 core/session/工具规划 crate 中直接创建或依赖 V8 implementation。`codex-code-mode`
  可以继续 re-export API 类型以保持兼容。
- 不依赖 `codex-core` 的 command runtime primitive，例如 command output buffer、process
  state、wait/write-stdin DTO、notification filter/state 和 yield/token/chunk id helper，应放在
  `codex-rs/command-runtime`（`codex-command-runtime`）。`ExecCommandHandler`、
  `CommandWaitHandler`、`WriteStdinHandler`、approval/sandbox/spawn、async watcher event
  emission、`Session`/`TurnContext` 编排继续留在 `codex-core`。命令输出的 legacy encoding 智能解码
  （`bytes_to_string_smart`、`chardetng`/`encoding_rs` 依赖和相关 CP1251/CP866/Windows-1252 回归测试）
  属于 command runtime 边界；`codex-protocol::exec_output` 只保留轻量 DTO，不要为了 DTO 或
  `StreamOutput` 把编码检测依赖重新拉回 shared protocol。
- shell command parsing/safety 和 shell executable PATH lookup 属于 `codex-rs/shell-command`
  （`codex-shell-command`）。`codex-core` 可以使用 `codex_shell_command::resolve_executable_in_path`
  这类 shell utility，但不要为了 shell discovery 直接依赖 `which`；测试需要探测本机可执行文件时可以
  把 `which` 作为 dev-dependency。
- OS/user display-name lookup 属于 `codex-rs/user-info`（`codex-user-info`）。`codex-core`
  需要当前用户名字、first name 或 greeting fallback 时应调用 `codex_user_info` helper，不要为了
  prompt/realtime 文案直接依赖 `whoami`；需要覆盖本机用户名行为的测试应通过该 crate helper 或 dev-dependency
  边界处理。
- model input adapter 属于 `codex-rs/model-input`（`codex-model-input`）：把 `UserInput`
  转成 model-visible `ResponseInputItem`、读取本地图片、resize/encode、生成 LocalImage
  placeholder 和图片 label 序列都在该 crate。`codex-protocol` 只保留 `UserInput`、Responses API
  DTO、pre-encoded image wrapping 和 image tag helper，不要为了 `LocalImage` 文件 IO 或
  `codex-utils-image` 把图片处理栈重新拉回 protocol。core/session/compact/prompt debug 等进入模型
  上下文的路径必须显式调用 `codex_model_input::response_input_item_from_user_input`。图片解码、
  resize/encode、内存图片尺寸读取和 `image` crate 依赖属于 `codex-utils-image`，`codex-core`
  不要直接依赖 `image` 来实现 token estimate、view-image 或 model input helper。
- `codex-protocol` 是 shared DTO/wire/error 语义层，不应为了 runtime helper 自动转换依赖
  `tokio`、`tokio-util`、`codex-async-utils` 或 `async-trait`。需要把
  `codex_async_utils::CancelErr`、`tokio::task::JoinError` 等 runtime error 转成 `CodexErr` 时，应在
  core/app-server 等调用方显式 `map_err` 到 `CodexErr::TurnAborted`、`CodexErr::TaskJoin` 或更具体的
  domain error，避免所有 protocol consumers 间接拉入 async runtime crates。
- 不依赖 `codex-core` 的通用 filesystem trait 和 unsandboxed 本地文件系统实现属于
  `codex-rs/file-system`（`codex-file-system`）。插件、技能、配置、AGENTS.md 加载等只需要本地文件
  读取/metadata 的路径应依赖 `codex_file_system::ExecutorFileSystem` 和
  `codex_file_system::LOCAL_FS`，不要为了 `LOCAL_FS` 或 trait 把 `codex-exec-server` 拉入轻量 crate。
  `codex-apply-patch` 也只应依赖 `codex-file-system` 这一层；它不应为了 standalone executable 或
  sandbox context 走 `codex-exec-server` re-export。
  `codex-exec-server` 继续拥有 sandbox-aware process/filesystem implementation、environment manager、
  JSON-RPC transport 和 remote executor runtime。
- SQLite state runtime、thread metadata、goal/agent-job/log DB model、migrations、telemetry layer 和
  `StateRuntime` 属于 `codex-rs/state`（`codex-state`）。CLI-only utilities 例如
  `codex-state-logs`、`clap` parser、`dirs` home lookup 和 colored terminal formatting 属于
  `codex-rs/state-cli`（`codex-state-cli`）；core、rollout、thread-store 或其他 runtime
  consumer 不应为了日志查看 CLI 让 `codex-state` 携带 `clap`、`dirs` 或 `owo-colors`。
- Fuzzy file search library、session runtime、search options、match DTO 和 nucleo/ignore walker 属于
  `codex-rs/file-search`（`codex-file-search`）。`codex-file-search` CLI parser、stdout reporter、
  JSON output formatting 和 no-pattern directory listing fallback 属于 `codex-rs/file-search-cli`
  （`codex-file-search-cli`）；rollout/thread-store/core 这类只需要搜索 library 的路径不应为了
  `codex-file-search` 二进制入口携带 `clap`、`serde_json` 或 `tokio::process`。
- Shell escalation runtime、escalation protocol/server/client、policy evaluation adapter 和
  `run_shell_escalation_execve_wrapper` 属于 `codex-rs/shell-escalation`
  （`codex-shell-escalation`）。`codex-execve-wrapper` binary、`clap` argument parser 和
  stderr tracing subscriber setup 属于 `codex-rs/shell-escalation-cli`
  （`codex-shell-escalation-cli`）；core shell runtime 不应为了 wrapper binary 让
  `codex-shell-escalation` 携带 `clap` 或 `tracing-subscriber`。
- exec-server JSON-RPC wire DTO 和 method 常量属于 `codex-rs/exec-server-protocol`
  （`codex-exec-server-protocol`），包括 `ProcessId`、exec/read/write/terminate request-response、
  output notifications、filesystem protocol payload、executor-side HTTP request payload 和 base64
  byte chunks。`codex-exec-server` 可以 re-export 这些类型以兼容旧路径，但 `codex-core`、
  `codex-rmcp-client` 或其他只需要 protocol DTO 的消费者应直接依赖 `codex-exec-server-protocol`。
  exec-server runtime capability traits 属于 `codex-rs/exec-server-api`
  （`codex-exec-server-api`），包括 `ExecBackend`、`ExecProcess`、`StartedExecProcess`、
  `ExecProcessEvent`/event log/receiver、`HttpClient`、`HttpResponseBody`、`ExecEnvironment`、
  `ExecEnvironmentProvider` 和低依赖
  `ExecRuntimeError`。`ExecServerRuntimePaths`、`LOCAL_ENVIRONMENT_ID` 和
  `REMOTE_ENVIRONMENT_ID` 也属于 `codex-exec-server-api`，因为它们是 host/runtime selection 的轻量
  API，而不是 concrete transport implementation。`codex-mcp`、`codex-rmcp-client` 或其他只需要
  host-provided process/HTTP capability 的 crate 应依赖 `codex-exec-server-api` +
  `codex-exec-server-protocol`，不要直接依赖 concrete `codex-exec-server`。`codex-core` unified
  exec 只需要 `ExecProcess`、`StartedExecProcess`、process event receiver、runtime paths 或标准
  environment id 时，也应直接依赖 `codex-exec-server-api`，不要经 `codex-exec-server`
  re-export；core 只需要对已选 environment 执行 process/HTTP/filesystem capability 时，应接收
  `Arc<dyn ExecEnvironment>` / `&dyn ExecEnvironment`，不要把 concrete `Environment` 继续穿透到
  session、MCP helper、AGENTS.md loader 或 unified exec request。
  core 只需要解析已配置 environment、默认 environment 列表或 local fallback 时，应接收
  `Arc<dyn ExecEnvironmentProvider>` / `&dyn ExecEnvironmentProvider`，不要把 concrete
  `EnvironmentManager` 继续穿透到 session services、thread manager runtime state、environment
  selection 或 connectors helper。app-server 这类组合根可以继续持有 concrete `EnvironmentManager`
  处理 environment add/upsert、local FS processor 和配置文件 discovery，但应把 runtime session
  路径投影成 `ExecEnvironmentProvider` 注入 core。
  core connector accessible 查询和 prompt debug 构造必须显式接收 `&dyn ExecEnvironmentProvider`
  或 `Arc<dyn ExecEnvironmentProvider>`；不要在 `core/src/connectors.rs`、`core/src/prompt_debug.rs`
  或 MCP tool fallback 中通过 `codex_home` 临时构造 `EnvironmentManager`。
  `codex-exec-server` 负责实现并 re-export 这些 API traits；本地/远程
  process implementation、`Environment`/`EnvironmentManager`、rich `ExecServerError`、transport
  clients、sandbox-aware FS/process implementation 和 remote executor runtime 继续留在
  `codex-exec-server`，由 core/app-server 等组合根投影成 MCP runtime environment 或后续
  `RuntimeServices`。不要把 `reqwest`、`tokio-tungstenite` 等 transport-specific error 类型放进
  `codex-exec-server-api`。
  `codex-core` normal dependency graph 不应包含 concrete `codex-exec-server`；需要测试 fixture
  或 legacy `EnvironmentManager::default_for_tests()` 时，把 `codex-exec-server` 保持在 optional
  `test-support` feature 或 dev-dependency 边界。MCP runtime environment 构造需要同时传入 selected
  environment 和 local environment：远端环境用于 remote stdio/HTTP，local environment 用于 local
  streamable HTTP，不要在 core 里直接构造 `ReqwestHttpClient`。
- 不依赖 `codex-core` 的 filesystem permissions runtime matcher，例如 read-deny glob matcher、
  normalized/canonical path candidates 和 `globset` 相关测试，应放在
  `codex-rs/permissions-runtime`（`codex-permissions-runtime`）。`codex-protocol::permissions`
  只保留 permissions DTO、read-only reason helper 和 wire/context 可见类型，不要为了
  `ReadDenyMatcher` 或 sandbox implementation 把 `globset` 重新拉回 shared protocol。
- 不依赖 `codex-core` 的 network proxy 纯 API/DTO，例如 network policy decision/source 等
  protocol/config/display 共享类型，应放在 `codex-rs/network-proxy-api`
  （`codex-network-proxy-api`）。`codex-protocol` 这类基础类型层不要为了共享 DTO 直接依赖
  Rama-backed `codex-network-proxy`。`NetworkProxyConfig`、`NetworkProxySettings`、
  `NetworkMode`、domain/unix socket permission DTO、`normalize_host`、`NetworkPolicyRequest`、
  `NetworkProtocol`、`NetworkDecision`、`NetworkPolicyDecider`、`NetworkProxyAuditMetadata`、
  `BlockedRequest`、`BlockedRequestObserver`、`NetworkHostPort`、`parse_network_host_port`、
  `host_and_port_from_network_addr`、`NetworkProxyConstraints`、`PartialNetworkConfig`、
  `PartialNetworkProxyConfig`、`NetworkProxyConstraintError`、`NetworkProxyRuntimeSnapshot`、
  proxy env key 常量和 proxy env apply helper 属于
  `codex-network-proxy-api`；
  `codex-network-proxy` 只可 re-export 这些类型以兼容旧 callsite。proxy backend、Rama runtime、
  state builder、config reloader、proxy handle、host policy evaluation、读取/写入环境变量的
  process wiring 和 runtime state 继续留在 `codex-network-proxy` 或后续明确的实现 crate。
  `codex-sandboxing` 这类只需要 sandbox policy 生成输入的 crate 应消费
  `NetworkProxyRuntimeSnapshot`，不要直接依赖 Rama-backed `codex-network-proxy`；真实
  `NetworkProxy` 到 snapshot 的转换由 core/app-server 等持有 runtime handle 的边界完成。
  `codex-network-proxy` 不应为了历史空 `Args` 兼容类型或未来二进制入口携带 `clap`；若需要
  network proxy CLI parser，应新建明确的 CLI crate，而不是把 CLI derive 放回 Rama runtime crate。
- 不依赖 `codex-core` 的 exec policy 纯策略模型，例如 `Decision`、`Evaluation`、
  `RuleMatch`、`Policy`、`PrefixRule`、network rule DTO 和 host executable lookup helper，应放在
  no-Starlark 的 `codex-rs/execpolicy-api`（`codex-execpolicy-api`）。append/amend 文件写入
  不依赖 Starlark，也属于 `codex-execpolicy-api`；Starlark parser 和 parser error display
  继续留在 `codex-execpolicy`。`ExecPolicyCheckCommand`、`codex-execpolicy` bin 入口和
  `clap`/JSON formatting 这类命令行解析/展示逻辑属于 `codex-rs/execpolicy-cli`
  （`codex-execpolicy-cli`）；`codex-cli` 的 `execpolicy check` 子命令应依赖
  `codex-execpolicy-cli`，不要从 parser crate 重新导出 CLI command。
  `codex-config`、`codex-protocol` 等基础类型层不得为了构造或持有 policy DTO 拉入 `starlark`。
  `codex-execpolicy` 会 re-export API policy types 和 append/amend writer 以保持 callsite
  简洁；纯 `Policy` mutation/validation 返回 `codex_execpolicy_api::Error`，append/amend
  返回 `codex_execpolicy_api::AmendError`，Starlark parser/location/display 错误才使用
  `codex_execpolicy::Error`。`codex-core` 的
  session、network proxy loader、shell runtime 和 tests 中只需要 `Policy`/`Decision`/`Evaluation`
  这类策略模型或 append/amend writer 时，也应直接依赖 `codex-execpolicy-api`；只有 parser 和
  parser error display 边界继续依赖 `codex-execpolicy`。
- 不要把 `codex-app-server-protocol` 当作全仓混合共享 DTO 层。app-server v2
  JSON-RPC/control/event envelope、`ThreadItem` display payload、thread/turn 请求响应和 schema export
  可以继续归 app-server protocol；但被 config/tools/connectors/login/model-provider-info/otel 等非
  app-server transport crate 共同使用的 auth/app/config/MCP elicitation 等 DTO，应迁移到更窄的 owner
  crate 或清理后的轻量 API crate。迁移前后必须用 `cargo tree --depth 1` 和
  `cargo tree --invert <heavy-crate> --edges normal` 证明没有通过 `codex-protocol`、`config`、
  `tools` 或新的 shared crate 间接拉回 reqwest/ICU/V8/Starlark/Rama/app-server v2 envelope。
- JSON-RPC 基础 wire 类型属于 `codex-rs/jsonrpc-types`（`codex-jsonrpc-types`），包括
  `RequestId`、request/notification/response/error message envelope 和 `JSONRPCErrorError`。
  `codex-app-server-protocol::jsonrpc_lite` 只做兼容 re-export；`codex-exec-server`、remote executor
  或其他只需要 JSON-RPC message/error 的消费者应直接依赖 `codex-jsonrpc-types`，不要为了基础
  JSON-RPC envelope 拉入 app-server v2 thread/event/display protocol。
- MCP elicitation request/schema 这类 core/app-server 共同使用、但不需要 app-server v2 envelope 或
  `rmcp` conversion 的 DTO 属于 `codex-rs/mcp-types`（`codex-mcp-types`）。`codex-app-server-protocol`
  可以 re-export 这些类型并继续拥有 server request/response wrapper 与 `rmcp` adapter；`codex-core`
  和 tool handlers 不应直接依赖 app-server-protocol 来构造 elicitation request/schema。MCP 相关的
  纯常量和 sandbox payload，例如 `CODEX_APPS_MCP_SERVER_NAME`、`MCP_TOOL_CODEX_APPS_META_KEY`、
  `MCP_SANDBOX_STATE_META_CAPABILITY` 和 `SandboxState`，也属于 `codex-mcp-types`。MCP elicitation
  response DTO 也属于 `codex-mcp-types`：`ElicitationAction` 复用 `codex-protocol` 的 approvals
  enum，`ElicitationResponse` 保持为不依赖 `rmcp` 的 typed DTO；`codex-rmcp-client` 只在 rmcp
  protocol 边界 re-export/转换这些类型，`codex-core` 和 `codex-mcp` 不应为了构造 response DTO
  依赖 rmcp-client 的兼容 re-export。MCP `ToolInfo`、raw `rmcp::model::Tool` metadata、
  OpenAI file-param meta 解析和 model-visible input schema masking 属于 `codex-rs/mcp-tool-types`
  （`codex-mcp-tool-types`），因为它们需要承载 `rmcp::model::Tool`，不应放入纯 `codex-mcp-types`。
  `codex-core` 只需要 MCP tool metadata 时应直接依赖 `codex-mcp-tool-types`，不要经
  `codex-mcp` re-export；`codex-mcp-tool-types` 可以依赖 `rmcp`，但不得间接拉回 full
  `codex-mcp` runtime、`codex-rmcp-client`、login、model-provider 或 exec-server。MCP OAuth
  login/browser callback flow 属于 MCP runtime boundary；core 中安装 skill MCP dependencies 等路径
  应通过 `codex-mcp` 暴露的 runtime entry 调用，不要直接依赖 `codex-rmcp-client`。
- Plugin policy/interface/display metadata 这类插件领域 DTO 属于 `codex-rs/plugin-types`
  （`codex-plugin-types`）。`codex-app-server-protocol` 可以 re-export 用于 wire/schema 兼容；
  `codex-core-plugins`、TUI 或其他插件领域消费者应直接依赖 plugin types，不要为了
  `PluginInstallPolicy`、`PluginAuthPolicy`、`PluginAvailability`、`PluginInterface` 或
  `SkillInterface` 依赖 app-server-protocol。`PluginId`、`AppConnectorId`、
  `PluginCapabilitySummary`、`PluginTelemetryMetadata` 和 `PluginHookSource` 也属于 `codex-plugin-types`；
  `codex-plugin` 只 re-export 这些类型并承载 loader/runtime outcome，轻量 API crate 不要为了插件
  id、telemetry metadata、capability summary 或 hook source 依赖 `codex-plugin`。`codex-hooks` 和
  `codex-mcp` 这类只消费 hook/capability/provenance DTO 的 crate 应直接依赖 `codex-plugin-types`；
  只有真实 plugin loader/manager/outcome owner 才依赖 `codex-plugin`。
- `codex-core-plugins` 和 `codex-core-skills` 不应直接依赖 `codex-analytics`。插件生命周期 analytics
  通过 `PluginAnalyticsEventSink` 这类窄 trait 从组合根注入；skill injection 应返回领域 invocation
  数据，由 core/app-server 这类已经拥有 analytics client 的边界转换并上报。不要为了打点把
  app-server protocol 事件 reducer 或 analytics client queue 拉回 plugin/skill core crate。
- `codex-core`、runtime/session 编排和其他不需要真实 analytics 队列的 crate 只能依赖
  `codex-rs/analytics-api`（`codex-analytics-api`）中的 analytics DTO 与 `AnalyticsEventsClient`
  facade。真实 `codex-analytics` 继续拥有事件队列、reducer、HTTP 发送、app-server protocol adapter
  和 `codex-app-server-protocol` 依赖，由 app-server 组合根创建后通过 `api_client()` 注入 core。
  core 内部缺省 analytics client 应使用 disabled facade，不要直接构造真实 analytics client，也不要为了
  `track_*` 调用把 app-server protocol 或 analytics reducer 拉回 runtime crate。`codex-analytics-api`
  只能依赖 `codex-plugin-types` 等轻量类型；不得依赖 `codex-plugin`，否则会经 plugin loader/runtime
  依赖链把 `codex-login` 等实现 crate 间接拉回 core。
- 不需要真实 OTEL runtime 的 crate 只能通过 `codex-rs/metrics-api`（`codex-metrics-api`）记录
  best-effort metrics。该 crate 承载 `MetricsSink`、低基数 originator tag helper、纯 metric
  name 常量、纯 telemetry tag/source enum（例如 `ToolDecisionSource`）、
  `StatsigMetricsSettings` 跨进程 DTO、全局 counter/histogram/duration helper 和
  lightweight duration timer，不得依赖
  `codex-otel`、`codex-api`、HTTP/SSE/WebSocket runtime、tokio runtime 或 app-server protocol。真实
  `codex-otel` 可以 re-export metrics API 的纯类型，在初始化时把 concrete `MetricsClient` 安装到
  metrics API facade，且 `SessionTelemetry` 实现 `MetricsSink` 以保留 per-session metadata tags；
  `codex-mcp`、agent runtime、
  core-plugins/core-skills、rollout 或其他 runtime-adjacent crate 不应为了 global counter/duration、
  skill metrics、DB telemetry、metric name 常量或纯 tag/source enum 直接依赖 `codex-otel`。
  `codex-core` 需要记录 best-effort 全局 counter、histogram 或 duration 时，应使用
  `codex_metrics_api::record_global_*` / `start_global_timer`；不要调用 `codex_otel::global()`
  获取 concrete metrics client。只有 `SessionTelemetry` 和 OTEL provider implementation 这类
  concrete telemetry runtime 边界才继续依赖 `codex-otel`，直到对应 facade 拆出。
  metrics-only helper 的函数签名应接收 `&dyn codex_metrics_api::MetricsSink` 或对应 facade 类型，
  不要把 helper 绑定到 `codex_otel::SessionTelemetry`；调用方可以继续传入 session telemetry 以保留
  per-session metadata tags，但 helper 本身必须停留在 metrics API 边界。
  如果 core 只需要持有一个 duration timer 直到 drop 记录指标，应隐藏为 boxed drop guard，避免在
  session/turn state 类型中公开 `codex_otel::Timer` 这类 concrete OTEL runtime 类型。
  只需要 metric tag sanitization 时，直接使用 `codex-utils-string::sanitize_metric_tag_value`，
  不要通过 `codex_otel::sanitize_metric_tag_value` re-export 形成语义上的 runtime dependency。
- W3C trace propagation helper 属于 `codex-rs/trace-context`（`codex-trace-context`）：
  current span trace id、W3C trace carrier、traceparent/tracestate validation、从环境变量恢复
  trace context、给 span 设置 parent context 这类 helper 应直接从该轻量 crate 引用。`codex-otel`
  可以 re-export 这些 helper 兼容旧路径，并在 provider init 中设置 tracestate；但 core/session/runtime、
  API client、测试或其他非 OTEL runtime crate 不应为了 trace helper 依赖 full `codex-otel`。
  `SessionTelemetry` 和 OTEL provider implementation 仍属于 `codex-otel`，后续需要单独设计
  session telemetry facade，不能把 W3C trace helper 拆出误写成 full OTEL direct edge 已消除。
- OTEL provider 初始化、process-start metrics 和 SQLite telemetry recorder 安装属于
  `codex-rs/otel-init`（`codex-otel-init`）。该 crate 是 app-server、TUI、exec、mcp-server
  这类组合根的 startup helper，可以依赖 `codex-otel`、`codex-rollout` 和 `codex-state`，但不得依赖
  `codex-core` 或 `codex-core::Config`。调用方应把完整 runtime config 投影成
  `codex_otel_init::OtelProviderConfig`（`codex_home`、`OtelConfig`、analytics flag、runtime metrics flag）
  后调用 `build_provider`；不要从 core re-export 或恢复 `codex_core::otel_init`。
- `codex-windows-sandbox` 默认 feature 可以为 standalone setup helper 保留 WFP Statsig metrics
  emission，但 `codex-core` 依赖它时必须关闭默认 feature，避免 core 通过 Windows sandbox helper
  间接拉入 full `codex-otel`。需要传递 setup metrics 配置时使用
  `codex_metrics_api::StatsigMetricsSettings` DTO；只有真正构造 OTEL provider 的 setup helper
  implementation 才能依赖 `codex-otel`。
- `codex-core` 的 fork/resume snapshot turn-state 判断不得依赖 app-server display history builder
  （例如 `ThreadHistoryBuilder` / `TurnStatus`）。core 只需要知道 rollout 是否截在显式 turn 中间时，
  应直接扫描 typed `RolloutItem::EventMsg(EventMsg::TurnStarted | TurnComplete | TurnAborted)`；
  完整 `ThreadHistoryBuilder` 继续属于 app-server-protocol display/history projection 边界。
- `codex-protocol` 的基础展示/计数格式化 helper 不应为了 locale-aware 展示拉入 ICU 或
  `sys-locale`。token 数、字符数这类跨 TUI/exec/protocol 复用的轻量格式化保持固定逗号分隔；如果未来
  需要真正 locale-aware 的 UI 展示，应放到 TUI/app-server UI 边界或新的窄 UI formatting crate，不要让
  config/protocol 消费者通过 `codex-protocol` 间接编译 ICU 数据栈。
- `codex-protocol::error` 是跨 runtime/UI 的错误语义层，不应保存 `reqwest::Error` 这类具体 transport
  implementation error。需要传递 HTTP 语义时保留 `http::StatusCode` 和用户可见 message/request id；
  reqwest/codex-client/codex-api 的具体错误应在 API/runtime adapter 边界映射成 protocol error DTO，避免
  config、protocol、TUI 和其他轻量消费者为了错误枚举编译 reqwest client stack。
- Responses API 请求 shape、stream event DTO 和 websocket 请求 metadata helper 属于 `codex-rs/api-types`
  （`codex-api-types`）：`Reasoning`、`TextControls`、`OpenAiVerbosity`、`ResponsesApiRequest`、
  `ResponseCreateWsRequest`、`ResponsesWsRequest`、`ResponseEvent`、websocket request metadata key 和
  `create_text_param_for_request` / `response_create_client_metadata` 这类纯 DTO/helper 应从该 crate 引用。
  Realtime session selection DTO（`RealtimeEventParser`、`RealtimeSessionMode`、
  `RealtimeSessionConfig`）也属于 `codex-api-types`；Realtime audio/event payload
  （`RealtimeAudioFrame`、`RealtimeEvent`）属于 `codex-protocol`，不要通过 full `codex-api`
  re-export 在 core/session runtime 中使用这些纯类型。
  SSE/WebSocket telemetry 对外也只能暴露 `SseEventTelemetry` / `WebsocketEventTelemetry` 这类
  transport-neutral summary DTO；`eventsource_stream::Event`、`EventStreamError` 和
  `tokio_tungstenite::tungstenite::Message/Error` 的分类归纳属于 `codex-api` runtime 边界，不要让
  `codex-core` 或 `codex-otel` 为 telemetry 实现直接依赖这些 transport parser/runtime 类型。
  `codex-api` 只 re-export 这些类型用于旧路径兼容，并继续拥有 API client、auth header adapter、
  HTTP transport、SSE/WebSocket parser、`ResponseStream` 和 endpoint runtime；
  core/session runtime 不应为了构造 request body、text controls 或匹配 response stream event 依赖完整
  `codex-api`。
- OpenAI file upload API 边界属于 `codex-rs/openai-files-api`（`codex-openai-files-api`）：
  `UploadedOpenAiFile`、`OpenAiFileUploader`、`SharedOpenAiFileUploader` 和 disabled uploader 从该轻量
  crate 引用；它只能依赖 auth provider/serde 等低层类型，不得拉 `reqwest`、`codex-client`、
  `codex-api`、`codex-core`、MCP runtime 或 app-server protocol。真实上传 runtime 属于
  `codex-rs/openai-files`（`codex-openai-files`）：`upload_local_file`、`OpenAiFileError`、
  `openai_file_uri`、文件上传限制常量和 `ReqwestOpenAiFileUploader` 实现从该 crate 引用，该 crate 可以拥有
  上传所需的 `reqwest` / `codex-client` custom CA runtime，但不得依赖 full `codex-api`、`codex-core`、
  `codex-otel`、MCP runtime 或 app-server protocol。`codex-core` 只应持有
  `Arc<dyn OpenAiFileUploader>`，由 app-server/mcp-server/test-support 组合根通过 constructor injection
  注入真实实现；不要让 core production manifest 直接依赖 `codex-openai-files`，也不要让 `codex-api` 为旧路径
  兼容 re-export 文件上传 helper，否则 core 只要依赖 API client 就会间接拉回文件上传 runtime。
- Feedback request tag API 边界属于 `codex-rs/feedback-api`（`codex-feedback-api`）：
  `FeedbackRequestTags`、`emit_feedback_request_tags` 和
  `emit_feedback_request_tags_with_auth_env` 应从该轻量 crate 引用；它只允许依赖
  `codex-auth-types` 和 `tracing` 这类低层 telemetry type/emission 依赖。带 auth env 的 helper 接收
  `AuthEnvTelemetryMetadata`，不要让 API crate 依赖 `codex-login::AuthEnvTelemetry`。`codex-feedback`
  继续拥有 feedback ring buffer、metadata layer、Sentry upload、diagnostics attachment 和
  `tracing-subscriber` runtime，并可 re-export 这些 API 以兼容旧路径；`codex-core`、`codex-model-provider`
  或其他只需要打 request tag 的 crate 不应 normal 依赖 heavy `codex-feedback`。
- `codex-otel` 不应为了 telemetry event classification 依赖完整 `codex-api`。记录 Responses stream event
  时直接依赖 `codex-api-types::ResponseEvent`；SSE/WebSocket poll 指标只能消费
  `codex-api-types` 的 telemetry summary DTO，不要重新匹配 raw transport event/message 类型。websocket
  telemetry 的外层错误参数应保持为 `Display`/轻量错误语义，不要绑定 `codex_api::ApiError`，否则
  `codex-otel -> codex-api` 会把 full API runtime 间接拉回 core 和 windows sandbox。`ApiError` 仍属于
  `codex-api` runtime boundary，只有 API client/adapter 层应直接匹配它。`SessionTelemetry` 不应暴露
  `reqwest::Response/Error` 绑定的 helper；API request telemetry 应由 API/client adapter 归纳成
  transport-neutral fields 后调用 `record_api_request`。
- HTTP client request/error/retry 基础类型属于 `codex-rs/client-types`（`codex-client-types`）：
  `Request`、`RequestBody`、`RequestCompression`、`PreparedRequestBody`、`Response`、`TransportError`、
  `StreamError`、`RetryPolicy` 和 `RetryOn` 应从该 crate 作为轻量类型层复用；`codex-client` 只 re-export
  这些类型并继续拥有 reqwest transport、SSE stream、custom CA、retry executor、request telemetry 和
  default client runtime。新 crate 不应为了构造或签名 request body 依赖完整 `codex-client`。
- Response debug context helper 属于 `codex-rs/response-debug-context`（`codex-response-debug-context`），
  只处理 `codex-client-types::TransportError` 中的 HTTP debug headers 和 transport error telemetry
  message。该 crate 不得依赖 `codex-api`、`codex-client`、reqwest、tokio、SSE/WebSocket runtime 或
  app-server protocol。`ApiError` 到 response debug context / telemetry message 的 adapter 属于
  `codex-api`，因为 `ApiError` 是 API runtime error；core 可以从 `codex-api` 引用该 adapter，但不要把
  `ApiError` helper 放回 response-debug-context 形成 `response-debug-context -> codex-api` 回流。
- API provider/auth 基础边界属于 `codex-rs/api-provider`（`codex-api-provider`）：`Provider`、
  `RetryConfig`、`AuthProvider`、`SharedAuthProvider`、`AuthProviderFuture`、`AuthError`、
  `AuthHeaderTelemetry`、`auth_header_telemetry`、session header helper 和 Azure endpoint detection 应从该
  crate 引用。`codex-api` 只 re-export 这些类型用于旧路径兼容，并继续拥有具体 endpoint client、
  `ApiError`、`ResponseStream`、HTTP/SSE/WebSocket runtime 和 file upload；`model-provider-api`、core 和
  model-provider implementation 不应为了 provider config 或 auth-header adapter 依赖完整 `codex-api`。
- `codex-protocol::items` 中 hook prompt 使用的 `<hook_prompt hook_run_id="...">...</hook_prompt>` 是受控
  internal marker，不应为了这一处 marker 重新引入通用 XML serde/parser 依赖。修改该 marker 时保持手写
  XML entity escape/unescape 的受控实现，并用 hook prompt roundtrip/legacy parse 测试覆盖；需要通用 XML
  解析时应先证明这是新的协议边界，而不是把 quick-xml 拉回 shared protocol。
- `codex-protocol::config_types::EnvironmentVariablePattern` 是 shell env policy 的轻量 wildcard 类型，仅支持
  `*` 和 `?` 的整串匹配以及显式大小写无关构造；不要为了环境变量 include/exclude pattern 重新把
  `wildmatch`、`regex` 或 glob 运行时拉入 shared protocol。修改该类型时用 env pattern、shell_environment
  和 exec_env 覆盖保证 include/exclude/default sensitive filtering 行为不变。
- `AuthMode` 和 ChatGPT workspace 登录限制 config 形状 `ForcedChatgptWorkspaceIds` 属于认证域共享类型，
  owner crate 是 `codex-rs/auth-types`
  （`codex-auth-types`）。login、model-provider-info、otel、models-manager、core、CLI/TUI 和
  app-server 需要认证模式或登录限制 DTO 时应直接依赖 `codex-auth-types`；`codex-app-server-protocol`
  或 `codex_config::config_toml` 只 re-export 这些类型用于 wire/旧路径兼容，不要把新的 auth domain type
  放回 app-server-protocol 或 full `codex-config`。`AuthEnvTelemetryMetadata` 和 `TelemetryAuthMode`
  这类认证环境 telemetry DTO / tag enum 也属于 `codex-auth-types`；`codex-login` 可以收集 auth env state
  并转换成该 DTO，但不要为了 DTO 或 tag enum 依赖 `codex-otel`，否则会把 `codex-api`/HTTP runtime
  经 telemetry 栈间接拉回 login/model-provider-api。
- 默认 Codex HTTP client metadata/helper 属于 `codex-rs/default-client`（`codex-default-client`）：
  `originator`、first-party originator 判断、User-Agent、default headers、residency header state、
  default `reqwest` client builder 和 `CodexHttpClient` constructor 应从该 crate 引用。`codex-login`
  只为旧路径兼容 re-export 这些 helper；不需要 token refresh、auth storage、login server 或 revoke
  runtime 的 crate 不应为了 default client helper 依赖 `codex-login`。`codex-default-client` 可以依赖
  `codex-client`、`codex-config-types` 和 terminal detection，但不得依赖 `codex-login`、
  keyring/agent-identity/login-server runtime、`codex-api` 或 model-provider implementation。
- config layer source/metadata/layer 属于配置域共享类型，owner crate 是
  `codex-rs/config-types`（`codex-config-types`）；`codex-config` 负责 layer stack、loader、
  diagnostics 和本地 loader/validation 集成，`codex-app-server-protocol` 只能 re-export 或包装这些 layer DTO 以保持 wire 兼容。
  `codex-config` 不得直接依赖 `codex-app-server-protocol` 来返回 `ConfigLayerSource`、
  `ConfigLayerMetadata`、`ConfigLayer` 或其他 v1/v2 transport payload。
- `config.toml` 的 schema-heavy 外层 shape 属于 `codex-rs/config-toml`
  （`codex-config-toml`）：`ConfigToml`、`ConfigProfile`、`ProfileTui`、`ToolsToml`、
  `RealtimeConfig`/`RealtimeToml`/`RealtimeAudioToml`、`DebugToml`、`AutoReviewToml`、
  `GhostSnapshotToml`、config schema helper，以及只服务这些外层 shape 的
  `AnalyticsConfigToml`、`FeedbackConfigToml`、`Tui`、`ShellEnvironmentPolicyToml` 都归这个 crate。
  它可以依赖 `codex-config-types`、`codex-config-loader`、`codex-config-permissions`、
  `codex-features`、`codex-model-provider-info` 和 `codex-protocol` 来描述现有 TOML schema，
  但不得依赖 full `codex-config`、`codex-app-server-protocol`、`codex-code-mode`、Starlark-backed
  `codex-execpolicy` 或 Rama-backed `codex-network-proxy`。`codex_config::config_toml`、
  `codex_config::profile_toml`、`codex_config::schema` 和相关 `codex_config::types`
  只作为旧路径兼容 re-export；core、app-server 或其他只需要 TOML shape/schema 的消费者应直接依赖
  `codex-config-toml`，不要为了 `ConfigToml`/schema helper 拉入 full `codex-config`。
- config schema 生成 CLI 属于 `codex-rs/config-schema`（`codex-config-schema`），bin 名称仍为
  `codex-write-config-schema`。它只应依赖 `codex-config-toml` 的 schema API、`clap` 和基础错误处理；
  不要把 schema 写入命令放回 `codex-core`，也不要让 `codex-core` 为这个维护工具携带 `clap`。
- local filesystem config layer loader 属于 `codex-rs/config-local-loader`
  （`codex-config-local-loader`）：system/user/profile/project/repo/runtime layer IO、strict TOML
  validation、legacy managed config 到 requirements 的映射、thread config source 到 layer stack 的投影、
  project-local trust/root/git checkout 处理、relative path resolution 和 system config/requirements path
  helper 都归这个 crate。`codex_config::loader::*` 只保留兼容 re-export；core、app-server 或测试 helper
  需要 `load_config_layers_state`、`load_requirements_toml`、`project_trust_key` 或
  `resolve_relative_paths_in_config_toml` 时应直接依赖 `codex-config-local-loader`。该 crate 可以依赖
  `codex-config-diagnostics`、`codex-config-loader`、`codex-config-requirements`、`codex-config-state`、
  `codex-config-toml`、`codex-config-types`、`codex-file-system`、`codex-git-utils`、
  `codex-model-provider-info` 和 `codex-protocol` 来完成现有 local layer 语义，但不得依赖 full
  `codex-config`、`codex-app-server-protocol`、`codex-code-mode`、Starlark-backed `codex-execpolicy` 或
  Rama-backed `codex-network-proxy`。不要把 effective runtime `Config` 构造、session defaults、network
  proxy backend/evaluator 或 app-server transport adapter 移入 local-loader；这些属于 core/runtime 或
  app-server 组合根边界。
- 不依赖 loader、diagnostics、filesystem/git/MDM/remote-thread-config 或 model-provider validation 的纯
  config DTO，例如 history settings、credential store mode、residency enum、TOML schema 子类型和
  memory settings、otel settings、UI/settings 枚举、Realtime transport/mode/resolved audio config
  （`RealtimeAudioConfig`）、Windows sandbox TOML DTO 和 workspace-write sandbox DTO
  （`SandboxWorkspaceWrite`），
  应优先放在 `codex-config-types`。TUI keymap schema
  （`TuiKeymap`、`KeybindingSpec` 等）、notification/session picker/tool-suggest/notices 这类纯 UI/config
  persistence DTO，以及 app connector 配置 DTO（`AppsConfigToml`、`AppsDefaultConfig`、`AppConfig`、
  `AppToolsConfig`、`AppToolConfig`）也属于 `codex-config-types`；OTEL config 的无 runtime 校验 helper
  （例如 span attribute key 和 W3C tracestate config grammar validator）也属于 `codex-config-types`，
  core/config loader 不要为了清洗 config metadata 依赖 `codex-otel` 或 OpenTelemetry SDK；
  `codex-otel` 可以 re-export 旧 validator 路径并在 provider/init 边界复用同一套 helper。
  `codex_config::types` 只做兼容 re-export。MCP server config/tool
  approval/env var/OAuth DTO 和 hook TOML/JSON DTO（`HooksFile`、`HooksToml`、`HookEventsToml`、`MatcherGroup`、
  `HookHandlerConfig`、managed hook requirements）也属于这个轻量类型边界；依赖
  `codex_protocol::HookEventName` 的 hook 事件投影留在 `codex-config`/`codex-hooks` 边界，不要让
  `codex-config-types` 反向依赖 protocol。`codex-config` 可以 re-export 这些类型保持
  兼容，但 message-history、login、rmcp-client、TUI helper 等轻量消费者应直接依赖
  `codex-config-types`，不要为了一个纯 DTO 拉入完整 `codex-config`。
- Agent role declaration DTO（`AgentsToml`、`AgentRoleToml`）、`ThreadStoreToml` 和外层
  config lockfile DTO（`ConfigLockfileToml<TConfig>`）属于
  `codex-config-types`，`codex_config::config_toml` 只做兼容 re-export。Agent role discovery
  的 required-description 校验不能放在通用目录 discovery 阶段，因为高优先级 role 可以先缺
  description 再从低优先级 layer 继承；只在最终插入或 plugin merge 等不会再发生继承的边界过滤。
- model provider domain TOML/helper 属于 `codex-model-provider-info`：`ModelOptionToml`、
  `validate_model_providers`、`validate_reserved_model_provider_ids`、`deserialize_model_providers`
  和 `validate_oss_provider` 应与 `ModelProviderInfo`/provider 常量同 owner。`codex_config::config_toml`
  只做旧路径兼容 re-export；core effective config、app-server model 展示或其他消费者需要 model option
  / provider validation 时应直接依赖 `codex-model-provider-info`，不要为了这类 model-provider domain
  helper 拉入完整 `codex-config`，也不要把它们塞入无 protocol 依赖的 `codex-config-types`。该 crate
  只能承载 provider DTO/catalog/validation 和不依赖 API client 的轻量 helper。把
  `ModelProviderInfo` 转换成 `codex_api_provider::Provider`、解析 HTTP header map、按 auth mode 选择 API
  base URL，以及把 `CodexAuth` 映射成 `codex_api_provider::AuthProvider` 的 request-header adapter 属于
  `codex-model-provider-api`；需要这些 request adapter 的 core/core-plugins/core-skills/codex-mcp
  消费者应直接依赖该 crate，不要为了 headers 或 provider config adapter 拉入完整
  `codex-model-provider`。只需要 `auth_provider_from_auth` 或 `unauthenticated_auth_provider` 的
  backend/app-server transport helper 也应直接依赖 `codex-model-provider-api`，不要经
  `codex-model-provider` 兼容 re-export 间接拉回 API/model runtime。`codex-model-provider-api` 不是纯基础类型层，它可以依赖 `codex-login`
  来理解 `CodexAuth`。runtime provider trait/types 也属于 `codex-model-provider-api`：
  `ModelProvider`、`SharedModelProvider`、`ModelProviderFuture`、`ProviderCapabilities`、
  `ProviderAccountState`、`ProviderAccountError` 和 `ProviderAccountResult` 应从 API crate 引用；不要为了
  trait object、provider capability 或 account state 类型拉入完整 `codex-model-provider`。
  `ModelProviderFactory` / `SharedModelProviderFactory` 也属于 `codex-model-provider-api`；core/session
  runtime 只能通过 constructor injection 持有该 factory trait，不要直接调用完整
  `codex-model-provider` 的 concrete constructor。`DefaultModelProviderFactory`、`create_model_provider`、
  configured provider、Bedrock provider、provider-scoped auth manager construction、model manager
  implementation selection 和 request execution 边界继续属于完整 `codex-model-provider`，只应由
  app-server/CLI/MCP server 等组合根或 core test support 构造并注入。core 单测需要 provider factory 时
  使用 `codex_core::test_support::model_provider_factory_for_tests()`，不要把 `codex-model-provider`
  加回 core normal dependency。
  不要让 config-facing info crate 依赖 `codex-api`、`http` 或 client stack。
- model catalog API 属于 `codex-rs/models-manager-api`（`codex-models-manager-api`）：
  `ModelsManager`、`SharedModelsManager`、`RefreshStrategy`、`TryListModelsError`、
  `ModelsManagerConfig` 和 `ModelMetadataOverride` 应由这个 API crate 承载。core、core-api、
  app-server、CLI、model-provider 或其他只需要模型目录 trait/config 的消费者应直接依赖
  `codex-models-manager-api`，不要为了 trait、refresh strategy 或 config override 拉入完整
  `codex-models-manager`。完整 `codex-models-manager` 继续拥有 bundled model catalog、cache、
  remote refresh、concrete manager、collaboration presets、model_info fallback/override 逻辑和测试。
  `codex-models-manager-api` 当前仍依赖 `codex-protocol::openai_models` 的模型 DTO；不要把它当作完全
  无 protocol 依赖的基础类型层。若后续要继续降低这条边，应先拆 model catalog DTO/config types，而不是
  通过 full manager re-export 绕回 implementation。`codex_core::test_support` 是非生产测试支撑边界，
  只能在 `codex-core/test-support` feature 或本 crate unit tests 中编译；该 feature 才允许 core
  拉入 full `codex-models-manager` 的 bundled catalog/offline helper/concrete manager。不要把
  `codex-models-manager` 加回 `codex-core` 默认 normal graph。需要 legacy re-export 的客户端测试应启用
  对应 crate 的 test-support feature，例如 `codex-app-server-client/test-support`，生产 dependency 不要默认
  re-export core test support。
- project trust config shape（`ProjectConfig`）、project trust key/lookup、project root marker 解析和 CLI override dotted TOML layer builder
  这类 loader 层路径/根检测/运行时 layer 构造 helper 属于
  `codex-config-loader`；需要读取项目 trust config、生成 `project_trust_key`、做 trust lookup，或从 merged TOML 解析
  `project_root_markers`、构造 `build_cli_overrides_layer` 的消费者应直接依赖
  `codex-config-loader`。`codex_config::loader` 和 `codex_config::*` 只作为旧路径兼容
  re-export，不要为了这些 helper 拉入完整 `codex-config`。
- permissions profile TOML、filesystem permission TOML、network profile TOML 和 profile 到
  `NetworkProxyConfig` 的 overlay helper 属于 `codex-rs/config-permissions`
  （`codex-config-permissions`）。这个 crate 可以依赖 `codex-protocol` 的 filesystem permission
  基础类型和 `codex-network-proxy-api` 的 proxy DTO，但不得依赖完整 `codex-config`、
  app-server protocol、Starlark、Rama 或 code-mode runtime。`codex_config::permissions_toml`
  只作为旧路径兼容 re-export；core runtime、network proxy loader 或其他只需要 permissions
  TOML/profile 的消费者应直接依赖 `codex-config-permissions`，不要为了 permissions profile
  拉入完整 `codex-config`。
- `codex-core-api` 这类 facade crate 为了导出纯 config DTO 时应直接依赖 `codex-config-types`，
  不要为了 `History`、credential store mode、TUI/settings、memories、otel 等纯类型 re-export
  重新 normal 依赖完整 `codex-config`。仍必须兼容导出的 full config loader/state 类型（例如
  `ConfigLayerStack`、Realtime config）可以暂时经 `codex_core::config` 旧 facade
  暴露；要消除这条间接路径时，应先拆出明确的 `codex-config-loader`/state 边界，而不是把 full
  config 直接加回 facade。
- `codex-rs/config-loader`（`codex-config-loader`）只承载 loader API：`LoaderOverrides`、
  `ConfigLoadOptions`、thread config source/loader trait、Noop/Static loader、`ProjectConfig`、project trust/root marker
  helper 和 CLI override TOML layer builder 等轻量边界。它不得依赖
  full `codex-config`，也不得包含 tonic/prost remote implementation。remote thread config loader 属于
  `codex-rs/config-loader-remote`（`codex-config-loader-remote`），由 app-server、app-server-client
  等组合根显式依赖；不要通过 `codex-config` re-export remote implementation，也不要让
  `codex-config -> codex-config-loader -> codex-config-loader-remote` 把 remote/gRPC 依赖间接拉回 full
  config。`codex-config` 负责把 thread config sources 投影成 `ConfigLayerEntry`，loader API crate 不拥有
  full config layer stack 或 local loader IO。
- `codex-rs/features`（`codex-features`）负责 feature registry、feature TOML DTO、legacy key
  handling 和 warning event construction，不得依赖 `codex-otel` 或其他 telemetry/runtime
  implementation。需要按 enabled feature 发 metrics 时，应在持有 `SessionTelemetry` 的 runtime/组合根
  边界实现 helper，避免 `codex-config -> codex-features -> codex-otel` 把 OTEL、tonic 或 prost
  间接拉入 config graph。
- `codex-rs/config-diagnostics`（`codex-config-diagnostics`）负责轻量 config 诊断类型和
  TOML span/formatting helper：`ConfigError`、`ConfigLoadError`、`TextRange`、
  `config_error_from_toml*`、`format_config_error*` 和 `io_error_from_config_error`。
  `codex-config` 只 re-export 旧路径；CLI/TUI/exec/app-server 这类只需要
  downcast/展示 config load error 的入口，应直接依赖 diagnostics crate 和 `codex-config-loader`，
  不要为了错误展示或 loader options 重新 normal 依赖 full `codex-config`。
- `codex-rs/config-state`（`codex-config-state`）负责轻量 config layer state：`ConfigLayerEntry`、
  `ConfigLayerStack`、`ConfigLayerStackOrdering`、`merge_toml_values`、key alias、origin helper
  和需要 `ConfigLayerStack`/filesystem 的 first-layer diagnostic 定位 helper。
  `codex-config` 只 re-export 旧路径并保留完整 loader/validation 集成；TUI debug display、
  app-server config manager 或其他只需要展示/排序/合并已加载 layer 的调用方，应直接依赖
  `codex-config-state`、`codex-config-requirements` 和 `codex-config-types`，不要为了 layer stack
  display/mutation 重新 normal 依赖 full `codex-config`。`codex-config-state` 不得依赖 full
  `codex-config`、app-server protocol、Starlark、Rama 或 code-mode runtime。
- Plugin 和 marketplace 的纯 TOML DTO（`PluginConfig`、`PluginMcpServerConfig`、
  `MarketplaceConfig`、`MarketplaceSourceType`）属于 `codex-config-types`；`codex-config`
  只做旧路径 re-export。`codex-core-plugins` 的读取侧应使用
  `PluginConfigLayerStack` / `PluginConfigLayerEntry` 这类 plugin 专用只读 view；core/app-server
  等组合根负责从完整 `codex_config::ConfigLayerStack` 投影过去。写用户 `config.toml` 的 plugin
  enable/clear、marketplace add/remove/upgrade、MCP server edit/load helper
  （`ConfigEditsBuilder`、`load_global_mcp_servers`）、`CONFIG_TOML_FILE` 和 `version_for_toml`
  属于 `codex-rs/config-edit`（`codex-config-edit`）；`codex-config` 只 re-export 以保持旧路径兼容。
  `codex-core-plugins` production path 不得 normal 依赖 full `codex-config`，测试 fixture 需要完整
  loader 时才允许 dev-depend `codex-config`。
- requirements TOML、normalized requirements、requirements exec policy TOML/evaluator 和 cloud requirements
  loader 属于 `codex-rs/config-requirements`（`codex-config-requirements`）。`codex-config`
  只作为旧路径兼容 re-export；loader、MDM、diagnostics、filesystem/git、profile/thread config
  stack 和 model-provider validation 继续留在 `codex-config` 或后续明确的 loader crate。`codex-cloud-requirements`
  这类只需要 requirements 解析/加载的小 crate 不得 normal 依赖完整 `codex-config`；需要 policy DTO 时依赖
  `codex-execpolicy-api`，需要 config DTO 时依赖 `codex-config-types`。
- `codex-hooks` 不得为了 hook discovery 直接 normal 依赖完整 `codex-config`。Hook runtime 需要已加载
  config stack 时，应使用 `codex_hooks::HookConfigLayerStack` / `HookConfigLayerEntry` 这类 hook
  专用只读 view；core/app-server 等组合根负责从 `codex_config::ConfigLayerStack` 投影过去。Hook
  event name 投影和 hook trust hash 可以留在 `codex-hooks` 或 `codex-config` 边界，但不要放入
  `codex-config-types` 反向依赖 protocol，也不要把 full loader/requirements evaluator 混进 hooks crate。
- `codex-core-skills` 不得为了读取 `[skills]`、技能开关或 project root marker 直接 normal 依赖完整
  `codex-config`。Skill runtime 需要已加载 config stack 时，应使用
  `codex_core_skills::SkillConfigLayerStack` / `SkillConfigLayerEntry` 只读 view；core、core-plugins 或
  app-server 等组合根负责从 `codex_config::ConfigLayerStack` 投影过去。`SkillsConfig` /
  `SkillConfig` / `BundledSkillsConfig` 属于 `codex-config-types`，允许 `codex-config-types` normal 依赖
  `toml` 来支持从 `toml::Value` 反序列化这些纯 DTO，但不得因此引入 full config loader、
  app-server protocol、Starlark、Rama 或 V8。
- Dynamic Workflow 的 manifest、registry、summary/details、discovery diagnostics 和 init-context
  renderer，以及 `WorkflowRun` / `WorkflowRunStatus` / `WorkflowAgentBinding` /
  `WorkflowRuntimeBridge` / `WorkflowRuntimeRequest` / `WorkflowRuntimeError`、
  `WorkflowRunController` 和 `WorkflowRunUpdateReceiver` 这类 run-control DTO/trait 属于
  `codex-rs/workflow-api`（`codex-workflow-api`）。core/app-server 等只需要列出、描述、展示 workflow
  run、实现 runtime bridge 或启动 workflow tool 时应直接依赖该 API crate；`codex-core` 不应 normal
  依赖 concrete `codex-workflow`。`WorkflowRunManager`、runner bridge implementation、
  Node/TypeScript runner process、snapshot persistence 和 abort/resume/status 运行时控制继续属于
  `codex-rs/workflow`（`codex-workflow`），由 app-server、MCP server 或 test support 这类组合根通过
  `ThreadManager::new_with_workflow_runs` 注入。
- 小型 UI/CLI helper crate 不得为了读取 `codex_core::config::Config` 的少数字段 normal 依赖
  `codex-core`。调用方已经持有 effective `Config` 时，应在组合根/调用方边界提取轻量输入传入
  helper，例如 `model: Option<&str>`、`ModelProviderInfo`、provider map、`PermissionProfile`、
  `SandboxPolicy` 或 `AbsolutePathBuf`。`codex-utils-sandbox-summary` 只负责 protocol sandbox/permission
  summary；`codex-utils-oss`、`codex-lmstudio`、`codex-ollama` 的 normal graph 不应依赖
  `codex-core`，测试需要 sandbox env 常量时只能作为 dev-dependency。
- code-mode runtime implementation 只能由产品入口/组合根显式注入，例如 app-server 或 mcp-server 使用
  `codex_code_mode::V8CodeModeRuntimeFactory` 创建 `ThreadManager`；`codex-core` 和 core tests 使用
  `codex-code-mode-api` trait/disabled factory，不要为了补调用点把 `codex-code-mode` 重新加回 core。
- connector app metadata 属于 connector domain 共享类型，owner crate 是
  `codex-rs/connectors-types`（`codex-connectors-types`）。`AppBranding`、`AppReview`、
  `AppScreenshot`、`AppMetadata`、`AppInfo`、`AppSummary` 这类生产路径类型应由
  connectors、chatgpt、tools、core connector/render 代码直接依赖 `codex-connectors-types`；
  `codex-app-server-protocol` 只 re-export 或包装 app list request/response/notification 的 wire
  payload，不作为 connector metadata 的 owner。
- request-plugin-install 的 tool discovery、安装建议参数、结果和 elicitation plan 属于
  `codex-rs/tools` 的 domain/tool planning 边界；`codex-tools` 不得直接依赖
  `codex-app-server-protocol` 来构造 `McpServerElicitationRequestParams`。需要发给客户端的 MCP
  elicitation request 应由 core/app-server runtime 边界把 tools 返回的 plan 投影成
  app-server protocol payload。
- domain crate 不要为了“方便转换”实现到 app-server protocol payload 的 `From`/`Into` 或直接返回
  app-server request/response DTO。例如 config crate 不应生成 v1/v2 `UserSavedConfig`、`Profile`、
  `SandboxSettings`、`ConfigLayer` 这类 transport payload；应由 app-server/protocol 边界把 domain type
  映射成 wire type。只有真正被多个非 transport crate 共享、且 owner 明确的小类型（例如 auth mode、
  connector app metadata、config layer source/metadata）才拆到 owner API/types crate，并由
  app-server-protocol re-export 或包装。
- 当小 crate 只需要 core/app-server 的少量运行时能力时，优先定义轻量 service facade/trait crate-local 边界，再由 app-server 或 core-adapter 侧通过 constructor injection 显式注入实现。小 crate 暴露业务请求/配置子集和 trait（例如 runtime、agent handle、prompt request），不要直接依赖 `ThreadManager`、`CodexThread`、`Config`、`ModelClient`、`Prompt` 或整套 session/turn loop。需要多个服务时可以在 host 侧组合成 `RuntimeServices` / service registry，但注册表应只是显式持有和传递 typed services，不要引入宏驱动或反射式 IoC 框架，除非有清晰的维护和编译收益。
- service facade 的实现方应放在拥有重依赖的 host crate，例如 app-server adapter 负责把轻量 request 转成 core `Prompt`、创建 `ModelClient`、构造 locked-down `Config`、spawn/shutdown internal thread；业务 crate 只表达“要做什么”。测试需要 core-backed 行为时，把 core adapter 放在 `#[cfg(test)]` 或 dev-dependency 路径，不能为了测试便利把 `codex-core` 拉回 normal dependencies。

Likewise, when reviewing code, do not hesitate to push back on PRs that would unnecessarily add code to `codex-core`.

## TUI style conventions

See `codex-rs/tui/styles.md`.

## TUI code conventions

- Use concise styling helpers from ratatui’s Stylize trait.
  - Basic spans: use "text".into()
  - Styled spans: use "text".red(), "text".green(), "text".magenta(), "text".dim(), etc.
  - Prefer these over constructing styles with `Span::styled` and `Style` directly.
  - Example: patch summary file lines
    - Desired: vec!["  └ ".into(), "M".red(), " ".dim(), "tui/src/app.rs".dim()]

### TUI Styling (ratatui)

- Prefer Stylize helpers: use "text".dim(), .bold(), .cyan(), .italic(), .underlined() instead of manual Style where possible.
- Prefer simple conversions: use "text".into() for spans and vec![…].into() for lines; when inference is ambiguous (e.g., Paragraph::new/Cell::from), use Line::from(spans) or Span::from(text).
- Computed styles: if the Style is computed at runtime, using `Span::styled` is OK (`Span::from(text).set_style(style)` is also acceptable).
- Avoid hardcoded white: do not use `.white()`; prefer the default foreground (no color).
- Chaining: combine helpers by chaining for readability (e.g., url.cyan().underlined()).
- Single items: prefer "text".into(); use Line::from(text) or Span::from(text) only when the target type isn’t obvious from context, or when using .into() would require extra type annotations.
- Building lines: use vec![…].into() to construct a Line when the target type is obvious and no extra type annotations are needed; otherwise use Line::from(vec![…]).
- Avoid churn: don’t refactor between equivalent forms (Span::styled ↔ set_style, Line::from ↔ .into()) without a clear readability or functional gain; follow file‑local conventions and do not introduce type annotations solely to satisfy .into().
- Compactness: prefer the form that stays on one line after rustfmt; if only one of Line::from(vec![…]) or vec![…].into() avoids wrapping, choose that. If both wrap, pick the one with fewer wrapped lines.

### Text wrapping

- Always use textwrap::wrap to wrap plain strings.
- If you have a ratatui Line and you want to wrap it, use the helpers in tui/src/wrapping.rs, e.g. word_wrap_lines / word_wrap_line.
- If you need to indent wrapped lines, use the initial_indent / subsequent_indent options from RtOptions if you can, rather than writing custom logic.
- If you have a list of lines and you need to prefix them all with some prefix (optionally different on the first vs subsequent lines), use the `prefix_lines` helper from line_utils.

## Tests

### Snapshot tests

This repo uses snapshot tests (via `insta`), especially in `codex-rs/tui`, to validate rendered output.

**Requirement:** any change that affects user-visible UI (including adding new UI) must include
corresponding `insta` snapshot coverage (add a new snapshot test if one doesn't exist yet, or
update the existing snapshot). Review and accept snapshot updates as part of the PR so UI impact
is easy to review and future diffs stay visual.

When UI or text output changes intentionally, snapshot coverage may be needed. Run snapshot
commands only when the change requires it; do not run them by default for every checkout:

- Generate any updated snapshots:
  - `cargo test -p codex-tui`
- Check what’s pending:
  - `cargo insta pending-snapshots -p codex-tui`
- Review changes by reading the generated `*.snap.new` files directly in the repo, or preview a specific file:
  - `cargo insta show -p codex-tui path/to/file.snap.new`
- Only if you intend to accept all new snapshots in this crate, run:
  - `cargo insta accept -p codex-tui`

If the tool is missing:

- Run `cargo install --locked cargo-insta` before snapshot commands.

### Test assertions

- Tests should use pretty_assertions::assert_eq for clearer diffs. Import this at the top of the test module if it isn't already.
- Prefer deep equals comparisons whenever possible. Perform `assert_eq!()` on entire objects, rather than individual fields.
- Avoid mutating process environment in tests; prefer passing environment-derived flags or dependencies from above.

### Spawning workspace binaries in tests (Cargo vs Bazel)

- Prefer `codex_utils_cargo_bin::cargo_bin("...")` over `assert_cmd::Command::cargo_bin(...)` or `escargot` when tests need to spawn first-party binaries.
  - Under Bazel, binaries and resources may live under runfiles; use `codex_utils_cargo_bin::cargo_bin` to resolve absolute paths that remain stable after `chdir`.
- When locating fixture files or test resources under Bazel, avoid `env!("CARGO_MANIFEST_DIR")`. Prefer `codex_utils_cargo_bin::find_resource!` so paths resolve correctly under both Cargo and Bazel runfiles.

### Integration tests (core)

- Prefer the utilities in `core_test_support::responses` when writing end-to-end Codex tests.

- All `mount_sse*` helpers return a `ResponseMock`; hold onto it so you can assert against outbound `/responses` POST bodies.
- Use `ResponseMock::single_request()` when a test should only issue one POST, or `ResponseMock::requests()` to inspect every captured `ResponsesRequest`.
- `ResponsesRequest` exposes helpers (`body_json`, `input`, `function_call_output`, `custom_tool_call_output`, `call_output`, `header`, `path`, `query_param`) so assertions can target structured payloads instead of manual JSON digging.
- Build SSE payloads with the provided `ev_*` constructors and the `sse(...)`.
- Prefer `wait_for_event` over `wait_for_event_with_timeout`.
- Prefer `mount_sse_once` over `mount_sse_once_match` or `mount_sse_sequence`

- Typical pattern:

  ```rust
  let mock = responses::mount_sse_once(&server, responses::sse(vec![
      responses::ev_response_created("resp-1"),
      responses::ev_function_call(call_id, "shell", &serde_json::to_string(&args)?),
      responses::ev_completed("resp-1"),
  ])).await;

  codex.submit(Op::UserTurn { ... }).await?;

  // Assert request body if needed.
  let request = mock.single_request();
  // assert using request.function_call_output(call_id) or request.json_body() or other helpers.
  ```

## App-server API Development Best Practices

These guidelines apply to app-server protocol work in `codex-rs`, especially:

- `app-server-protocol/src/protocol/common.rs`
- `app-server-protocol/src/protocol/v2.rs`
- `app-server/README.md`

### Core Rules

- All active API development should happen in app-server v2. Do not add new API surface area to v1.
- Follow payload naming consistently:
  `*Params` for request payloads, `*Response` for responses, and `*Notification` for notifications.
- Expose RPC methods as `<resource>/<method>` and keep `<resource>` singular (for example, `thread/read`, `app/list`).
- Always expose fields as camelCase on the wire with `#[serde(rename_all = "camelCase")]` unless a tagged union or explicit compatibility requirement needs a targeted rename.
- Exception: config RPC payloads are expected to use snake_case to mirror config.toml keys (see the config read/write/list APIs in `app-server-protocol/src/protocol/v2.rs`).
- Always set `#[ts(export_to = "v2/")]` on v2 request/response/notification types so generated TypeScript lands in the correct namespace.
- Never use `#[serde(skip_serializing_if = "Option::is_none")]` for v2 API payload fields.
  Exception: client->server requests that intentionally have no params may use:
  `params: #[ts(type = "undefined")] #[serde(skip_serializing_if = "Option::is_none")] Option<()>`.
- Keep Rust and TS wire renames aligned. If a field or variant uses `#[serde(rename = "...")]`, add matching `#[ts(rename = "...")]`.
- For discriminated unions, use explicit tagging in both serializers:
  `#[serde(tag = "type", ...)]` and `#[ts(tag = "type", ...)]`.
- Prefer plain `String` IDs at the API boundary (do UUID parsing/conversion internally if needed).
- Timestamps should be integer Unix seconds (`i64`) and named `*_at` (for example, `created_at`, `updated_at`, `resets_at`).
- For experimental API surface area:
  use `#[experimental("method/or/field")]`, derive `ExperimentalApi` when field-level gating is needed, and use `inspect_params: true` in `common.rs` when only some fields of a method are experimental.

### Client->server request payloads (`*Params`)

- Every optional field must be annotated with `#[ts(optional = nullable)]`. Do not use `#[ts(optional = nullable)]` outside client->server request payloads (`*Params`).
- Optional collection fields (for example `Vec`, `HashMap`) must use `Option<...>` + `#[ts(optional = nullable)]`. Do not use `#[serde(default)]` to model optional collections, and do not use `skip_serializing_if` on v2 payload fields.
- When you want omission to mean `false` for boolean fields, use `#[serde(default, skip_serializing_if = "std::ops::Not::not")] pub field: bool` over `Option<bool>`.
- For new list methods, implement cursor pagination by default:
  request fields `pub cursor: Option<String>` and `pub limit: Option<u32>`,
  response fields `pub data: Vec<...>` and `pub next_cursor: Option<String>`.

### App-Server API Validation

- Update app-server docs/examples when API behavior changes (at minimum `app-server/README.md`).
- Regenerate schema fixtures when API shapes change if the change requires it:
  `just write-app-server-schema`
  (and `just write-app-server-schema --experimental` when experimental API fixtures are affected).
- For app-server protocol/runtime/root-worker backend startup changes, the default binary validation is
  `cargo build -p codex-app-server --bin codex-app-server`; use `cargo build -p codex-cli` only when
  the CLI/TUI entrypoint or `codex app-server` subcommand wrapper changed. Add
  `cargo test -p codex-app-server-protocol` when protocol coverage is specifically needed.
- Avoid boilerplate tests that only assert experimental field markers for individual
  request fields in `common.rs`; rely on schema generation/tests and behavioral coverage instead.
