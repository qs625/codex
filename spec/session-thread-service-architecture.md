# Session/Thread Service 化架构

## 目标

把 `codex-thread-runtime` 从“新 core”收缩成 session/thread orchestration owner。各 domain 通过稳定 API trait 暴露能力，由 concrete service 实现，并在 composition root 中通过 IoC 注入给需要的 consumer。

本重构不是机械改名，而是建立可验证的依赖方向：

```text
API = trait + DTO/request/response contract
Service = concrete implementation of API
Capability = session/thread orchestration 暴露给 domain service 的最小能力接口
Session/Thread = orchestration，不作为跨 domain IoC 容器
```

API 抽取准则：

- 跨 owner crate、需要多实现、或需要被底层 domain service 依赖的边界，才抽成 API contract。
- 只被 entrypoint、CLI、app-server 或 composition root 直接消费的 concrete runtime/facade，不需要为了“统一”额外抽 API。
- `SessionRuntime` / session handle 默认不是 API contract；底层 crate 只能依赖当前需要的窄 capability traits。
- capability trait 由“能力提供方”的 API crate 拥有：当前统一对外 owner 为 `thread-api`。session 提供给 tool/MCP/agent/workflow 的能力和 thread 提供的 spawn/shutdown/lookup 能力都应收敛到这个统一边界；不要把 session capability 放进 tool service/runtime crate，也不要留在 `thread-runtime` 内部当长期边界。
- service 的 `api crate` 只定义“该 service 自己提供的 API”；不要把该 service 运行时依赖外部的 capability trait 混进自己的 `api crate`。
- 一个 service 依赖其他 domain 时，应直接依赖对方 owner API crate 中的 trait；不要在当前 service 的 API 中再定义 `WorkflowTurnApi`、`ToolHost` 这类反向包装接口。
- service API 默认使用 trait object 和显式 capability 参数，不在 API 层携带 `Turn`、`Session` 一类 runtime context 泛型；具体 runtime context 由能力提供方通过自己的 capability trait 暴露。
- `thread-runtime` 自己也要有清晰的 service/facade 边界；thread/session owner API 中的 trait 优先由现有 `ThreadService`、`Session`、`TurnContext` 等对象直接实现，不再额外引入独立 capability wrapper 类型承接这些接口。

### Service API 与 Runtime Capability API

必须严格区分两类边界：

- `Service API`：全局 domain service 对外暴露的业务能力接口。
- `Runtime Capability API`：当前 live thread/session/turn 在一次运行期调用里提供的最小能力接口。

全局 service API 的粒度规则：

- 默认按 domain 保持“一个 service 一个 API”。
- 不要把全局 service API 为了概念整洁拆成很多细碎小 trait。
- 只有当某个 API 明显跨 owner、需要独立 mock/替换、或已经形成稳定子域时，才额外拆分。
- 与之相对，运行期 capability API 可以保持更窄，因为它表达的是当前 live runtime 的最小能力面。

判定规则：

- 脱离当前 live turn 仍然成立的能力，属于 `Service API`。
- 依赖当前 turn、当前 session 状态、当前 display/event sink、当前 permission/environment 的能力，属于 `Runtime Capability API`。
- handler 的长期依赖通过构造函数注入 `Service API`。
- handler 的本次调用期依赖通过 `dispatch request` 或 invocation context 传入 `Runtime Capability API`。

禁止混用：

- 不要把当前 turn/session 的运行期状态方法塞进全局 `Service API`。
- 不要让全局 `Service API` 反向依赖某个具体 `Session` / `TurnContext`。
- 不要把 `ToolHost`、`WorkflowHost` 这类“全局 service 依赖 + 运行期 capability + concrete runtime” 混成一个大接口传给 handler。

对 tool domain 的直接约束：

- `ToolService` 是全局 service，负责 tool discovery、tool assembly 和 tool dispatch。
- `ToolService` 对外主入口只有 `dispatch_tool(...)`；`tool_specs(...)` 作为 tool 规格/发现接口保留。
- `ToolService` 内部的不同 handler 可以依赖不同的全局 `Service API`，例如 `AgentApi`、`WorkflowApi`、`CommandExecutionApi`、`McpToolApi`。
- 当前 live thread/session/turn 的 event、hook、goal accounting、display emit、权限/环境等运行期能力，不属于这些全局 service API，而属于 `ToolSessionCapability` / `ToolTurnCapability` 一类 runtime capability。

命名约定：

- 目标架构、API、service、port 和未来 crate 示例不使用产品名前缀；优先使用 `thread-runtime`、`tool-api`、`ToolService` 这类 domain 名。
- 只有描述当前仓库事实、现有路径、现有 crate 或验证命令时，才保留真实 `codex-*` / `codex-rs/...` 名称。
- 新增 API trait 不要使用 `Codex*` 前缀；旧类型可保留 compatibility alias，但不要在新边界中继续扩散。

## 当前概念边界

### Thread

Thread 是产品、持久化和外部 API 概念：

- `thread_id`、metadata、status、history、resume、shutdown。
- app-server v2、CLI、TUI 等外部入口主要面向 thread。
- `ThreadService` 负责多 thread 生命周期、registry、恢复、组合根资源装配。
- `CodexThread` 当前是单 thread facade。

### Session

Session 是 runtime 执行概念：

- 驱动某个 thread 当前活着的一次 runtime 实例。
- 负责 submission loop、turn lifecycle、model call、tool dispatch、mailbox、goal continuation、active task、event emission。
- `Session` 当前是 runtime aggregate，`TurnContext` 是单 turn runtime context。

两者应继续分开：thread identity 可持久化和恢复，session runtime 可创建、关闭、替换或迁移。

## 最终态边界

最终 `thread-runtime` 只保留 session/thread/turn orchestration：

- thread/session lifecycle、turn loop、model call sequencing、pending input、mailbox、active task、post-turn scheduling。
- 持有 `Arc<dyn XxxApi>` 调用 domain service。
- 实现 `thread-api` 中的窄 capability traits，供 domain service 通过 `Weak<dyn Capability>` 回调。

最终 `thread-runtime` 不保留其他 domain 的 concrete service implementation：

- ToolService：tool registry、tool dispatch runtime、apply-patch/shell/unified-exec/code-mode tool host、tool event/orchestrator runtime。
- McpService：MCP call/resource/OAuth/approval/elicitation/runtime adapter。
- AgentService / GoalService / WorkflowService：agent control tool runtime、goal persistence/runtime policy、workflow run bridge。
- CommandService：command session/process manager / process runtime。
- ApprovalService：cached approval、guardian review、user approval、permission hook、network approval decision。
- SandboxService：sandbox selection、sandbox transform、managed network sandbox preparation。
- Hook/Skill/Plugin/Extension service：hook execution、skill/plugin discovery/render/injection、extension executor glue。

当前 `codex-rs/thread-runtime` 中这些模块仍存在，只能视为迁移期遗留或 composition glue，不能作为最终 owner 边界：

- `tools/`、`session_tool_domain_host.rs`、`code_mode_host.rs`
- `mcp/`
- `agent/`
- `goal/`
- `workflow_runs.rs`、`workflow_tool_host.rs`、`workflows/`
- `unified_exec/`、`network_approval.rs`、`shell_escalation_adapter.rs`
- `guardian/`
- `plugins/`、`skills/`、`connectors.rs`

迁出完成标准：

- owner service implementation 已迁到对应 owner crate。
- `thread-runtime` 只保留构造注入、orchestration callback impl 或短期 adapter，并且 adapter 在 progress 中标为未完成。
- owner crate normal 和 normal,dev graph 不拉回 `codex-thread-runtime` / `codex-core`。

## Thread/Session struct 定义

本节定义当前 thread/session 相关 struct 在目标架构中的职责，以及它们应该实现或消费的 trait/capability。这里的 API 只在跨 owner crate 或底层 service 需要依赖时才抽取；entrypoint 直接消费 concrete handle 时不强制抽 trait。

### ThreadService

当前位置：

- `codex-rs/thread-runtime/src/thread/manager.rs`

目标定义：

- 多 thread lifecycle 与 composition root。
- 拥有 live thread registry、thread store、auth/runtime factories、spawn/resume/shutdown 装配逻辑。
- 可以作为 concrete facade 被 app-server/CLI 直接使用。

应实现的 capability/API：

- `ThreadSpawnApi`：给 `AgentService` / `WorkflowService` 创建 child thread。
- `ThreadShutdownApi`：给需要关闭 child thread 的 service 使用；不需要时不抽。
- `LiveThreadLookupApi`：只有 app-server 或其他 crate 需要跨 owner 查找 live thread 时才抽。

不应做的事：

- 不把完整 `ThreadService` 暴露给底层 service。
- 不抽大而全的 `ThreadApi`。

### ThreadServiceState

当前位置：

- `codex-rs/thread-runtime/src/thread/manager.rs`

目标定义：

- `ThreadManager` 的内部 shared state。
- 保存 live thread map、thread created channel、thread store、runtime factories、skills/plugins/MCP managers 等装配资源。

应保持：

- crate-private。
- 不作为 API contract。
- 不被 domain service 直接持有。

### ThreadHandle

当前类型：

- `CodexThread`

当前位置：

- `codex-rs/thread-runtime/src/thread/codex.rs`

目标定义：

- 单 thread 的对外 handle。
- 面向 entrypoint 提供 submit、shutdown、runtime status、resume lifecycle、goal resume 等操作。

应实现的 capability/API：

- 通常不需要单独 API；app-server/CLI 可以直接持有 concrete handle。
- 如果未来外部 service 只需要极小能力，再抽 `ThreadSubmitApi` / `ThreadStatusApi` 这类窄 trait。

不应做的事：

- 不作为底层 domain service 的依赖对象。
- 不把内部 `SessionRuntime` 全量暴露出去。

### SessionRuntime

当前类型：

- `Session`

当前位置：

- `codex-rs/thread-runtime/src/session/session.rs`

目标定义：

- 单 thread 当前 live runtime 的执行核心。
- 拥有 active turn、mailbox、goal runtime、guardian、workflow run controller、domain service references、event emission、submission loop 所需 state。
- 负责实现当前需要的窄 session capability traits。

应实现的 capability/API：

- `ToolSessionCapability`：tool service 需要的 event、hook、goal-accounting 等能力。
- `McpSessionCapability`：MCP service 需要的 approval、elicitation、display event、metadata 等能力。
- `AgentSessionCapability`：agent service 需要的 parent thread、child completion、agent event、wait backoff 等能力。
- `GoalSessionCapability`：goal service 需要的 display update、goal context injection 等能力。
- `WorkflowSessionCapability`：workflow service 需要的 workflow progress event、workflow agent spawn 等能力。
- `CommandSessionCapability`：command service 需要的 command event、process id allocation、stdin/wait display 等能力。

不应抽取：

- 不抽 `SessionRuntimeApi` 给底层 domain crate。
- 不抽大而全的 `SessionApi`。

依赖规则：

- `SessionRuntime` 可以持有 `Arc<dyn ToolApi>`、`Arc<dyn AgentApi>`、`Arc<dyn McpToolApi>` 等 domain APIs。
- domain service 只能持有 `Weak<dyn XxxSessionCapability>`，不能持有 `Weak<SessionRuntime>`。

### SessionConfiguration

当前位置：

- `codex-rs/thread-runtime/src/session/session.rs`

目标定义：

- session runtime 的配置快照与配置更新逻辑。
- 包含 model/provider、collaboration mode、approval policy、permission profile、cwd/workspace roots、dynamic tools、environment 等。

应保持：

- 配置/state 类型，不命名为 service。
- 不作为跨 domain API。
- 更新逻辑可继续迁往 config owner，但不参与 service back-reference 设计。

### SessionState

当前位置：

- `codex-rs/thread-runtime/src/state/session.rs`

目标定义：

- session-scoped mutable state。
- 包含 history/context manager、rate limit、dependency env、MCP prompted set、thread skills、connector selection、granted permissions 等。

应保持：

- state 类型，不命名为 service。
- 不直接暴露给 domain service。
- 需要暴露能力时通过窄 capability 方法，例如 history record、context snapshot、permission merge。

### SessionServices

当前位置：

- `codex-rs/thread-runtime/src/state/service.rs`

目标定义：

- 迁移期 service bag。
- 当前混合了 MCP、command、sandbox、hooks、model、skills/plugins/extensions、agent control、thread store、telemetry、tool router 等多 domain resource。

重构方向：

- 先拆成 crate-private domain bundles，降低平铺字段复杂度。
- 最终由明确的 domain service references 替代，例如 `Arc<dyn ToolApi>`、`Arc<dyn McpToolApi>`、`Arc<dyn AgentApi>`。
- 不作为跨 crate API。

### SessionHandle

当前类型：

- `Codex`

当前位置：

- `codex-rs/thread-runtime/src/session/mod.rs`

目标定义：

- submit/event queue facade。
- 持有 `Arc<SessionRuntime>` 和 session loop termination handle。
- 面向 entrypoint 使用。

应保持：

- concrete handle 即可。
- 不为统一形式抽 API，除非未来确实出现多个 runtime handle implementation。

### TurnContext

当前位置：

- `codex-rs/thread-runtime/src/session/turn_context.rs`

目标定义：

- 单 turn runtime context。
- 包含 turn id、cwd/environment、model info、sandbox/permission profile、telemetry、extension data、tool context 等。

应保持：

- context 类型，不命名为 service。
- 不被 owner service crate 直接持有；跨 crate 需要的字段通过 request DTO 或 capability 方法传递。

### ActiveTurn / RunningTask / ActiveTasks

当前位置：

- `codex-rs/thread-runtime/src/state/turn.rs`

目标定义：

- active turn 和 running task 的 runtime state。
- 归 `SessionRuntime` 内部编排使用。

应保持：

- crate-private state。
- 不作为 API contract。
- 不给 domain service 直接依赖。

### SessionTaskContext / SessionTask

当前位置：

- `codex-rs/thread-runtime/src/tasks/mod.rs`

目标定义：

- task runner 的内部上下文和任务 trait。
- 驱动 regular turn、review、compact、user shell command 等 session task。

应保持：

- thread-runtime 内部 orchestration trait。
- 不作为 domain service API。

## 目标分层

```text
clients / entrypoints
  app-server / cli / mcp-server / tui / tests

composition root
  ThreadManager + factories
  构造 domain services，把 Weak<dyn Capability> 回指注入给需要 session/thread 能力的 service

session-thread orchestration
  ThreadManager -> ThreadHandle -> SessionRuntime -> TurnContext
  只消费 trait object，不依赖 domain concrete implementation

session/thread capabilities
  ToolSessionCapability / McpSessionCapability / AgentSessionCapability / ThreadSpawnApi / ...
  由 SessionRuntime / ThreadManager 或轻量 adapter 实现，通过 Weak<dyn Capability> 注入给 domain service

api / port crates
  tool-api / mcp-api / agent-api / workflow-api / capability traits / ...
  trait + DTO + request/response

domain service crates
  tool-service / mcp-service / command-service / ...
  struct XxxService impl XxxApi

foundation crates
  protocol / config-types / tool-types / rollout-api / state-api / utils
```

## 断环规则

禁止形成这种构造环：

```text
SessionService { tool: Arc<dyn ToolApi> }
ToolService { session: Arc<dyn SessionApi> }
```

不要用完整 `SessionApi` / `ThreadApi` 解决环依赖。正确做法是把“完整 session/thread 能力”拆成当前需要的窄 capability traits，由大的 `SessionRuntime`、`ThreadManager` 或对应 service 本体直接实现。底层 service 持有 `Weak<dyn Capability>`，而不是依赖 concrete `SessionRuntime` / `ThreadManager`。

目标形态：

```text
SessionRuntime
  tool: Arc<dyn ToolApi>

ToolService
  session: Weak<dyn ToolSessionCapability>

impl ToolSessionCapability for SessionRuntime
```

构造顺序：

```text
1. 创建 SessionRuntime / ThreadManager 的 Arc 壳，或创建可后填充的 service holder。
2. 把 Arc<SessionRuntime> / Arc<ThreadManager> upcast 成 Arc<dyn Capability>。
3. 把 Weak<dyn Capability> 注入 ToolService、McpService、AgentService 等 domain services。
4. 把 domain service trait objects 注入 SessionRuntime。
```

依赖方向：

```text
SessionRuntime -> ToolApi
ToolService -> Weak<dyn ToolSessionCapability>
ToolSessionCapability impl lives in SessionRuntime / Session service itself
```

这不是“隐藏环”：底层 service 只依赖 trait object，不依赖 concrete session/thread 类型；`Weak` 只表达 runtime 回指和生命周期可能失效。`upgrade()` 失败必须返回明确错误，不能 panic 或静默丢事件。

### Capability 与 Weak 边界

不创建额外 core 层。直接由 `SessionRuntime` / `ThreadManager` 或对应 service 本体实现窄 capability，底层 service 只持有 `Weak<dyn Capability>`。不要为了规避环依赖长期维护一组小 adapter；当 capability 增长时，应让大的 owner service 直接实现接口。

允许：

- `ToolService { session: Weak<dyn ToolSessionCapability> }`
- `AgentService { session: Weak<dyn AgentSessionCapability>, thread_spawn: Weak<dyn ThreadSpawnApi> }`
- capability trait 只包含当前服务实际需要的方法。
- capability impl 优先由 `SessionRuntime` / `ThreadManager` / owner service 本体实现。

禁止：

- 底层 service 持有 `Weak<SessionRuntime>` 或 `Weak<ThreadManager>` concrete 类型。
- 抽一个泛化大 `SessionApi` / `ThreadApi` 给所有 domain 依赖。
- 用 `Weak` 暴露任意 session 方法；capability trait 必须窄。
- `upgrade()` 失败后 panic 或吞掉错误。

## 目标 crate 依赖树

以下树状图描述目标依赖方向，不表示当前代码已经完全达到。树中上层 crate 可以依赖下层 crate；下层 crate 不应反向依赖上层 crate。

### Thread / Session

```text
app-server / cli / mcp-server / tui
└── thread-runtime
    ├── thread-lifecycle-api
    ├── session-capability traits
    ├── agent-api 或 agent-runtime(api 部分)
    ├── tool-api
    ├── mcp-api
    ├── workflow-api
    ├── command-service
    ├── context-service
    ├── thread-store-api
    ├── state-api
    └── foundation crates
        ├── protocol
        ├── config-types
        ├── tool-types
        ├── rollout-api
        └── utils-*
```

目标：

- `thread-runtime` 是 orchestration owner，不是 domain implementation owner。
- 旧 core facade 不出现在新依赖树中，只保留 compatibility re-export。

### Tool

```text
thread-runtime
└── tool-api
    ├── tool-types
    ├── tool-planning
    └── foundation crates

tool-service
├── tool-api
├── session-api（ToolSessionCapability / ToolTurnCapability）
├── agent-api 或 agent-runtime(api 部分)
├── mcp-api
├── workflow-api
└── foundation crates
```

目标：

- `tool-service` 不依赖 `thread-runtime`。
- `tool-service` 本身实现 tool 执行能力，不再拆出单独的 `tool-runtime` crate 作为中间层。
- tool 实现需要 session 能力时，只依赖 `ToolSessionCapability` 这类 port API。
- extension tool 属于 tool domain：extension registry/data 由 session/extension owner 提供，extension executor 收集和 tool 装配由 `tool-service` 完成，session runtime 不保留 extension tool implementation。

### MCP

```text
thread-runtime
└── mcp-api
    ├── mcp-types
    ├── mcp-tool-types
    └── foundation crates

mcp-service
├── mcp-api
├── mcp-types
├── mcp-tool-types
├── mcp-session-capability trait
└── foundation crates

mcp-client
├── mcp-api
├── mcp-types
└── transport/client crates
```

目标：

- MCP adapter implementation 归 MCP owner crate。
- session 只提供 `McpSessionCapability`，不把完整 `Session` / `TurnContext` 传给 MCP。

### Command / Approval / Sandbox

```text
thread-runtime
├── command-service
├── approval-api
└── sandbox-api

command-service
├── process-exec
├── command-display
├── protocol
└── foundation crates

approval-service
├── approval-api
├── execpolicy-api
└── foundation crates

sandbox-service
├── sandbox-api
├── execpolicy-api
├── sandboxing-api
└── foundation crates
```

目标：

- command process/session implementation 归 command owner。
- approval/cached-approval/guardian/user-review implementation 归 approval owner。
- sandbox selection/transform implementation 归 sandbox owner。
- session 只负责 turn lifecycle 和 typed event sink。

### Agent / Goal

```text
thread-runtime
├── agent-service
├── agent-tool-service
└── state-api

agent-tool-service
├── agent-api
├── tool-api
└── foundation crates

agent-service
├── state-api
├── thread-lifecycle-api
├── agent-session-capability trait
└── foundation crates
```

目标：

- `agent-tool-service` 不依赖 `thread-runtime`。
- agent/goal 需要 thread/session 编排能力时，只依赖 `AgentSessionCapability` / `ThreadSpawnApi` / goal state port。

### Workflow

```text
thread-runtime
└── workflow-api

workflow-service
├── workflow-api
├── agent-api
└── foundation crates
```

目标：

- workflow run/controller/progress implementation 归 workflow owner。
- session 只注入 workflow API，并提供 workflow session capability。

### Context / Persistence / Telemetry

```text
thread-runtime
├── context-service
├── context-usage
├── thread-store-api
├── state-api
├── rollout-api
├── rollout-trace-api
├── session-telemetry-api
└── analytics-api
```

目标：

- history/compact/context usage 归 context owner。
- persistence/telemetry 是底层服务，可被 session/thread orchestration 持有，但不依赖 domain service。

### Extension / Skill / Plugin

```text
thread-runtime
├── skill-api
├── plugin-api
├── extension-api
└── plugin-types

skill-service
└── skill-api

plugin-service
└── plugin-api
```

目标：

- discovery/render/injection 规则归对应 owner。
- session 只消费 skill/plugin/extension API 和 turn/session capability。

## API contract 草案

本节是目标 API 形状草案。真实落地时应把 DTO 放在对应 `*-api` crate 中，并根据 object-safety 选择 RPITIT 或 boxed future facade。

### Shared Capability Traits

```rust
pub trait EventSink: Send + Sync {
    fn emit_event(&self, event: EventMsg) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn emit_item_started(&self, item: TurnItem) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn emit_item_completed(&self, item: TurnItem) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub trait HistoryStore: Send + Sync {
    fn record_model_items(&self, items: Vec<ResponseItem>)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn snapshot(&self) -> impl Future<Output = ContextManager> + Send;
}

pub trait TurnStateStore: Send + Sync {
    fn turn_id(&self) -> &str;
    fn truncation_policy(&self) -> TruncationPolicy;
    fn cancellation_token(&self) -> CancellationToken;
}

pub trait MailboxStore: Send + Sync {
    fn enqueue_pending_input(&self, input: PendingInputItem)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn notify_mailbox_changed(&self);
}
```

这些 trait 是可复用 capability 示例，不要求先创建新的 core crate。第一阶段应由 `SessionRuntime`、`ThreadManager` 或对应 owner service 本体直接实现；若迁移期不得不保留 wrapper，必须在 progress 中标为未完成项。

### Thread Lifecycle API

```rust
pub trait ThreadLifecycleApi: Send + Sync {
    fn start_thread(&self, request: StartThreadRequest)
        -> impl Future<Output = anyhow::Result<StartedThread>> + Send;
    fn resume_thread(&self, request: ResumeThreadRequest)
        -> impl Future<Output = anyhow::Result<StartedThread>> + Send;
    fn shutdown_thread(&self, thread_id: ThreadId)
        -> impl Future<Output = anyhow::Result<ThreadShutdownReport>> + Send;
}

pub trait ThreadRegistryApi: Send + Sync {
    fn get_thread(&self, thread_id: ThreadId)
        -> impl Future<Output = Option<Arc<dyn ThreadHandleApi>>> + Send;
    fn list_live_threads(&self) -> impl Future<Output = Vec<LiveThreadInfo>> + Send;
}

pub trait ThreadSpawnApi: Send + Sync {
    fn spawn_child_thread(&self, request: SpawnChildThreadRequest)
        -> impl Future<Output = anyhow::Result<SpawnedChildThread>> + Send;
}
```

这些 trait 可以放在 `thread-lifecycle-api` 或当前 `thread-api` 的瘦身后继中。它们面向 composition root 和需要 thread lifecycle 能力的 service，不是完整 `ThreadManager` API。

### Session Runtime Handle

`SessionRuntime` / session handle 不作为目标 API contract。它只面向 entrypoint、composition root 或同 crate 内部 facade，例如 submit、shutdown、runtime status 这类操作。

不要为 `SessionRuntime` 额外抽 `SessionRuntimeApi`，除非未来出现真正跨 owner crate 的多实现需求。底层 domain crate 不依赖 session handle；它们只能依赖下面的 capability API。

### Tool API

```rust
pub trait ToolApi: Send + Sync {
    fn dispatch_tool(&self, request: ToolDispatchRequest)
        -> impl Future<Output = Result<ToolDispatchOutput, FunctionCallError>> + Send;
    fn tool_specs(&self, request: ToolSpecRequest)
        -> impl Future<Output = anyhow::Result<Vec<ToolSpec>>> + Send;
}

pub trait ToolSessionCapability: Send + Sync {
    fn emit_tool_event(&self, event: ToolDisplayEvent)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn run_pre_tool_hooks(&self, request: PreToolHookRequest)
        -> impl Future<Output = PreToolUseHookOutcome> + Send;
    fn run_post_tool_hooks(&self, request: PostToolHookRequest)
        -> impl Future<Output = PostToolUseHookOutcome> + Send;
    fn account_goal_tool_completed(&self, tool_name: ToolName)
        -> impl Future<Output = Result<(), String>> + Send;
}
```

这里进一步固定一条实现规则：

- `ToolApi` 属于全局 `Service API`。
- `ToolSessionCapability` / `ToolTurnCapability` 属于运行期 `Capability API`。
- `ToolService` 负责根据 tool name 将 `dispatch_tool(request)` 分发到内部 handler。
- 每个 handler 的长期依赖在构造时显式注入对应的全局 service API；不要为所有 handler 统一传一个大 host。
- 每个 handler 的运行期上下文通过 `ToolDispatchRequest` 或内部 invocation context 传入；不要把当前 turn/session 的运行态长期保存在 handler 或 `ToolService` 字段中。

`ToolSessionCapability` 和 `ToolTurnCapability` 放在能力提供方 API crate，也就是 `session-api`；不要放到 tool runtime/api crate。`ToolService` 持有 `Weak<dyn ToolSessionCapability>` 或在 dispatch request 中接收 capability view。`ToolSessionCapability` 由当前 live runtime owner 直接实现；当前 turn 所需只读能力通过单独的 `ToolTurnCapability` API view 暴露，不让 `ToolSessionCapability` 带 `TurnContext` 泛型。

extension tool 构建参数只暴露 contributor 和 session/thread extension data：

```rust
pub struct ExtensionToolBuildParams<'a> {
    pub tool_contributors: &'a [Arc<dyn ToolContributor>],
    pub session_store: &'a ExtensionData,
    pub thread_store: &'a ExtensionData,
}
```

组合根和 session runtime 只传这组数据；把 contributor 转成 executable extension tool 的逻辑属于 `ToolService` 内部实现。

#### Tool 依赖矩阵

当前各类 tool 实现的目标依赖关系应整理为下表。这里的 `构造期 Service API` 表示长期持有的全局 service 依赖；`调用期 Capability API` 表示一次 `dispatch_tool(...)` 调用内传入的 live runtime 能力或只读 turn view。

| tool | 构造期 Service API | 调用期 Capability API | 备注 |
| --- | --- | --- | --- |
| `ApplyPatchHandler` / `apply_patch` | `ApprovalApi`、`SandboxApi` | `ToolSessionCapability`、`ToolTurnCapability` | tool 编排属于 `ToolService`；`apply_patch` 只依赖 approval/sandbox service 和当前 turn capability，不再保留 `ApplyPatchHandlerHost` / `ToolOrchestratorApi`。 |
| `ShellCommandHandler` / `shell_command` | `ApprovalApi`、`SandboxApi`、`CommandExecutionApi` | `ToolSessionCapability`、`ToolTurnCapability` | 现状混在 `ShellCommandHandlerHost` / `ShellExecutionHost`；目标是 tool-service 编排 approval+sandbox+command execution。 |
| `ExecCommandHandler` / `exec_command` | `ApprovalApi`、`SandboxApi`、`CommandExecutionApi` | `ToolSessionCapability`、`ToolTurnCapability` | unified exec 也按同样结构收敛；不应继续暴露 `ExecCommandHandlerHost` 大口子，也不再把 orchestrator 当独立 service。 |
| `CommandWaitHandler` / `command_wait` | `CommandExecutionApi` | `CommandSessionCapability` | `begin_command_wait`、`finish`、display event 属于 command runtime capability。 |
| `WriteStdinHandler` / `command_write_stdin` | `CommandExecutionApi` | `CommandSessionCapability` | 与 `command_wait` 同组。 |
| `McpHandler` / namespace MCP tools | `McpToolApi` | `McpSessionCapability`、`ToolSessionCapability`、`ToolTurnCapability` | 业务调用属于 `McpToolApi`；hook/display/telemetry 属于 tool runtime capability；approval/display 归 `McpSessionCapability`。 |
| `ListMcpResourcesHandler` / `ReadMcpResourceHandler` | `McpResourceApi` | `McpSessionCapability` | 当前绑定在 `SessionMcpResourceCaller<Turn>`；目标是资源业务能力与 live runtime 能力分离。 |
| `WorkflowStartHandler` / `WorkflowStatusHandler` / `WorkflowResumeHandler` / `WorkflowAbortHandler` | `WorkflowApi` | `WorkflowSessionCapability`、`ToolTurnCapability` | `workflow_list` / `workflow_describe` 只需要 turn-local registry view；其余执行型 handler 依赖 workflow service。 |
| `WorkflowListHandler` / `WorkflowDescribeHandler` | 无，或 `WorkflowCatalogApi` | `ToolTurnCapability` | 当前只读 workflow registry，适合保留为 turn-local 只读能力。 |
| `SpawnAgentHandler` / `FollowupTaskHandler` / `WaitAgentHandler` / `CloseAgentHandler` / `ListAgentsHandler` | `AgentApi` | `AgentSessionCapability`、`ToolTurnCapability` | 当前通过 `MultiAgentToolSession<Turn>` 混合；目标是 agent 业务入口与 live session callback 分开。 |
| `GetGoalHandler` / `CreateGoalHandler` / `UpdateGoalHandler` | `GoalApi` | `GoalSessionCapability` | 目标是 goal 业务 service 自己处理 persistence / status，live runtime 只负责 display / continuation / budget report 等能力。 |
| `RequestPluginInstallHandler` | `PluginInstallApi` 或 `PluginCatalogApi` | `ToolSessionCapability`、`ToolTurnCapability` | discoverable tool 过滤可以是 service API，用户 elicitation / 完成态写入是运行期 capability。 |
| `PlanHandler` / `RequestPermissionsHandler` / `RequestUserInputHandler` / `DynamicToolHandler` / `ViewImageHandler` | `FunctionToolApi`（必要时拆成 `PlanApi`、`UserInputApi`、`PermissionRequestApi`、`ImageReadApi`） | `ToolSessionCapability`、`ToolTurnCapability` | 当前统一依赖 `FunctionToolSession<Turn>` / `FunctionToolTurn`；长期应拆成更清晰的 function-tool service 与当前 turn view。 |
| `ExtensionToolHandler` | `ExtensionToolApi` 或 `ToolContributorApi` | `ToolSessionCapability`、`ToolTurnCapability` | extension tool 属于 tool domain；executor 发现和 handler 装配在 `ToolService` 内部完成。 |
| `ToolSearchHandler` | `ToolCatalogApi` | 无，或 `ToolTurnCapability` | 主要是静态/半静态 discovery，不需要 session side effect。 |
| `TestSyncHandler` | 无 | 无 | 测试专用，不作为长期架构边界。 |

这张表对应两条强约束：

- handler 构造函数里只注入它实际需要的全局 `Service API`。
- 一次 `dispatch_tool(...)` 调用里，统一传入当前 live runtime 的 `Capability API` / turn view；不要把这些运行期状态长期挂在 `ToolService` 或 handler 上。

### MCP API

```rust
pub trait McpToolApi: Send + Sync {
    fn call_mcp_tool(&self, request: McpToolCallRequest)
        -> impl Future<Output = anyhow::Result<CallToolResult>> + Send;
}

pub trait McpResourceApi: Send + Sync {
    fn list_resources(&self, request: ListResourcesRequest)
        -> impl Future<Output = anyhow::Result<ListResourcesResult>> + Send;
    fn read_resource(&self, request: ReadResourceRequest)
        -> impl Future<Output = anyhow::Result<ReadResourceResult>> + Send;
}

pub trait McpSessionCapability: Send + Sync {
    fn approval_context(&self, request: McpApprovalRequest)
        -> impl Future<Output = McpApprovalContext> + Send;
    fn persist_mcp_approval(&self, approval: McpApprovalDecision)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn emit_mcp_display_event(&self, event: McpDisplayEvent)
        -> impl Future<Output = anyhow::Result<()>> + Send;
}
```

### Command API

```rust
pub trait CommandExecutionApi: Send + Sync {
    fn exec_command(&self, request: ExecCommandRequest)
        -> impl Future<Output = Result<ExecCommandOutput, CommandExecutionError>> + Send;
    fn wait_command(&self, request: CommandWaitRequest)
        -> impl Future<Output = Result<CommandWaitOutput, CommandSessionError>> + Send;
    fn write_stdin(&self, request: WriteStdinRequest)
        -> impl Future<Output = Result<WriteStdinOutput, CommandSessionError>> + Send;
}

pub trait CommandSessionCapability: Send + Sync {
    fn emit_command_event(&self, event: CommandDisplayEvent)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn allocate_process_id(&self) -> impl Future<Output = UnifiedExecProcessId> + Send;
    fn release_process_id(&self, id: UnifiedExecProcessId)
        -> impl Future<Output = ()> + Send;
}
```

### Agent API

```rust
pub trait AgentApi: Send + Sync {
    fn spawn_agent(&self, request: SpawnAgentToolRequest)
        -> impl Future<Output = anyhow::Result<SpawnAgentToolOutput>> + Send;
    fn followup_task(&self, request: FollowupTaskRequest)
        -> impl Future<Output = anyhow::Result<FollowupTaskOutput>> + Send;
    fn wait_agent(&self, request: WaitAgentRequest)
        -> impl Future<Output = anyhow::Result<WaitAgentOutput>> + Send;
    fn close_agent(&self, request: CloseAgentRequest)
        -> impl Future<Output = anyhow::Result<CloseAgentOutput>> + Send;
    fn list_agents(&self, request: ListAgentsRequest)
        -> impl Future<Output = anyhow::Result<ListAgentsOutput>> + Send;
}

pub trait AgentSessionCapability: Send + Sync {
    fn parent_thread_id(&self) -> ThreadId;
    fn enqueue_child_completion(&self, completion: ChildCompletion)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn emit_agent_event(&self, event: AgentDisplayEvent)
        -> impl Future<Output = anyhow::Result<()>> + Send;
}
```

`AgentApi` 可以依赖 `ThreadSpawnApi` 与 `AgentSessionCapability`，但不能依赖 `SessionRuntime` 或 session handle。

### Goal API

```rust
pub trait GoalApi: Send + Sync {
    fn get_goal(&self) -> impl Future<Output = anyhow::Result<GoalSnapshot>> + Send;
    fn create_goal(&self, request: CreateGoalRequest)
        -> impl Future<Output = anyhow::Result<GoalSnapshot>> + Send;
    fn update_goal(&self, request: UpdateGoalRequest)
        -> impl Future<Output = anyhow::Result<GoalSnapshot>> + Send;
    fn evaluate_post_turn(&self, event: GoalRuntimeEvent)
        -> impl Future<Output = anyhow::Result<ThreadPostTurnState>> + Send;
}

pub trait GoalSessionCapability: Send + Sync {
    fn emit_goal_update(&self, update: GoalDisplayUpdate)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn inject_goal_context(&self, item: PendingInputItem)
        -> impl Future<Output = anyhow::Result<()>> + Send;
}
```

### Workflow API

```rust
pub trait WorkflowApi: Send + Sync {
    fn start_workflow(&self, request: StartWorkflowRequest)
        -> impl Future<Output = anyhow::Result<WorkflowRunSnapshot>> + Send;
    fn resume_workflow(&self, request: ResumeWorkflowRequest)
        -> impl Future<Output = anyhow::Result<WorkflowRunSnapshot>> + Send;
    fn abort_workflow(&self, request: AbortWorkflowRequest)
        -> impl Future<Output = anyhow::Result<WorkflowRunSnapshot>> + Send;
}

pub trait WorkflowSessionCapability: Send + Sync {
    fn spawn_workflow_agent(&self, request: WorkflowSpawnAgentRequest)
        -> impl Future<Output = anyhow::Result<WorkflowAgentHandle>> + Send;
    fn emit_workflow_progress(&self, event: WorkflowProgressEvent)
        -> impl Future<Output = anyhow::Result<()>> + Send;
}
```

### Context / Persistence / Telemetry API

```rust
pub trait ContextHistoryApi: Send + Sync {
    fn build_context(&self, request: BuildContextRequest)
        -> impl Future<Output = anyhow::Result<ModelContext>> + Send;
    fn compact_history(&self, request: CompactRequest)
        -> impl Future<Output = anyhow::Result<CompactResult>> + Send;
    fn context_usage(&self) -> impl Future<Output = ThreadContextUsage> + Send;
}

pub trait ThreadPersistenceApi: Send + Sync {
    fn persist_event(&self, event: EventMsg)
        -> impl Future<Output = anyhow::Result<()>> + Send;
    fn read_thread(&self, request: ReadThreadRequest)
        -> impl Future<Output = ThreadStoreResult<StoredThread>> + Send;
    fn update_metadata(&self, patch: ThreadMetadataPatch)
        -> impl Future<Output = ThreadStoreResult<()>> + Send;
}

pub trait SessionTelemetryApi: Send + Sync {
    fn record_metric(&self, metric: TelemetryMetric);
    fn start_timer(&self, name: &'static str) -> SessionTelemetryTimer;
}
```

### Extension / Skill / Plugin API

```rust
pub trait ExtensionSkillPluginApi: Send + Sync {
    fn available_skills(&self, request: SkillDiscoveryRequest)
        -> impl Future<Output = anyhow::Result<Vec<SkillMetadata>>> + Send;
    fn skill_injections(&self, request: SkillInjectionRequest)
        -> impl Future<Output = anyhow::Result<SkillInjections>> + Send;
    fn available_plugins(&self, request: PluginDiscoveryRequest)
        -> impl Future<Output = anyhow::Result<Vec<PluginMetadata>>> + Send;
    fn extension_data(&self) -> ExtensionDataView;
}
```

如果 extension contributor 暴露的是 native tool，它进入 ToolService 的 tool registry/dispatch 路径；ExtensionSkillPluginService 只负责 registry/data/discovery，不拥有 extension tool handler 或 executor 装配。

## Service 划分

### ThreadLifecycleService

职责：

- thread 创建、恢复、关闭、live registry、thread status。
- 组合 `Config`、auth、store、runtime factories，并启动 session runtime。

当前位置：

- `codex-rs/thread-runtime/src/thread/manager.rs`
- `codex-rs/thread-runtime/src/thread/codex.rs`

目标 API：

- `ThreadLifecycleApi`
- `ThreadRegistryApi`
- `ThreadSpawnApi`

### SessionExecutionService

职责：

- submission loop、turn lifecycle、active task、turn cancellation、post-turn state。
- 调度 model、tool、goal、compact、child completion，但不拥有这些 domain implementation。

当前位置：

- `codex-rs/thread-runtime/src/session`
- `codex-rs/thread-runtime/src/tasks`
- `codex-rs/thread-runtime/src/state/turn.rs`

目标结构：

- `SessionRuntime` 持有 domain APIs，并实现当前需要的窄 session capability traits。
- 底层 service 通过 `Weak<dyn Capability>` 回调 session，不依赖 concrete `SessionRuntime`。

### ToolService

职责：

- tool registry、dispatch、pre/post hook 调用、tool event、tool read metrics、goal tool accounting。
- extension tool executor discovery and tool assembly。
- tool 实现只依赖对应 domain API，不依赖 `Session` / `TurnContext` concrete。
- 作为全局 singleton service 由 composition root 创建，并显式注入其依赖的全局 service API；例如当前 workflow 相关 tool 通过 `ToolService::new(Arc<dyn WorkflowApi>)` 注入，而不是在 `build_tool_router()` 内部隐式创建 `WorkflowService`。

当前位置（历史遗留，当前目标是继续收口到 `tool-service` / `tool-service-api`）：

- `codex-rs/tool-service`
- `codex-rs/tool-service-api`
- `codex-rs/thread-runtime/src` 中残留的 capability implementation

目标 API/Service：

- `ToolApi`
- `ToolSessionCapability`
- `ToolService`

目标形态：

- 原 `codex-rs/tool-runtime` 中属于 tool dispatch/runtime 的通用能力应并入目标 `tool-service` 或 `tool-api` 支撑层。
- 不再保留“ToolService 依赖 ToolRuntime”的长期分层；service 本身就是 domain runtime implementation。
- 只有纯 DTO、trait、registry planning、tool type 这类稳定 contract 保留在 API/foundation crate。
- `thread-runtime` 只传 `ExtensionToolBuildParams`，不再收集 extension executors；`thread-runtime/src/tools/extension_tools.rs` 这类 tool implementation 文件不应恢复。

### McpService

职责：

- MCP connection、tool call、resource read、OAuth/login/retry、approval、elicitation。

当前位置：

- `codex-rs/mcp-runtime`
- `codex-rs/mcp-runtime-api`
- `codex-rs/codex-mcp`
- `codex-rs/thread-runtime/src/mcp`

目标 API/Service：

- `McpToolApi`
- `McpResourceApi`
- `McpSessionCapability`
- `McpService`

### CommandExecutionService

职责：

- exec command、unified exec process、command wait/stdin、process lifecycle。

当前位置：

- `codex-rs/command-runtime`
- `codex-rs/process-exec`
- `codex-rs/thread-runtime/src/unified_exec`
- `codex-rs/thread-runtime/src/exec.rs`

目标 API/Service：

- `CommandExecutionApi`
- `CommandSessionApi`
- `CommandSessionCapability`
- `CommandExecutionService`

### ApprovalService

职责：

- cached approval、guardian review、request command approval、request patch approval、permission hook、retry approval、network approval decision。

当前位置：

- `codex-rs/permissions-runtime`
- `codex-rs/thread-runtime/src/network_approval.rs`
- `codex-rs/thread-runtime/src/tools/sandboxing.rs`
- `codex-rs/thread-runtime/src/tool_orchestrator_host.rs`

目标 API/Service：

- `ApprovalApi`
- `ApprovalCapability`
- `ApprovalService`

当前迁移状态：

- 已新增 `codex-rs/approval-service-api`，先把 `apply_patch` 审批从 `thread-api::ToolRuntimeSessionCapability` 挪到独立 `ApprovalServiceApi`。
- 当前 concrete `ThreadApprovalService` 仍临时实现在 `codex-rs/thread-runtime/src/approval_service.rs`，下一步再继续把 `network_approval`、`request_command_approval`、guardian 相关实现整体迁出。

### SandboxService

职责：

- sandbox policy selection、sandbox transform、managed network sandbox preparation、platform-specific sandbox command preparation。

当前位置：

- `codex-rs/sandboxing-api`
- `codex-rs/sandboxing`
- `codex-rs/thread-runtime/src/unified_exec`
- `codex-rs/thread-runtime/src/shell_escalation_adapter.rs`

目标 API/Service：

- `SandboxApi`
- `SandboxCapability`
- `SandboxService`

当前迁移状态：

- `codex-rs/sandboxing-api::SandboxRuntime` 已经是可注入的 sandbox service API 雏形。
- `thread-runtime` 中仍残留 `unified_exec` / `shell_escalation_adapter` / tool runtime 对 sandbox orchestration 的直接依赖，下一步需要继续收敛到独立 `SandboxService`。

### ModelService

职责：

- model client、provider auth、models manager、service tier、OpenAI file upload、attestation。

当前位置：

- `codex-rs/model-client`
- `codex-rs/model-provider-api`
- `codex-rs/models-manager-api`
- `codex-rs/openai-files-api`
- `codex-rs/thread-runtime/src/session`

目标 API/Service：

- `ModelApi`
- `ModelProviderApi`
- `ModelSessionCapability`
- `ModelService`

### ContextHistoryService

职责：

- conversation history、context usage、compact、token/rate-limit state、replacement history。

当前位置：

- `codex-rs/context-manager`
- `codex-rs/context-usage`
- `codex-rs/thread-runtime/src/compact.rs`
- `codex-rs/thread-runtime/src/state/session.rs`

目标 API/Service：

- `HistoryApi`
- `CompactApi`
- `ContextUsageApi`
- `ContextHistoryService`

### AgentService

职责：

- `spawn_agent`、`followup_task`、`wait_agent`、`close_agent`、`list_agents`、child completion、agent status。

当前位置：

- `codex-rs/agent-runtime`
- `codex-rs/thread-runtime/src/agent`

目标 API/Service：

- `AgentApi`
- `AgentSessionCapability`
- `ThreadSpawnApi`
- `AgentService`

断环要求：

- `AgentService` 可以依赖 `AgentSessionCapability` / `ThreadSpawnApi`。
- `AgentSessionCapability` 应由 `SessionRuntime` / Session service 本体实现。
- `AgentService` 不得依赖完整 `SessionRuntime`。

### GoalService

职责：

- goal state、goal tool、post-turn continuation、goal display lifecycle。

当前位置：

- `codex-rs/agent-runtime` 的 goal state/mutation pieces
- `codex-rs/thread-runtime/src/goal`

目标 API/Service：

- `GoalApi`
- `GoalSessionCapability`
- `GoalService`

### WorkflowService

职责：

- workflow registry、run controller、progress event、workflow tool bridge。

当前位置：

- `codex-rs/workflow-api`
- `codex-rs/workflow`
- `codex-rs/thread-runtime/src/workflow_tool_host.rs`
- `codex-rs/thread-runtime/src/workflows.rs`

目标 API/Service：

- `WorkflowApi`
- `WorkflowSessionCapability`
- `WorkflowService`

### ExtensionSkillPluginService

职责：

- skills discovery/injection、plugins/apps discovery、extension registry/data。

当前位置：

- `codex-rs/core-skills-api`
- `codex-rs/plugin-service-api`
- `codex-rs/ext/extension-api`
- `codex-rs/thread-runtime/src/skills.rs`
- `codex-rs/thread-runtime/src/plugins`
- `codex-rs/thread-runtime/src/apps`

目标 API/Service：

- `SkillApi`
- `PluginApi`
- `ExtensionApi`
- `ExtensionSkillPluginService`

### PersistenceTelemetryService

职责：

- thread store、live thread、state db、rollout trace、session telemetry、analytics。

当前位置：

- `codex-rs/thread-store-api`
- `codex-rs/state-api`
- `codex-rs/rollout-trace-api`
- `codex-rs/session-telemetry-api`
- `codex-rs/analytics-api`
- `codex-rs/thread-runtime/src/state_db_bridge.rs`

目标 API/Service：

- `ThreadPersistenceApi`
- `SessionTelemetryApi`
- `AnalyticsApi`
- `PersistenceTelemetryService`

## 重构计划

### Phase 0：术语和边界固化

目标：

- 在 `.codex/pm-progress.md` 和 `AGENTS.md` 中固定 API / Service / Capability / Weak back-reference / SessionRuntime 术语。
- 明确 `SessionServices` 是待拆 service bag，不是长期边界。

完成标准：

- 文档明确禁止 `Service A <-> Service B` 互相持有。
- 文档明确底层 service 只能持有 `Weak<dyn Capability>`，不得持有 concrete `SessionRuntime` / `ThreadManager`。

### Phase 1：定义最小 capability traits

目标：

- 为当前需要跨 service 回调 session/thread 的地方定义窄 capability traits。
- 先覆盖 ToolService 试点需要的能力，例如 event emission、pre/post hooks、goal tool accounting。
- trait 放在能力提供方的 API crate；例如 tool dispatch 需要回调 session side effect 时，contract 放 `session-api`，由 `SessionRuntime` 实现，ToolService 只消费该 API。

完成标准：

- capability trait 只包含当前 consumer 实际需要的方法。
- `SessionRuntime` / `ThreadManager` 或对应 owner service 本体可以直接实现这些 traits。
- domain service 只看到 `dyn Capability`，不引用 concrete session/thread 类型。

当前落地状态：

- 已在 `codex-rs/session-api/src/lib.rs` 为 tool side effect 引入无泛型 `ToolSessionCapability`，并新增 `ToolTurnCapability` 作为当前 turn 的 API view。
- `ToolSessionCapability` 由 `Session` 本体实现，`ToolTurnCapability` 由 `TurnContext` 本体实现；`thread-runtime` 不再创建绑定 `Session + TurnContext` 的 per-turn adapter。组合根把 `Weak<dyn ToolSessionCapability>` 注入 tool router，避免 ToolService 反向持有 concrete session/thread 类型。`CoreToolDispatchHost` 已删除，tool owner crate 中的 `SessionToolDispatchHost` 只保存注入的 Weak。
- extension tool 已按 tool domain 收口一层：组合根只传 extension contributor/data；将其解析为 executable extension tool 的逻辑收敛在 `codex-tool-service` 内部完成。
- `thread-runtime/src/function_tool.rs` 这类只包装 tool 类型的 thread-runtime facade 应删除；当前 `FunctionCallError` 已直接从 `codex_tool_types` 引用。
- `thread-runtime/src/tools/context.rs` 已删除；不再用 thread-runtime 内部模块包装 `ToolInvocation` 类型。
- `thread-runtime/src/tools/router.rs` 和 `thread-runtime/src/tools/router_tests.rs` 已删除；router implementation 和行为测试归 tool owner crate，session/thread 测试只在 `test_support` 中保留 composition helper。
- `thread-runtime/src/tools/events.rs` 和 `thread-runtime/src/tools/orchestrator.rs` 已删除；event/orchestrator 相关 capability implementation 已迁回 owner crate，避免 `tools/` 继续承载 router/orchestrator facade。
- `thread-runtime/src/shell_tool_host.rs` 和 `thread-runtime/src/unified_exec/tool_host.rs` 已删除；对应 impl 合并到 `session_tool_domain_host.rs`。这不是完成态，只是先消除分散 facade，下一步必须拆掉 `ToolDomainHost` 粗 contract。
- `request_plugin_install` 已从粗 `ToolDomainHost` 拆出：`RequestPluginInstallHost` 不再继承 `ApplyPatchHandlerHost`，`codex-session-api` 提供 `SessionRequestPluginInstallCaller` / `SessionRequestPluginInstallHost`，由 `Session` 本体实现 caller，并通过 `ToolRuntimeBuildParams.request_plugin_install_host` 显式注入 tool assembly。
- `thread-runtime/src/tools/registry.rs` 已收缩为 `cfg(test)` 单元测试辅助，不再作为 `test-support` feature surface。
- 这只是 API contract 下沉和 Weak 注入试点，不代表 ToolService 完成拆分：当前 `ToolInvocation` 仍携带 concrete `Arc<Session>` / `Arc<TurnContext>`，`SessionToolDomainHost` 仍承载 apply-patch、shell、exec-command、command interaction、code-mode 等粗粒度能力。真正完成还需要继续把这些 session side effect 收敛为 `codex-session-api` 中的窄 capability，并把 ToolService implementation 迁出 `thread-runtime`。

### Phase 2：Weak 注入 ToolService 试点

目标：

- 以 tool domain 验证 API + Service + Weak capability + IoC 模式。
- 将当前 tool 执行能力合并进 `ToolService` 目标边界，不再把 `tool-runtime` 作为 service 之下的长期依赖层。
- 把 `thread-runtime/src/tools`、`session_tool_domain_host.rs`、`code_mode_host.rs` 相关 session side effect 分解为 `ToolSessionCapability` 或更窄的 owner capability。

完成标准：

- `ToolService` 不依赖 `Session` / `TurnContext` concrete。
- `ToolService` 持有 `Weak<dyn ToolSessionCapability>`。
- `upgrade()` 失败返回明确错误。
- `codex-tool-service` normal/dev graph 不拉回 `codex-thread-runtime` 或 `codex-core`。
- 目标设计中没有独立 `tool-runtime` service layer；当前代码应只保留 `tool-service` / `tool-service-api` 两层。

### Phase 3：拆分 SessionServices 为 domain bundles

目标：

- 把 `SessionServices` 字段按 domain 分组，先形成 crate-private bundle。
- 不急于 public API 化，只先降低大 service bag 的认知复杂度。

建议首批 bundle：

- `ToolServiceBundle`
- `McpServiceBundle`
- `CommandServiceBundle`
- `ModelServiceBundle`
- `HookServiceBundle`
- `ExtensionServiceBundle`
- `PersistenceTelemetryBundle`
- `AgentServiceBundle`

完成标准：

- `SessionServices` 不再直接平铺几十个字段。
- 每个 bundle 有清晰 owner 注释，说明是最终 service 还是迁移中 bundle。

### Phase 4：McpService 迁移

目标：

- MCP adapter 不再读取完整 `SessionServices` / `TurnContext` internals。
- MCP call/resource/OAuth/approval/elicitation side effect 通过 `McpSessionCapability` 表达。

完成标准：

- MCP production adapter implementation 迁到 MCP owner crate 或明确 owner service crate。
- `thread-runtime/src/mcp` 只保留 orchestration glue 或被删除。
- MCP owner crate normal/dev graph 不拉回 `codex-thread-runtime`。

### Phase 5：Agent/Goal/Workflow service 化

目标：

- 将 agent、goal、workflow 的 tool-facing handler 与 session/thread orchestration capability 解耦。
- `AgentService` 持有 `Weak<dyn AgentSessionCapability>` 和 `Weak<dyn ThreadSpawnApi>`，不依赖 concrete `SessionRuntime` / `ThreadManager`。
- `GoalService` 依赖 goal state store/event sink，不依赖完整 session。
- `WorkflowService` 依赖 workflow run controller 和 workflow session port。

完成标准：

- tool registry 只看到 `AgentApi` / `GoalApi` / `WorkflowApi`。
- session runtime 只编排 post-turn / child completion / workflow continuation。

### Phase 6：Command/Sandbox/Permission service 化

目标：

- command session、unified exec、sandbox permission、network approval 从 session concrete host 中拆出。
- command wait/stdin/output lifecycle 继续走 typed `EventMsg` display path。

完成标准：

- `CommandExecutionService` 拥有 process/session controller implementation。
- session runtime 只消费 command API 和 event sink。
- command service owner crate normal/dev graph 不拉回 `codex-thread-runtime`。

### Phase 7：收敛 Thread/Session 命名

目标：

- 保留 thread/session 概念分离。
- 逐步减少旧产品名前缀在 runtime 架构命名中的扩散。

候选命名：

- 旧 submit/event facade -> `SessionHandle` 或 `SessionIoHandle`
- 旧 thread facade -> `ThreadHandle`
- `Session` -> `SessionRuntime`
- `SessionServices` -> 删除或收缩为 `SessionRuntimeServices`

完成标准：

- public compatibility 可以保留 deprecated alias。
- 新代码不再扩散旧命名。

## 验证策略

每个 phase 至少做三类验证：

1. 编译/测试
   - 修改 crate 的 `cargo check -p <crate> --lib` 或更窄测试。
   - 涉及 app-server/runtime/protocol/root-worker 后端启动路径时，运行 `cargo build -p codex-app-server --bin codex-app-server`。

2. 依赖门禁
   - `cargo tree -p <owner-crate> --edges normal | rg "codex-core v|codex-thread-runtime"`
   - `cargo tree -p <owner-crate> --edges normal,dev | rg "codex-core v|codex-thread-runtime"`

3. 静态架构检查
   - `rg "Arc<Session>|Arc<TurnContext>|ThreadManager" <owner-crate>`
   - `rg "impl .*Capability for Session|impl .*Capability for SessionRuntime" codex-rs/thread-runtime`
   - 确认 owner crate 不持有 concrete session/thread 类型；`thread-runtime` 内部可以实现 capability。

## 非目标

- 不把所有 struct 机械改名为 `Service`。
- 不把 `State`、`Context`、`Options`、`Request`、`Response` 改成 service。
- 不用 `Weak<ConcreteSession>` 或 `Weak<ThreadManager>` 隐藏 concrete 依赖；允许 `Weak<dyn Capability>` 作为 runtime back-reference。
- 不为了拆分复制 `codex-thread-runtime` 大块代码到新 crate。
- 不重新引入 `codex-core` 作为实现归宿。
