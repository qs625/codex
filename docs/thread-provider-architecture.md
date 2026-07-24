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
   行为。advertised external id 可以进入 root `thread/start`，但只允许
   external root startup 明确支持的参数集合；native-only role、dynamic tool、
   environment、permission override 等输入仍在 route preflight 阶段拒绝。
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
- external provider 的 Errored / Shutdown 终态是 durable lifecycle fact，应通过
  external-specific bounded terminal event 进入 `Limited`。不要为此全局扩大
  generic `Error` 或 `ShutdownComplete` 的 Limited policy；Completed / Interrupted
  继续复用既有 `TurnComplete` / `TurnAborted`。
- Root-worker tree 和 right-panel state 消费 normalized thread metadata 与 typed
  thread item。parent-child edge 必须来自 thread metadata/spawn edge，而不是
  client 里的 orphan promotion。

## External Live Restore Contract

External provider 是一等 `ThreadProvider`，但 read-only snapshot 和 live
interactive restore 是两个不同 capability。`restoreSnapshot` 表示客户端可以读取
persisted metadata、bounded history 和 normalized lifecycle projection；`restoreThread`
表示后端已经重新接入同一个 provider session，后续 input、status、wait 和 close 都能继续
落到正确 live runtime。external provider 当前保持 `restoreThread=false`、
`restoreSnapshot=true`，直到下列 contract 有实现和测试证据后才允许翻转。

### 当前语义

- External root `thread/start` 可以创建新的 external live root thread，并把 provider
  output 归一化为 typed `EventMsg`。
- External root `thread/resume` 对 persisted external thread 只返回 read-only snapshot。
  Completed / Errored / Shutdown 由 persisted terminal fact 投影；Open + running 但没有
  live provider process 的 thread 投影为 Interrupted，不创建 live session。
- External root `turn/start` 只接受已经存在的 live external root record。read-only
  snapshot resume 不会让后续 `turn/start` 成功。
- External root `thread/fork` 当前创建 native Morpheus thread from bounded persisted
  history。它不是 external-to-external fork，也不继承 external provider session。
- External child `list_agents` 会合并 live external records 与 Open persisted
  descendants。terminal external child 可列出；Open + running/no-live child 以
  Interrupted 展示；Closed + Shutdown child 默认不进入协作集合。
- External child path reference 在 live miss 后可以通过 Open persisted edge 和 metadata
  找到 terminal external child，并返回原 thread id；它只注册 metadata，不创建 native
  `CodexThread`，也不注册 fake input sink。Open + running/no-live child 必须返回
  explicit interrupted / cannot reconnect 错误。Closed edge 仍不可通过普通 path
  reference 恢复。

### Persisted State Matrix

| Scope | Edge | Persisted lifecycle | Reconnect handle | Expected behavior |
| --- | --- | --- | --- | --- |
| Root | n/a | Completed / Errored / Shutdown | any | `thread/resume` returns read-only snapshot; no live input sink |
| Root | n/a | Running / no terminal | absent | startup restore and `thread/resume` project Interrupted / read-only; `turn/start` rejects |
| Root | n/a | Running / no terminal | present | future-only: register external live record, restore input sink and wait state, then `restoreThread` may be true |
| Child | Open | Completed / Errored / Shutdown | any | list/status/reference can resolve persisted facts; no native resume and no fake live sink |
| Child | Open | Running / no terminal | absent | list/status project Interrupted; path/followup reference rejects as non-reconnectable |
| Child | Open | Running / no terminal | present | future-only: restore external live record before accepting followup input |
| Child | Closed | Shutdown | any | hidden from default list/reference; history remains readable only through explicit thread history surfaces |

“Terminal” means the persisted `Limited` history contains a durable final fact accepted by the
agent lifecycle final-status contract: `TurnComplete` for Completed, or external terminal
Errored / Shutdown. `TurnAborted` Interrupted is a durable non-reconnectable interruption fact,
but it is not a terminal final status for path-reference live restore. For external list/status
projection, no-terminal persisted external threads may also be displayed as Interrupted after
restart. Neither form of Interrupted is a reconnect handle, and neither may be used to register a
live input sink.

### Facts Required Before Flipping `restoreThread`

Turning on external live restore requires persisted facts that prove continuity with the same
provider session. At minimum:

- Provider session identity: a provider-owned session id or resume token that uniquely identifies
  the original external conversation, not just our thread id.
- Adapter reconnect descriptor: transport-specific data needed to reattach. Claude stream-json,
  OpenCode HTTP/SSE and Codex CLI app-server may store different descriptors, but the generic
  thread provider layer only sees a typed provider id plus an opaque, bounded descriptor.
- Provider lifecycle fact: last known provider state, active turn id if any, and whether the
  provider has already emitted a terminal event.
- Input ownership: proof that exactly one restored live record owns the input sink for that
  provider session, including how pending root `turn/start` or child followup input is delivered
  after reconnect.
- Wait/poll state: enough state to wake `poll_event`, child completion waits and command/provider
  output waits without resurrecting stale waits or dropping late terminal events.
- Idempotent terminal causality: reconnect must not duplicate Completed / Errored / Shutdown
  events, deliver an old child completion envelope twice, or turn a terminal persisted thread back
  into Active.

If any of these facts are absent for a running external thread, the correct recovery is
Interrupted / cannot reconnect. Starting a fresh provider process is a new conversation and cannot
be presented as live restore of the persisted thread.

### Boundary Rules

- Startup restore may only recreate external live records for running external threads with a
  valid reconnect descriptor. Otherwise it records/logs the skip and leaves the persisted thread
  read-only.
- Root `thread/resume` must keep the read-only path separate from live restore. A read-only
  snapshot can include turns, metadata, lifecycle and bounded diagnostics, but it must not install
  input sinks or mark the thread loaded.
- Root `turn/start` and child followup routing must require a live external record. Persisted
  metadata alone is enough to resolve terminal references, but not enough to accept new input.
- `list_agents` and path reference may consume Open persisted spawn edges, agent metadata and
  terminal facts. They must not parse raw provider stdout, adapter JSON envelopes, or UI strings.
- `thread/fork` from external history stays native until the protocol has an explicit target
  provider choice and an external fork contract. External-to-external fork cannot be hidden behind
  the current native fork-from-history behavior.
- Capability descriptors are promises, not wish lists. External `restoreThread` stays false until
  startup restore, explicit resume, input delivery, status watch and close semantics all satisfy
  this section for at least one provider.

## External Fork Contract

External-to-external fork is a future `ThreadProvider` capability, not the current
native fork-from-external-history behavior. Fork and restore solve different problems:
restore reconnects the same persisted provider conversation when a provider-owned
resume handle exists; fork creates a new target conversation from a bounded source
snapshot. Starting a fresh external provider process is therefore a fork only when the
request explicitly asks for that target provider and the runtime records a new target
thread/session. It is not live restore of the source thread.

### Definitions

- Source provider: the provider that owns the source thread history. It may be native
  or external, live or persisted read-only.
- Target provider: the provider that will own the new forked thread. It may be native
  or external, but it must be selected explicitly by the request or capability layer.
- Source snapshot: bounded typed facts materialized from persisted thread history,
  not raw provider stdout, adapter JSON, marker text, or UI display strings.
- Target session: a fresh provider-owned conversation/input sink for the new thread.
  It must not reuse the source input owner, source wait state, or source terminal
  causality.

### Current Behavior

Current external-source `thread/fork` materializes bounded persisted history and
starts a native Morpheus target thread. That behavior is intentional compatibility:
it is native fork-from-history with an external source, not external-to-external
fork. Until protocol/API shape carries an explicit target provider choice and this
contract is implemented, external target providers must not be selected implicitly
from the source provider, global defaults, or provider catalog ordering.

### Request And Capability Shape

The eventual fork request must carry enough provider-neutral data to distinguish:

- source thread id and source provider;
- explicit target provider id;
- target provider model selection or provider-default selection;
- fork snapshot policy, such as bounded turns/items and model-context seed shape;
- parent/child/fork edge metadata to persist before target input is accepted.

Capability descriptors are promises. A provider may advertise external fork support
only after route preflight, target provider session creation, bounded history
materialization, event persistence/replay, reload/list/status/input/close behavior
and abort/close ownership are all tested for that provider. `restoreThread=true` is
not sufficient proof of fork support, and fork support is not proof of live restore.

### History Materialization

External fork seed history must come from normalized, bounded, replayable facts:

- rollout/history entries actually consumed by reload/read paths, especially
  `Limited` persisted facts when reload depends on that view;
- typed `EventMsg` / thread-history / app-server-protocol replay output;
- model-context-oriented seed items derived from those typed facts.

The generic layer must not parse provider raw JSON, stdout, SSE frames, app-server
JSON-RPC envelopes, adapter markers, or root-worker display strings to reconstruct
history. Display/replay facts and model-visible seed history can be different
projections, but both must derive from the same normalized persisted source facts
with bounded payloads.

### Metadata And Edges

External-to-external fork creates a new thread identity owned by the target provider.
Persisted metadata must make that identity unambiguous:

- target thread id, provider id, model/provider selection and provider session id or
  opaque target descriptor;
- source thread id, source provider id, source snapshot boundary and fork timestamp;
- fork edge or child/root relationship appropriate to the request surface;
- target lifecycle state from the target provider, not copied from the source;
- source attribution for audit/replay without letting source metadata own target
  input or status.

If the fork is created as a child, its parent-child edge must come from persisted
thread metadata/spawn/fork edge facts. It must not be reconstructed by root-worker
or by path heuristics after reload.

### Runtime Lifecycle And Input Ownership

A target external fork must allocate a fresh external live record and provider
session before accepting input. The target record owns:

- its own input sink and pending input queue;
- status watch and event stream subscription;
- close/cancel/archive ownership for the target session;
- `poll_event` wakeups, completion delivery and terminal event idempotency for the
  target thread.

The source thread remains unchanged. Source live input sinks, pending waits,
command/provider output waits, child completion bookkeeping and terminal causality
must not be transferred into the fork. If target provider startup fails after
metadata reservation, the target thread must receive a bounded failed/interrupted
lifecycle fact rather than silently falling back to native or marking the source as
modified.

### Reload, List, Status And Replay

After creation, the target external fork must behave like any other external thread:

- reload/read returns a bounded typed snapshot from persisted target history;
- list/status uses target provider metadata and lifecycle facts, not source status;
- followup/root input requires a live target external record unless the provider has
  satisfied the live restore contract for that target session;
- close/cancel affects only the target provider session;
- replay emits typed target thread items and fork/source attribution without parsing
  raw provider transport output.

If the target provider cannot reconnect after process loss, a running target external
fork follows the same Interrupted / cannot reconnect rules as other external running
threads. The source thread is not a reconnect fallback.

### Flip Gates

Before any external provider advertises external-to-external fork support, at least
one provider-specific implementation must prove:

1. route preflight rejects missing or unsupported target provider choice;
2. source external history materializes only through bounded typed persisted facts;
3. target external thread metadata, fork edge and provider session identity persist;
4. target input reaches the fresh target provider session after fork;
5. target status/list/read/reload project target facts without source-status leakage;
6. close/cancel/archive shut down or hide only the target thread as specified;
7. event replay restores assistant/tool/lifecycle display from typed target events;
8. abort or startup failure produces bounded target lifecycle facts and no native
   fallback;
9. tests cover native->external, external->external and external->native routing
   choices where the advertised capabilities allow them.

Until those gates are met, external fork capability stays false and current
external-source fork remains explicitly native-target behavior.

## Provider-Specific Restore Notes

Claude, OpenCode and Codex CLI differ only inside their transport adapters. The generic provider
contract must not expose raw stream-json, SSE payloads, app-server JSON-RPC messages, terminal
logs, or adapter markers to root-worker, replay, history builders, or app-server protocol clients.
Adapters may persist bounded opaque reconnect descriptors, but their normalized public output
remains `EventMsg`, `ThreadProviderDescriptor`, lifecycle status and typed unsupported-operation
errors.

Suggested implementation slices:

1. Persist a provider-scoped reconnect descriptor and session identity for one external provider.
2. Teach startup restore to distinguish reconnectable running from no-handle running.
3. Restore external live record ownership, input sink and status watch for reconnectable running
   threads.
4. Add root `turn/start` and child followup tests that prove input reaches the reconnected provider
   session.
5. Only then flip that provider's `restoreThread` capability; keep other providers false until
   they satisfy the same live restore contract.

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
  `shutdown_all_threads_bounded`、per-thread live shutdown/removal、
  `subscribe_thread_created` 和 `active_event_subscriptions`，并承载 live thread
  agent/runtime status read / status watch，供 app-server thread/turn processor
  直接依赖。agent status read 仍保留 native/external live record fallback 语义；
  runtime status read 对 native 保留 post-turn wait semantics，对 external live record
  只提供 Active/Complete 粗粒度映射；status watch 支持 native live thread subscription
  和 external live record watch。status-changed internal event 可携带权威
  `AgentStatus` payload，供 app-server 在 live record 不可读时仍发出 terminal
  lifecycle notification；外发 `ThreadStatusChangedNotification` shape 不变。
  live-thread removal 会删除 native live thread 或 external live record；external close
  会在 terminal Shutdown notification/persistence 后清理 external live record。durable
  default list / agent-reference recovery 基于 Open edge：Open edge 的 completed
  external agent 可恢复，显式 close 后的 Closed edge external agent 不进入默认协作集合。
  这仍不宣称 external root provider execution 已完成。root start/resume/fork
  已在 app-server thread processor 收口 route/preflight，但仍不属于该
  provider-neutral trait，因为这些请求还携带完整
  `Config` 与 native dynamic tool/environment 结构，直接搬入 `thread-service-api`
  会引入不合适的依赖方向；它们当前收口到 thread-service crate 内的
  `NativeThreadCreationRuntime` / `NativeThreadEnvironmentRuntime`。
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
已从 app-server-local `ThreadProcessorThreadRuntime` 迁到 thread-service native
creation/environment runtime。app-server 只把 `NewThread` 投影成 response assembly
需要的 `thread_id`、telemetry-only created-thread handle 和
`SessionConfiguredEvent`。这仍不是 external provider root start support；后续阶段
需要把 provider-neutral request DTO 和 routing 从这些 native-only DTO 中继续拆出。

live thread runtime 的 command/inspection 能力也已开始迁移到 provider-neutral
surfaces，status、app-server archive 与 listener idle-unload 的 per-thread shutdown
和 teardown removal 已进一步收口到 lifecycle 边界：

- `LiveThreadInspectionRuntime` 承载 loaded ids、loaded check、
  `LiveThreadInfo` 和 `LiveThreadSnapshot` 等 copied fact。
- status read / subscribe 已提升到 `ThreadLifecycleRuntime`；旧
  `LiveThreadStatusRuntime` compatibility surface 已删除。
- single-thread shutdown 已提升到 `ThreadLifecycleRuntime`；旧
  `LiveThreadShutdownRuntime` compatibility surface 已删除。
- app-server archive、listener idle-unload 和 native agent cleanup 的 live-thread
  removal 已提升到 `ThreadLifecycleRuntime`；该 primitive 会删除 native live thread 或
  external live record；`LiveThreadCommandRuntime::remove_live_thread` compatibility method
  已删除。
- listener handle 不再暴露 shutdown/wait；idle-unload teardown 通过
  `ThreadLifecycleRuntime::shutdown_live_thread` 和 `remove_live_thread` 完成。
- `LiveThreadCommandRuntime` 承载 submit op、submit op with trace 和 client info 写入。

app-server thread processor 的 thread loaded list、thread/read live snapshot merge、
resume-running thread
checks、submit op、client info 写入和 out-of-band elicitation counter 操作已改到这些
窄 runtime。archive 前 shutdown/removal 和 listener idle-unload 的 live-thread
shutdown/removal 已改到 lifecycle runtime。turn processor 的 turn/start
snapshot 读取、turn/review/realtime/interrupt `Op` 提交、
realtime feature check 和 app-server client info 写入也已改到这些窄 runtime。
app-server 的 turns/list live status、thread started/status notifications、turn
interrupt status check、archive 前 live shutdown、archive/listener teardown
removal，以及 listener lifecycle 的 live `AgentStatus` 和 TurnComplete post-turn
`ThreadRuntimeStatus` 读取已改到 lifecycle runtime。
apps processor 的 apps feature check、feedback
processor 的 live rollout path lookup、thread goal processor 的 live rollout path /
ephemeral-thread checks 也已改到 inspection runtime。thread goal processor 的
external goal prepare/apply runtime effects、cold resume/fork 后的 active goal
continuation，以及 running resume 的 goal resume effects/idle continuation 已改到
goal runtime。feedback processor 的 subtree ids、guardian
rollout path 和 session source 读取已改到 feedback runtime。listener 的 skill watch
path resolution 已改到 skill-watch runtime。MCP processor 的 thread-bound
resource/tool request loaded check 已改到 inspection runtime；MCP refresh 的 live
thread ids、config refresh snapshot 和 queued `Op::RefreshMcpServers` submit 已改到
inspection / command runtime；inspection runtime 的 live ids、loaded check 和
thread info 现在会包含 external live records。bespoke `CollabCloseEnd` receiver loaded check 已改到
inspection runtime。start/resume/fork/listing/review response assembly，以及
listener generation / running resume / rollback response / permissions request 所需的 copied session/config
reads 也已改到 inspection runtime。thread read/listing、cold
resume/fork 和 running resume usage replay 的 copied token/context usage reads 已改到 usage runtime。
turn context override validation 已改到 live turn runtime。thread/read、
thread/turns/list 和 rollback response 的 live persisted history / stored-thread
读取已改到 history runtime。listener event
stream 仍在 listener runtime；bespoke approval /
elicitation / user-input / permissions responses 以及 dynamic tool responses 的
listener submit 已改到 command runtime；memory consolidation
startup/shutdown/status/token usage 已改到 memory-specific handle。detached review
thread assembly 的 read-thread 也已改到 narrow runtime/store read 路径。旧
`LiveThreadRegistry` / `AppServerLiveThreadRegistry` surface 和 app-server
transitional `AppServerLiveThreadHandle` 均已删除；thread/turn request 的
environment selection validation 已共用 `NativeThreadEnvironmentRuntime`。turn
processor 的 `thread/inject_items` 已改到 conversation injection runtime，明确区别
于 subscription append 的 async-input path。`turn/steer` 已改到 native live steer
runtime，保留 active-turn steer validation 和 typed `SteerInputError` 映射。detached
review 的 current-history fork 和 metadata-only stored read 已改到 native detached
review runtime；app-server 仍负责 listener attach、watch upsert、
`ThreadStarted` notification 和 review turn submit orchestration。turn memory
startup 使用的 live full `Config` 读取已改到 native memory-startup config runtime；
它继续与 copied `ThreadConfigSnapshot` 分工，前者只服务 native memory startup
adapter/settings，后者仍服务 app-server response/session-source assembly。turn
processor 已不再保留 app-server-local `TurnProcessorRuntime` leftover bucket。thread processor
已不再保留 broad creation facade；native root start/resume/fork 仍通过
`NativeThreadCreationRuntime` 明确标记为 native-only 过渡层。后续阶段再继续拆出更窄
handle。

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
  省略或 `native`；external root `thread/start` 已通过 provider route 支持隐藏
  external provider id，并在 preflight 中拒绝 native-only 参数。
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
