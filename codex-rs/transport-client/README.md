# transport-client

这个 crate 当前应被视为过渡命名的 `transport` crate。

它的职责是提供 provider-agnostic 的底层网络能力，而不是承担任何业务 domain 的 owner 角色。后续可以 rename 为更中性的名字，例如 `transport`，但在 crate rename 之前，应先把架构边界和公开 API 语义收正。

## 职责边界

这个 crate 负责：

- `reqwest` client 构造
- custom CA / rustls 配置
- cookie store
- 通用 `Request` / `Response` / `StreamResponse`
- `HttpTransport` / `ReqwestTransport`
- retry / backoff
- SSE / bytes stream 等流式基础能力
- transport error 语义

这个 crate 不负责：

- provider 选择
- provider 配置发现
- provider auth 决策
- model / backend / file / MCP / login 等业务语义

一句话说清楚：

- `transport-client` 负责“怎么发请求”
- 不负责“这个请求属于哪个 provider / 业务”

## 与 model-service 的关系

`model-service` 的定位应理解为 **provider 语义 owner**，而不只是“模型调用 service”。

这意味着：

- provider-aware 请求应由 `model-service` 负责
- provider-agnostic 请求才直接依赖 `transport-client`

其中：

- provider-aware：需要 provider 选择、provider auth、provider 配置里的 base URL / headers / routing
- provider-agnostic：只是普通 HTTP / stream / TLS 能力，不带 provider 业务判断

所以 `transport-client` 不应该取代 `model-service`，也不应该把 provider 语义吸进来。

## 推荐分层

推荐分层如下：

1. `transport`
   - provider-agnostic 网络基础设施
   - 即当前 `transport-client` 的目标归宿
2. `service-api`
   - 每个 domain 自己的业务 API contract
3. `service`
   - 每个 domain 的实现
   - 内部按需依赖 `transport`

原则是：

- provider-aware 的业务，优先依赖 `model-service`
- provider-agnostic 的业务，直接依赖 `transport`
- 不要把 provider 语义下沉到 `transport`

## 哪些 crate 可以继续直接依赖 transport

只要它们的请求不需要 provider 语义判断，就可以直接依赖 `transport`。

当前典型包括：

- `exec-server`
  - owner：`http/request` 的实际执行方
- `login`
  - owner：device code、token exchange、本地 callback server 后续请求
- `rmcp-client`
  - owner：streamable HTTP MCP client

这些 crate 直接依赖 transport 是合理的，因为它们需要的是底层 HTTP 能力，不是 provider 选择能力。

## 哪些 crate 需要重新审视

下面这些 crate 虽然当前可能直接依赖 transport，但如果它们的请求语义属于某个 provider 生态，就要重新判断是否应回到 `model-service`：

- `backend-client`
  - 如果它代表 ChatGPT / Codex backend 生态请求，可能更接近 provider-aware
- `openai-files`
  - create / finalize 这类请求如果属于 provider 生态，可能应回到 `model-service`
- `cloud-tasks`
  - 如果只是普通环境探测 HTTP，可以直接依赖 transport；如果后续明显依赖 provider 语义，则应重新评估

所以判断标准不是“要不要鉴权”，而是：

- 这个请求是否属于 provider 语义 owner 的范围

## 重命名策略

当前不建议先立即做 crate rename。

更稳的顺序是：

1. 先明确 `model-service` 与 `transport` 的 owner 边界
2. 让 `transport-client` 只剩下 transport 语义
3. 再统一 rename 成中性名字，例如：
   - `transport`
   - `http-transport`
   - `network-transport`

推荐优先考虑 `transport`，语义最干净。

## 对外 API 去 `Codex` 化

在 crate rename 之前，公开 API 应优先使用中性命名，减少新的调用点继续固化 `Codex` 语义。

当前建议的迁移方向：

- `CodexHttpClient` -> `TransportHttpClient`
- `CodexRequestBuilder` -> `TransportRequestBuilder`
- `create_client()` -> `create_transport_client()`
- `build_reqwest_client()` -> `build_default_reqwest_client()`
- `try_build_reqwest_client()` -> `try_build_default_reqwest_client()`

当前仍然保留旧名字做兼容，但新增调用点应优先使用中性导出名。
