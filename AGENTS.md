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
- If local lint verification is specifically needed, owner may include `just argument-comment-lint` in the fixed tester `/root/my_codex_pm/rust_cargo_tester` followup request after code review passes. This is powered by Bazel, so running it the first time can be slow if Bazel is not warmed up, though incremental invocations should take <15s. Most of the time, it is best to update the PR and let CI take responsibility for checking this. Note CI checks all three platforms, which the local run does not.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Newly added traits should include doc comments that explain their role and how implementations are expected to use them.
- Discourage both `#[async_trait]` and `#[allow(async_fn_in_trait)]` in Rust traits.
  - Prefer native RPITIT trait methods with explicit `Send` bounds on the returned future, as in `3c7f013f9735` / `#16630`.
  - Preferred trait shape:
    `fn foo(&self, ...) -> impl std::future::Future<Output = T> + Send;`
  - Implementations may still use `async fn foo(&self, ...) -> T` when they satisfy that contract.
  - Do not use `#[allow(async_fn_in_trait)]` as a shortcut around spelling the future contract explicitly.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- Do not add general product or user-facing documentation to the `docs/` folder. The official Codex documentation lives elsewhere. The exception is app-server API documentation, which is covered by the app-server guidance below.
- Prefer private modules and explicitly exported public crate API.
- If you change `ConfigToml` or nested config types, update `codex-rs/core/config.schema.json` when needed; owner may include `just write-config-schema` in the fixed tester `/root/my_codex_pm/rust_cargo_tester` followup request after code review passes.
- When working with MCP tool calls, prefer using `codex-rs/codex-mcp/src/mcp_connection_manager.rs` to handle mutation of tools and tool calls. Aim to minimize the footprint of changes and leverage existing abstractions rather than plumbing code through multiple levels of function calls.
- 对话/线程展示相关的结构化语义以 typed `ResponseItem` 为 canonical source；`ThreadItem` 应通过共享 projector 统一生成。新增或修改 event-command、schedule、collab 这类展示项时，必须复用 `codex-rs/app-server-protocol/src/protocol/response_item_projection.rs`，不要新增 raw response item 展示分支，也不要从 message marker 文本或 assistant message JSON 解析 typed item。provider 请求侧为了 wire/model 输入需要保留 marker 包装时，只能作为单向 formatting，不得作为展示或 history 重建的解析来源。schedule subscribe/unsubscribe 仍属于这套 typed projection，不要作为旧 generic event-driven 兼容路径移除。
- 工具执行完成后的纯历史记录路径应尽早 canonicalize 为 typed `ResponseItem`；pending user-hook 路径使用 `PendingInputItem::HookInspectable(ResponseItem)` 表达“需要 hook 检查的对话项”。`ResponseInputItem` 只保留在 Responses API request 输入和 client/request 输入适配层，不要作为工具输出、pending history 或 hook history 的核心中转类型继续扩散。
- active turn 注入需要经过 pending/user prompt hook 检查的输入时，使用语义明确的 `inject_hook_inspectable_items`；直接写入 model-visible history 的 typed `ResponseItem` 使用 `inject_conversation_items` 或 `record_conversation_items`。不要新增模糊的 `inject_response_items` 入口。
- `record_conversation_items` 只负责 typed `ResponseItem` 写入 history/rollout/context usage，不得顺带发送 live `RawResponseItem`；需要客户端可见的 live item 时，使用显式 typed lifecycle 入口（例如 record 后 emit `item/completed`），并继续通过 shared projector 生成 `ThreadItem`。
- live `item/started` 和 `item/completed` payload 在 app-server v2 边界以 typed `ThreadItem` 为 canonical display payload；core/rollout legacy `TurnItem` 只能在 app-server protocol 的 lifecycle adapter 中转换为 `ThreadItem`，后续 live notification/reducer 路径不要继续消费 `TurnItem`，也不要新增 raw response item 或 message marker 解析路径。
- root-worker prototype、SDK 示例或其他非 TUI 客户端展示 app-server v2 thread/live 内容时，只能消费 typed `ThreadItem` / v2 payload；不要从 `agentMessage.text`、`eventDrivenTool.text`、compact replacement raw `ResponseItem`、`<event_driven_tool>`、`<event_command>`、`<subagent_notification>` 或 inter-agent JSON envelope 反解 display item。旧 raw structured message 如需兼容，应在 app-server/client 边界过滤或 canonicalize 为 typed item，不得原样作为 child-completion/subagent/event-command 展示。
- root-worker composer slash 菜单中，能表达为 Skill 的命令应来自 Skills discovery；例如 `/init` 由 embedded system skill 提供，不要作为 root-worker builtin command 硬编码。只有依赖 runtime/thread state 或客户端本地动作的命令（例如 `/clear`、`/goal <objective|pause|resume|cancel|clear>`）才放入 root-worker builtin slash command registry，并且执行时不得作为普通 user message 发送给模型；`/cancel-goal` 只能作为兼容别名，不作为主展示命令。
- command session 的 output/exit notification 必须作为独立 typed `ResponseItem` / `ThreadItem` 展示项或等价 typed lifecycle item 表达，并通过 typed command item id 关联原 `CommandExecution`；`command_wait` 和 `command_write_stdin` 的等待/写 stdin 行为也必须记录为独立 typed `ResponseItem` / `ThreadItem`。`ExecCommandOutputDelta` 只用于更新 command cell live tail，不得作为 raw marker、assistant 文本或按 output 内容反解出的 conversation event。
- root-worker Agent Tree 的主状态必须消费后端 canonical `ThreadStatus` / `thread/status/changed`，例如 `activeFlags` 中的 `running`、`waitingOnSubagent`、`waitingOnEventTool`；不要从 turn/items/raw marker/legacy JSON envelope 自行推导 running、waiting 或 idle。旧 payload 兼容只能在 app-server/client 边界 canonicalize，不能作为 live tree 状态主路径。
- Go/Goal post-turn 流程必须按 `ThreadActive -> ThreadIdle -> GoContextContinuation / ThreadCompletion` 状态模型推进；Goal `Active` 且 thread idle 时注入 `<goal_context>` 并阻止 child completion，Goal `Complete` / `Paused` / `BudgetLimited` / 不存在时才允许 child completion。`ThreadActive` 判定必须复用 canonical recursive active helper，不要只用当前 turn complete，也不要复制 subagent、event command、mailbox 或 queued input 的近似条件。
- 外部工作唤醒必须采用 typed runtime event + active state evaluator + goal scheduler 的分层模型：runtime event 表达“发生了什么”（child completed/failed/interrupted、command output/exit/stdin、schedule fired、workflow updated 等），active state 表达“现在能不能继续”（local turn、active child、active command/event tool、queued input、pending external event），goal 只表达“thread idle 后是否继续”。不要让 goal 轮询 child/command 状态，也不要把长期 subagent/command 等待改成阻塞 turn；外部事实变化必须写入 typed `ResponseItem`/`ThreadItem` 并唤醒 scheduler。`wait_agent` 和 `command_wait` 只是显式短窗口等待/用户可见等待动作，不是系统调度主机制。subagent 异常、丢失或中断必须作为 typed child lifecycle event 传回 parent，不能静默依赖 parent goal continuation 猜测。
- root-worker prototype 的 `ThreadItem` 写入、snapshot normalization 和 pending/live 合并只能按 `ThreadItem.id` 判断同一个 item；不同 id 必须作为不同 item 保留，不得根据 text/content/status/semantic key/raw marker/legacy JSON envelope 合并或丢弃。每个 typed `ThreadItem` 至少生成一个 `ConversationEntry`；`ConversationCell` 只能做视觉分组，不能丢 entry。
- root-worker live 模式下，已经进入本地 live cache 的 thread 在切换展示时只能使用持续接收的 live `ThreadItem`，不要触发 `thread/read` 或用 snapshot/history rebuild 对 item 做 destructive/non-destructive merge；`thread/read` 仅用于 cold start、缺失本地 thread 或显式恢复路径。
- root-worker 已初始化 thread 的 live `turn/started` / `turn/completed` 只能更新 turn lifecycle metadata，不得把通知中的 `turn.items` 当作 snapshot 覆盖本地 items；conversation item 内容必须通过 typed `item/started` / `item/completed` 或 agent delta 增量进入 cache。
- root-worker prototype 会话消息布局应继续以 typed `ThreadItem -> ConversationEntry -> ConversationCell` 为展示链路；连续普通 agent message 需要保持为一个 message cell、一个外层 agent bubble，内部可用 segment 展示多条 entry；user message 右对齐展示。新增展示分组或布局逻辑时，不要跨 user/tool/event/schedule、childCompletion/subagentNotification、replacement history 等语义边界合并。
- root-worker prototype 的 conversation 搜索、过滤、定位或高亮能力只能基于已投影出的 `ConversationEntry` / `ConversationCell` 派生；搜索结果不得参与 `ThreadItem.id` 合并或去重，不得从 raw marker、assistant message JSON、legacy envelope 或 agent text 中反解 display item。
- MultiAgent 运行时只保留 V2 工具和 typed child completion 路径；不要重新引入 V1 `send_input`/`resume_agent` 工具、legacy completion watcher、raw `inject_user_message_without_turn` child completion fallback，或通过配置在 V1/V2 之间切换。
- MultiAgent V2 的 `wait_agent` 只能等待 canonical typed subagent 更新：调用开始必须先非消费式检查 parent pending input/mailbox 中已有的 typed `InterAgentCommunication` / child completion / status，然后再通过 status watch 与 mailbox sequence notify 进入 runtime backoff；不得 drain mailbox，不得从 raw marker、assistant text 或 legacy JSON envelope 反解唤醒条件。`features.multi_agent_v2.default_wait_timeout_ms` 表示 initial window，`max_wait_timeout_ms` 表示 hard cap。
- Compact 当前用户可见和默认路径是 Local Compact：手动 `/compact`、`thread/compact/start` 和自动 context-limit compact 都应走 `codex-rs/core/src/compact.rs` 的本地 summarization 流程，并把 `CompactedItem.replacement_history` 持久化到 rollout 以便 thread/history 和 app-server 展示检查；compact 完成的 live `item/completed` 也必须携带同一份 replacement history，已 live/loaded 的 root-worker thread 不得依赖 `thread/read` 回填。Local Compact summary 在 active history 中是带 `SUMMARY_PREFIX` 的 user message，context usage 分类必须把它计入 compact 类别，不能当作普通 user message 导致 compact ratio 丢失。`compact_remote.rs` / `compact_remote_v2.rs` 只保留为未路由的历史兼容实现，不要重新接到默认入口、用户触发入口或模型请求 beta header。
- Dynamic Workflow 已实现 registry/init context、`workflow_list`、`workflow_describe`、TypeScript 子进程 runner、`$CODEX_HOME/workflow-runs/<runId>` snapshot 持久化、`workflow_start/status/resume/abort` run control，以及 app-server v2 `workflow/list|describe|start|status|resume|abort` 控制面和 `workflow/run/updated` notification；home workflow 位于 `$CODEX_HOME/workflows/<workflow-id>/`，project workflow 位于各 active project `.codex/workflows/<workflow-id>/`，project 同 id 覆盖 home；`WORKFLOW.md` 是 canonical workflow manifest 与说明文件，frontmatter 必须包含 `id`、`name`、`description`、`entry`，其中 `id` 必须等于目录名，`entry` 必须指向同目录内存在的 TypeScript 文件；init context 会注入发现到的 workflow 的 name、description 和 `WORKFLOW.md` 正文说明截断内容。workflow progress 展示必须继续走 typed `ResponseItem -> ThreadItem` 路径，并通过显式 typed lifecycle 发出 live item；不要新增 raw marker、assistant message JSON 或 legacy envelope 解析。当前 TS runner 的 `wf.Agent`/`wf.shell` 仍是结构化占位 API，尚未接入真实 MultiAgent runtime callback 或 durable shell step。
- If you change Rust dependencies (`Cargo.toml` or `Cargo.lock`), update `MODULE.bazel.lock`
  when needed. Owner may include `just bazel-lock-update` and `just bazel-lock-check` in the
  fixed tester `/root/my_codex_pm/rust_cargo_tester` followup request, but these are not part of
  the default worktree validation set.
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
- owner 在完成开发、功能修改、重构或性能优化后，必须检查并更新 `AGENTS.md`，维护当前仓库规则、协作流程和约束；如果确认无需更新，也要在交付中明确说明原因。
- owner 和 reviewer 都不能直接执行 Rust/Cargo 相关测试、构建、格式化、lint 或 benchmark 命令，包括 `cargo test/check/build/bench`、`cargo insta`、`just test/fix/fmt`、Bazel Rust lock 验证等；reviewer 只做 code review，不执行命令也不 followup tester。需要这些命令时，owner 必须先让同一个 reviewer 线程多轮 review 到无阻塞问题，再自行通过 `followup_task` 发送给项目唯一固定 tester `/root/my_codex_pm/rust_cargo_tester` 串行执行。
- 项目唯一 Rust/Cargo tester 的 canonical path 固定为 `/root/my_codex_pm/rust_cargo_tester`。不要为每个 owner/reviewer/worktree 新建测试 agent；PM 首次需要 Rust/Cargo 验证时用 `task_name=rust_cargo_tester`、`agent_type=test_agent`、`fork_turns=none` 创建这个 tester，后续所有 Rust/Cargo 验证请求都由 owner 复用该线程。
- owner followup 给固定 tester 时必须提供 JSON 请求，包含 `type: "rust_cargo_validation_request"`、`request_id`、`requested_by`、`report_to`、`worktree`、`branch` 和按顺序排列的 `commands`；每个 command 必须包含 `id` 和完整 `exec_command` 参数（至少包含带 `rtk` 前缀的 `cmd`、`workdir`、`initial_wait_ms`、`notify_on`、`max_output_tokens`）。tester 只按清单串行执行收到的 `exec_command`，并把每条命令的退出码和 stdout/stderr 原文回传给请求方；不要总结、压缩或改写命令输出，不做测试设计、补充命令、风险判断、失败归因、修复建议或范围扩展。
- 同一 owner 任务只创建一个 `@code-review` reviewer；首次独立 review 后必须记录 reviewer 线程，后续所有修复复审都通过 `followup_task` 发给同一个 reviewer。不要因为新 diff、修复了一轮 findings 或需要复审就再创建新的 reviewer，除非 reviewer 线程不可用或用户明确要求更换。
- `@explorer` 不是默认前置步骤。轻量代码查找、少量文件阅读、已知模块内的依赖盘点应由主 agent 或 owner 自己完成，依靠自动 compact 管理上下文。只有在调研范围跨多个模块、预计会读取大量无关上下文、需要并行探索多个方向、需要明确只读隔离，或主线程正在等待其他 owner/tester 且可以并行准备下一步时，才创建 `@explorer`。派发 explorer 时必须写清只读范围、问题清单和期望输出；交付时说明 explorer 已调用或跳过的原因。
- 仅修改 agent 指令、协作规则、spec 或 README 等文档时，如果用户明确允许简化流程，可以直接做文本级修改和验证，不强制创建 owner/reviewer/tester 流程；该例外不适用于产品代码、测试代码、构建配置、schema 或运行时行为改动。
- 验证 Rust 编译和测试时，不要为当前 worktree 配置独立的 `TARGET_DIR`；使用项目默认的共享 target 目录，避免把验证环境和常规开发/CI 环境分叉。
- 如果多个 Rust 测试或构建命令出现文件锁竞争，使用 `exec_command` 启动命令并通过 `command_wait` 等待通知；不要通过反复轮询、sleep 循环或持续检查进程状态来等待锁释放。
- Rust/Cargo/`just` 长时间验证命令一旦使用 `command_wait` 等待完成事件，当前验证流程必须进入静默等待：不要查询该命令状态、不要查看日志、不要启动替代测试、不要派发额外 reviewer/tester 重复验证同一结果。
- 同一任务中同一时间只允许一个会竞争 Rust 共享 target 或 Cargo 文件锁的长命令运行；在它完成前，不要连续启动新的 `cargo test`、`cargo check`、`cargo build`、`just fix` 或其他 Rust 验证命令。可以继续处理不依赖该命令结果、且不竞争 Rust/Cargo 资源的前端、文档或只读设计工作。

Rust 代码变更完成并通过 code review 后，默认只让固定 tester `/root/my_codex_pm/rust_cargo_tester` 串行执行两类验证；不要让 owner 或 reviewer 直接运行：

1. 修改模块的单元测试或最小 crate 测试。例如改 `codex-rs/tui` 时，include `cargo test -p codex-tui`；更窄的单测命令可用时优先用更窄命令。
2. 验证与入口匹配的 binary 能编译：只涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时，include `cargo build -p codex-app-server --bin codex-app-server` from `codex-rs`；只有确实改到 CLI/TUI 或 CLI app-server 子命令包装时，才 include `cargo build -p codex-cli` from `codex-rs`.

Do not run full workspace `cargo test`, `just test`, broad `just fix`, or snapshot/schema/lockfile workflows by default in every worktree. Add those commands only when the change specifically requires them or the user asks for broader validation.

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
- 不依赖 `codex-core` 的 command runtime primitive，例如 command output buffer、process
  state、wait/write-stdin DTO、notification filter/state 和 yield/token/chunk id helper，应放在
  `codex-rs/command-runtime`（`codex-command-runtime`）。`ExecCommandHandler`、
  `CommandWaitHandler`、`WriteStdinHandler`、approval/sandbox/spawn、async watcher event
  emission、`Session`/`TurnContext` 编排继续留在 `codex-core`。

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

When UI or text output changes intentionally, snapshot coverage may be needed. Owner can include the
snapshot workflow in the fixed tester `/root/my_codex_pm/rust_cargo_tester` followup request when the
change requires it; do not run it by default for every worktree:

- Generate any updated snapshots:
  - `cargo test -p codex-tui`
- Check what’s pending:
  - `cargo insta pending-snapshots -p codex-tui`
- Review changes by reading the generated `*.snap.new` files directly in the repo, or preview a specific file:
  - `cargo insta show -p codex-tui path/to/file.snap.new`
- Only if you intend to accept all new snapshots in this crate, have tester run:
  - `cargo insta accept -p codex-tui`

If tester doesn’t have the tool:

- Include `cargo install --locked cargo-insta` in the tester command list before snapshot commands.

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

### Development Workflow

- Update app-server docs/examples when API behavior changes (at minimum `app-server/README.md`).
- Regenerate schema fixtures when API shapes change if the change requires it. Owner can include:
  `just write-app-server-schema`
  (and `just write-app-server-schema --experimental` when experimental API fixtures are affected).
- For app-server protocol/runtime/root-worker backend startup changes, the default binary validation is
  `cargo build -p codex-app-server --bin codex-app-server`; use `cargo build -p codex-cli` only when
  the CLI/TUI entrypoint or `codex app-server` subcommand wrapper changed. Owner may add
  `cargo test -p codex-app-server-protocol` when protocol coverage is specifically needed.
- Avoid boilerplate tests that only assert experimental field markers for individual
  request fields in `common.rs`; rely on schema generation/tests and behavioral coverage instead.
