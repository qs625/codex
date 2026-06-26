# PM Progress

## 当前目标

继续推进 thread 相关重构。当前重点是把公开边界统一到 `ThreadService` / `codex-thread-api`，并继续收缩 `codex-thread-runtime` 中残留的 session 命名与跨 domain 耦合，防止它变成新的大 core。

## 当前阶段

- id: `core-crate-topology-refactor-plan`
- status: active
- checkout: `/Users/bytedance/Projects/my-codex`
- current step: `6D-thread-service-public-boundary`
- branch: `feat/tool-callback`

## 当前仓库事实

- `codex-rs/core/src/lib.rs` 只是旧包名兼容层，当前只 re-export `codex-thread-runtime`；不要把 implementation 放回 `codex-rs/core/src`。
- `codex-rs/thread-runtime` 已承接 session/thread/turn/task/agent-control 等旧 core runtime，但它仍是过重的 owner crate，需要继续按 domain 拆分。
- `spec/session-thread-service-architecture.md` 已记录目标架构：API 是 trait + DTO contract，Service 是 concrete implementation，Capability 是 session/thread orchestration 暴露给 domain service 的最小能力接口。
- `ThreadService` 已成为 thread runtime 的唯一公开 service 名称；旧 `ThreadManager` 语义正在从 runtime 与 sample 中清理。
- `codex-thread-runtime` 的公开 helper 类型名也开始向 thread 语义收口：对外优先暴露 `ThreadRuntimeSession`、`ThreadTurnContext`、`ThreadToolDomainHost`、`ThreadWorkflowCapability`、`ThreadToolRuntimeRouter`，不再鼓励下游继续从 runtime 入口直接依赖 `Session` / `TurnContext` 等旧命名。
- handler / workflow wiring 对应的 concrete 实现名也开始收口：`thread-runtime` 中承载 tool host 能力的 `SessionToolDomainHost` 已切成 `ThreadToolDomainHost`，workflow bridge capability 已切成 `ThreadWorkflowCapability`；相关 app-server、mcp-server、cli、core-test-support、thread-service-sample 与 runtime 自测 wiring 均已跟进。
- 目标设计不再引入泛化的完整 `session-api` / `thread-api` 给底层 domain crate 依赖；底层 service 只持有当前需要的 `Weak<dyn Capability>`。
- `SessionRuntime` / session handle 只面向 entrypoint、composition root 或同 crate facade；除非未来出现真正跨 owner 多实现需求，否则不额外抽 `SessionRuntimeApi`。
- `mcp-server`、`thread-service-sample`、`core-test-support`、`app-server-client`、`app-server-test-client`、`app-server-transport`、`app-server`、`memories-extension`、`cloud-tasks`、`linux-sandbox`、`exec`、`cli`、`lmstudio`、`ollama`、`chatgpt`、`memories-write` 已按整 crate 方式从 `codex-core` 切到 `codex-thread-runtime` 作为 runtime 类型入口，不再混用两套 crate 名。
- `app-server`、`mcp-server` 与 `thread-service-sample` 当前调用的 thread runtime 构造路径要求 concrete `StateDbHandle`；相关入口已统一回到直接传 `state_db.clone()`，不再依赖旧兼容入口下偶然可用的 trait object 分支。
- `cli/src/main.rs` 的 prompt-debug 等其他 consumer 若命中需要 `SharedStateDbRuntime` 的构造路径，仍应显式做 trait object 转换；是否转换取决于具体构造函数签名，而不是旧 crate 名。
- 当前 workspace 中已无 crate 继续通过 `codex-core` 作为 thread runtime 入口参与编译；剩余 `codex_core::...` 命中只在注释、文档示例或测试 fixture 文本中，不再是实际代码依赖。
- 最终 `thread-runtime` 只保留 session/thread/turn orchestration，不保留 Tool/MCP/Agent/Goal/Workflow/Command/Guardian/Hook/Skill/Plugin/Extension 等 domain 的 concrete service implementation；当前混在 `thread-runtime` 的相关模块都是迁移期遗留。
- 目标设计不再保留独立 `tool-runtime` service layer；ToolService 本身实现 tool runtime 能力，当前 `codex-rs/tool-runtime` 只能作为迁移期 crate 或纯底层 helper。
- `codex-thread-api` 现在是统一公开 API 面，并已直接承接原 `codex-session-api` 中的 runtime contract 定义，包括 `PendingInputItem`、`SessionToolRouter`、MCP call/resource、workflow、goal、agent-job 等 trait/DTO。
- `codex-session-api` compatibility crate、workspace member 和 workspace dependency 已删除；对外统一 contract owner 只保留 `codex-thread-api`。
- tool router wrapper 已从 `thread-runtime` 删除：生产路径通过 `codex-tool-handlers::SessionToolRouterAdapter` 注入 `SessionToolRouter`；原 `thread-runtime/src/tools/router.rs`、`thread-runtime/src/tools/router_tests.rs` 不再保留。session/thread 测试需要的 composition helper 已迁到 `thread-runtime/src/test_support.rs::TestToolService`，不再挂在 `tools` domain 下。
- `app-server`、`mcp-server`、`cli`、`core-test-support`、`thread-service-sample` 的 tool router factory 已切到上述 thread-facing 公开类型名；外部 consumer 不再直接使用 `codex_thread_runtime::Session`、`TurnContext`、`SessionWorkflowCapability`、`SessionToolDomainHost` 或 `CoreToolRuntimeRouter` 这些旧公开名字。
- 当前还保留 session 语义的重点位置，已经从 runtime public helper 名和 concrete host/capability 名，进一步收缩到少量 `thread-api` contract 名（如 `SessionGoalCaller` 等）以及 `thread-runtime` crate 内部文件/模块名；原 `SessionToolRouterFactory` contract 已删除，tool 注入边界统一到 `codex-tool-service-api::ToolServiceApi`。
- 普通 function tools 已脱离大 host；multi-agent tool handler 已迁到 `codex-agent-tool-handlers`，执行通过 `codex-agent-runtime::MultiAgentToolSession`。
- `CoreToolDomainHost` / `CoreApplyPatchHandlerHost` 旧公开边界已移除；apply-patch、shell、unified-exec command interaction 相关 session side effect 当前集中到 `SessionToolDomainHost`。这只是剩余粗 `ToolDomainHost` contract 的未完成项，不是最终边界。
- 第一条 ToolService 试点已继续推进：`CoreToolDispatchHost` 已从 `thread-runtime` 删除，tool owner crate 中的 `codex-tool-handlers::SessionToolDispatchHost` 依赖统一后的 `codex-thread-api::ToolSessionCapability`。该 capability trait 不带 `TurnContext` 泛型，也不使用 per-turn adapter；由大的 `Session` 本体实现，`TurnContext` 本体实现 `ToolTurnCapability` 作为当前 turn 的 API view。组合根创建 `ThreadService` / `thread-runtime` 时直接注入 `Arc<dyn ToolServiceApi<...>>`；`app-server`、`mcp-server`、`cli`、`thread-service-sample` 已删除本地 `*ToolRouterFactory` 并统一注入全局 `ToolService`。同时 `ToolService` 已改成显式持有 `WorkflowApi`，不再在旧 `build_tool_router()` 公开边界内隐式 `new WorkflowService`。当前公开 `ToolServiceApi` 已切成 `tool_specs(...)` / `dispatch_tool(...)` 两个入口；`thread-runtime` 当前 turn 直接持有 `PreparedToolSet`，不再额外构造 `ToolCallRuntime`。当前 `ToolInvocation` 仍携带 concrete `Arc<Session>` / `Arc<TurnContext>`，`ThreadToolDomainHost` 及其 runtime/orchestrator/event host 也仍留在 `thread-runtime`，真正完成还需要继续抽出这些 tool 所需 session side effect 并把 ToolService implementation 彻底迁出 `thread-runtime`。
- 为继续拆 `ThreadToolDomainHost`，`codex-thread-api` 已新增 `ToolRuntimeTurnCapability` / `ToolRuntimeSessionCapability`，先把 apply-patch、shell、unified-exec 当前真正依赖的运行期能力以 thread owner contract 形式显式收敛；`thread-runtime` 已由 `TurnContext` / `Session` 本体实现这些新 capability，并通过 `rtk cargo check -p codex-thread-api -p codex-thread-runtime --lib` 编绿。下一步是让 `tool-service` / `tool-handlers` 改为直接依赖这些 capability，随后删除 `ThreadToolDomainHost` 对 `tool-service` 的 concrete 绑定。
- `ToolService` 当前还保留对 `ThreadToolDomainHost` 与 tool runtime/orchestrator/event host 的 concrete 依赖，但全局 service 依赖已经先收正：`GoalApi`、`McpResourceApi`、`RequestPluginInstallApi` 不再由 `ToolService` 内部直接 `new`，而是改为在 composition root 显式注入。也就是说，`ToolService` 内部当前剩下的 thread-runtime 直接依赖，已经主要收缩到 tool 运行期 host 这条链。
- MCP tool call 装配也已经从 `apply_patch_host` 这条粗 host 链上拆开：`codex-tool-handlers::ToolRuntimeBuildParams` 现在单独接收 `mcp_tool_call_host`，`ToolService` 与 `thread-runtime test_support` 改为使用 `codex-thread-api::SessionMcpToolCallHost` 通用 capability host；`McpHandler` 不再通过 `ThreadToolDomainHost` 进入 session/runtime。这样 `ThreadToolDomainHost` 目前主要只剩 apply-patch、shell、unified-exec 这一条运行期能力链。
- tool 事件桥接也已从 `thread-runtime` 主路径抽离：`codex-thread-api` 新增 `SessionToolEventHost`、`SharedToolTurnDiffTracker` 以及 `ToolRuntimeSessionCapability` 上的 event emit contract，`thread-runtime` 改由 `Session` / `TurnContext` 本体实现这些能力；`session_tool_domain_host` 与 unified-exec watcher/process manager 已切到通用 `SessionToolEventHost`，原 `thread-runtime/src/tool_event_host.rs` 已删除。现在 `thread-runtime` 在 apply-patch/shell/exec 这条链上剩余的主要 tool-specific 代码集中到 `tool_orchestrator_host.rs` 与 `tools/runtimes/`。
- `request_plugin_install` 已从粗 `ToolDomainHost` 拆出：统一 owner API 现在通过 `codex-thread-api` 暴露 `SessionRequestPluginInstallCaller` / `SessionRequestPluginInstallHost`，由 `Session` 本体实现 caller，并通过 `ToolRuntimeBuildParams` 显式注入 request-plugin-install host；`SessionToolDomainHost` 不再实现 `RequestPluginInstallHost`。
- extension tool 也按 tool domain 处理：`thread-runtime` 只向 `ToolRouterBuildParams` 提供 extension contributor 和 session/thread extension data；extension executor 收集和 handler 装配由 `codex-tool-handlers` 内部完成，`thread-runtime/src/tools/extension_tools.rs` 已删除。`ToolRuntimeBuildParams` 不再要求 app-server、CLI、MCP server、core-api 或 thread-runtime test-support 预先收集 extension executors。
- `thread-runtime/src/function_tool.rs` 这个只 re-export `FunctionCallError` 的假 tool facade 已删除；thread-runtime 内部直接引用 `codex_tool_types::FunctionCallError`，避免继续把 tool 类型包装成 thread-runtime 自有模块。
- `thread-runtime/src/tools/context.rs` 这个只承载 concrete `ToolInvocation` alias 的 facade 已删除；thread-runtime 测试和 trace 代码直接使用 `codex_tool_runtime::ToolInvocation<Arc<Session>, Arc<TurnContext>, SharedTurnDiffTracker>` 或 owner crate DTO。
- `thread-runtime/src/tools/events.rs`、`thread-runtime/src/tools/orchestrator.rs` 已删除；session-owned tool event/orchestrator capability implementation 移到 `thread-runtime/src/tool_event_host.rs` 和 `thread-runtime/src/tool_orchestrator_host.rs`。旧 `ToolOrchestrator` wrapper 不再保留，调用点直接使用 `codex_tool_runtime::ToolOrchestrator`。
- `thread-runtime/src/shell_tool_host.rs`、`thread-runtime/src/unified_exec/tool_host.rs` 已删除；对应 impl 合并进 `thread-runtime/src/session_tool_domain_host.rs`，避免继续保留分散的 tool host facade。
- `thread-runtime/src/tools/registry.rs` 已降为 `cfg(test)` 单元测试辅助，不再作为 `test-support` feature 暴露的 tool runtime surface。

## 下一步优先级

1. 继续拆 `SessionToolDomainHost` / `ToolDomainHost` 粗 contract：request-plugin-install 已完成第一块拆出；下一步把 apply-patch、shell、exec-command、command interaction、code-mode 所需 session side effect 分别下沉到统一 owner API 中的窄 capability，并让 `Session` / `TurnContext` 本体直接实现。
2. 将 ToolService implementation 从 `thread-runtime/src/tools` / `*_tool_host.rs` 迁到 tool owner crate；`thread-runtime` 只保留注入和 capability impl。
3. 盘点并迁出 `mcp/`、`agent/`、`goal/`、`workflow_*`、`unified_exec/`、`network_approval.rs`、`guardian/`、`plugins/`、`skills/` 等非 session/thread service implementation。
4. 把 `SessionServices` 从平铺字段拆成迁移期 domain bundles，明确每个 bundle 的目标 owner crate 和待删除 adapter。
5. `codex-core` / `codex-session-api` 兼容注册已删完；下一阶段集中清理 `thread-runtime` crate 内部残留的 `Session*` / `TurnContext` / `SessionTool*` 文件名与实现命名，并推动 runtime 内部 domain implementation 迁出 `thread-runtime`。

## Guardrails

- 不做伪拆分：不得把 `codex-thread-runtime` 大块复制或搬到另一个 `*-runtime` / `*-hosts` crate 后声明完成。
- Host 不是边界：`Session` / `TurnContext` / `ThreadManager` 不得作为跨 crate IoC 容器传给 domain implementation。
- 底层 service 只能持有 `Weak<dyn Capability>`；不得持有 `Weak<SessionRuntime>`、`Weak<ThreadManager>` 或任何 concrete session/thread 类型。
- capability trait 必须窄，只包含当前 consumer 实际需要的方法；不抽泛化大 `SessionApi` / `ThreadApi`。
- 只被 entrypoint / CLI / app-server / composition root 直接消费的 concrete runtime/facade，不为统一形式强行抽 API contract。
- API crate 只承载跨 owner 的稳定 contract；同一 owner runtime 内部协作保持 crate-private。
- capability trait 放在能力提供方 API crate；当前对外统一 owner 为 `codex-thread-api`，不要再把新的能力定义放回任何 session 兼容层或 consuming service runtime crate。
- 完成标准必须同时满足 implementation、测试和依赖门禁：owner crate normal 与 normal,dev graph 都不能拉回不该依赖的 heavy runtime。
- 不新增 Rust `unsafe`；新增 trait 需要文档说明角色和实现预期；避免 `async_trait`，优先 RPITIT + `Send` future。

## 验证基线

最近已通过的关键验证包括：

- `cargo check -p codex-thread-runtime --lib`
- `cargo check -p codex-thread-runtime --features test-support --lib`
- `cargo check -p codex-thread-runtime --features test-support --lib -p codex-app-server -p codex-mcp-server -p codex-cli -p core_test_support -p codex-thread-service-sample`
- `cargo check -p codex-thread-runtime --features test-support --lib -p codex-app-server -p codex-mcp-server -p codex-cli -p core_test_support -p codex-thread-service-sample`
- `cargo check -p codex-mcp-server -p codex-thread-service-sample -p codex-app-server --bin codex-app-server --tests`
- `cargo check -p codex-thread-api -p codex-thread-runtime -p codex-tool-handlers -p codex-agent-tool-handlers -p codex-hooks -p codex-app-server -p codex-mcp-server -p codex-thread-service-sample -p core_test_support`
- `cargo check -p codex-mcp-server -p codex-thread-service-sample`
- `cargo check -p core_test_support`
- `cargo check -p codex-app-server-client -p codex-app-server-test-client -p codex-memories-extension -p codex-cloud-tasks -p codex-linux-sandbox`
- `cargo check -p codex-exec`
- `cargo check -p codex-cli`
- `cargo check -p codex-app-server-transport --tests`
- `cargo check -p codex-lmstudio -p codex-ollama -p codex-chatgpt -p codex-memories-write --tests`
- `cargo check -p codex-app-server --bin codex-app-server --tests`
- `cargo check -p core_test_support -p codex-app-server --bin codex-app-server`
- `cargo check -p codex-tool-handlers --lib`
- `cargo test -p codex-thread-runtime dispatch_lifecycle_trace --no-fail-fast`
- `cargo test -p codex-thread-runtime fatal_tool_error_stops_turn_and_reports_error --no-fail-fast`
- `cargo test -p codex-thread-runtime handle_output_item_done_returns_contributed_last_agent_message --no-fail-fast`
- `cargo test -p codex-tool-handlers extension_tools_do_not_replace_builtin_tools --no-fail-fast`
- `cargo check -p codex-app-server --bin codex-app-server`
- `cargo check -p codex-cli --lib`
- `cargo check -p codex-mcp-server --lib`
- `cargo check -p codex-tool-runtime-api --lib`
- `cargo check -p codex-agent-tool-handlers --lib`
- `cargo build -p codex-app-server --bin codex-app-server`
- `cargo tree -p codex-tool-handlers --edges normal | rg "codex-core v|codex-thread-runtime"` 无命中
- `cargo tree -p codex-tool-handlers --edges normal,dev | rg "codex-core v|codex-thread-runtime"` 无命中
- `cargo tree -p codex-thread-api --edges normal | rg "codex-core v|codex-thread-runtime"` 只应保留 thread runtime 允许的边界，不再检查已删除的 `codex-session-api`
- `cargo tree -p codex-thread-api --edges normal,dev | rg "codex-core v|codex-thread-runtime"` 只应保留 thread runtime 允许的边界，不再检查已删除的 `codex-session-api`
- `cargo tree -p codex-tool-runtime-api --edges normal | rg "codex-core v|codex-thread-runtime"` 无命中

后续 Rust 代码变更默认按修改范围运行最小 crate 测试或 `cargo check`，涉及 app-server/runtime/protocol/root-worker 后端启动路径时再运行 `cargo build -p codex-app-server --bin codex-app-server`。
