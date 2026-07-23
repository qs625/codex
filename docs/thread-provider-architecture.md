# Thread Provider 架构

本文档描述目标架构：让 native Morpheus thread 与外部 CLI 支撑的 thread
共享同一套生命周期 contract。第一阶段先引入 provider descriptor catalog，
并把 New conversation 流程接到该 catalog；完整 runtime 迁移仍分阶段推进。

## 目标

- 让 `EventMsg` 成为 provider 的 canonical output。native runtime event 以及
  external provider 的 stdout、JSON、SSE message，必须先由各自 adapter 归一化，
  再进入 app-server、rollout、replay 或 root-worker。
- 给客户端提供一套 provider-neutral 的发现方式，用于判断 thread lifecycle
  支持范围、agent role、model selection 和 active capability。
- 当 provider 字段省略时，保留既有 native `thread/start` 行为。
- external provider 与 native `agent_type` 保持分离；`codex_cli`、
  `claude_cli`、`opencode` 是 provider id，不是 Morpheus role。

第一阶段非目标：

- 不一次性把所有 runtime operation 都迁到新 trait 后面。
- 不通过假装支持的方式为 external provider 暴露 native-only feature。
- 不在 root-worker 解析 external raw message。
- 不改变 ACP，也不引入第三方 ACP 依赖。

## Provider Contract

目标后端 contract 如下：

```rust
trait ThreadProvider {
    fn provider_kind(&self) -> ThreadProviderKind;
    async fn start_thread(&self, request: ThreadStartRequest) -> Result<ThreadHandle>;
    async fn send_input(&self, thread: ThreadHandle, input: ThreadInput) -> Result<()>;
    async fn close_thread(&self, thread: ThreadHandle, mode: CloseMode) -> Result<ThreadStatus>;
    async fn status(&self, thread: ThreadHandle) -> Result<ThreadStatus>;
    async fn list_children(&self, thread: ThreadHandle) -> Result<Vec<ThreadHandle>>;
    fn event_stream(&self, thread: ThreadHandle) -> BoxStream<'static, EventMsg>;
    async fn restore_thread(&self, metadata: PersistedThreadMetadata) -> Result<ThreadHandle>;
}
```

`ThreadProviderDescriptor` 是该 contract 的只读 discovery shape：

- `id` 和 `kind` 标识 provider owner。
- `agentTypes` 是 provider scoped。native 暴露 Morpheus roles；external CLI
  provider 当前不暴露任何 role。
- `modelSelection` 描述 model choice 来源：catalog、provider default、none，
  或后续的 external config catalog。
- `capabilities` 只用于 gate active request 和 UI control。event consumption
  永远不根据 capability 分叉。

## Event 归一化

raw message parsing 由 adapter 负责：

- Native Morpheus runtime 直接从 session、tool runtime、agent control 和
  command runtime 发出现有 `EventMsg`。
- Claude stream-json、OpenCode SSE/HTTP、Codex CLI app-server transport 在
  external adapter 内解析 provider-specific message，并为 assistant output、
  tool call、tool result、lifecycle status、completion 和 error 发出有界的
  `EventMsg`。
- raw provider stdout、provider JSON envelope、assistant marker text 和
  transport log 都不是 display fact。它们只能作为 adapter-owned error 背后的
  有界诊断信息保留。

不支持的 active operation 应在 provider boundary 返回 typed unsupported-action
error。它们不改变 replay 或 display handling。

## API 与客户端流程

兼容迁移路径如下：

1. `threadProvider/list` 按 cwd 返回 provider descriptor。
2. 既有 `agentType/list` 和 `model/list` 继续服务 legacy client。
3. `ThreadStartParams.threadProvider` 是可选字段。省略或传 `native` 时保持当前
   行为。external id 可以被 advertised，但在 root start path 完成迁移前，
   `thread/start` 会拒绝它们。
4. Root-worker New conversation 先选择 provider，再展示 provider scoped role
   和 model selection：
   provider -> agent role/type -> model provider/model -> reasoning/service
   tier -> create。
5. `modelSelection: providerDefault` 的 external provider 会禁用全局 model
   picker，而不是借用无关的 config model。

## Runtime 边界

- `thread/start`、`thread/read`、`thread/resume`、`thread/list`、status
  notification、followup input、close/cancel/archive、fork、compact、workflow、
  goal、schedule、command session、approval、sandbox profile 和 dynamic tool
  都应指向 provider-neutral handle。
- Provider descriptor 可以禁用 compact、workflow、command session、permission
  或 `poll_event` 等 active call；但只要 provider 为某个展示项发出了合法
  event，下游 replay 仍应通过 typed `EventMsg -> ThreadItem` 路径处理。
- Rollout `Limited` 仍是 reload contract。任何 reload 需要的 provider event
  都必须以有界 payload 持久化到 thread history/replay 实际消费的 view 中。
- Root-worker tree 和 right-panel state 消费 normalized thread metadata 与 typed
  thread item。parent-child edge 必须来自 thread metadata/spawn edge，而不是
  client 里的 orphan promotion。

## Thread-Service API 与 Capability 拆分

当前 thread-service 上挂了不少 API 和 capability trait，它们并不都应该进入
`ThreadProvider`。迁移时要按调用者和语义拆成四类：

- Provider-neutral lifecycle API：面向 app-server 和客户端请求，描述 thread
  生命周期与 metadata。包括 start、send input、close/cancel、status、list/read、
  resume/restore、parent-child edge、event stream。这类 API 应逐步迁到
  `ThreadProvider` 或 provider-neutral handle 后面。
- Shared coordination kernel：面向 native 与 external provider 共同复用的后端事实源。
  包括 thread registry/thread store、spawn edge、pending input、completion
  delivery、status notification、`poll_event` wakeup、bounded `EventMsg`
  persistence。这类能力属于 thread-service core，不属于任意一个 provider trait。
- Native/internal agent API：只服务 Morpheus 内部 agent/tool surface。包括
  `spawn_agent` 的 `agent_type`/role/model 语义、固定 owner/reviewer 约定、
  native `list_agents` 的 role/nickname/status 解释、native session 的 model-visible
  tool 注入。这些应收口到 `NativeThreadProvider` 或 `NativeAgentControl` 内部，
  不要求 external provider 实现。
- Provider transport API：只存在于 external adapter 内部。包括 Claude stream-json、
  OpenCode HTTP/SSE、Codex CLI app-server transport，以及 raw provider message
  parsing。这些 API 的对外输出只能是 normalized `EventMsg` 与 provider terminal
  status，不能泄漏到 root-worker 或 generic replay 层。

因此 `thread_service_api::ThreadServiceApi` 不应继续无限扩张成所有 thread/tool
操作的集合。迁移方向是把它拆成更窄的接口：

- `ThreadLifecycleRuntime`：provider-neutral lifecycle/request facade，由
  app-server 的 thread processor 使用。
- `ThreadCollaborationRuntime`：模型工具侧的协作入口，保留 `spawn_agent`、
  `followup_task`、`close_agent`、`list_agents` 等 tool surface，但内部先解析目标
  provider，再路由到 native/external provider。这里可以继续保留 native/external
  tool 名称分离，但实现不应复制两套 registry。
- `ThreadEventRuntime`：`emit_event`、display event emission、rollout persistence、
  status notification、`poll_event` wakeup 等 event kernel 能力。provider 只调用它
  发 fact，不拥有 replay/display 分叉逻辑。
- `NativeAgentRuntime`：Morpheus-only extension trait，只由 native provider 和
  internal agent control 使用；external provider 不实现，也不通过空实现假装支持。

`ThreadTurnCapability`、`ThreadSessionCapability`、`ThreadRuntimeCapability` 仍是
active turn/tool dispatch 的 capability，不是 provider capability。它们的改法是
继续缩窄、按工具域拆分，而不是挂到 `ThreadProvider` 上：

- shell/exec/apply_patch/network approval 相关方法留在 exec/sandbox tool runtime
  的 turn capability。
- MCP、dynamic tool、app tool policy、auth elicitation 留在 tool/session dispatch
  capability。
- multi-agent tool 只需要拿到 thread id、turn id、cwd/config、event emitter 和
  collaboration runtime，不应该拿完整 native `TurnContext` 后再到处 downcast。
- external provider adapter 不应持有完整 `TurnContext` 或 `Session`；它只拿
  `ProviderExecutionContext`，其中包含 provider id、thread handle、cwd、bounded
  event sink、input receiver、shutdown token 和必要的 persisted metadata writer。

第一步可以保持现有 public trait 兼容，在 thread-service 内部新增窄接口并让
`ThreadService` 同时实现旧接口和新接口。随后把调用点从旧的宽 `ThreadServiceApi`
迁到窄接口，最后再删除旧接口里的 native/external 重复方法。这个顺序能避免一次性
重写所有 tool crate。

当前过渡实现已将 `thread_service_api::ThreadServiceApi` 拆成四个窄边界：

- `ThreadLifecycleRuntime`：provider-neutral lifecycle 边界；当前已承载
  `shutdown_all_threads_bounded`、`subscribe_thread_created` 和
  `active_event_subscriptions`，供 app-server thread processor 直接依赖。root
  start/resume/fork 仍保留在 app-server 的过渡 trait 中，因为这些请求还携带完整
  `Config` 与 native dynamic tool/environment 结构，直接搬入 `thread-service-api`
  会引入不合适的依赖方向。
- `NativeAgentRuntime`：承载 native Morpheus `spawn_agent`、`followup_task`、
  `close_agent` 和 `list_agents`。
- `ThreadCollaborationRuntime`：承载 external collaboration tool surface，并继承
  `NativeAgentRuntime` 以保持现有 model-visible collaboration facade 兼容。
- `ThreadEventRuntime`：承载 `poll_event`、wait backoff 和
  `record_model_items_and_emit_display_events`。

旧 `ThreadServiceApi` 现在只是
`ThreadLifecycleRuntime + ThreadCollaborationRuntime + ThreadEventRuntime` 的兼容
facade，并通过 blanket impl 自动为实现窄 trait 的 runtime 提供旧 API。app-server
thread processor 的 shutdown、thread-created 订阅和 active event subscription
tracker 已改到 `ThreadLifecycleRuntime`；root start/resume/fork 的具体创建调用点
仍通过 app-server-local 过渡 trait 继承该 lifecycle 边界。后续阶段再把 tool
service 和 app-server 剩余调用逐步改到窄 trait。

live thread runtime 的 command/status/inspection 也已开始迁移到 provider-neutral
surfaces：

- `LiveThreadInspectionRuntime` 承载 loaded ids、loaded check、
  `LiveThreadInfo` 和 `LiveThreadSnapshot` 等 copied fact。
- `LiveThreadStatusRuntime` 承载 copied `AgentStatus` / status watch。
- `LiveThreadCommandRuntime` 承载 submit op、submit op with trace、client info
  写入和 loaded-thread remove。
- `LiveThreadShutdownRuntime` 承载不暴露 concrete handle 的 shutdown-and-wait
  语义。

app-server thread processor 的 thread loaded list、thread/read live snapshot merge、
turns/list live status、thread started/status notifications、resume-running thread
checks、submit op、client info 写入、out-of-band elicitation counter 操作，以及
archive 前 shutdown/remove 路径已改到这些窄 runtime。listener idle-unload 的
live-thread removal 已改到 command runtime。turn processor 的 turn/start
snapshot 读取、turn/review/realtime/interrupt `Op` 提交、interrupt status check、
realtime feature check 和 app-server client info 写入也已改到这些窄 runtime。
apps processor 的 apps feature check、feedback
processor 的 live rollout path lookup、thread goal processor 的 live rollout path /
ephemeral-thread checks 也已改到 inspection runtime。thread goal processor 的
external goal prepare/apply runtime effects 已改到 goal runtime。feedback processor
的 subtree ids、guardian rollout path 和 session source 读取已改到 feedback
runtime。listener 的 skill watch path resolution 已改到 skill-watch runtime。MCP
processor 的 thread-bound resource/tool request loaded check 已改到
inspection runtime；MCP refresh 的 live thread ids、config refresh snapshot 和
queued `Op::RefreshMcpServers` submit 已改到 inspection / command runtime。
bespoke `CollabCloseEnd` receiver loaded check 已改到 inspection runtime。
thread read/listing 的 copied token/context usage reads 已改到 usage runtime。
turn context override validation 已改到 live turn runtime。thread/read 和
thread/turns/list 的 live persisted history 读取已改到 history runtime。listener
event stream、idle unload 和 running resume 所需的 live handle 读取已改到
listener runtime，旧 `LiveThreadRegistry` / `AppServerLiveThreadRegistry` surface
已删除；turn processor 里剩余
的 app-server-local turn runtime 只覆盖环境选择校验、live `Config` 读取、
conversation item injection、steer 和 detached review fork 这些尚未
provider-neutral 化的 native-only 能力。后续阶段再继续拆出更窄 handle。

禁止路径：

- 不要把 `ThreadTurnCapability` 或完整 `Session` 塞进 external provider adapter。
- 不要让 `ThreadProvider` 继承 shell、MCP、approval、dynamic tool、agent job 等
  tool capability。
- 不要让 external provider 通过空的 `agent_type`/role 实现来满足 native agent API。
- 不要为 native 与 external 维护两套 parent completion、pending input、status
  notification 或 list registry。

## 第一阶段

当前已实现：

- `ThreadProviderDescriptor`、provider capability、scoped model selection 和
  `threadProvider/list` 的 protocol type。
- 可选的 `ThreadStartParams.threadProvider`；当前 native `thread/start` 只接受
  省略或 `native`。
- App-server catalog descriptor source，覆盖 native Morpheus 以及 external
  `claude_cli`、`opencode`、`codex_cli` skeleton descriptor。
- Root-worker New conversation provider selector，以及面向 agent type 和 model
  field 的 provider-scoped gating。

## 后续阶段

- 将 native thread start/status/input/close/list/restore 移到具体的 native
  `ThreadProvider` 后面。
- 将 external spawn registry 和 live snapshot logic 移到 external provider
  handle 后面。
- 持久化 external provider thread metadata 与有界 normalized event，使 completed
  external thread 在 reload 后可被列出，interrupted running session 也能拥有明确
  terminal state。
- 在 provider 能枚举 model 时，增加 provider-scoped external model catalog。
- 通过 provider capability 和 typed unsupported-action error 收口
  compact/workflow/goal/schedule/tool availability。
- 当 root-worker 和其他 client 默认消费 provider descriptor 后，移除临时重复的
  catalog path。

## 测试矩阵

- 不带 provider 的 native default `thread/start` 与 legacy behavior 等价。
- Native provider descriptor 包含 Morpheus roles 和 catalog-backed model
  provider。
- External descriptor 可以被列出，但不暴露 native agent type，也不暴露
  native-only capability。
- Root-worker provider selector 默认选 native；对 provider-default external
  provider 禁用 model control；对 native 保留 legacy `agentType` 和 model field。
- Thread read/list/resume/status display 继续消费 typed
  `EventMsg -> ThreadItem` fact，不解析 raw provider。
