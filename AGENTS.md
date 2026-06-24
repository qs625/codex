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
- 不要新增 Rust `unsafe` 功能（例如 `unsafe fn`、`unsafe impl` 或 `unsafe { ... }` block）来完成常规功能、
  重构或依赖拓扑优化；优先使用 safe Rust、owner crate API、trait boundary 和 composition root 注入。
  如果确实认为必须使用 unsafe，先停下来说明原因、边界和验证方式，等待明确确认。
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
- If you change `ConfigToml` or nested config types, update `codex-rs/config/config.schema.json` when needed; use `just write-config-schema` when regenerating it.
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
- 拆 `codex-core` 时优先按 1-3 万行级别的功能域、runtime boundary 或 owner API 聚合迁移，
  目标是让 session、thread/agent control、tool host、guardian/MCP 等大块能形成可并行编译的
  crate。必要时可以先整体迁移一组强耦合模块，再在新 owner crate 内部继续分层；不要为了几百行
  helper 单独创建新 crate，除非它是已经存在 owner crate 的自然扩展或会被多个 consumer 复用且不会
  引入 heavy graph。小型纯规则应优先合并到相邻 owner crate，而不是制造 crate 碎片。文件大小规则是
  方向而不是固定目标：“一两千行”只是单文件过大时的上限参考，不是每个文件都要接近这个规模；本身
  清晰的小文件不需要为了行数强行拆分，也不要为了“不要太细”把本来有自然 ownership 的小文件硬合并。
- 合理的 domain 应该能作为独立 crate 编译和测试；如果某个 domain 不能独立，通常说明它直接依赖了
  `Session`、`TurnContext`、全局 services 或 transport/runtime implementation。拆分时优先定义该
  domain 真正需要的 API/trait/DTO boundary，并由 `codex-core` 或组合根实现和注入，不要把 concrete
  core 类型作为“方便的 IoC 容器”传入 owner crate，也不要只把 direct dependency 改成 indirect
  dependency。
- 依赖倒置（dependency inversion）是 core 拆分的默认规则：当低层 domain/runtime 需要调用
  `codex-core`、app-server 或其他组合根能力时，把稳定的 trait/DTO contract 放到 owner API crate
  或 protocol-neutral API crate；core/app-server 只实现并通过 constructor/service registry 显式注入该
  contract，domain/runtime crate 只依赖 contract，不依赖 concrete implementation。不要为了实现 trait
  让 core 依赖完整 implementation crate，也不要通过 facade/re-export 把 direct dependency 伪装成
  indirect dependency；依赖门禁必须证明 contract crate 没有拉回 implementation graph。
- 新 owner crate 的依赖门禁必须同时覆盖生产代码和测试代码：除了 `cargo tree -p <crate>
  --edges normal`，还要检查 `cargo tree -p <crate> --edges normal,dev`，确保 dev-dependency、测试
  fixture、test-support helper 也没有通过 indirect dependency 拉回 `codex-core`、app-server v2、
  V8 runtime、Starlark/Rama、sqlx/state 或 concrete API/runtime implementation。测试为了方便拉回
  heavy graph 和生产代码直接拉回 heavy graph 一样需要修正。
- 收缩 `codex-core` 的同时要处理大文件问题：能直接拆到外部 owner crate 的代码不要先停在 core
  内部聚合层，应该直接迁出并把测试移到 owner crate；只有仍强绑定 `Session`、`TurnContext`、
  core service registry 或 runtime side effect、暂时无法定义稳定 trait/DTO 边界的代码，才先在
  `codex-core` 内按 domain 聚合。若暂时不能整块迁出 crate，先把超大的 orchestration/module 文件拆成
  有清晰 ownership 的子模块，并把类型文档和相关测试逐步移到 owner 文件附近；不要让 core 从“大 crate”
  退化成少数几千行大文件。
- `codex-rs/core/src` 根目录不是新的 domain 默认归宿。`thread_manager`、`codex_thread`、goal
  scheduler、MCP bridge、shell/command bridge、hook runtime、network approval 这类 root-level
  orchestration 文件要定期按整体架构复盘。移动前先判断它们是否可以直接外迁到已有或新建 owner crate；
  如果可以，直接拆出去并验证 owner crate normal / normal,dev graph；如果不能，再收敛到
  `core/src/thread/`、`core/src/session/`、`core/src/agent/`、`core/src/mcp/`、`core/src/shell/` 等
  core domain module。不要因为文件在根目录就把它当作合理的长期 facade。
- Session/thread live 操作 API 属于轻量 contract crate，而不是 `codex-core` facade：
  `codex-rs/session-api`（`codex-session-api`）承载 live session command/status trait，
  `codex-rs/thread-api`（`codex-thread-api`）承载 live thread command/status/runtime-status 和 registry
  trait。`codex-session-api` / `codex-thread-api` 不得依赖 `codex-core`、app-server、tool runtime、
  code-mode runtime、state/sqlx 或 concrete thread/session implementation；core、app-server、agent
  control、workflow bridge 和 tool host 需要操作 live session/thread 时，应逐步依赖这些 trait/DTO，
  由 core 或后续 session/thread runtime owner crate 实现并注入。后续迁 `core/src/session`、
  `core/src/tasks`、`pending_input`、`state::ActiveTurn` / `RunningTask`、`core/src/thread` 时，先扩展这些
  API 的稳定 trait/DTO，再整块迁实现；不要让新的 runtime crate 反向依赖 concrete `Codex`、
  `CodexThread` 或 `ThreadManager`。
- 不依赖 `codex-core` 内部实现的稳定 MultiAgent runtime 基础层，例如 agent registry、status helper、
  agent fork history selector（`SpawnAgentForkMode` / `select_forked_rollout_items`）、
  spawn/list control-plane DTO（`SpawnAgentOptions` / `LiveAgent` / `ListedAgent`）、agent path prefix
  matcher、spawn input preview 和 post-turn scheduler outcome（`ThreadPostTurnState` /
  `ThreadIdleReason`），应放在
  `codex-rs/agent-runtime`（`codex-agent-runtime`）。`AgentControl`、session/turn loop、绑定
  `PendingInputItem` 的 mailbox 状态、tool dispatch adapter 等偏编排的代码，在边界被明确拆分前继续留在
  `codex-core`。
- Agent role declaration runtime owner 是 `codex-rs/agent-roles`（`codex-agent-roles`）：包括
  `AgentRoleConfig` / `AgentRoleSource` / `AgentCapabilityAllowlist`、`agents` TOML layer 合并、
  `.agent.md` / `.agent.toml` discovery、Markdown frontmatter parser、role description/nickname/config
  file validation 和 plugin agent role merge。`codex-core` 只保留旧 `crate::config` re-export/facade
  以及 apply-role/session 编排；新增 agent role 解析或 discovery 规则不要重新放进
  `core/src/config/agent_roles.rs` 或 `core/src/config/agent_role_types.rs`。
- 不依赖 `codex-core` 的基础工具契约类型，例如 `ToolName` re-export、`ToolCall`、`ToolPayload`、
  `ToolOutput` / `JsonToolOutput`、`FunctionCallError`、`ToolExecutor` trait、JSON schema subset、
  Responses API tool shape 和 `ToolSpec`，属于 `codex-rs/tool-types`（`codex-tool-types`）。
  `codex-tool-types` 不得依赖 `codex-tools`、`codex-extension-api` 或 `codex-core`；extension API
  这类只需要工具契约的 crate 应依赖 `codex-tool-types`，不要为了 extension-owned tool executor
  拉入完整 tool planning crate。不依赖 `codex-core` 的工具配置和轻量模型能力 helper，例如
  `ToolsConfig`、`ToolsConfigParams`、shell backend/unified exec mode、request-user-input mode
  计算、image-detail capability/sanitization，属于 `codex-rs/tool-config`（`codex-tool-config`）。
  `codex-tool-config` 不得依赖 `codex-tools`、`codex-extension-api` 或 `codex-core`；core/session 需要
  tool config 时应直接依赖 `codex-tool-config`，不要经 `codex-tools` re-export。tool discovery 模型、
  request-plugin-install plan/helper、MCP/dynamic tool 适配、code-mode tool spec 兼容层、
  hosted model tool specs、namespace 合并、agent tool pattern 过滤、code-mode exec prompt plan 和
  不依赖 core runtime 的内建 tool spec 构造器，属于 `codex-rs/tool-planning`
  （`codex-tool-planning`）。`codex-tools` 只是兼容 facade，可以 re-export
  `codex-tool-planning` 保持旧路径兼容；`codex-core` 和新增 consumer 不应依赖
  `codex-tools`。新增基础类型应先放在 `codex-tool-types`，新增配置/capability helper 应先放在
  `codex-tool-config`。`codex-tool-planning` 解析 MCP tool spec 时应消费
  `codex-mcp-tool-types::McpTool` 这类 protocol-neutral DTO，不要直接依赖 `rmcp::model::Tool`。
  `tool_search` 的本地搜索 ranking 属于 `codex-tool-runtime` 的轻量内存实现；默认 graph 不应为了该
  handler 引入外部 BM25/search engine crate。需要扩展时优先维护 owner crate 内的小型 scorer 或先拆出
  真正可注入的 tool-search runtime，而不要把 `bm25` 这类 crate 重新作为 `codex-core` 或
  `codex-tool-runtime` direct / indirect normal dependency 拉回。
  `FunctionCallError` 是 `codex-tool-types` 的小型本地错误 contract，应手写
  `Display` / `std::error::Error`，不要为了 derive 在 `codex-tool-types` 或兼容 facade
  `codex-tool-planning` normal graph 中重新引入 `thiserror` proc-macro。
  `ToolExecutor` / `ToolExposure` 的契约 owner 是 `codex-tool-types`；`codex-tool-planning` 只
  re-export 该契约并承载 spec/planning/helper，不要在 planning crate 重新定义 executor trait 或为
  旧 trait 形状保留 `async-trait` proc-macro dependency。
  不依赖 `codex-core` 的 tool runtime API/IoC contract 属于 `codex-rs/tool-runtime-api`
  （`codex-tool-runtime-api`）：包括 hook-facing `HookToolName`、generic `ToolHandler` /
  `RegisteredTool` / `ToolArgumentDiffConsumer` / `ToolRegistryView`、pre/post hook payload、
  `AnyToolResult`、generic dispatch host/trace boundary、hook outcome DTO、`Approvable` /
  `Sandboxable` / `ToolRuntime`、`ApprovalCtx` / `ToolCtx` / `SandboxAttempt`、
  `ToolError`、network approval spec、`ToolSandboxContext`、`ToolOrchestratorHost` 和
  `OrchestratorRunResult`、`ToolEventHost`、apply_patch/shell/unified-exec request、approval key、
  environment、runtime host trait、handler host trait、command interaction host trait、
  `RunExecLikeArgs`、registry assembly host trait 和粗粒度 `ToolDomainHost` facade。新增 tool handler
  需要 core/session 能力时，先把能力抽象进 `codex-tool-runtime-api` 的 host/service contract，再由
  core 注入实现；不要让 handler implementation 直接依赖 `Session` / `TurnContext`。`codex-core` 需要实现
  tool host/dispatch/adapter trait 时应直接依赖该 API crate，不要为了 trait contract 依赖完整
  implementation crate。`codex-tool-runtime-api` 只能依赖 protocol/tool-types/planning/telemetry、
  sandboxing/permissions/hooks、已拆出的 command/process/apply-patch primitive 和 filesystem/exec-server
  API 这类 contract/DTO crate，normal/dev graph 不能拉回 `codex-tool-runtime`、`codex-core` 或 app-server。
  不依赖 `codex-core` 的 tool runtime implementation 属于 `codex-rs/tool-runtime`
  （`codex-tool-runtime`）：包括 model-visible tool output formatting、generic `ToolInvocation`
  envelope、host-neutral `ToolRouter`、generic registry container/builder、typed tool event emission
  编排、apply_patch turn diff tracker、shell/runtime helper DTO、sandbox command/env/snapshot helper，
  tool-call parallel dispatch/cancellation runtime、`ToolOrchestrator` implementation、
  approval/sandbox/retry/network lifecycle 状态机，以及 shell/apply_patch/unified-exec 这类 tool
  runtime 主体、Response item 到 tool call 后的 registry/router/dispatch 运行时、model-visible spec
  planning 后的 handler collection ordering、extension-owned tool handler adapter、tool-search/test-sync
  host-neutral handler、plan/goal/request-permissions/request-user-input/view-image/MCP-resource/agent-job 等
  已抽象为 `ToolDomainHost` capability 的 handler，以及 runtime-only output/planning DTO。request、
  approval key、host trait 和 handler host
  contract 归属 `codex-tool-runtime-api`。`codex-tool-runtime` 可以 re-export
  `codex-tool-runtime-api` 的 contract 保持旧路径兼容，但 contract owner 仍是 API crate；
  `codex-core`
  只实现 `Session` / `TurnContext` host adapter、approval/hook/Guardian bridge、trace/telemetry/goal
  bridge、process manager / stdout stream / filesystem environment 注入和真实 handler 编排。新增
  tool event、runtime helper、runtime 状态机或 turn diff 状态机不要重新放回 `core/src/tools` 或
  `core/src/turn_diff_tracker.rs`。如果 runtime 需要 core 能力，优先通过 `ToolDomainHost` /
  `CoreToolDomainHost` 这种粗粒度 service facade 暴露一组 typed capability，再由 core adapter
  注入实现；tool domain 的 assembly ownership（router/registry/spec planning、handler collection、
  dispatch host 注入）应在 `codex-tool-runtime`。core 只允许通过 coarse external phase 临时注入尚未迁出的
  code-mode、MCP tool call、workflow、多 agent 或 request-plugin-install handler；不要让 core 按
  `append_*` 为每个已迁 handler 逐项组装 registry。避免继续为每个小 handler 发散独立 core-facing trait，
  除非该 subtrait 会被 `ToolDomainHost` 组合并服务于整体 `core/src/tools` domain 迁移。不要让 owner crate direct 或
  indirect 依赖 `codex-core`。`codex-tool-runtime`
  的 normal/dev graph 都不能拉回 `codex-core`、app-server v2、V8/code-mode implementation、
  network proxy implementation、exec-server、sqlx/state、codex-api/openai-files/core-skills。
  `agent_jobs` 的 CSV 输入解析、输出 escaping、instruction template 渲染、worker prompt 构造和默认
  output path 规则属于 `codex-state-api` 的 `AgentJob` DTO owner；`codex-core` 的 agent job handler 只
  保留 tool argument validation、Session/AgentControl 编排、状态 DB mutation 和 worker lifecycle。
  默认 `codex-core` graph 不应为了 `spawn_agents_on_csv` 重新 direct 或 indirect 依赖 `csv` crate。需要
  支持更完整的 CSV dialect 时，先评估 `codex-state-api` 是否仍能保持无 tokio/sqlx/core/runtime 依赖；
  不能满足时再抽出可注入 parser，不要把通用 CSV runtime 拉回 core default graph。
  `Session`、`TurnContext`、hooks、approval、
  telemetry、真实 tool handler 执行、`dispatch_any` 和 turn loop 编排继续留在 `codex-core`，
  除非先拆出稳定共享接口。
- code-mode 的描述层、工具定义、tool namespace description、exec/wait public tool name、
  exec prompt builder、pragma parser 和 JSON schema 到 TypeScript 的渲染属于轻量 API 边界，
  owner crate 是 `codex-rs/code-mode-api`（`codex-code-mode-api`）。`codex-tools` 这类只做
  tool spec/planning 的 crate 应依赖 `codex-code-mode-api`，不得为了生成 code-mode tool
  description 或 nested tool definition 直接依赖带 V8 runtime 的 `codex-code-mode`。
  `codex-rollout-trace` 这类只需要 code-mode public tool name、runtime response DTO 或 trace
  serialization 的 crate 也应依赖 `codex-code-mode-api`，不应直接依赖 runtime implementation。
  rollout trace 的 raw event schema、payload refs、writer、thread/inference/tool/compaction trace context
  和 trace lifecycle status 属于 `codex-rs/rollout-trace-api`（`codex-rollout-trace-api`）；`codex-core`
  production graph 只能依赖该 API crate。`codex-rollout-trace` 继续拥有 `replay_bundle`、reducer、
  reduced `RolloutTrace` viewer/debug model 和相关 replay tests，并可 re-export API 类型保持兼容；
  core tests 或 CLI/replay tooling 需要 `replay_bundle` 时可以用 `codex-rollout-trace` dev-dependency，
  但不要把 reducer implementation 重新放回 core normal graph。
  `codex-code-mode-api` 还承载 `CodeModeRuntimeService`、`CodeModeRuntimeFactory`、
  `CodeModeTurnHost` 和 execute/wait request/outcome 这些 runtime trait/DTO；`codex-core`
  和后续 session runtime 只能依赖这些 trait/DTO。V8-backed `codex-code-mode` 是 runtime
  implementation crate，由 app-server/CLI/TUI 这类组合根通过 constructor injection 注入，
  不要在 core/session/工具规划 crate 中直接创建或依赖 V8 implementation。`codex-code-mode`
  可以继续 re-export API 类型以保持兼容。
- 不依赖 `codex-core` 的 command runtime primitive，例如 command output buffer、process
  state、wait/write-stdin DTO、notification filter/state、yield/token/chunk id helper，以及
  unified exec transport-level process wrapper（本地 PTY / exec-server process 的 output pump、exit
  state、stdin write、sandbox-denial detection 和 termination primitive），应放在
  `codex-rs/command-runtime`（`codex-command-runtime`）。`ExecCommandHandler`、
  `CommandWaitHandler`、`WriteStdinHandler`、approval/sandbox policy selection、process spawn request
  assembly、network approval、async watcher event emission、`Session`/`TurnContext` 编排继续留在
  `codex-core`。命令输出的 legacy encoding 智能解码
  （`bytes_to_string_smart`、`chardetng`/`encoding_rs` 依赖和相关 CP1251/CP866/Windows-1252 回归测试）
  属于 command runtime 边界；`codex-protocol::exec_output` 只保留轻量 DTO，不要为了 DTO 或
  `StreamOutput` 把编码检测依赖重新拉回 shared protocol。unified exec process id 分配应复用
  core 既有 `uuid` v4 capability 或后续 command-runtime owner helper；不要为了单个随机整数生成
  在 `codex-core` normal dependencies 中重新添加 `rand`。
- `codex-core` 中普通可热更新的 `Arc<T>` runtime snapshot（例如 hooks runtime、exec policy
  policy snapshot）优先使用标准库 `std::sync::RwLock<Arc<T>>` 或已有 owner service API；不要为了
  低频 reload/update 路径在 core normal graph 中重新引入 `arc-swap`。只有明确处在高频 lock-free
  notification hot path（例如 exec-server client notification state）并有 owner crate 归属时，才使用
  `arc-swap`。
- 轻量 shell token rendering (`shlex_join`) 和 shell executable PATH lookup 属于
  `codex-rs/shell-utils`（`codex-shell-utils`）；新的轻量 consumer 不应为了这两个 helper 依赖
  `codex-shell-command`。命令展示解析、`ParsedCommand` metadata 提取、Bash tree-sitter parser 和
  PowerShell AST parser wrapper 属于 `codex-rs/command-display`（`codex-command-display`）；
  app-server-protocol、TUI 展示或其它只需要 command action metadata 的 crate 应依赖该 display
  parser crate，不要依赖 full `codex-shell-command`。shell safety、approval canonicalization、
  PowerShell UTF-8 prefixing 和 executable probing 等执行/安全边界仍属于
  `codex-rs/shell-command`（`codex-shell-command`），该 crate 可以 re-export display parser 以兼容旧路径。
  不要为了 shell discovery 在 consumer crate 中直接依赖 `which`，也不要为了 token rendering 在
  `codex-core` normal dependencies 中重新添加 `shlex`；测试代码需要探测本机可执行文件或构造 shell
  command 时可以使用 dev-dependency。
- Unix shell escalation protocol/server/client 和 object-safe runtime traits 属于
  `codex-rs/shell-escalation`（`codex-shell-escalation`）。`EscalationPolicy` 和
  `ShellCommandExecutor` 会作为 `Arc<dyn ...>` 在 core shell runtime 中注入，因此 trait 方法应使用
  owner crate 导出的 boxed future aliases（`EscalationPolicyFuture`、`ShellCommandRunFuture`、
  `PrepareEscalatedExecFuture`），不要在 shell-escalation 或 core production graph 中重新引入
  `#[async_trait]` / `async-trait` proc-macro。core 的测试 mock 若仍需要旧外部 trait 形状，只能把
  `async-trait` 作为 dev-dependency 使用。
- OS home directory lookup 属于 `codex-rs/utils/home-dir`（`codex-utils-home-dir`）。core、
  core-skills、config loader 和其他 runtime crate 需要用户 home 目录时应调用
  `codex_utils_home_dir::home_dir` 或更高层 `find_codex_home`，不要为了局部 lookup 在 normal
  dependencies 中直接依赖 `dirs`。测试需要对比系统 home 行为时可以把 `dirs` 作为 dev-dependency。
- OS/user display-name 和当前 IANA timezone lookup 属于 `codex-rs/user-info`
  （`codex-user-info`）。`codex-core` 需要当前用户名字、first name、greeting fallback 或
  turn context timezone、当前用户 login shell path 时应调用 `codex_user_info` helper，不要为了
  prompt/realtime/turn-context/shell detection 文案或 shell fallback 直接依赖 `whoami`、
  `iana-time-zone` 或 `libc`；需要覆盖本机用户名/时区/shell 行为的测试应通过该 crate helper
  或 dev-dependency 边界处理。
- PTY/process-group OS helper 属于 `codex-rs/utils/pty`（`codex-utils-pty`）。core 需要
  pre_exec TTY detach、parent-death signal、process group kill 或 parent pid capture 时应调用
  `codex_utils_pty::process_group`，不要为了局部 process-group helper 在 core normal dependencies
  中直接依赖 `libc`。Linux seccomp signal-exit 判定应使用 core 明确的 Linux signal constant 或
  后续 sandboxing API helper，测试需要系统调用时可把 `libc` 保留为 dev-dependency。
- model input adapter 属于 `codex-rs/model-input`（`codex-model-input`）：把 `UserInput`
  转成 model-visible `ResponseInputItem`、读取本地图片、resize/encode、生成 LocalImage
  placeholder 和图片 label 序列都在该 crate。`codex-protocol` 只保留 `UserInput`、Responses API
  DTO、pre-encoded image wrapping 和 image tag helper，不要为了 `LocalImage` 文件 IO 或
  `codex-utils-image` 把图片处理栈重新拉回 protocol。core/session/compact/prompt debug 等进入模型
  上下文的路径必须显式调用 `codex_model_input::response_input_item_from_user_input`。图片解码、
  base64 image payload/data URL decode、resize/encode、内存图片尺寸读取和 `image` crate 依赖
  属于 `codex-utils-image`，`codex-core` 不要直接依赖 `image` 或 `base64` 来实现 token estimate、
  view-image、model input 或 image generation payload helper；测试可按需使用 dev-dependency。
  image 相关 LRU / content digest cache 应由 `codex-utils-image` 自己在 owner crate 内实现或持有，
  不要让 `codex-core` 为 image token estimate 直接依赖通用 cache crate。旧的
  `codex-utils-cache` workspace crate 已删除；后续不要为了单一 owner 的小型缓存重新引入它，
  除非先证明多个 owner crate 需要同一 cache abstraction 且不会把 async runtime 或 heavy graph
  拉回轻量 crate。
- model provider request client、Responses/Chat Completions transport orchestration、compact endpoint
  request builder、realtime WebRTC/WebSocket join helper、`Prompt` / `ResponseStream` 和 request
  attestation provider API 属于 `codex-rs/model-client`（`codex-model-client`）。`codex-core` 只保留
  legacy facade/re-export 和 session/turn/realtime/compact 调用 adapter；新增模型请求、realtime 或
  compact client 行为应优先放在 `codex-model-client`，不要重新扩张 `core/src/client.rs`、
  `core/src/client_common.rs` 或 `core/src/attestation.rs`。`codex-model-client` 不得依赖
  `codex-core`、app-server display protocol、code-mode runtime、exec policy/network proxy runtime；
  测试需要 provider factory 时在该 crate 的 `#[cfg(test)]` support 中实现，不能为了测试便利把
  `codex-core` 拉回 normal graph。
- 后端 realtime conversation runtime 属于 `codex-rs/realtime`（`codex-realtime`）：WebSocket/WebRTC
  sideband input loop、audio/text/handoff channel 管理、realtime event fanout input parser、默认 backend
  prompt、voice validation、API provider/header/auth helper、handoff delegation formatting，以及 realtime
  startup context 的 current-thread/recent-work/workspace-map 渲染都归该 crate。`codex-core` 只保留读取
  `Session`/`Config`/thread-store 输入、发送 `EventMsg` 和把 handoff 文本路由回 turn 的 adapter；不要把
  realtime transport loop、prompt template、ChatGPT backend rewrite、API key fallback 或 startup-context
  格式化/扫描逻辑重新放回 core。`codex-realtime` 不得依赖 `codex-core`、app-server display protocol、
  code-mode runtime、exec/sandbox runtime、MCP runtime、login runtime 或 state/sqlx runtime；只需要认证
  snapshot/env helper 时依赖 `codex-auth-types`，只需要 recent-thread DTO 时依赖 thread-store API。
- model-visible conversation history 和 context-manager 纯历史规则属于 `codex-rs/context-manager`
  （`codex-context-manager`）：`ContextManager`、history normalization、tool call/output pair 修复、
  image token byte estimate glue、user-turn boundary 判断，以及 contextual user/dev message marker
  classifier 都在该 crate。`codex-core` 只保留需要读取 `TurnContext`、构造 runtime context update 或
  调度 session/turn 的 adapter；不要把 `TurnContext`、Session、tool dispatch、app-server display
  projection 或 runtime service 反向塞进 `codex-context-manager`。新增 context fragment 时应继续实现
  `codex_context_manager::ContextualUserFragment`；如果新增的 fragment 需要被 rollback/context usage
  识别，必须同步维护 `codex-context-manager` 的 marker classifier 和测试，不能在 core 里另建一套
  字符串规则。
- context usage 分类属于 `codex-rs/context-usage`（`codex-context-usage`）：扫描 `ContextManager`
  历史、按 compact/skills/tools/user/assistant/reasoning 分类 `ThreadContextUsage`、解析显式/隐式 skill
  usage 的纯逻辑都在该 crate。`codex-core` 只负责把 runtime `TurnContext` 投影成
  `ContextUsageSkillDetection`，并把 compact summary predicate 作为函数指针传入；不要让
  `codex-context-usage` 依赖 core `TurnContext`、compact runtime、session/turn loop、app-server
  display protocol 或 concrete `codex-core-skills` loader。
- rollout history truncation 纯规则属于 `codex-rs/rollout-api`（`codex-rollout-api`）：按真实 user
  message、typed inter-agent `trigger_turn` 和 `ThreadRolledBack` marker 计算 fork/truncation 边界的
  helper 应放在该 crate。`codex-core` 只在 session/thread/agent runtime 调用这些 helper，不要在 core
  里重新维护一套 rollout truncation 或 event-mapping display projection。真实 user message predicate
  由 `codex-context-manager` 提供；不要为了这个判断把 `TurnItem` 投影、app-server display protocol、
  concrete rollout implementation 或 session runtime 反向拉入 `codex-rollout-api`。
- `codex-utils-string::find_uuids` 使用固定 ASCII UUID scanner，避免让 `codex-utils-string`
  和 `codex-core` default normal graph 为单一正则拉入 `regex-lite`。新增固定格式、小范围
  ASCII token 查找时优先使用清晰的手写 scanner；只有确实需要通用正则语义时才在 owner crate
  明确引入 regex 依赖。`codex-core` 测试可继续把 `regex-lite` 作为 dev-dependency。
- `codex-protocol` 是 shared DTO/wire/error 语义层，不应为了 runtime helper 自动转换依赖
  `tokio`、`tokio-util`、`codex-async-utils` 或 `async-trait`。需要把
  `codex_async_utils::CancelErr`、`tokio::task::JoinError` 等 runtime error 转成 `CodexErr` 时，应在
  core/app-server 等调用方显式 `map_err` 到 `CodexErr::TurnAborted`、`CodexErr::TaskJoin` 或更具体的
  domain error，避免所有 protocol consumers 间接拉入 async runtime crates。
- `codex-async-utils` 的 `OrCancelExt` 是 Future extension trait，不是 dyn runtime boundary；保持原生
  `impl Future + Send` 返回类型，不要为它重新引入 `async-trait`。normal dependency 只保留生产
  `select!` / `CancellationToken` 所需的最小 async runtime feature，测试 runtime/time feature 放在
  dev-dependency。
- 做依赖拓扑重构时不得为了移除依赖改变既有功能、兼容输入、错误分类、安全边界或用户可见行为；也不要为了
  编译图收益大量手写替代成熟三方库已经承载的 parser、diff、YAML/TOML、HTTP、archive、shell 语法、
  encoding、schema 等复杂语义。优先选择 API/type crate 拆分、optional feature gate、组合根注入、
  owner crate 下沉或 facade 收窄；只有固定格式、小范围、行为完全由本仓测试覆盖的 trivial scanner/helper
  才可以手写，并且必须在 progress 中说明为什么不是成熟库语义的替代。
- `codex-core` 里只在本地使用的小型错误类型不要为了 `#[derive(thiserror::Error)]` 重新增加
  `thiserror` normal dependency；优先手写 `Display` / `std::error::Error` / `From`。如果错误类型是共享
  API 或跨 crate contract，应把它放到对应 owner API crate，由该 crate决定是否使用 `thiserror`。该规则
  不应泛化为在已有 crate 中机械移除成熟 derive/proc-macro；如果 crate 已经有稳定 error derive 并且
  不是 core 本地小型错误，应优先保持可维护性。
- 不依赖 `codex-core` 的通用 filesystem trait 和 unsandboxed 本地文件系统实现属于
  `codex-rs/file-system`（`codex-file-system`）。插件、技能、配置、AGENTS.md 加载等只需要本地文件
  读取/metadata 的路径应依赖 `codex_file_system::ExecutorFileSystem` 和
  `codex_file_system::LOCAL_FS`，不要为了 `LOCAL_FS` 或 trait 把 `codex-exec-server` 拉入轻量 crate。
  `codex-apply-patch` 也只应依赖 `codex-file-system` 这一层；它不应为了 standalone executable 或
  sandbox context 走 `codex-exec-server` re-export。apply-patch 相关的文本 unified diff helper 也属于
  `codex-apply-patch`，因为该 crate 已拥有 patch diff engine；`codex-core` 的 turn diff 展示或 apply-patch
  delta 消费路径应复用该 helper，不要为了生产路径直接依赖 `similar`。
  `ExecutorFileSystem` 是 object-safe trait，方法应返回 owner crate 导出的 `FileSystemFuture`
  boxed future alias；不要在 `codex-file-system` 或只实现 filesystem trait 的 crate 中为了该 trait
  重新引入 `#[async_trait]` / `async-trait` proc-macro。具体本地、sandboxed、remote filesystem
  implementation 继续由 `codex-exec-server` 持有。
  `codex-exec-server` 继续拥有 sandbox-aware process/filesystem implementation、environment manager、
  JSON-RPC transport 和 remote executor runtime。
- 通用路径规范化、WSL/native workdir 规范化、symlink write-path resolution 和 config/plugin edit
  使用的 lightweight atomic write helper 属于 `codex-rs/utils/path-utils`
  （`codex-utils-path`）。该 crate 的 production graph 不得依赖 `tempfile`；临时文件 helper 应使用
  `std::fs::OpenOptions::create_new` 在目标同目录创建唯一临时文件并 rename，测试中需要临时目录时才把
  `tempfile` 保留在 dev-dependency。`codex-core` normal dependency graph 不应为了测试 fixture 或
  atomic write helper 拉入 `tempfile`。
  Windows verbatim path cleanup、`dunce::canonicalize`/`simplified` 这类平台路径实现细节应由
  `codex-utils-absolute-path` 或 `codex-utils-path` 持有；`codex-core` 生产代码应调用
  `AbsolutePathBuf::canonicalize`、`canonicalize_preserving_symlinks` 或
  `codex_utils_path::normalize_for_native_workdir`，不要重新 direct normal 依赖 `dunce`。core 测试需要
  构造平台 canonical fixture 时可以把 `dunce` 保留为 dev-dependency。
- SQLite state runtime、migrations、telemetry layer、SQLite row mapping 和 concrete `StateRuntime` 属于
  `codex-rs/state`（`codex-state`）。不依赖 SQLite/sqlx/tokio runtime 的 state 共享 API 属于
  `codex-rs/state-api`（`codex-state-api`）；例如 thread-spawn edge status、thread goal/status/update、
  goal accounting mode/outcome、agent-job/thread-metadata DTO，以及 core-facing `StateDbRuntime` /
  `ThreadStateRuntime` / `GoalStateRuntime` / `AgentJobStateRuntime` / `MemoryStateRuntime` traits
  应放在 API crate；thread goal 的 protocol DTO 转换、预算校验、external mutation DTO、
  token/wall-clock accounting snapshot、token accounting delta、goal-update model item 构造，以及
  agent-job CSV/prompt/path shaping 也应在这里统一维护，core/app-server 不要各自复制状态转换、预算规则、
  accounting primitive 或 agent-job 数据格式规则。
  `codex-state-api` 由 `codex-state` re-export 或实现以兼容旧路径。`codex-state-api` 不得依赖
  full `codex-state`、`sqlx`、`tokio` 或 `tracing-subscriber`；core/session 默认 production graph
  只允许依赖 `codex-state-api` 的 DTO 和 `Arc<dyn StateDbRuntime>` 这类 trait object，不要为了
  goal/agent-job/thread metadata 类型或 state helper 拉入 full SQLite runtime。full `codex-state`
  只应出现在 app-server、mcp-server、thread-store、rollout/log DB 这类组合根/实现 crate，或
  `codex-core` 的 tests/test-support 边界；组合根持有 concrete `StateRuntime` 时，应显式投影成
  `Arc<dyn codex_state_api::StateDbRuntime>` 再注入 core。CLI-only utilities 例如
  `codex-state-logs`、`clap` parser、`dirs` home lookup 和 colored terminal formatting 属于
  `codex-rs/state-cli`（`codex-state-cli`）；core、rollout、thread-store 或其他 runtime
  consumer 不应为了日志查看 CLI 让 `codex-state` 携带 `clap`、`dirs` 或 `owo-colors`。
- Thread persistence API 属于 `codex-rs/thread-store-api`（`codex-thread-store-api`）：
  `ThreadStore` trait、`ThreadStoreError`/`ThreadStoreResult`、thread/turn/item list/read/update
  DTO、`ThreadMetadataPatch`、`ThreadEventPersistenceMode`、`ThreadPersistenceMetadata`、
  `LiveThreadHandle`、`LiveThreadFactory` 和 `SharedLiveThread`
  应从该 crate 引用。该 crate 不得依赖 `codex-thread-store`、`codex-rollout`、`codex-state`、
  `tokio`、file-search、git helper 或 local JSONL/state-db implementation。`codex-thread-store`
  只保留 local/in-memory implementation、live writer、`DefaultLiveThreadFactory`、
  rollout/state-db integration 和旧路径
  re-export；core/app-server 中只需要 trait/DTO/error 的代码应依赖 `codex-thread-store-api`，
  `ThreadStoreError` 这类 storage-neutral 小型 shared error 应手写 `Display` /
  `std::error::Error`，不要为了 derive 在 API crate 自身重新引入 `thiserror`。
  `ThreadStore`、`LiveThreadHandle` 和 `LiveThreadFactory` 这类 object-safe trait 应使用
  `ThreadStoreFuture` boxed future alias，不要在 `codex-thread-store-api` 或 concrete store
  implementation 中为了这些 trait 重新引入 `#[async_trait]` / `async-trait` proc-macro。
  只有实际构造 `LocalThreadStore` / `InMemoryThreadStore` 或访问 implementation escape hatch 的组合根
  才依赖 full `codex-thread-store`。session/runtime code 应持有 `SharedLiveThread` 和
  `Arc<dyn LiveThreadFactory>`，不要把 concrete `codex_thread_store::LiveThread` 放进
  `SessionServices` 或其他 core runtime state。`Config` 到 concrete thread-store 的 factory 属于
  app-server、mcp-server、CLI、TUI 或 sample facade 这类组合根；`codex-core` 不应导出
  `thread_store_from_config`，prompt/debug 这类 core API 也应接收 `Arc<dyn ThreadStore>` 和
  `Arc<dyn LiveThreadFactory>` 注入。session/thread/goals 运行时需要 SQLite state DB 时，应通过
  runtime service / spawn args 显式注入 `StateDbHandle`，不要从 `ThreadStore::as_any()` downcast 到
  `LocalThreadStore` 再反查 implementation state；这样会把 concrete store 细节重新扩散进 core。
  core 内部读取 dynamic tools、标记 memory mode polluted 或为 goal 初始化补齐 thread metadata 时，应通过
  `codex-rs/core/src/state_db_bridge.rs` 的窄 bridge；这个 bridge 只能依赖 `codex-state` 这类状态 API，
  不得重新依赖 `codex_rollout::state_db` 或 rollout 文件扫描/backfill runtime。需要从 JSONL rollout
  重新解析、reconcile 或 backfill metadata 的 full rollout state DB helper 只能集中在 `codex-rollout`、
  `codex-thread-store` 或 app-server、exec、CLI、TUI、MCP server 这类组合根边界，避免把 rollout
  文件扫描/backfill runtime 散回 core。app-server、exec、CLI、TUI、MCP server、
  app-server/core 测试或其他外部 crate 需要 `StateDbHandle`、state DB init/get helper、
  rollout list/find/metadata/recorder helper 时，应直接依赖 `codex-rollout`；不要为了
  `StateDbHandle`、`init_state_db`、`RolloutRecorder`、`SessionMeta`、`read_head_for_summary`、
  `find_thread_meta_by_name_str`、`ThreadSortKey` 等 legacy facade/re-export 依赖 `codex-core`。
  rollout 路径常量和配置视图属于 `codex-rs/rollout-api`（`codex-rollout-api`）：`SESSIONS_SUBDIR`、
  `ARCHIVED_SESSIONS_SUBDIR`、`RolloutConfig` 和 `RolloutConfigView` 应从该 crate 引用；full
  `codex-rollout` 继续拥有 JSONL list/find、recorder、metadata extraction、session index 和 state-db
  reconcile runtime，并可 re-export API 类型保持兼容。只需要判断 rollout 目录、构造 rollout config 或实现
  config view 的 crate 不要依赖 full `codex-rollout`。`codex-core` production graph 不应依赖 full
  `codex-rollout`；core 只保留 `RolloutConfigView` impl、rollout truncation 这类本地 helper 和 test/dev
  直接依赖。`StateDbHandle` / `init_state_db` 这类 state DB helper 不应从 `codex-core` public facade 暴露，
  新增调用点不得继续扩散 core rollout facade。core 内部 shell snapshot cleanup 这类只需要 active rollout
  是否存在的路径，应使用 state DB path lookup 加轻量文件名 fallback，不要调用 rollout list/file-search helper。
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
  streamable HTTP，不要在 core 里直接构造 `ReqwestHttpClient`。MCP runtime placement DTO
  `McpRuntimeEnvironment` / `McpRuntimeEnvironmentParams` 属于 `codex-rs/mcp-runtime-api`
  （`codex-mcp-runtime-api`），该 crate 可以依赖 `codex-exec-server-api` 的 host-provided
  exec/http traits，但不得依赖 full `codex-mcp`、`codex-rmcp-client`、`rmcp`、login、
  model-provider 或 concrete `codex-exec-server`。`codex-mcp` 可以保留兼容 re-export，但 core
  和 app-server 这类调用方应直接依赖 `codex-mcp-runtime-api` 获取 runtime environment 类型。
  Codex Apps MCP auth 只应通过 `McpAuthHeaderProvider` /
  `StaticMcpAuthHeaderProvider` 这类 header-only provider 进入 `codex-mcp-runtime-api` 和
  `codex-mcp`；不要为了 MCP streamable HTTP header 注入让这两个 crate 依赖 full
  `codex-api-provider`、`codex-model-provider` 或 login runtime。core/app-server 等已经持有
  `CodexAuth` / `RequestAuthSnapshot` 的组合根负责把 auth state 投影成 auth headers，再传入
  MCP runtime API。
  MCP OAuth/status runtime capability，例如 `McpAuthRuntime`、`McpOAuthLoginRequest` 和
  auth status 计算入口，也属于 `codex-mcp-runtime-api`；core/session 只能通过该 trait 做 OAuth
  discovery、login 和 auth status 计算，不要直接调用 `codex-mcp` / `codex-rmcp-client` 的 OAuth
  helper。该 trait 不应返回 `rmcp` 类型，也不承载 `McpConnectionManager` 的 tool/resource 调用面；
  concrete RMCP lifecycle、provider error 判定和浏览器 login flow 继续留在 `codex-mcp` 实现 crate。
  app-server、mcp-server、CLI/TUI facade 或测试支持这类组合根需要创建 thread runtime 时，应优先用
  `ThreadManager::new_with_mcp_auth_runtime` 或更底层 constructor 显式注入
  `Arc<dyn McpAuthRuntime>` 和 `Arc<dyn McpConnectionRuntimeFactory>`；`ThreadManager::new` 仅作为
  过渡兼容/样例便捷构造器，使用 disabled MCP runtime，不要在新的组合根路径上依赖它创建 concrete
  `DefaultMcpAuthRuntime` 或 `DefaultMcpConnectionRuntimeFactory`。
  MCP tool-call runtime capability，例如 `McpToolRuntime`，也属于 `codex-mcp-runtime-api`，只覆盖
  `CallToolResult`、server metadata、memory pollution 和 sandbox-state capability 这类 protocol/JSON
  可表达的 tool-call 运行时能力。MCP connection manager 的 protocol-neutral 调用面属于
  `McpConnectionRuntime`：包括 list tools / hard refresh、resource list/read、elicitation resolve、
  startup failure/status、approval policy/profile 和 shutdown。该 trait 只能使用
  `codex-mcp-tool-types::ToolInfo`、`codex_protocol::mcp::{ListResourcesResult,
  ListResourceTemplatesResult, ReadResourceResult, PaginatedRequestParams, ReadResourceRequestParams,
  RequestId, Resource, ResourceTemplate}` 和 `codex-mcp-types::ElicitationResponse` 这类
  protocol-neutral DTO；不得让 `rmcp::model::*` 进入 runtime-api、core tool handlers 或其他轻量
  服务层。`SessionServices` 只能持有 `Box<dyn McpConnectionRuntime>`，session startup/refresh、
  connector accessible 查询和 prompt debug 必须通过 `McpConnectionRuntimeFactory` 创建或接收 runtime；
  不要在 `codex-core` 生产代码里直接构造 `codex_mcp::McpConnectionManager`。`codex-core` normal
  dependency graph 不应包含 full `codex-mcp` 或 `rmcp`；concrete `codex-mcp` 只能出现在 app-server、
  mcp-server、CLI/TUI 这类组合根，或 core 的 optional `test-support` / dev-dependency 边界。
  MCP tool approval 这类不需要 core `Session`/`TurnContext` 的规则属于 `codex-mcp-runtime`：
  custom MCP approval mode resolution、MCP request `_meta` 构造、Guardian MCP review request 构造、
  ARC monitor action/callsite helper、Codex Apps 和 custom/plugin MCP approval persistence 都应留在
  `codex-mcp-runtime`。core 只负责把 turn metadata、plugin runtime、guardian/hook/user-input/session
  memory 和 config reload 等 host 状态接入这些 owner helper，不要为了方便把 approval/persistence
  逻辑重新放回 `core/src/mcp_tool_call.rs`。
  MCP tool-call telemetry/display shaping 也属于 `codex-mcp-runtime`：metric name/tag 构造、MCP
  tool-call tracing span metadata、result span telemetry promotion、`TurnItem::McpToolCall`
  started/completed payload builder 都应保持 host-neutral。core 只负责把 `Session`/`TurnContext`
  中的 conversation id、turn id、telemetry sink 和 event emitter 传入或调用 owner helper。
- 不依赖 `codex-core` 的 filesystem permissions runtime matcher 和基础执行审批决策，例如 read-deny glob matcher、
  normalized/canonical path candidates、`ExecApprovalRequirement`、默认 exec approval requirement 计算、
  exec-policy 到 approval requirement 的纯 evaluation、shell command candidate parsing、unmatched-command
  heuristics、prefix amendment/reason derivation、intercepted exec policy evaluation 和 `globset` 相关测试，应放在
  `codex-rs/permissions-runtime`（`codex-permissions-runtime`）。`codex-protocol::permissions`
  只保留 permissions DTO、read-only reason helper 和 wire/context 可见类型，不要为了
  `ReadDenyMatcher`、approval runtime helper 或 sandbox implementation 把 `globset` 或 tool runtime
  编排重新拉回 shared protocol。`SandboxOverride`、approval cache、hook/guardian/user approval 编排和
  真实 tool runtime execution 仍留在 `codex-core`，直到对应 service boundary 被明确拆出。
- Guardian approval review 的功能域 owner 是 `codex-rs/ext/guardian`（`codex-guardian`）：
  `GuardianApprovalRequest` / network trigger / MCP annotations、approval request 到 JSON 的 shaping、
  reviewed action analytics mapping、action formatting/truncation、guardian transcript rendering、
  prompt policy template、output schema 和 assessment JSON parsing 都应放在该 crate。`codex-core`
  只保留 Session/TurnContext review routing、从 `ResponseItem`/history 收集 guardian transcript entry
  的 adapter、review session spawn/cache/reuse、circuit breaker、event emission 和用户/guardian
  approval 编排，直到这些 runtime service boundary 被进一步拆出。`codex-guardian` 不得依赖
  `codex-core`、`codex-app-server-protocol`、`codex-code-mode`、Starlark/Rama、full
  `codex-mcp`/`codex-rmcp-client`、login/model-provider、concrete exec-server 或 tokio；新增 guardian
  prompt/template 文件时应放在 guardian crate owner 目录，并按 `include_str!` 的 Bazel 规则检查。
- Sandboxing 纯 API、DTO 和权限投影 helper 属于 `codex-rs/sandboxing-api`
  （`codex-sandboxing-api`）：`SandboxType`、`SandboxablePreference`、`SandboxCommand`、
  `SandboxExecRequest`、`SandboxTransformRequest`、`SandboxTransformError`、platform sandbox
  selection、legacy `SandboxPolicy` compatibility projection 和 `policy_transforms` 应从该 crate
  引用。`codex-sandboxing` 继续拥有 `SandboxManager`、seatbelt/landlock/bwrap command
  transformation、platform executable probing 和 `which`/`libc`/sandbox script runtime；它可以
  re-export API 类型保持兼容。core/app-server/exec-server 只需要 sandbox DTO、sandbox tag、permission
  merge/intersection 或 legacy policy projection 时应直接依赖 `codex-sandboxing-api`，不要为了这些纯
  helper 拉 full platform sandbox runtime。真正需要 `SandboxManager::transform` 或 landlock/seatbelt
  command generation 的执行边界才依赖 `codex-sandboxing`。`codex-core` 的 tool/orchestrator、
  standalone exec 和 session runtime 必须通过 `codex_sandboxing_api::SandboxRuntime` /
  `SharedSandboxRuntime` 接收 sandbox transform capability；app-server、MCP server、exec CLI 或测试
  harness 这类组合根负责构造 `codex_sandboxing::SandboxManager` 并注入。不要在 core 生产代码中
  直接调用 `SandboxManager::new()`、re-export landlock/bwrap helper，或通过 core config facade 暴露
  `system_bwrap_warning`。
- 不依赖 `codex-core` 的 network proxy 纯 API/DTO，例如 network policy decision/source 等
  protocol/config/display 共享类型，应放在 `codex-rs/network-proxy-api`
  （`codex-network-proxy-api`）。`codex-protocol` 这类基础类型层不要为了共享 DTO 直接依赖
  Rama-backed `codex-network-proxy`。`NetworkProxyConfig`、`NetworkProxySettings`、
  `NetworkMode`、domain/unix socket permission DTO、`normalize_host`、`NetworkPolicyRequest`、
  `NetworkProtocol`、`NetworkDecision`、`NetworkPolicyDecider`、`NetworkProxyAuditMetadata`、
  `BlockedRequest`、`BlockedRequestObserver`、`NetworkHostPort`、`parse_network_host_port`、
  `host_and_port_from_network_addr`、`NetworkProxyConstraints`、`PartialNetworkConfig`、
  `PartialNetworkProxyConfig`、`NetworkProxyConstraintError`、`NetworkProxyRuntimeSnapshot`、
  `NetworkProxyRuntime`、`SharedNetworkProxyRuntime`、`StartedNetworkProxyRuntime`、
  `SharedStartedNetworkProxyRuntime`、`NetworkProxyStartRequest`、
  `NetworkProxyRuntimeFactory`、`SharedNetworkProxyRuntimeFactory`、
  `DisabledNetworkProxyRuntimeFactory`、`validate_policy_against_constraints`、proxy env key 常量
  和 proxy env apply helper 属于 `codex-network-proxy-api`；
  `codex-network-proxy` 只可 re-export 这些类型以兼容旧 callsite。proxy backend、Rama runtime、
  state builder、config reloader、proxy handle、host policy evaluation、读取/写入环境变量的
  process wiring 和 runtime state 继续留在 `codex-network-proxy` 或后续明确的实现 crate。
  `codex-sandboxing` 这类只需要 sandbox policy 生成输入的 crate 应消费
  `NetworkProxyRuntimeSnapshot`，不要直接依赖 Rama-backed `codex-network-proxy`；真实
  `NetworkProxy` 到 snapshot 的转换由 core/app-server 等持有 runtime handle 的边界完成。
  core/session 的 turn context、shell/unified exec、spawn、network approval、guardian review
  和 metrics 等只需要已启动 proxy 能力的执行路径应持有 `SharedNetworkProxyRuntime` 或
  `&dyn NetworkProxyRuntime`；不要把 concrete `NetworkProxy` 重新放回这些请求/上下文结构。
  core/session 的 proxy 启动和重载路径应接收 `NetworkProxyRuntimeFactory` /
  `SharedNetworkProxyRuntimeFactory`，由 app-server、CLI、TUI、mcp-server 或测试组合根注入
  concrete `DefaultNetworkProxyRuntimeFactory`；不要在 core default normal graph 中直接依赖
  Rama-backed `codex-network-proxy`。concrete `NetworkProxy`、`NetworkProxyState`、
  `ConfigReloader`、`ConfigState` 和 `NetworkProxyHandle` 只能留在 `codex-network-proxy` 实现
  crate、组合根 wiring 或 test-support/dev 测试构造中。
  `codex-network-proxy` 不应为了历史空 `Args` 兼容类型或未来二进制入口携带 `clap`；若需要
  network proxy CLI parser，应新建明确的 CLI crate，而不是把 CLI derive 放回 Rama runtime crate。
- 不依赖 `codex-core` 的 exec policy 纯策略模型，例如 `Decision`、`Evaluation`、
  `RuleMatch`、`Policy`、`PrefixRule`、network rule DTO 和 host executable lookup helper，应放在
  no-Starlark 的 `codex-rs/execpolicy-api`（`codex-execpolicy-api`）。append/amend 文件写入
  不依赖 Starlark，也属于 `codex-execpolicy-api`；Starlark parser 和 parser error display
  继续留在 `codex-execpolicy`。Starlark rules file loader 和 parser warning formatting 属于
  `codex-rs/execpolicy-loader`（`codex-execpolicy-loader`），并通过
  `codex_core::ExecPolicyLoader` trait 由 app-server、mcp-server、exec 等组合根注入；
  `codex-core` 只拥有 runtime evaluator、`ExecPolicyManager` 和 amendment update 逻辑，生产
  normal graph 不得直接依赖 `codex-execpolicy` 或 Starlark。core 单元测试若需要 parser 覆盖，
  只能走 test-only/dev-dependency helper，不要重新导出生产 parser API。`ExecPolicyCheckCommand`、`codex-execpolicy` bin 入口和
  `clap`/JSON formatting 这类命令行解析/展示逻辑属于 `codex-rs/execpolicy-cli`
  （`codex-execpolicy-cli`）；`codex-cli` 的 `execpolicy check` 子命令应依赖
  `codex-execpolicy-cli`，不要从 parser crate 重新导出 CLI command。
  `codex-config`、`codex-protocol` 等基础类型层不得为了构造或持有 policy DTO 拉入 `starlark`。
  `codex-execpolicy` 会 re-export API policy types 和 append/amend writer 以保持 callsite
  简洁；纯 `Policy` mutation/validation 返回 `codex_execpolicy_api::Error`，append/amend
  返回 `codex_execpolicy_api::AmendError`，Starlark parser/location/display 错误才使用
  `codex_execpolicy::Error`。`codex-core` 的
  session、network proxy loader、shell runtime 和 tests 中只需要 `Policy`/`Decision`/`Evaluation`
  这类策略模型或 append/amend writer 时，也应直接依赖 `codex-execpolicy-api`；只有 parser、
  parser error display 和 rules-file loading 边界继续依赖 `codex-execpolicy`，且该边界应停留在
  `codex-execpolicy-loader` 或 test-only helper。
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
  依赖 rmcp-client 的兼容 re-export。MCP elicitation reviewer 的 protocol-neutral request、trait
  和 handle 也属于 `codex-mcp-types`；request 使用 `codex_protocol::approvals::ElicitationRequest`
  与 `codex_protocol::mcp::RequestId`，不得把 `rmcp::model::CreateElicitationRequestParams` 或
  `rmcp::model::RequestId` 放进该轻量 API。`codex-mcp` runtime 负责把 RMCP server request
  转换为该 reviewer request，并可保留旧路径兼容 re-export。Codex Apps auth elicitation 的 connector auth failure
  metadata constants、`CodexAppsConnectorAuthFailure`、`CodexAppsAuthElicitation`、
  `CodexAppsAuthElicitationPlan`、auth elicitation id/payload builder 和 completed-result helper 也属于
  `codex-mcp-types`；`codex-mcp` 只做旧路径兼容 re-export，core/tool handlers 不要为了这些
  protocol-neutral payload helper 依赖 full MCP runtime。MCP permission prompt auto-approve context/helper
  也属于 `codex-mcp-types`；它可以依赖轻量 `codex-config-types::AppToolApproval` 和 protocol
  permission DTO，但不得为了这个纯 policy helper 依赖 `codex-mcp`、`rmcp` 或 MCP connection runtime。
  MCP tool approval 的纯 prompt/response 规则也属于 `codex-mcp-types`：包括
  `McpToolApprovalDecision`、`McpToolApprovalMetadata`、approval key、question id/label constants、
  prompt options、approval question builder、elicitation meta/request builder、response parser、
  approval-mode normalization、requires-approval hint rule、tool params display builder，以及
  consequential tool approval template JSON 和 template renderer。`codex-core` 继续拥有
  `maybe_request_mcp_tool_approval`、hook/guardian/ARC monitor 编排、session memory、config persistence、
  connection-manager metadata lookup 和真实 MCP tool-call execution；不要为了这些纯 approval DTO/helper
  让 `codex-mcp-types` 依赖 Session、TurnContext、guardian、config-edit、`codex-mcp` runtime、
  `rmcp` 或 app-server protocol。core 可以保留旧路径 `pub(crate) use` 兼容内部调用，但实现 owner
  只能在 `codex-mcp-types`。
  MCP tool-call 的 protocol-neutral metadata shaping 也属于 `codex-mcp-types`：包括 server origin
  host/port span fields、result `_meta["codex/telemetry"].span` allowlist/truncation、MCP app resource URI
  fallback、Codex Apps OpenAI file input param exposure plan，以及 request `_meta.threadId` 注入 helper。
  `codex-core` 只负责 Session/TurnContext wiring、MCP execution、event emission 和 tracing span record；
  不要为了这些纯 metadata helper 重新在 core 本地解析，也不要让 `codex-mcp-types` 依赖 full
  MCP runtime、`rmcp-client`、login/model-provider、app-server protocol、code-mode runtime、Starlark/Rama
  或 concrete exec-server。
  Codex Apps auth context/cache key 这类 auth-scoped 纯 DTO/helper 也属于 `codex-mcp-types`，包括
  `CodexAppsAuthContext`、`CodexAppsToolsCacheKey` 和 `codex_apps_tools_cache_key`；`codex-mcp`
  只做旧路径兼容 re-export，core/app-server 不要为了这些 DTO 或 cache key helper 依赖 full
  MCP runtime。MCP config/server view 和 plugin provenance DTO 也属于 `codex-mcp-types`：
  `McpConfig` 只表达从 root config/plugin state 投影出的 MCP runtime settings，
  `EffectiveMcpServer` 只表达配置解析后的 server view，`ToolPluginProvenance` 只表达
  connector/server 到 plugin display-name 的来源映射；`configured_mcp_servers`、
  `effective_mcp_servers`、`effective_mcp_servers_from_configured`、`with_codex_apps_mcp`、
  `host_owned_codex_apps_enabled` 和 `tool_plugin_provenance` 这类纯 server-view helper 也应
  从 `codex-mcp-types` 取得。`codex-mcp` 可以保留兼容 re-export 并在 runtime 边界从这些 DTO
  派生连接 metadata，但 `codex-core` 不应为了读取这些纯配置/来源 DTO 依赖 `codex-mcp`
  re-export。MCP OAuth scopes resolution 的纯策略也属于 `codex-mcp-types`：
  `McpOAuthScopesSource`、`ResolvedMcpOAuthScopes` 和 `resolve_oauth_scopes` 不应为了
  OAuth discovery/login runtime 依赖 `codex-mcp`。MCP OAuth login discovery 的结果 DTO
  `McpOAuthLoginConfig` / `McpOAuthLoginSupport` 和 auth status entry `McpAuthStatusEntry`
  也属于 `codex-mcp-types`；调用方可以直接匹配这些结果类型，但 `oauth_login_support`、
  `discover_supported_scopes`、`compute_auth_statuses` 和识别 provider error 的 `should_retry_without_scopes` 继续属于
  `codex-mcp` / `codex-rmcp-client` runtime 边界。MCP client elicitation support 的语义配置属于 `codex-mcp-types`：
  `McpClientElicitationSupport` 表达 disabled/auth-elicitation 这类 host capability intent；
  `rmcp::model::ElicitationCapability` 的 wire DTO 构造只应发生在 `codex-mcp` runtime protocol
  boundary，`codex-core`/config 不应直接构造或传递 rmcp elicitation capability 类型。
  MCP `ToolInfo`、protocol-neutral `McpTool` / `ToolAnnotations` metadata、
  OpenAI file-param meta 解析和 model-visible input schema masking 属于 `codex-rs/mcp-tool-types`
  （`codex-mcp-tool-types`）。该 crate 不得依赖 `rmcp`；`rmcp::model::Tool` 到 `McpTool` 的转换只应发生在
  `codex-mcp` runtime protocol boundary，例如 RMCP list-tools 返回后进入 connection manager cache 之前。
  `codex-core` 只需要 MCP tool metadata 时应直接依赖 `codex-mcp-tool-types`，不要经
  `codex-mcp` re-export；`codex-mcp-tool-types` 不得间接拉回 full
  `codex-mcp` runtime、`codex-rmcp-client`、login、model-provider 或 exec-server。MCP
  tool result 的 model-visible image content 清洗和 event/display payload 截断也属于
  `codex-mcp-tool-types`；调用方应传入自身的 max-bytes 展示策略，不要让该 crate 依赖 pty、
  command runtime、app-server display protocol 或 concrete MCP runtime。MCP
  resource list/read DTO，例如 `PaginatedRequestParams`、`ReadResourceRequestParams`、
  `ListResourcesResult`、`ListResourceTemplatesResult`、`ReadResourceResult`、`Resource`、
  `ResourceTemplate` 和 `ResourceContent` 属于 `codex_protocol::mcp`；core/session 和 tool
  handlers 不应直接使用 `rmcp::model::*` resource DTO。`codex-mcp` runtime boundary 负责把
  RMCP list/read response 转成这些 protocol-neutral DTO，再返回给 core 或 app-server status snapshot。
  MCP request id 和 elicitation resolve 边界也使用 `codex_protocol::mcp::RequestId`；
  `codex-core`、session turn state 和 tool handlers 不得直接构造、存储或匹配
  `rmcp::model::RequestId` / `NumberOrString`。RMCP request id 到 protocol request id 的转换只应发生在
  `codex-mcp` runtime callback/connection manager boundary。MCP telemetry 只需要从 server origin
  记录 span 的 host/port 时，core 应使用已有 `http::Uri` 或 protocol-neutral helper，不要为了该场景在
  `codex-core` default normal graph 中重新引入 `url`；完整 URL 解析/构造能力应归属于具体 provider、
  shell-command 或 MCP runtime owner crate。
  MCP OAuth
  login/browser callback flow 属于 MCP runtime boundary；core 中安装 skill MCP dependencies 等路径
  应通过 `codex-mcp` 暴露的 runtime entry 调用，不要直接依赖 `codex-rmcp-client`。Codex Apps
  host-owned MCP server 需要的 auth-scoped 数据应作为 `CodexAppsAuthContext` 这类调用方预先投影的
  snapshot 传入 `codex-mcp`，包括是否使用 Codex backend、account id、ChatGPT user id 和 workspace
  标记；runtime auth provider 也应由调用方从 `codex_auth_types::RequestAuthSnapshot` 预投影成
  header-only `SharedMcpAuthHeaderProvider` 后传入 MCP manager / `codex-rmcp-client` streamable HTTP
  adapter，不要把完整 `codex-api-provider::AuthProvider`、`CodexAuth` 或 `AuthManager` 穿透到
  MCP manager 构造和刷新 helper 中；`codex-mcp` 不应为了这些缓存 key / runtime-auth availability
  判断直接依赖 `codex-login`。
- Plugin policy/interface/display metadata 这类插件领域 DTO 属于 `codex-rs/plugin-types`
  （`codex-plugin-types`）。`codex-app-server-protocol` 可以 re-export 用于 wire/schema 兼容；
  `codex-core-plugins`、TUI 或其他插件领域消费者应直接依赖 plugin types，不要为了
  `PluginInstallPolicy`、`PluginAuthPolicy`、`PluginAvailability`、`PluginInterface` 或
  `SkillInterface` 依赖 app-server-protocol。`PluginId`、`AppConnectorId`、
  `PluginCapabilitySummary`、`PluginTelemetryMetadata`、`PluginHookSource`、`PluginSkillRoot`、
  `LoadedPlugin`、`PluginLoadOutcome`、`EffectiveSkillRoots`、`prompt_safe_plugin_description`、
  `TOOL_MENTION_SIGIL` 和 `PLUGIN_TEXT_MENTION_SIGIL` 也属于 `codex-plugin-types`；
  `codex-plugin` 只作为旧路径兼容 facade re-export 这些类型，轻量 API crate 不要为了插件
  id、telemetry metadata、capability summary、hook source、skill root 或 load outcome 依赖
  `codex-plugin`。
  `codex-plugin` 不应依赖 `codex-utils-plugins`；`codex-utils-plugins` 只保留旧路径 re-export 和
  MCP connector/plugin utility helper，不能作为纯 DTO 或 mention syntax 的新增依赖入口。`codex-hooks` 和
  `codex-mcp` 这类只消费 hook/capability/provenance DTO 的 crate 应直接依赖 `codex-plugin-types`；
  真实 plugin loader/manager 可以继续通过 `codex-plugin` 兼容 facade 引用这些类型，但新的 API/trait
  crate 应直接依赖 `codex-plugin-types`。
- plugin manifest path discovery 和 skill path 到 plugin namespace 的 filesystem helper 属于
  `codex-rs/plugin-manifest`（`codex-plugin-manifest`）。`codex-core-skills` 这类只需要为 skill 解析
  plugin namespace 的 crate 应依赖 `codex-plugin-manifest`，不要为了 `plugin_namespace_for_skill_path`
  拉入 `codex-utils-plugins`、MCP connector helper 或 default-client API。`codex-utils-plugins` 可以
  re-export 这些 helper 保持旧路径兼容，但 `codex-core` default normal graph 不应经 plugin/skill
  mention 或 namespace helper 间接回到 `codex-utils-plugins`。
- `codex-core-plugins` 和 `codex-core-skills` 不应直接依赖 `codex-analytics`。插件生命周期 analytics
  通过 `PluginAnalyticsEventSink` 这类窄 trait 从组合根注入；skill injection 应返回领域 invocation
  数据，由 core/app-server 这类已经拥有 analytics client 的边界转换并上报。不要为了打点把
  app-server protocol 事件 reducer 或 analytics client queue 拉回 plugin/skill core crate。
- 本地 plugin manager/cache/marketplace/loading 属于 `codex-rs/core-plugins`
  （`codex-core-plugins`）；remote installed plugin 的纯状态 DTO 可以留在这里供 loader 投影。
  不依赖真实 plugin manager/cache/filesystem/runtime 的轻量 API 属于 `codex-rs/core-plugins-api`
  （`codex-core-plugins-api`），包括 plugin 专用 `PluginConfigLayerStack` /
  `PluginConfigLayerEntry` view、`PluginsConfigInput`、marketplace name 常量和 tool-suggest
  discoverable allowlist，以及 `PluginRuntime` / `SharedPluginRuntime`、`PluginLoadOutcome` 和
  `is_configured_plugin_installed` 这类 core session/MCP 只读 runtime 能力。`codex-core`、
  `codex-chatgpt` 或其他只需要这些纯输入/常量/只读 runtime 能力的路径应直接依赖 API crate，不要为了
  config view、allowlist、enabled plugin load outcome 或 plugin installed bool 拉入 full
  `codex-core-plugins`。`config-test-support` feature 只允许测试/dev graph 用于从完整
  `codex_config::ConfigLayerStack` 做兼容转换，默认 normal graph 不得启用。`codex-core-plugins-api`
  应直接依赖 `codex-plugin-types` 获取 plugin load outcome 类型，其 default graph 不得经 `codex-plugin` 或
  `codex-utils-plugins` 间接拉入 `codex-file-system`、`tokio`、full `codex-config`、
  `codex-git-info`、`codex-git-utils` 或 full `codex-core-plugins`。
  full `codex-core-plugins` 继续拥有 `PluginsManager`、plugin store/loader、marketplace install/read/list/
  upgrade、local filesystem/cache 和 analytics sink 注入。core production 路径需要插件能力时只能通过
  `SharedPluginRuntime` / `PluginRuntime` constructor injection 传入；`ThreadManager`、`Session`、
  `McpManager` 和 `Config::to_mcp_config` 不得持有或暴露 concrete `PluginsManager`，也不要恢复
  `ThreadManager::plugins_manager()` 这类管理入口。app-server、mcp-server、TUI 或其他组合根/客户端边界
  可以自己持有 full `PluginsManager` 来处理 install/list/read/marketplace management API，并把同一个
  manager clone 成 `SharedPluginRuntime` 注入 core。
  remote plugin catalog/share/install/uninstall/sync、remote bundle 下载/校验、curated repo startup sync 和
  workspace share checkout 这类需要 HTTP、archive 或 full default-client 的 runtime 属于
  `codex-rs/core-plugins-remote`（`codex-core-plugins-remote`）。`codex-core-plugins` production
  graph 不得为了 remote plugin runtime 拉入 `codex-default-client`、`reqwest`、`flate2`、`tar`、`zip`、
  `url` 或 `codex-login`；`codex-core` 也不得依赖 `codex-core-plugins-remote`。
- remote plugin HTTP API 需要的 auth 应表示为 `codex_core_plugins_remote::RemotePluginAuth`，只承载
  `RequestAuthSnapshot`、account id、ChatGPT user id 和 workspace 标记；startup/background sync 需要
  当前 auth 时，通过 `RemotePluginAuthProvider` 由 app-server 这类 login-aware 组合根注入。组合根负责从
  `CodexAuth` / `AuthManager` 投影这些轻量 DTO，并显式依赖 `codex-core-plugins-remote`；不要把
  `CodexAuth`、`AuthManager`、`codex_login::default_client` 或 remote HTTP implementation 穿透回
  `codex-core-plugins` / `codex-core`。
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
  `StatsigMetricsSettings` 跨进程 DTO、`RuntimeMetricTotals` / `RuntimeMetricsSummary`
  runtime metrics display DTO、全局 counter/histogram/duration helper 和 lightweight duration
  timer，不得依赖
  `codex-otel`、`codex-api`、HTTP/SSE/WebSocket runtime、tokio runtime 或 app-server protocol。真实
  `codex-otel` 可以 re-export metrics API 的纯类型，在初始化时把 concrete `MetricsClient` 安装到
  metrics API facade；
  `codex-mcp`、agent runtime、
  core-plugins/core-skills、rollout 或其他 runtime-adjacent crate 不应为了 global counter/duration、
  skill metrics、DB telemetry、metric name 常量或纯 tag/source enum 直接依赖 `codex-otel`。
  `codex-core` 需要记录 best-effort 全局 counter、histogram 或 duration 时，应使用
  `codex_metrics_api::record_global_*` / `start_global_timer`；不要调用 `codex_otel::global()`
  获取 concrete metrics client。
  metrics-only helper 的函数签名应接收 `&dyn codex_metrics_api::MetricsSink` 或对应 facade 类型，
  不要把 helper 绑定到 `codex_otel::SessionTelemetry`；调用方可以继续传入 session telemetry 以保留
  per-session metadata tags，但 helper 本身必须停留在 metrics API 边界。
  UI/runtime display 只需要展示 runtime metrics totals/summary 时应直接依赖
  `codex-metrics-api`；`codex-otel` 可以在 snapshot 边界把 OpenTelemetry SDK data 转成该 DTO 并
  re-export 旧路径兼容，但下游不要为了 DTO 依赖 full OTEL runtime。
  如果 core 只需要持有一个 duration timer 直到 drop 记录指标，应隐藏为 boxed drop guard，避免在
  session/turn state 类型中公开 `codex_otel::Timer` 这类 concrete OTEL runtime 类型。
  只需要 metric tag sanitization 时，直接使用 `codex-utils-string::sanitize_metric_tag_value`，
  不要通过 `codex_otel::sanitize_metric_tag_value` re-export 形成语义上的 runtime dependency。
- session-scoped telemetry facade 属于 `codex-rs/session-telemetry-api`
  （`codex-session-telemetry-api`）：`SharedSessionTelemetry`、`SharedSessionTelemetryFactory`、
  `SessionTelemetryCreateParams`、boxed timer handle 和 `log_tool_result_with_tags` 这类
  session/runtime 需要的 API 都应放在该轻量 crate。`codex-core`、session/thread/turn 编排、
  model client、task/tool registry 只能依赖该 facade 和 `codex-metrics-api`；不得为了
  conversation start、tool result、runtime metrics summary、SSE/WebSocket telemetry 或 per-session
  timer 直接依赖 `codex-otel`。
  `codex-otel` 负责实现 `SessionTelemetry`、持有 OTEL SDK/exporter/resource/snapshot 细节，并提供
  `OtelSessionTelemetryFactory`；app-server、CLI/TUI、mcp-server、test-support 等组合根通过 factory
  注入 concrete implementation。facade crate 不要反向包含 provider init、resource metrics snapshot、
  OpenTelemetry SDK 类型或 HTTP client runtime；测试需要断言 concrete OTEL snapshot 时可以在
  dev-dependency/test-support 边界直接使用 `codex-otel`。
- W3C trace propagation helper 属于 `codex-rs/trace-context`（`codex-trace-context`）：
  current span trace id、W3C trace carrier、traceparent/tracestate validation、从环境变量恢复
  trace context、给 span 设置 parent context 这类 helper 应直接从该轻量 crate 引用。`codex-otel`
  可以 re-export 这些 helper 兼容旧路径，并在 provider init 中设置 tracestate；但 core/session/runtime、
  API client、测试或其他非 OTEL runtime crate 不应为了 trace helper 依赖 full `codex-otel`。
  session telemetry facade 已由 `codex-session-telemetry-api` 承载，不要把 W3C trace helper 或
  telemetry facade 任一路径重新合并回 full `codex-otel` direct edge。
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
  Windows deny-read glob/path 解析属于纯 sandbox policy helper，应由 `codex-sandboxing-api`
  提供；`codex-windows-sandbox` 可以 re-export 旧路径，但 `codex-core` 的默认 non-Windows
  normal graph 不得为了该 resolver 依赖 Windows sandbox runtime，Windows-only spawn/setup 调用应放在
  target-specific dependency 边界。
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
- Responses API 请求 shape、stream event DTO、API error 语义和 websocket 请求 metadata helper 属于 `codex-rs/api-types`
  （`codex-api-types`）：`Reasoning`、`TextControls`、`OpenAiVerbosity`、`ResponsesApiRequest`、
  `ResponsesOptions`、`ResponseCreateWsRequest`、`ResponsesWsRequest`、`ResponseEvent`、`Compression`、`ChatCompletionsPath`、
  `ResponseStream`、`ApiError`、websocket request metadata key、`X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER` 和
  `CompactionInput`、`MemorySummarizeInput`、`MemorySummarizeOutput`、`RawMemory`、`RawMemoryMetadata`、
  `RealtimeCallResponse`、`ResponsesWebsocketClose`、`ResponsesWebsocketProbe`、ARC monitor typed request/result
  DTO（`ArcMonitorRequest`、`ArcMonitorResult`、metadata/policy/message/evidence/risk/outcome types）和
  history-to-ARC-message shaping helper、
  `create_text_param_for_request` / `response_create_client_metadata` 这类纯 DTO/helper 应从该 crate 引用；
  `ResponseDebugContext`、`extract_response_debug_context`、
  `extract_response_debug_context_from_api_error`、`telemetry_transport_error_message` 和
  `telemetry_api_error_message` 也属于该 crate，因为它们只把 transport/API error 归纳成 response
  debug context / telemetry-safe message，不需要 full API transport runtime。
  Realtime session selection DTO（`RealtimeEventParser`、`RealtimeSessionMode`、
  `RealtimeSessionConfig`）也属于 `codex-api-types`；Realtime audio/event payload
  （`RealtimeAudioFrame`、`RealtimeEvent`）属于 `codex-protocol`，不要通过 full `codex-api`
  re-export 在 core/session runtime 中使用这些纯类型。Realtime audio frame 的 transport-neutral
  base64/sample helper 属于 `codex-api-types`；`codex-core` 不要为了 audio frame duration/sample
  估算直接依赖 `base64`。
  API error bridge `map_api_error`、rate-limit header parser 和 rate-limit event parser
  也属于 `codex-api-types`；core/model-provider 需要把 `ApiError` 映射成 protocol error 或解析
  rate-limit snapshot 时应直接依赖 `codex-api-types`，不要为了这些纯 helper 通过 full `codex-api`
  re-export。`codex-api` 只保留旧路径兼容并继续拥有真实 endpoint client/runtime。
  `ApiError` 是该类型层的本地错误 contract；保持手写 `Display` / `std::error::Error` / `From`，
  不要为了 derive 在 `codex-api-types` normal graph 中重新引入 `thiserror` proc-macro。
  SSE/WebSocket telemetry 对外也只能暴露 `SseEventTelemetry` / `WebsocketEventTelemetry` 这类
  transport-neutral summary DTO，以及 `SseTelemetry` / `WebsocketTelemetry` 这类不绑定 transport parser 的 sink
  trait；`eventsource_stream::Event`、`EventStreamError` 和
  `tokio_tungstenite::tungstenite::Message/Error` 的分类归纳属于 `codex-api` runtime 边界，不要让
  `codex-core` 或 `codex-otel` 为 telemetry 实现直接依赖这些 transport parser/runtime 类型。
  `ResponseStream` 是 transport-neutral stream wrapper，只能暴露 `futures::Stream` 和
  upstream request id；具体 `tokio::sync::mpsc` receiver、SSE/WebSocket parser 和 endpoint task
  spawning 留在 `codex-api` runtime adapter 中，不要把 channel 字段或 tokio runtime 类型放进
  `codex-api-types` public API。
  `ResponsesWebsocketConnectionRuntime`、`ResponsesWebsocketConnectRequest` 和
  `ResponsesWebsocketConnectorRuntime` 这类 WebSocket connection consumer/opening trait 也属于
  `codex-api-types`，只能暴露 typed request、transport-neutral `ResponseStream`、boxed `Send` future、
  `HeaderMap`、`ApiError` 和 telemetry sink trait；不要把 tungstenite message/error、tokio channel、
  background task handle 或 concrete `ResponsesWebsocketClient` / `ResponsesWebsocketConnection` 放进该
  trait 边界。真实 WebSocket URL construction、auth header application、custom-CA/TLS connect、
  request serialization、pump task 和 connection lifecycle owner 继续属于 `codex-api`，由后续
  runtime/factory 注入到 core。
  Realtime WebSocket 的 `RealtimeWebsocketClientRuntime`、connection、writer、events trait 和
  connect/sideband request DTO 也属于 `codex-api-types`；trait 只能暴露 typed realtime session
  config、headers、`RealtimeAudioFrame`/`RealtimeEvent`、boxed `Send` future 和 `ApiError`。
  concrete `RealtimeWebsocketClient`、writer/events channel、tungstenite message/error、retry loop、
  custom-CA/TLS connect、session.update encoding 和 pump task 继续属于 `codex-api` runtime。core
  的 realtime conversation 只能通过 `SharedApiRuntimeFactory` / `ModelClient` 获取 runtime trait
  object，不要直接依赖或构造 concrete realtime websocket client/writer/events。
  API runtime factory trait 属于 `codex-rs/api-runtime-api`（`codex-api-runtime-api`）：
  `ApiRuntimeFactory` / `SharedApiRuntimeFactory` 只负责从 provider/auth 生成 endpoint runtime trait
  object，目前包括 `ResponsesWebsocketConnectorRuntime`、`CompactClientRuntime`、
  `MemoriesClientRuntime`、`ChatCompletionsClientRuntime`、`RealtimeCallClientRuntime` 和
  `RealtimeWebsocketClientRuntime`、`ResponsesClientRuntime`、`ArcMonitorClientRuntime`。
  compact/memories/realtime call 这类 unary endpoint 和 Chat Completions-compatible streaming endpoint
  通过 `codex-api-types` 中的 typed runtime request 传入 payload、headers、path、session config 和
  request telemetry；Responses HTTP/SSE streaming 通过 `ResponsesStreamRuntimeRequest` 传入
  `ResponsesApiRequest`、`ResponsesOptions`、request telemetry 和 SSE telemetry。core 不应直接构造
  `CompactClient`、`MemoriesClient`、`ChatCompletionsClient`、`RealtimeCallClient`、
  `RealtimeWebsocketClient`、`ResponsesClient` 或 `ReqwestTransport` 来调用这些 endpoint。该 crate 只能依赖
  `codex-api-provider` 和 `codex-api-types`，不得依赖 `codex-api`、`codex-client`、reqwest、tokio、
  SSE/WebSocket parser、tungstenite、MCP runtime 或 app-server protocol。`codex-api` 提供
  `DefaultApiRuntimeFactory` concrete implementation 并拥有真实 HTTP/SSE/WebSocket transport、
  default reqwest/custom-CA setup、endpoint serialization、request adaptation 和 response parsing；
  core/session runtime、ThreadManager 和 subagent spawn 路径应接收 `SharedApiRuntimeFactory` 注入，
  不要直接构造 concrete WebSocket connector、unary endpoint client 或后续 endpoint runtime。
  `codex-api` 只 re-export 这些类型用于旧路径兼容，并继续拥有 API client、auth header adapter、
  HTTP transport、SSE/WebSocket parser 和 endpoint runtime；
  core/session runtime 不应为了构造 request body、text controls 或匹配 response stream event 依赖完整
  `codex-api`，也不应为了 `ApiError` 或 API error debug/telemetry helper 依赖完整 `codex-api`。
  ARC monitor 的 request/result DTO 和 history-to-message shaping 属于 `codex-api-types`；core 只保留
  session/env token/auth snapshot headers、endpoint 选择、runtime 调用和 outcome 映射 adapter，不要把
  ARC monitor DTO/shaping 重新放回 `codex-core`。实际 HTTP POST、timeout、reqwest/custom-CA setup 必须通过
  `ArcMonitorClientRuntime` 由 concrete `codex-api` runtime 执行；不要在 core/session/tool runtime 内直接调用
  `codex_default_client::build_reqwest_client`。
- OpenAI file upload API 边界属于 `codex-rs/openai-files-api`（`codex-openai-files-api`）：
  `UploadedOpenAiFile`、`OpenAiFileUploader`、`SharedOpenAiFileUploader` 和 disabled uploader 从该轻量
  crate 引用；文件上传只需要 header-only `OpenAiFileUploadAuth`，该 API crate 不得依赖完整
  `codex-api-provider::AuthProvider` / request-signing contract，也不得拉 `reqwest`、`codex-client`、
  `codex-api`、`codex-core`、MCP runtime 或 app-server protocol。真实上传 runtime 属于
  `codex-rs/openai-files`（`codex-openai-files`）：`upload_local_file`、`OpenAiFileError`、
  `openai_file_uri`、文件上传限制常量和 `ReqwestOpenAiFileUploader` 实现从该 crate 引用，该 crate 可以拥有
  上传所需的 `reqwest` / `codex-client` custom CA runtime，但不得依赖 full `codex-api`、`codex-core`、
  `codex-otel`、MCP runtime 或 app-server protocol。`codex-core` 只应持有
  `Arc<dyn OpenAiFileUploader>`，由 app-server/mcp-server/test-support 组合根通过 constructor injection
  注入真实实现；不要让 core production manifest 直接依赖 `codex-openai-files`，也不要让 `codex-api` 为旧路径
  兼容 re-export 文件上传 helper，否则 core 只要依赖 API client 就会间接拉回文件上传 runtime。上传执行路径
  需要 auth 时应消费 login-aware 边界预投影出的 `codex_auth_types::RequestAuthSnapshot`，或从已注入
  `SharedAuthRuntime` 读取 request snapshot，不要直接调用 `AuthManager::auth()` 后再重新投影；再调用
  `codex_api_auth::auth_provider_from_auth_snapshot` 生成 request auth；不要把 `CodexAuth` 或
  `AuthManager` 继续穿透到 OpenAI file upload argument rewrite helper 内部。
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
  `codex-otel -> codex-api` 会把 full API runtime 间接拉回 core 和 windows sandbox。需要 API error
  语义时应依赖 `codex-api-types::ApiError`；只有 API client/adapter 层应构造或细分匹配它。
  `SessionTelemetry` 不应暴露
  `reqwest::Response/Error` 绑定的 helper；API request telemetry 应由 API/client adapter 归纳成
  transport-neutral fields 后调用 `record_api_request`。
- HTTP client request/error/retry 基础类型属于 `codex-rs/client-types`（`codex-client-types`）：
  `Request`、`RequestBody`、`RequestCompression`、`PreparedRequestBody`、`Response`、`TransportError`、
  `StreamError`、`RetryPolicy`、`RetryOn` 和 `RequestTelemetry` 应从该 crate 作为轻量类型层复用；`codex-client` 只 re-export
  这些类型并继续拥有 reqwest transport、SSE stream、custom CA、retry executor、request telemetry 和
  default client runtime。`codex-client-types` 默认 normal graph 不应拉 `zstd`、reqwest、
  eventsource/tungstenite 或 concrete transport runtime；JSON body serialization 可留在类型层，
  但 zstd body compression implementation 只能通过显式 `body-compression` feature 或 concrete
  `codex-client` runtime 启用。需要为 SigV4 等 auth 签名最终 body bytes 的 implementation crate
  必须显式依赖启用了该 feature 的 owner/runtime 边界，不要让 core default graph 为压缩实现付费。
  新 crate 不应为了构造或签名 request body、实现 request telemetry callback 依赖完整
  `codex-client`。作为共享轻量类型层，`TransportError` / `StreamError` 这类小型本地错误 enum 应手写
  `Display` / `std::error::Error`，不要为了 derive 重新引入 `thiserror` proc-macro 依赖。
- `codex-rs/response-debug-context`（`codex-response-debug-context`）只是旧路径兼容 facade，re-export
  `codex-api-types` 中的 response debug context helper；新增代码不要依赖它。`codex-core`、
  `codex-model-provider`、`codex-api` 和其他已经依赖 `codex-api-types` 的 crate 需要解析
  `TransportError` debug headers、生成 telemetry-safe transport/API error message 或提取
  `ApiError` debug context 时，应直接依赖 `codex-api-types`。该 facade 不得重新拥有实现，也不得被
  core default normal graph 间接拉回。
- API provider/auth 基础边界属于 `codex-rs/api-provider`（`codex-api-provider`）：`Provider`、
  `RetryConfig`、`AuthProvider`、`SharedAuthProvider`、`AuthProviderFuture`、`AuthError`、
  `AuthHeaderTelemetry`、`auth_header_telemetry`、session header helper 和 Azure endpoint detection 应从该
  crate 引用。`codex-api` 只 re-export 这些类型用于旧路径兼容，并继续拥有具体 endpoint client、
  HTTP/SSE/WebSocket runtime 和 file upload，并 re-export `codex-api-types::ResponseStream` / `ApiError`
  用于旧路径兼容；`model-provider-api`、core 和
  model-provider implementation 不应为了 provider config 或 auth-header adapter 依赖完整 `codex-api`。
  `codex-api-provider` 默认 graph 不应为 concrete WebSocket URL parsing 拉 `url`；`Provider` 可提供
  string-level URL/path construction，`Url::parse`、http/https 到 ws/wss scheme conversion 和
  tungstenite/reqwest-specific request setup 属于 `codex-api` concrete runtime。`SessionSource` /
  `SubAgentSource` 到 `x-openai-subagent` 这类 endpoint header 的映射也属于 `codex-api` request
  adapter；不要为了 header adapter 让 `codex-api-provider` 依赖 `codex-protocol`。`AuthError` 这类小型
  shared auth contract error 应手写 `Display` / `std::error::Error`，不要让 provider/auth 基础层为了
  derive 拉入 `thiserror`。
- `codex-protocol::items` 中 hook prompt 使用的 `<hook_prompt hook_run_id="...">...</hook_prompt>` 是受控
  internal marker，不应为了这一处 marker 重新引入通用 XML serde/parser 依赖。修改该 marker 时保持手写
  XML entity escape/unescape 的受控实现，并用 hook prompt roundtrip/legacy parse 测试覆盖；需要通用 XML
  解析时应先证明这是新的协议边界，而不是把 quick-xml 拉回 shared protocol。
- `codex-protocol` / `codex-app-server-protocol` 中少量特殊 wire serde 应优先使用本地受控 helper，
  而不是为了单个字段重新引入 `serde_with` / proc-macro。`ConversationStartParams.prompt` 和
  app-server v2 patch params 的 missing/null/value 三态使用本地 double-option helper；
  `ExecCommandOutputDeltaEvent.chunk` 使用本地 base64 bytes helper 和直接 `base64` crate。新增
  isolated base64、double-option 或 marker serde 时，先扩展这些本地 helper 并补 roundtrip 测试，
  不要把 `serde_with` 拉回 protocol/default core/app-server graph。
- `codex-app-server-protocol` 中只包装 wire/display message 的本地简单 error DTO，例如
  `TurnError { message, ... }`，应手写 `Display` / `std::error::Error`，不要为了单字段
  `#[derive(thiserror::Error)]` 重新引入 direct `thiserror` proc-macro。复杂 error 分类、source chain
  或跨 crate error contract 应放在真正 owner API crate 中，由该 crate 决定是否使用 `thiserror`；做
  依赖图验证时要区分 direct edge 收缩和经 `codex-protocol` / `ts-rs` / `rmcp` 等既有 owner path 的
  间接路径，不能把前者记录成全图移除。
- `codex-app-server-protocol` 的 schema/export CLI 依赖属于 bin-only 边界。`clap` 只能作为 optional
  `cli` feature dependency，被 `src/bin/export.rs` 和 `src/bin/write_schema_fixtures.rs` 通过
  `required-features = ["cli"]` 使用；运行时或库消费者依赖 app-server-protocol 时不应默认拉入
  `clap`。更新 schema 相关 Justfile/脚本时要显式传 `--features cli`，不要把 CLI dependency 重新放回
  default normal graph。
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
  这类认证环境 telemetry DTO / tag enum 也属于 `codex-auth-types`；`AuthEnvTelemetryInput` 和
  `collect_auth_env_telemetry` 也归属该 crate。core、model-provider、feedback 或其他调用方只需要
  auth env telemetry 时，应从 full provider/auth state 投影出轻量 input 后调用 `codex-auth-types`，
  不要为了该 helper 依赖 `codex-login`。`codex-login` 只保留旧路径兼容 wrapper；不要为了 DTO、
  helper 或 tag enum 依赖 `codex-otel`，否则会把 `codex-api`/HTTP runtime 经 telemetry 栈间接拉回
  login/model-provider-api。`AuthManagerConfig` 这类 login-aware runtime constructor 需要的轻量配置
  trait 也属于 `codex-auth-types`；core、ChatGPT connector 或其他只需要实现该 trait 的 crate 应直接
  依赖 `codex-auth-types`，不要为了 trait import 依赖 `codex-login`。`codex-login` 只保留兼容
  re-export 并继续拥有 `AuthManager` / `CodexAuth` / token storage/refresh runtime。`OPENAI_API_KEY` /
  `CODEX_API_KEY` 这类 auth env 常量和 non-empty `read_openai_api_key_from_env` helper 也属于
  `codex-auth-types`；core realtime、tests 或其他只需要 env 常量/helper 的路径不要通过
  `codex-login` re-export。`RequestAuthSnapshot` 是 model/API request auth 和 Codex Apps auth metadata
  的轻量边界，必须包含 account id、ChatGPT user id、workspace 标记和 FedRAMP 标记这类调用方需要的
  认证事实；login-aware 边界从 `CodexAuth` / `AuthManager` 投影 snapshot 后再向下传递。core/app-server
  的 Codex Apps MCP、connector cache、tool-suggest 或 resource/status helper 需要
  `CodexAppsAuthContext` 时，应从 `RequestAuthSnapshot` 投影，不要重新把 `CodexAuth` 或
  `AuthManager` 穿透到这些 helper 中。core connector cache、accessible connector 查询和
  Codex Apps MCP 临时 runtime start 这类 API 应接收调用方预投影的 `RequestAuthSnapshot`，由
  app-server/TUI/CLI 这类组合根读取 login runtime 后传入；不要在 `core/src/connectors.rs` 内通过
  `Config` 重新构造 `codex_login::AuthManager`。`AuthRuntime` / `SharedAuthRuntime` 也属于
  `codex-auth-types`：core 的 turn-time 逻辑（例如 Apps enablement、image generation tool gating、
  ARC monitor request auth 和 telemetry snapshot）应消费该 trait 或 `RequestAuthSnapshot`，不要把
  `codex-login::AuthManager` / `CodexAuth` 枚举细节继续穿透到这些判断路径；`AuthManager` 只应保留在
  login-aware 组合根、session services 和仍未拆出的过渡边界。
- 默认 Codex client identity/residency header helper 属于 `codex-rs/client-identity`
  （`codex-client-identity`）：`originator`、first-party originator 判断、residency header state、
  `default_identity_headers` 和 originator override 常量应从该 crate 引用。该 crate 不得依赖
  `codex-default-client-api`、`codex-default-client`、`codex-terminal-detection`、`os_info`、`codex-client`、
  reqwest、`codex-login`、keyring/agent-identity/login-server runtime、`codex-api` 或 model-provider
  implementation。只需要 originator/residency/default identity headers 的 `codex-core`、runtime API
  crate、rollout、telemetry init、plugins/connectors 工具类 crate 必须依赖 `codex-client-identity`，
  不要依赖 `codex-default-client-api`、full `codex-default-client` 或通过 `codex-login::default_client`
  回流 full login runtime。
- 默认 Codex HTTP User-Agent/default full header helper 属于 `codex-rs/default-client-api`
  （`codex-default-client-api`）：`get_codex_user_agent`、User-Agent suffix 和 `default_headers` 继续由
  该 crate 拥有，并通过 `codex-client-identity` 合并 originator/residency headers。该 crate 可以依赖
  `codex-terminal-detection` 和 `os_info`，但不得依赖 `codex-client`、reqwest、`codex-login`、
  keyring/agent-identity/login-server runtime、`codex-api` 或 model-provider implementation。只需要
  full User-Agent/default headers 但不需要 reqwest runtime 的 crate 才依赖它；`codex-core` production
  graph 不得为了 User-Agent probing 或 HTTP runtime default headers 依赖该 crate。core 发起 WebSocket/
  realtime request 时只传 `default_identity_headers`；`codex-api` concrete runtime 在真实连接边界合并
  `codex_default_client::default_headers()`，并允许 caller-provided headers 覆盖默认值。
  session telemetry / network proxy audit 需要 terminal token 时，由 app-server、mcp-server、CLI/TUI
  或测试 harness 这类组合根调用 `codex-terminal-detection` 后通过 `ThreadManager::with_terminal_type`
  注入；`codex-core` production graph 不得直接或间接为了 terminal probing 依赖
  `codex-terminal-detection` / `codex-default-client-api`。
- 默认 Codex HTTP reqwest runtime 属于 `codex-rs/default-client`（`codex-default-client`）：
  `build_reqwest_client`、`try_build_reqwest_client`、`create_client` 和 `CodexHttpClient` constructor
  继续由该 crate 拥有，并 re-export `codex-default-client-api` 保持旧路径兼容。只有真实 HTTP runtime、
  CLI/diagnostic/login/remote download implementation 或 test-support 需要 reqwest client construction
  时才依赖 full `codex-default-client`；runtime/service API crate 和 `codex-core` production graph 不得为了
  identity/header helper 拉入 full default-client。
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
  `resolve_relative_paths_in_config_toml` 这类只依赖 `ConfigToml` shape 和
  `AbsolutePathBufGuard` 的相对路径解析 helper 也属于 `codex-config-toml`；agent role、config layer
  merge 准备或其他只需要按 config 文件所在目录解析 `AbsolutePathBuf` 字段的路径应直接依赖
  `codex-config-toml`，不要为了该 helper 拉入 local filesystem loader。
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
  project-local trust/root/git checkout 处理和 system config/requirements path
  helper 都归这个 crate。`codex_config::loader::*` 只保留兼容 re-export；core、app-server 或测试 helper
  需要 local layer IO implementation 时应优先注入 `codex-config-loader::ConfigLayerLoader`；
  只有 local-loader 自测、迁移期兼容 re-export 或需要直接验证 layer IO 的测试 helper 才直接调用
  `load_config_layers_state`。`codex-config-local-loader` 提供 concrete `LocalConfigLayerLoader`
  implementation；CLI/TUI/app-server/exec 等组合根需要 production `ConfigBuilder` 时，必须显式注入
  `Arc<dyn ConfigLayerLoader>`，通常使用 `Arc::new(LocalConfigLayerLoader::default())`，不要让
  `codex-core` production path 默认构造 local loader 或 normal 依赖 `codex-config-local-loader`。
  `codex-core` 只允许在 test-support / unit-test fallback 中使用 local-loader，避免大面积测试 fixture
  churn；deprecated raw `ConfigToml` loading helper 的 production 调用方也应使用显式带 loader 的入口。
  `project_trust_key` 这类纯 helper 应从 `codex-config-loader` 引用。该 crate 可以 re-export
  `codex-config-toml` 的 `resolve_relative_paths_in_config_toml` 保持旧路径兼容，但新增调用方应直接从
  `codex-config-toml` 引入。该 crate 可以依赖
  `codex-config-diagnostics`、`codex-config-loader`、`codex-config-requirements`、`codex-config-state`、
  `codex-config-toml`、`codex-config-types`、`codex-file-system`、`codex-git-info`、
  `codex-model-provider-info` 和 `codex-protocol` 来完成现有 local layer 语义，但不得依赖 full
  `codex-config`、`codex-app-server-protocol`、`codex-code-mode`、Starlark-backed `codex-execpolicy` 或
  Rama-backed `codex-network-proxy`。不要把 effective runtime `Config` 构造、session defaults、network
  proxy backend/evaluator 或 app-server transport adapter 移入 local-loader；这些属于 core/runtime 或
  app-server 组合根边界。
- git baseline repository/diff runtime 属于 `codex-rs/git-baseline`（`codex-git-baseline`）：
  memory workspace 这类内部目录需要用 Git 作为 baseline/diff 实现细节时，应直接依赖该 crate。
  `codex-git-baseline` 继续拥有 `gix`、`similar` 和 baseline reset/diff 测试；不要为了 baseline
  runtime 重新依赖 `gix` 或 re-export baseline API。
- git repository metadata/discovery 属于 `codex-rs/git-info`（`codex-git-info`）：repo root、
  worktree trust root、remote URL normalization、branch/default branch、recent commits、merge-base、
  Git blob object-id helper、`GitInfo` / `GitSha` re-export 和 thread metadata 所需 git command helper
  都归这个 crate。
  `codex-core`、config loader、plugins、TUI、app-server、thread-store、rollout/state metadata 这类只需要
  git metadata 或 branch query 的路径应直接依赖 `codex-git-info`，不要经 `codex-git-utils` facade。
  `codex-git-info` 可以直接依赖 `sha1` 来计算 Git blob object ID，但不得依赖 `codex-git-utils`、
  `codex-core`、`regex`、`tempfile` normal dependency、`walkdir`、`gix`、`similar` 或
  `codex-git-baseline`。
- git patch/apply helper 属于 `codex-rs/git-utils`（`codex-git-utils`）：`ApplyGitRequest`、
  `apply_git_patch`、apply output parser、staging helper 和 symlink helper 继续留在这里。该 crate
  可以 re-export `codex-git-info` 旧路径保持兼容，但新增 metadata/branch consumer 不应依赖它。
- memories read 的轻量 API 属于 `codex-rs/memories/read-api`
  （`codex-memories-read-api`）：memory citation parser、memory root helper、以及
  `MemoryToolDeveloperInstructionsProvider` /
  `SharedMemoryToolDeveloperInstructionsProvider` / disabled provider 这类 session runtime 注入 trait
  都归这个 crate。`codex-core`、session/thread manager 和只需要解析 memory citation 或持有 memory
  prompt provider trait 的路径应直接依赖 `codex-memories-read-api`，不要为了这些轻量能力拉入完整
  `codex-memories-read`。基于 shell parser/safety 的 memory usage telemetry classifier 属于
  `codex-core` runtime telemetry 边界，不要放回 `codex-memories-read-api` 使 API crate 依赖
  `codex-shell-command`。完整 `codex-memories-read` 继续拥有 filesystem-backed developer instructions
  provider、模板渲染、output truncation 和 Tokio fs runtime，并可以 re-export API crate 保持旧路径兼容；
  app-server、mcp-server、CLI/TUI 等组合根需要真实文件系统 memory prompt 时负责注入
  `FsMemoryToolDeveloperInstructionsProvider`。
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
- `ConfigToml` 专属 config lockfile helper 属于 `codex-config-toml`，包括
  `read_config_lock_from_path`、`config_lockfile`、`validate_config_lock_replay`、
  `config_without_lock_controls`、`clear_config_lock_debug_controls` 和 TOML round-trip/diff
  这类纯 lock replay 逻辑。`codex-core` 只保留 session export/validation 编排，以及把 lockfile
  materialize 成 `ConfigLayerEntry` 的 adapter；不要把这些 helper 放进 `codex-config-state` 造成
  `config-loader -> config-state -> config-toml -> config-loader` 依赖环。
- model provider domain TOML/helper 属于 `codex-model-provider-info`：`ModelOptionToml`、
  `validate_model_providers`、`validate_reserved_model_provider_ids`、`deserialize_model_providers`
  和 `validate_oss_provider` 应与 `ModelProviderInfo`/provider 常量同 owner。`codex_config::config_toml`
  只做旧路径兼容 re-export；core effective config、app-server model 展示或其他消费者需要 model option
  / provider validation 时应直接依赖 `codex-model-provider-info`，不要为了这类 model-provider domain
  helper 拉入完整 `codex-config`，也不要把它们塞入无 protocol 依赖的 `codex-config-types`。该 crate
  只能承载 provider DTO/catalog/validation 和不依赖 API client 的轻量 helper。把
  `ModelProviderInfo` 转换成 `codex_api_provider::Provider`、解析 HTTP header map 和按 auth mode 选择 API
  base URL 属于 `codex-model-provider-api`；把 `codex_auth_types::RequestAuthSnapshot` 映射成
  `codex_api_provider::AuthProvider` / bearer provider / unauthenticated provider 的 request-header adapter
  属于 `codex-rs/api-auth`（`codex-api-auth`）。只需要请求 headers 的
  core/core-plugins/core-skills/backend/app-server transport helper 应直接依赖 `codex-api-auth`，并通过
  `CodexAuth::request_auth_snapshot()` 或 host 已投影的 snapshot 调用
  `auth_provider_from_auth_snapshot`，不要为了 headers 拉入完整 `codex-model-provider-api` 或
  `codex-model-provider`。`codex-mcp` runtime 应接收
  调用方预先构造好的 `SharedAuthProvider` 和 `CodexAppsAuthContext` 用于 host-owned Codex Apps MCP
  server，不要为了 `auth_provider_from_auth` 直接依赖 `codex-model-provider-api`，也不要为了从
  `CodexAuth` 提取 cache/auth-status 字段直接依赖 `codex-login`。`codex-model-provider-api` 不得依赖
  `codex-login`；需要 full `CodexAuth` / `AuthManager` 行为的 crate 应在 login-aware 边界先投影成
  `RequestAuthSnapshot`、`SharedAuthProvider`、`CodexAppsAuthContext` 或
  `SharedModelProviderAuthManager`。`codex-login` 的 `model_provider_auth_manager` adapter 只能通过
  `model-provider-auth` feature 暴露；只需要 AuthManager、token refresh、remote control 或 backend client
  auth snapshot 的 crate 不得启用该 feature，避免默认 login graph 把 `codex-model-provider-api` 间接拉回。
  `codex-backend-client` 这类 backend HTTP client 只接受 `RequestAuthSnapshot` /
  `codex-api-auth` provider；调用方负责在 login-aware 边界设置 User-Agent。Agent Identity 的
  task-scoped request header signing 属于
  `codex-rs/agent-identity-api`（`codex-agent-identity-api`）：`AgentIdentityKey`、
  `AgentTaskAuthorizationTarget` 和 `authorization_header_for_agent_task` 应从该轻量 crate 引用；
  `codex-model-provider-api` 不得为了 header adapter 依赖完整 `codex-agent-identity`，否则会把
  `reqwest`、JWKS fetch 和 task registration runtime 间接拉回 core。完整 `codex-agent-identity`
  继续拥有 Agent Identity JWT decode/verify、JWKS fetch、task registration、key generation 和 URL
  helper，并可以 re-export API crate 的 signing helper 保持旧路径兼容。
  session/telemetry/service-tier 这类只需要认证摘要的 core 路径应消费
  `AuthRuntime::telemetry_snapshot()`，由 login-aware runtime 计算诸如
  `uses_enterprise_default_service_tier` 的轻量布尔语义；不要为了 account plan 判断把
  `codex_protocol::account::PlanType` 下沉到 `codex-auth-types`，也不要在 core 里直接调用
  `AuthManager::auth_cached()` 后匹配 `CodexAuth`。runtime provider trait/types 也属于 `codex-model-provider-api`：
  `ModelProvider`、`SharedModelProvider`、`ModelProviderFuture`、`ProviderCapabilities`、
  `ProviderAccountState`、`ProviderAccountError` 和 `ProviderAccountResult` 应从 API crate 引用；不要为了
  trait object、provider capability 或 account state 类型拉入完整 `codex-model-provider`。
  `ModelProviderAuthManager` / `SharedModelProviderAuthManager`、401 recovery trait 和
  `ModelProviderFactory` / `SharedModelProviderFactory` 也属于 `codex-model-provider-api`；core/session
  runtime 只能通过 constructor injection 持有这些 trait，不要直接调用完整
  `codex-model-provider` 的 concrete constructor。`DefaultModelProviderFactory`、`create_model_provider`、
  configured provider、Bedrock provider、model manager implementation selection 和 request execution 边界
  继续属于完整 `codex-model-provider` 或 app-server/CLI 这类组合根，不能泄漏回 API trait crate；
  login-backed model-provider auth manager adapter 属于 `codex-login`，因为它需要匹配 `CodexAuth`、
  `RefreshTokenError` 和 unauthorized recovery state。默认 `codex-core` runtime 不得依赖
  `codex-login` 或调用 `codex_login::model_provider_auth_manager`；需要 login-backed provider auth
  时，由 app-server/CLI/MCP server/thread-manager sample 这类组合根先把 `AuthManager` 投影成
  `ThreadAuthRuntimes`（`SharedAuthRuntime` + `SharedModelProviderAuthManager`）后注入
  `ThreadManager` / prompt-debug 边界。core test-support 可以保留 dummy auth fixture 和投影 helper，
  但不要在 core 默认 normal graph 中新增 `CodexAuth -> ModelProviderAuthManager` adapter、
  `CodexAuth` 枚举匹配或 refresh error 映射。`ModelClient`、`TurnContext` provider
  construction、models manager construction 和其他底层 model runtime consumer 应接收已经投影好的
  `SharedModelProviderAuthManager`；不要在这些低层模块直接调用 `codex_login::model_provider_auth_manager`
  或依赖 `AuthManager`。只应由 app-server/CLI/MCP server 等组合根或 core test support 构造并注入。
  core 单测需要 provider factory 时使用
  `codex_core::test_support::model_provider_factory_for_tests()`，不要把 `codex-model-provider` 加回 core
  normal dependency。`TurnContext` / `SessionServices` / `CodexSpawnArgs` / active turn runtime 不得携带完整
  `AuthManager`；需要请求/遥测 auth snapshot 时只持有 `codex-auth-types::SharedAuthRuntime`，需要 model
  provider 认证时只持有 `SharedModelProviderAuthManager` 或 `ModelProvider::auth_manager()` 返回的 trait
  object。`Session::new`、`ThreadManager` start/resume/fork 和 subagent/delegate spawn 应接收这些已投影
  trait object，不能在 session/thread runtime 内调用 `codex_login::model_provider_auth_manager`。测试中若
  只是重建 provider 或 models manager，应使用已投影 provider auth 的 test helper，不要把 full
  `AuthManager` 重新挂回 turn context、session services 或 thread manager state。active turn task
  ordering 属于 session runtime 内部实现细节：需要保留 first-task 语义时使用 `ActiveTasks`
  这类本地 `HashMap + Vec` 容器，不要为了该局部顺序语义把 `indexmap` 加回 `codex-core` normal
  dependency。
  不要让 config-facing info crate 依赖 `codex-api`、`http` 或 client stack。
- model catalog API 属于 `codex-rs/models-manager-api`（`codex-models-manager-api`）：
  `ModelsManager`、`SharedModelsManager`、`RefreshStrategy`、`TryListModelsError`、
  `ModelsManagerConfig` 和 `ModelMetadataOverride` 应由这个 API crate 承载。core、core-api、
  app-server、CLI、model-provider 或其他只需要模型目录 trait/config 的消费者应直接依赖
  `codex-models-manager-api`，不要为了 trait、refresh strategy 或 config override 拉入完整
  `codex-models-manager`。完整 `codex-models-manager` 继续拥有 bundled model catalog、cache、
  remote refresh、concrete manager、collaboration presets、model_info fallback/override 逻辑和测试。
  完整 manager 只需要 provider auth 状态时应接收 `SharedModelProviderAuthManager`，不要直接依赖
  `codex-login::AuthManager`；login-backed adapter 留在 `codex-model-provider`、core test support 或测试
  dev-dependency 边界。
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
  helper、CLI override TOML layer builder，以及 full config layer loading 的
  `ConfigLayerLoader` trait / `ConfigLayerLoadRequest` contract 等轻量边界。它可以依赖
  `codex-config-state` 和 `codex-config-requirements` 来表达 layer stack 和 requirements 输入输出，
  但不得依赖
  full `codex-config`，也不得包含 tonic/prost remote implementation。remote thread config loader 属于
  `codex-rs/config-loader-remote`（`codex-config-loader-remote`），由 app-server、app-server-client
  等组合根显式依赖；不要通过 `codex-config` re-export remote implementation，也不要让
  `codex-config -> codex-config-loader -> codex-config-loader-remote` 把 remote/gRPC 依赖间接拉回 full
  config。`codex-config` 负责把 thread config sources 投影成 `ConfigLayerEntry`；loader API crate 只拥有
  trait contract，不拥有 local filesystem/git/MDM/platform loader IO implementation。
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
  、config layer version fingerprint helper `version_for_toml`，以及需要
  `ConfigLayerStack`/filesystem 的 first-layer diagnostic 定位 helper。
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
  （`ConfigEditsBuilder`、`load_global_mcp_servers`）、model/personality/service-tier/project trust/feature
  flag/user preference 写入属于 `codex-rs/config-edit`（`codex-config-edit`）。`CONFIG_TOML_FILE`
  属于 `codex-config-types`，config layer fingerprint helper `version_for_toml` 属于
  `codex-config-state`；`codex-config` 和必要旧 facade 只 re-export 以保持旧路径兼容。
  CLI、测试和 runtime 调用方需要读取或编辑全局 MCP servers 时应直接依赖 `codex-config-edit`，
  不要新增或重新使用 `codex-core::config` 的全局 MCP loader/edit facade。
  `codex-config-state` 和 `codex-config-local-loader` 不得为了 config 文件名常量或 layer fingerprint
  normal 依赖 `codex-config-edit`。core/app-server/CLI/core-plugins 这类生产路径需要 config 文件名时，
  应直接依赖 `codex-config-types::CONFIG_TOML_FILE`，或使用已存在的 core 兼容 re-export；不要从
  `codex-config-edit` 的兼容 re-export 获取该常量。
  `codex-core` 不得为了 config 写入直接 normal 依赖 `toml_edit`；需要使用 core `Config` 或
  `codex_features::FEATURES` 这类 core-only metadata 时，只能在 `core/src/config/edit.rs` wrapper
  中注入到 `codex-config-edit` 的窄 API。`codex-config-edit` 不得 normal 依赖 full `codex-config`、
  `codex-features`、`codex-protocol`、app-server protocol、Starlark、Rama 或 code-mode runtime。
  `codex-core-plugins` production path 不得 normal 依赖 full `codex-config`，测试 fixture 需要完整
  loader 时才允许 dev-depend `codex-config`。
- requirements TOML、normalized requirements、requirements exec policy TOML/evaluator 和 cloud requirements
  loader 属于 `codex-rs/config-requirements`（`codex-config-requirements`）。`codex-config`
  只作为旧路径兼容 re-export；loader、MDM、diagnostics、filesystem/git、profile/thread config
  stack 和 model-provider validation 继续留在 `codex-config` 或后续明确的 loader crate。`codex-cloud-requirements`
  这类只需要 requirements 解析/加载的小 crate 不得 normal 依赖完整 `codex-config`；需要 policy DTO 时依赖
  `codex-execpolicy-api`，需要 config DTO 时依赖 `codex-config-types`。
- hook declaration/key 这类不需要命令执行器的轻量 helper 属于 `codex-rs/hooks-api`
  （`codex-hooks-api`）：`PluginHookDeclaration`、`plugin_hook_declarations`、
  `plugin_hook_key_source`、`hook_events_into_matcher_groups`、`hook_event_key_label` 和
  `hook_key` 应从该 crate 引用。`codex-core-plugins`、plugin detail rendering、hook state key
  计算或其他只需要声明/metadata projection 的消费者不得依赖 full `codex-hooks`；full
  `codex-hooks` 只 re-export 这些 API helper 并继续拥有 hook discovery、trust/hash、schema parsing、
  command execution、output parsing/spilling 和 Tokio process runtime。
- `codex-hooks` 不得为了 hook discovery 直接 normal 依赖完整 `codex-config`。Hook runtime 需要已加载
  config stack 时，应使用 `codex_hooks::HookConfigLayerStack` / `HookConfigLayerEntry` 这类 hook
  专用只读 view；core/app-server 等组合根负责从 `codex_config::ConfigLayerStack` 投影过去。Hook
  trust hash 可以留在 `codex-hooks` 或 `codex-config` 边界，但不要放入 `codex-config-types`
  反向依赖 protocol，也不要把 full loader/requirements evaluator 混进 hooks crate。
- session/core 中执行 hooks 时只能依赖 `codex-hooks-api` 的 `HookRuntime`、
  `SharedHookRuntime`、`HookRuntimeFactory`、`SharedHookRuntimeFactory` 和 `HooksConfig` 边界；不要在
  `codex-core` production path 中构造 `codex_hooks::Hooks`、依赖 full `codex-hooks`，或直接持有 hook
  command execution/runtime 类型。app-server、MCP server、exec/test harness 等组合根负责注入
  `codex_hooks::HooksRuntimeFactory` 或 disabled factory。`codex-hooks-api` 的
  `config-test-support` feature 只允许 test/dev graph 为旧 `ConfigLayerStack -> HookConfigLayerStack`
  fixture 转换开启，默认 normal graph 不得因此拉回 full `codex-config`、`codex-hooks`、Tokio process runtime、
  `regex`、`sha2` 或 output truncation。
- `codex-core-skills` 不得为了读取 `[skills]`、技能开关或 project root marker 直接 normal 依赖完整
  `codex-config`。Skill runtime 需要已加载 config stack 时，应使用
  `codex_core_skills::SkillConfigLayerStack` / `SkillConfigLayerEntry` 只读 view；core、core-plugins 或
  app-server 等组合根负责从 `codex_config::ConfigLayerStack` 投影过去。`SkillsConfig` /
  `SkillConfig` / `BundledSkillsConfig` 属于 `codex-config-types`，允许 `codex-config-types` normal 依赖
  `toml` 来支持从 `toml::Value` 反序列化这些纯 DTO，但不得因此引入 full config loader、
  app-server protocol、Starlark、Rama 或 V8。future-only remote skill list/export HTTP client
  不参与默认 core/session graph；需要启用时必须走显式非默认 feature，并接收调用方预投影的
  `codex_auth_types::RequestAuthSnapshot`，再调用
  `codex_api_auth::auth_provider_from_auth_snapshot`。只有该 remote HTTP feature 可以依赖
  full `codex-default-client` 做 reqwest client construction，也只有该 feature 可以依赖 `zip` 来解压远程
  skill export payload；默认 `codex-core-skills` graph 不得为了未接线的
  remote skill API 拉 `codex-auth-types`、`codex-model-provider-api`、`codex-default-client`、reqwest、
  `codex-login`、`codex_login::CodexAuth` 或 `codex_login::default_client`，也不得为了远程 skill export 解压拉 `zip`、`flate2`、`zstd`、`bzip2`、
  `xz2`、`aes` 或 PBKDF/HMAC 这类 archive/encryption implementation dependency。
  `SKILL.md` frontmatter 和 `agents/openai.yaml` metadata 解析使用 `codex-core-skills` 内部窄
  schema parser：frontmatter 只支持 `name`、`description`、`metadata.short-description` 和
  description block scalar；metadata 只支持 `interface`、`dependencies.tools` 和 `policy` 当前字段，
  JSON metadata 继续走 `serde_json`。不要为了 skill discovery 或 metadata 重新给
  `codex-core-skills` 增加 `serde_yaml` normal dependency；扩展 skill metadata schema 时必须同步扩展
  parser 和 loader tests，并用 `cargo tree -p codex-core --invert serde_yaml --edges normal` 与
  `cargo tree -p codex-core-skills --invert serde_yaml --edges normal` 证明默认 graph 没有 YAML runtime 回流。
  `codex-core-skills` 生产路径需要 canonical path 时应使用
  `codex_utils_absolute_path::AbsolutePathBuf::canonicalize()` 或已有 path owner helper，不要为了
  test fixture 风格的路径规范化 direct normal 依赖 `dunce`；loader/manager tests 需要构造本机
  canonical fixture 时可以把 `dunce` 保留为 dev-dependency。验证时用
  `cargo tree -p codex-core-skills --depth 1 --edges normal` 确认 direct deps 不显示 `dunce`，再用
  `cargo tree -p codex-core-skills --invert dunce --edges normal` 记录剩余路径只经
  `codex-utils-absolute-path` 等 path owner crate。
- skill 的模型上下文类型、渲染/注入 helper、mention/implicit invocation 检测、`SkillsLoadInput` 和
  `SkillsRuntime` trait 属于 `codex-rs/core-skills-api`（`codex-core-skills-api`）。`codex-core`
  production path 只能依赖这个 API crate，并通过 `SharedSkillsRuntime` 持有 host-provided runtime；
  不要让 `codex-core` 默认 normal graph 依赖 concrete `codex-core-skills`、embedded `codex-skills` /
  `include_dir` system skill loader、skill filesystem discovery/cache implementation 或 remote skill HTTP
  implementation。app-server、MCP server、CLI/TUI 等组合根负责创建
  `codex_core_skills::SkillsManager` 并注入 `ThreadManager`；test-support/dev graph 可以显式启用 concrete
  manager。新增 skill 上下文逻辑时先判断是否属于 API crate；新增 discovery、system install、cache 或
  remote transport 行为时应留在 `codex-core-skills` concrete implementation，并用
  `cargo tree -p codex-core --invert codex-core-skills --edges normal` 证明没有回流到 core 默认 graph。
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
  `codex-workflow-api` 的 `WORKFLOW.md` frontmatter parser 只支持 workflow manifest schema 所需的窄
  YAML 子集（标量、`when_to_use`/`whenToUse` block list、`inputs` 两层 block map），不得为了 manifest
  discovery 重新 normal 依赖 `serde_yaml` 或 full workflow runtime。需要扩展 manifest schema 时优先扩展
  该窄 parser 和单测，并继续用 `cargo tree -p codex-core --invert serde_yaml --edges normal` 确认
  `codex-workflow-api -> serde_yaml -> codex-core` 路径没有回流。
- Markdown agent role 文件（`*.agent.md` / project `.codex/agents/*.md`）的 frontmatter 解析属于
  `codex-core::config::agent_roles` 当前边界，但 parser 必须保持窄 schema：只支持 `name`、
  `description`、`model`、`effort`、`model_reasoning_effort` 标量，以及 `tools` / `skills` 的
  `"*"`、简单 block list 或简单 inline list。未知字段继续忽略以兼容已有 agent 文件 metadata。
  不要为了 agent role discovery 重新给 `codex-core` 增加 `serde_yaml` normal dependency；需要扩展
  Markdown agent role schema 时，先扩展该窄 parser 和 config tests，并用
  `cargo tree -p codex-core --depth 1 --edges normal` 与
  `cargo tree -p codex-core --invert serde_yaml --edges normal` 确认 core direct YAML path 没有回流。
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
  payload，不作为 connector metadata 的 owner。core connector cache、tool-suggest directory lookup
  和 plugin-install completion refresh 这类路径只需要是否使用 Codex backend、account id、ChatGPT user id
  和 workspace 标记时，应传递 `codex_mcp_types::CodexAppsAuthContext` 这类调用方预投影的 auth
  context；不要新增 connector 专属重复 auth context DTO，也不要把 `CodexAuth` 继续作为 connector
  helper 的 public/internal API 参数扩散。
- connector metadata/filter/merge/accessible/directory cache 这类不需要联网 directory fetch 的轻量 helper
  属于 `codex-rs/connectors-api`（`codex-connectors-api`）。`codex-core`、TUI 或 tools 只需要本地
  cache、available connector 过滤、workspace/account scoped cache key、install URL/名字 normalization
  时应直接依赖 `codex-connectors-api`，不要依赖 full `codex-connectors`。full `codex-connectors`
  继续拥有远端 directory listing/fetch runtime、HTTP/query encoding 和相应测试 fixture，并可以 re-export
  API helper 保持旧路径兼容；不得为了旧路径兼容让 `codex-connectors-api` 反向依赖 full
  `codex-connectors`。
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
