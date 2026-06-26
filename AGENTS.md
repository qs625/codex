# 项目工作指令

全程使用中文沟通和记录。专业名词可以保留 English，但整体表达要清晰自然。

## 命令执行

- 所有 shell 命令必须以 `rtk` 开头，例如 `rtk git status`、`rtk cargo test -p codex-thread-runtime`。
- `rtk` 不支持的复杂 shell 场景使用 `rtk proxy <cmd>`，例如 `rtk proxy find . -maxdepth 3 -type f`。
- 搜索文本优先用 `rg`，列文件优先用 `rg --files`。
- 构建、测试、格式化、lint 命令不要重定向到日志文件；直接让 stdout/stderr 进入 command session。
- 长时间 Rust/Cargo/`just` 验证命令启动后，通过 `command_wait` 等待完成通知；不要轮询、sleep 循环、重复查询或启动竞争同一 target 的第二个 Rust 命令。
- 同一 checkout 内同一时间只允许一个会竞争 Rust target/Cargo lock 的长命令运行。不要跨 checkout 共用 `TARGET_DIR`，也不要把 `codex-rs/target` 做成共享 symlink。

## Rust 通用规则

- Rust crate 命名遵循 workspace 现有约定和目标 owner 边界；新增拆分 crate 不要求统一使用 `codex-` 前缀。`codex-rs/core` 的 crate 名仍是旧兼容层 `codex-core`。
- 能在 `format!` 中 inline 的变量必须 inline。
- 遵循 clippy 常见约束：collapse nested `if`，避免 redundant closure，尽量使用 exhaustive `match`。
- 避免 bool 或含义不清的 `Option` 位置参数；必要时按 `argument_comment_lint` 在 opaque literal 前加精确参数名注释，如 `/*sandbox*/ None`。
- 新增 trait 必须写 doc comment，说明角色和实现预期。
- 不新增 Rust `unsafe` 来完成常规功能、重构或依赖拓扑优化；确实必须使用时，先说明原因、边界和验证方式并等待确认。
- 不鼓励 `#[async_trait]` 和 `#[allow(async_fn_in_trait)]`。trait 异步能力优先写成 RPITIT，并显式要求返回 future `Send`。
- 测试断言优先比较完整对象，避免逐字段零散比较。
- 不要新增只被调用一次的小 helper method。
- 新增 `include_str!`、`include_bytes!`、`sqlx::migrate!` 等编译期文件读取时，同步更新对应 `BUILD.bazel` 的 `compile_data`、`build_script_data` 或 test data。
- 修改 Rust 依赖时按需更新 `MODULE.bazel.lock`，需要时运行 `just bazel-lock-update` 和 `just bazel-lock-check`。

## 验证规则

- Rust 代码变更后默认串行执行两类验证：
  - 修改模块的单元测试或最小 crate 测试，例如 `rtk cargo test -p codex-tui`。
  - 涉及 app-server、runtime、protocol 或 root-worker 后端启动路径时，运行 `rtk cargo build -p codex-app-server --bin codex-app-server`。
- 只有改到 CLI/TUI 或 CLI app-server 子命令包装时，才运行 `rtk cargo build -p codex-cli`。
- 默认不要运行 full workspace `cargo test`、`just test`、宽泛 `just fix`、snapshot/schema/lockfile 命令；除非本次变更明确需要或用户要求。
- 如果 Rust async/integration 测试因默认 test harness 栈过小 stack overflow，优先把测试改成普通 `#[test]` wrapper，用 `std::thread::Builder::stack_size(...)` 创建大栈线程，并在线程内创建 tokio runtime 后 `block_on` 原 async 测试体。

## 当前拆分状态

- `codex-core` 是旧包名兼容层：`codex-rs/core/src/lib.rs` 只 re-export `codex-thread-runtime`。
- 当前 session/thread/turn/task/agent-control 等强耦合 runtime implementation 位于 `codex-rs/thread-runtime`。
- `codex-thread-runtime` 仍过重，后续重构目标是把 tool、MCP、workflow、goal、guardian、command 等 domain implementation 迁到对应 owner crate，而不是继续把它当新 core 扩张。
- 新增或迁移实现代码不要放回 `codex-rs/core/src`。
- 旧文档里提到的 `core/src/session`、`core/src/thread`、`core/src/tasks`、`core/src/agent`、`core/src/tools`、`core/src/mcp`、`core/src/compact` 等 runtime implementation 路径，当前应理解为 `codex-rs/thread-runtime/src/...` 的待继续拆分代码。

## 拆分硬规则

- 拆分目标是 domain implementation 迁到 owner crate，跨 domain 依赖只通过轻量 API trait/DTO contract 表达，composition root 负责注入 concrete implementation。
- service 的 `api crate` 只定义“该 service 自己对外提供的 API”；不要把该 service 依赖别人的 capability/port trait 放回自己的 `api crate`。
- 一个 service 依赖别的 domain 能力时，应直接依赖能力提供方的 `api trait`；不要在当前 service 的 `api crate` 中重新定义一层 `*TurnApi`、`*Host`、`*Port` 来反包对方能力。
- capability trait 由能力提供方实现，优先直接由大的 concrete runtime/service（如 `Session`、`TurnContext`、`ThreadManager`）实现；不要为了避免环依赖再创建大量细碎 adapter。
- `thread-runtime` 自身也应通过明确的 service/facade 对外暴露能力；`session-api` 中定义的接口应优先由现有 `ThreadManager`、`Session`、`TurnContext` 等对象直接实现，而不是再定义独立 wrapper 类型承接。
- service API 优先使用 trait object 和显式 capability 参数；不要为了传 runtime context 在 service API 上引入无意义的 `Turn`/`Session` 泛型。
- 如果某个 trait 不是该 service 对外提供的能力，而只是它构造或运行时依赖的外部能力，那么这个 trait 不属于该 service 的 `api crate`。
- 禁止伪拆分：不要复制或整体搬迁 `codex-core` / `codex-thread-runtime` 大块实现到另一个 `*-runtime` / `*-hosts` crate 后声明完成。
- Host 实现不是最终边界：`CoreToolDomainHost`、`CoreSessionToolRouter`、`session-tool-hosts` 或类似 concrete host/facade 只能是迁移中的未完成项。
- session/thread 依赖 tool、MCP、workflow、guardian、command、goal、agent control 等能力时，只能消费 owner API trait/DTO 或注入后的 trait object。
- 不要把 `Session`、`TurnContext`、`ThreadManager` 或 service registry 当作跨 crate IoC 容器传给 domain implementation。
- API crate 只用于跨 owner crate 的稳定 contract；同一 owner runtime 内部协作应保持 crate-private。
- 完成标准必须包括 implementation、相关测试和依赖门禁都迁到 owner crate，且原 owner 不再保留该 domain 的 handler、host、adapter、router facade 或执行逻辑。
- 新 owner crate 的依赖门禁同时检查生产和测试图：`cargo tree -p <crate> --edges normal` 与 `cargo tree -p <crate> --edges normal,dev` 都不能拉回不该依赖的 heavy runtime。
- owner crate 不应通过 facade/re-export 把 direct dependency 伪装成 indirect dependency。

## 当前优先级

- 优先继续拆 `codex-thread-runtime` 内 tool adapter：`src/tools`、`apply_patch_tool_host.rs`、`shell_tool_host.rs`、`unified_exec/tool_host.rs`、`code_mode_host.rs`、`plugins/request_plugin_install.rs` 等。
- `CoreApplyPatchHandlerHost` 仍是迁移中的 concrete session side-effect host；后续要按 capability 拆分或迁出，不要把它当最终边界。
- MCP adapter、workflow bridge、goal runtime、guardian/hook、compact、command/unified exec 继续按 capability contract 收缩，再整体迁往 owner crate。
- app-server、CLI、MCP server、core-api 等组合根应逐步从 `codex_core` import 切到真实 owner crate。

## Typed Display 与历史

- 对话/线程展示以 typed `EventMsg` 为 runtime/UI display source，`ThreadItem` 通过共享 `EventMsg -> ThreadItem` projector 生成。
- `ResponseItem` 只作为模型交互、context manager/provider history、compact、guardian 和模型可见工具输出的 source。
- 新增 event-command、schedule、collab、goal、command session 等展示项时，必须新增 display-capable `EventMsg` variant，并复用 `codex-rs/app-server-protocol/src/protocol/event_item_projection.rs` 的边界。
- 不要新增 display-only `ResponseItem` variant，不要从 marker 文本、assistant message JSON、raw response item、legacy envelope 或 live function call output 反解展示。
- `record_conversation_items` 只写 model-visible history；需要同时写模型上下文和客户端可见 live item 时，使用 dual-write helper，例如 `record_model_items_and_emit_display_events`。
- 业务代码不得直接通过 `send_event_raw` 发 conversation display item；先形成 typed display event，确实需要模型可见时再双写 `ResponseItem`。
- Local Compact 是默认路径：手动 `/compact`、`thread/compact/start` 和自动 context-limit compact 都走 `codex-rs/thread-runtime/src/compact.rs`，并持久化 `CompactedItem.replacement_history`。

## MultiAgent 与 Goal

- MultiAgent 运行时只保留 V2 工具和 typed child completion 路径；不要重新引入 V1 `send_input`/`resume_agent`、legacy completion watcher 或 raw child-completion fallback。
- `wait_agent` 只能等待 canonical typed subagent 更新；不能 drain mailbox，不能从 raw marker、assistant text 或 legacy JSON envelope 反解唤醒条件。
- `wait_agent` 和 `command_wait` 每次只等待当前 backoff window；timeout 后返回 running 并推进同一目标的下一次窗口，事件命中时 reset。
- root-worker Agent Tree 主状态必须消费后端 canonical `ThreadStatus` / `thread/status/changed`，不要从 items、raw marker、legacy JSON 或 children 递归推导。
- thread lifecycle 顺序固定为 pending input -> active goal continuation -> incomplete direct child -> wait command -> complete。
- child completion 只对 direct parent 生效；普通 non-management subagent 完成后必须向 parent 投递 typed child completion pending input。
- Goal 展示走 dedicated typed display lifecycle，primary path 是 `EventMsg::ThreadGoalUpdateCompleted -> ThreadItem::ThreadGoalUpdate`。

## Root-Worker 客户端

- app-server v2 thread/live 内容只能消费 typed `ThreadItem` / v2 payload。
- 不要从 `agentMessage.text`、`eventDrivenTool.text`、`<event_driven_tool>`、`<event_command>`、`<subagent_notification>` 或 inter-agent JSON envelope 反解 display item。
- slash 菜单中能表达为 Skill 的命令来自 Skills discovery；只有依赖 runtime/thread state 或客户端本地动作的命令才放入 builtin registry。
- live 模式下，已进入本地 live cache 的 thread 切换展示时只能使用持续接收的 live `ThreadItem`；`thread/read` 仅用于 cold start、缺失本地 thread 或显式恢复。
- conversation item 合并只能按 `ThreadItem.id` 判断同一 item；不同 id 必须保留。

## 文件体量与模块边界

- 优先添加新模块而不是继续放大大文件。
- Rust 模块目标控制在约 500 LoC 内，测试除外；超过约 800 LoC 的文件除非有明确理由，否则不要继续添加新功能。
- 高触达文件尤其要克制：`codex-rs/tui/src/app.rs`、`codex-rs/tui/src/bottom_pane/chat_composer.rs`、`codex-rs/tui/src/bottom_pane/footer.rs`、`codex-rs/tui/src/chatwidget.rs`、`codex-rs/tui/src/bottom_pane/mod.rs`。
- 从大模块抽代码时，把相关测试、类型文档和 invariants 移到新 owner 文件附近。

## TUI 约定

- ratatui style 优先用 `"<text>".dim()`、`"text".bold()`、`"text".fg(Color::...)`，少用手写 `Style`。
- 默认 foreground 使用 inherited color，不要显式写 `Color::White`。
- text wrapping 使用 `textwrap::Options`，不要手写 wrapping。
- snapshot 变更需要用 `cargo insta review` 审核；工具不存在时安装 `cargo-insta`。
- snapshot 只接受本次意图内的变更；大范围 snapshot 更新要逐个确认。

## App-Server API

- API 文档只维护在 `docs/app-server-api/`。
- App-server v2 协议类型在 `codex-rs/app-server-protocol`。
- 新 request/notification 必须同步 schema、TypeScript 生成物、文档和验证测试。
- 请求 payload 命名使用 `*Params`，保持 Rust/TypeScript schema 一致。
- app-server 边界不要重新持有 concrete `Arc<CodexThread>` / `Arc<ThreadManager>` 作为普通 domain 依赖；需要 object-safe facade 时把 future boxing 限制在 app-server 边界。

## 文档边界

- 不要在 `docs/` 下新增通用产品或用户文档；官方 Codex 文档在别处。
- 例外是 app-server API 文档，按上面的 app-server 规则维护。
