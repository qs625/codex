# Model Service 化架构

## 目标

收缩当前 model 相关 crate 拓扑，把“模型目录管理 + provider 解析 + 模型请求发送”统一到一个 domain service 上，减少上层同时理解 `model-client`、`models-manager`、`model-provider`、`api-runtime` 多层概念的负担。

目标形态：

```text
Service API = model-service-api
Service = model-service
内部 transport = model-service 内部实现层
```

这里的收缩不仅是把 model domain 对外公开边界收口到一个 service，也要求把现有对业务上层暴露的 transport/runtime factory 概念收回到 `model-service` 内部。

## 当前问题

当前调用链大致是：

```text
thread-service / memory-service / app-server
  -> model-client
  -> model-provider-api / model-provider
  -> models-manager-api / models-manager
  -> api-runtime-api
  -> codex-api
  -> default-client / api-provider / api-auth / api-types
```

问题不在于层数绝对错误，而在于公开边界过碎：

- 上层要同时理解“模型目录”“provider”“session-scoped client”“底层 runtime factory”四组概念。
- `models-manager`、`model-provider`、`model-client` 都在 model 请求链路上，但 owner 边界对上层并不自然。
- `model-provider` 这个命名存在歧义：有时像 provider config，有时像 auth provider，有时又像 inference client。
- `model-client` 更像一次 turn/session 的请求 orchestrator，但在 crate 拓扑里和 manager/provider 并列，阅读成本高。

## 核心原则

### 1. 对外只保留一个全局 service API

上层只依赖：

- `ModelServiceApi`

不要继续让上层直接持有：

- `ModelsManager`
- `ModelProviderFactory`
- `ModelProvider`
- `ApiRuntimeFactory`

这些概念如果还需要，应该收缩为 `model-service` 内部协作对象。

### 2. 运行期请求能力与全局目录能力分开，但仍归属同一个 service

`ModelServiceApi` 既要能回答：

- 有哪些模型
- 默认模型是什么
- 某个模型支持哪些能力

也要能回答：

- 当前请求应该落到哪个 provider
- 该请求如何创建一个可复用的 model client

但不建议把所有请求方法直接堆到 `ModelServiceApi` 上。更合理的方式是：

- `ModelServiceApi`：全局模型发现、选择、解析、创建 client
- `ModelClientApi`：某个已解析模型/provider 的请求执行能力

### 3. transport 能力收进 model-service 的内部实现层

对业务上层来说，不应再直接感知：

- `ApiRuntimeFactory`
- `Provider`
- `AuthProvider`
- `default-client`
- transport-specific runtime client trait

这些能力应由 `ModelClientApi` 封装，并作为 `model-service` 的内部实现细节存在。

这不等于内部实现必须压成单文件或单模块，而是表示：

- transport 不再是业务上层直接依赖的独立公开边界
- 上层只通过 `ModelServiceApi` / `ModelClientApi` 使用模型请求能力

## 目标依赖关系

```text
thread-service / app-server / memory-service / other consumers
  -> model-service-api

model-service
  -> internal transport modules
```

禁止的依赖方向：

```text
model-service-api -> transport concrete crates
thread-service -> models-manager / model-provider / model-client / transport concrete crates
```

## 建议 API

### ModelServiceApi

`model-service-api` 对外只保留一个主 service trait。

```rust
/// 模型服务，对外提供模型目录、选择、解析与请求客户端创建能力。
pub trait ModelServiceApi: Send + Sync {
    fn list_models(&self, request: ListModelsRequest) -> ModelFuture<Result<Vec<ModelInfo>, ModelServiceError>>;

    fn get_model(&self, id: &str) -> ModelFuture<Result<Option<ModelInfo>, ModelServiceError>>;

    fn resolve_default_model(
        &self,
        request: ResolveDefaultModelRequest,
    ) -> ModelFuture<Result<Option<ModelInfo>, ModelServiceError>>;

    fn create_client(
        &self,
        request: CreateModelClientRequest,
    ) -> ModelFuture<Result<Box<dyn ModelClientApi>, ModelServiceError>>;
}
```

说明：

- `list_models` / `get_model` / `resolve_default_model` 负责模型目录能力。
- `create_client` 负责 provider 解析、auth 解析、请求能力选择。
- 上层不直接操作 `ModelProviderFactory` 或 `ModelsManager`。

### ModelClientApi

`ModelClientApi` 是 `model-service-api` 中的第二个公开 trait，但它不是第二个 service，只是 `ModelServiceApi` 返回的运行对象。

```rust
/// 已绑定模型/provider/auth 的请求客户端。
pub trait ModelClientApi: Send + Sync {
    fn stream_responses(
        &self,
        request: ResponsesModelRequest,
    ) -> ModelFuture<Result<ResponseStream, ModelRequestError>>;

    fn create_realtime_call(
        &self,
        request: RealtimeModelRequest,
    ) -> ModelFuture<Result<RealtimeCallHandle, ModelRequestError>>;

    fn compact(
        &self,
        request: CompactModelRequest,
    ) -> ModelFuture<Result<CompactionOutput, ModelRequestError>>;

    fn summarize_memories(
        &self,
        request: MemorySummarizeModelRequest,
    ) -> ModelFuture<Result<Vec<MemorySummary>, ModelRequestError>>;
}
```

说明：

- 这里表达的是“已解析后的模型请求能力”。
- 不再对上层暴露 `ApiRuntimeFactory`。
- `chat_completions` 如果只是 transport fallback，可继续作为 `stream_responses` 的内部实现细节；只有当上层确实需要显式选择 wire API 时，才在 request DTO 中体现。

### Turn 级运行对象

当前实现进一步明确区分了两层运行对象：

- `ModelClientApi`：session 级已解析 client，持有跨 turn 的 websocket fallback / window generation 等稳定状态。
- `ModelTurnClientApi`：从 `ModelClientApi` 派生出的 turn 级 client，持有单个 turn 内的 websocket 连接、sticky routing token、incremental request cache 等瞬时状态。

这层区分的理由是：

- `thread-service` 的 regular turn、compact、startup prewarm 需要复用单个 turn 内的 websocket / sticky state。
- 这些状态不应该继续由 `thread-service` 直接 new 旧 `ModelClientSession`。
- 但它们也不适合提升为新的全局 service。

因此当前 contract 采用：

```text
ModelServiceApi
  -> create_client(...) -> ModelClientApi
ModelClientApi
  -> create_turn_client() -> ModelTurnClientApi
```

约束：

- `ModelTurnClientApi` 只暴露 turn 运行期真正需要的能力，如 `stream_responses`、`prewarm_websocket`、`reset_websocket_session`、`try_switch_fallback_transport`。
- 不把 `provider/auth` concrete type 直接暴露进 `model-service-api`，避免把 `model-provider-api` 重新拉回公开依赖图并形成 cycle。
- 当某个 turn 需要使用不同 provider 时，由上层重新向 `ModelServiceApi` 请求一个新的 `ModelClientApi`，而不是让 `ModelTurnClientApi` 自己持有 provider factory。

### 当前迁移落点

当前已经切换到新 contract 的主链：

- `thread-service/src/session/turn.rs`
- `thread-service/src/compact.rs`
- `thread-service/src/session_startup_prewarm.rs`

也就是说，thread 侧这三条路径已经不再直接构造旧 `ModelClientSession`，而是统一通过 `model-service-api` 派生 turn client。

当前仍保留的遗留：

- `SessionServices` 内还保留 legacy concrete `model_client` 字段，主要是迁移期兼容与测试构造残留
- `thread-service` 的部分测试仍直接 new 旧 `ModelClient`
- review/task 子链虽然已经不再通过 concrete `model_client` 反查 auth，但相关 session 构造路径仍沿用旧的 `provider_auth_manager` / `model_provider_factory` 组合
下一步应优先收掉 `SessionServices` 中残留的 concrete `model_client` 字段，并同步清理测试构造与 review/task 辅助路径上的 legacy model wiring。

当前 bridge 实现约束：

- `ResponsesModelRequest` 应直接表达旧 `Prompt` 所需的业务字段，而不是退化成 transport-specific request。

## provider 相关附属接口的归类

`model-service` 不只是“模型推理调用”的 owner，它也是 provider 语义 owner。

因此除了 `responses` / `realtime` / `compact` 之外，凡是满足下面条件的接口，也应优先归到 `model-service` 的边界下思考：

- 请求语义属于某个 provider 生态
- 需要使用 provider 绑定的 auth
- 需要遵循 provider 的 base URL / routing / header 规则
- 本质上是 provider-aware 的通用网络能力，而不是某个产品后端的专用 typed API

当前可以分成三类：

### 1. 已经适合直接走 `model-service` 的请求

- remote plugin catalog / featured plugins
- remote skill catalog
- 其他已经使用 `ModelServiceApi::execute_provider_http(...)` 的 provider-aware HTTP

这些路径已经符合目标方向：

- provider 选择在 `model-service`
- auth 绑定在 `model-service`
- transport 仍由底层 `codex-client` 承担

### 2. 不应直接并入 `model-service` 的 backend 专用 domain

- `backend-client`

它当前承载的是：

- backend path style 选择
- ChatGPT backend 相关请求编排
- 专用 DTO / path / decode 逻辑

这些接口虽然也依赖鉴权、base URL、header 等 provider-aware 能力，但它们本质上是 OpenAI / ChatGPT backend 的专用业务接口，不应直接并入 `model-service` 的 typed API。

后续目标是：

- `backend-client` 或其后续重命名后的 service 保留 backend typed 语义
- `model-service-api` 只向它提供通用 provider-aware 能力
- 不让 `model-service` 直接承载 cloud task / usage / wham 之类产品专用 endpoint

### 3. 可以暂时直接依赖 transport 的路径

- signed upload URL 直传
- 与 provider 语义解耦的公网下载
- loopback / 本地测试 HTTP

这类路径即使带 header，也不自动等于 provider-aware。

判断标准不是“有没有鉴权”，而是：

- 请求是否真的属于 provider 语义 owner 的边界

## 当前收口原则

短期内先做两件事：

1. 底层 HTTP client 构造统一收敛到 `codex-client`
2. 通用 provider-aware 能力收敛到 `model-service`
3. backend 专用 typed 接口保留在各自 domain crate

因此当前允许的过渡态是：

- `backend-client` / `openai-files` / `cloud-tasks` 仍可能保留各自的 endpoint 逻辑
- 但不要继续各自复制 `reqwest + custom_ca + cookie + retry` 构造细节

这部分基础 transport 细节应统一复用 `codex-client`。
- `ModelResponseEvent` 应保持结构化 typed event，覆盖旧 `ResponseEvent` 的主要语义，避免退回 `serde_json::Value` metadata。
- `model-service` 内部允许暂时桥接旧 `model-client`、`models-manager`、`model-provider`，但这些概念不应再泄漏到上层 consumer。

## DTO 归属

### 应属于 model-service-api 的 DTO

这些 DTO 表达的是 model domain 语义，而不是底层 transport 细节：

- `ModelInfo`
- `ModelCatalogEntry`
- `ModelCapability`
- `ModelSelectionPolicy`
- `ListModelsRequest`
- `ResolveDefaultModelRequest`
- `CreateModelClientRequest`
- `ModelServiceError`
- `ModelRequestError`
- `RealtimeCallHandle`
- provider 选择结果里真正需要对上层可见的 metadata

### 继续属于 transport internal types 的 DTO

这些 DTO 本质上是 transport-neutral API request/response contract：

- `ResponsesApiRequest`
- `ResponsesStreamRuntimeRequest`
- `ChatCompletionsRuntimeRequest`
- `RealtimeCallRuntimeRequest`
- `ApiError`
- `ResponseStream`
- websocket / SSE / timing / rate-limit 相关 transport DTO

规则：

- 如果一个类型主要表达“模型业务语义”，放 `model-service-api`。
- 如果一个类型主要表达“底层接口如何发请求/收响应”，放在 `model-service` 内部 transport 模块或其内部 types crate。

## 现有 crate 的目标映射

### 合并进 model-service-api

- `model-provider-api`
- `models-manager-api`
- `model-provider-info` 中真正跨 owner 需要暴露的公共 DTO
- `model-client` 当前对外需要保留的 client trait 边界

### 合并进 model-service

- `model-provider`
- `models-manager`
- `model-client`

### 收进 model-service 内部实现层

- `codex-api-runtime-api`
- `codex-api`
- `codex-default-client`
- `codex-default-client-api`
- `codex-api-provider`
- `codex-api-auth`
- `api-types`

## 对现有概念的重解释

### models-manager

保留语义，但降为 `model-service` 内部模块，不再作为上层直接依赖的 public service。

职责：

- 加载 bundled catalog
- 合并 override
- 刷新模型目录
- 解析默认模型

### model-provider

保留语义，但降为 `model-service` 内部模块，不再作为对上层的 public 概念。

职责：

- provider config 解析
- auth 解析
- provider-specific endpoint / wire api 适配

### model-client

重命名语义为 `model-service` 里的请求执行模块，而不是独立 owner。

职责：

- per-session / per-turn 请求 orchestration
- websocket 复用
- fallback / sticky routing / telemetry
- 通过 `ApiRuntimeFactory` 发起具体请求

## 为什么返回 ModelClientApi 而不是 ModelProvider

不建议新的公开边界继续返回 `ModelProvider`，原因是 `provider` 一词在当前仓库里已经有多重含义：

- provider config
- auth provider
- API deployment
- 具体 inference endpoint

如果公开 API 返回 `ModelProvider`，后续很容易再次把目录元数据、provider 解析和请求执行混在一起。

`ModelClientApi` 的语义更直接：

- 这是一个已经完成模型与 provider 解析的可请求客户端
- 它的职责就是发请求

## transport 的新定位

transport 能力属于 `ModelClientApi` 背后的实现能力，而不是上层直接依赖的 service API。

也就是说：

- 对上层公开语义：`ModelClientApi`
- 对内部实现语义：HTTP / SSE / WebSocket / Realtime / auth / default headers / retry / telemetry

`ModelClientApi` 消费这些 transport 能力，但不等于 transport 类型本身继续对外暴露。

## 迁移步骤

### 第一步：先定 API，同时冻结 transport 为内部实现

- 新增 `model-service-api`
- 在文档中冻结 `ModelServiceApi` / `ModelClientApi` / DTO 归属
- 明确 `codex-api*` 栈不再作为业务上层可见边界

### 第二步：把上层 consumer 改成只依赖 model-service-api

- `thread-service`
- `app-server`
- `memory-service`
- 其他直接依赖 `model-client` / `models-manager-api` / `model-provider-api` 的 crate

目标是让上层不再感知旧的碎片 crate，也不再直接感知 transport/runtime factory 概念。

### 第三步：收实现 crate 与 transport crate

- 把 `model-provider`、`models-manager`、`model-client` 合入 `model-service`
- 把 `codex-api-runtime-api`、`codex-api`、`default-client`、`api-provider`、`api-auth`、`api-types` 收进 `model-service` 内部模块或内部 types/support crate
- 旧 crate 先保留兼容壳，最终删除

## 非目标

当前不做以下事情：

- 不把 transport 再作为业务上层单独公开 service
- 不重新设计 `api-types` 全部 DTO
- 不把所有模型请求形式都抽象成一个超大统一 request enum
- 不为了形式统一继续创建新的 `manager`、`provider factory`、`runtime factory` facade

## 完成标准

满足以下条件，才算 model 域 service 化完成：

- 上层 crate 只依赖 `model-service-api`，不再直接依赖 `model-client`、`models-manager-api`、`model-provider-api`
- 上层 crate 也不再直接依赖 `codex-api*`、`default-client*`、`api-provider`、`api-auth`、`api-types`
- `model-service` 成为唯一 concrete owner
- `model-client`、`models-manager`、`model-provider`、transport 相关实现 crate 不再以业务上层直接依赖的公开 owner 身份存在
- `model-service-api` 不反向依赖任何 concrete transport crate
- request 执行相关公开边界不再暴露 `ApiRuntimeFactory`、`ModelProviderFactory` 这类内部协作概念
