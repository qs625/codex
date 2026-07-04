# model-service

`model-service` 当前不应理解为“只负责模型调用”的 service。

它更准确的定位是 **provider 语义 owner**：负责所有和模型提供方相关的配置、鉴权、路由和 provider-aware 请求能力。

## 职责边界

这个 crate 负责：

- provider 配置发现与读取
- provider 选择
- provider auth 读取与绑定
- model client 构造
- provider-aware HTTP 请求
- provider 相关的 realtime / SSE / 其他附属接口

这个 crate 不负责：

- provider-agnostic 的底层 HTTP / TLS / retry 基础设施
- 与 provider 生态无关的普通网络请求

一句话说清楚：

- `model-service` 负责“这个请求属于哪个 provider，以及怎么带上 provider 语义”
- `transport` 负责“把请求发出去”

## 什么叫 provider-aware

满足以下任一条件的请求，都应视为 provider-aware：

- 需要 provider 选择
- 需要 provider/account auth
- 需要 provider 配置里的 base URL / headers / routing
- 语义上属于某个 provider 生态

典型例子：

- responses
- chat completions
- realtime
- compact
- provider catalog / models
- provider 生态里的通用 provider-aware HTTP

当前已经适合直接走这条边界的包括：

- remote plugin / remote skill 等 provider-aware HTTP

当前不应直接并入 `model-service` 的包括：

- `backend-client` 这类 OpenAI / ChatGPT backend 专用业务接口

这类接口虽然也需要鉴权和 base URL 处理，但它们是特定产品 backend 的 typed domain，不是通用 provider 能力。

更合理的边界是：

- `model-service` 提供通用 provider-aware 能力
- OpenAI backend domain 依赖 `model-service-api`
- 上层业务依赖 OpenAI backend 自己的 service / client

## 什么不属于 model-service

只有真正 provider-agnostic 的请求，才应该直接依赖底层 `transport`：

- signed URL 直传
- 公开下载
- 本地 loopback
- 和 provider 生态无关的普通 HTTP

是否需要鉴权，不是判断标准。

真正的判断标准是：

- 这个请求是否属于 provider 语义 owner 的范围

## 与 transport 的关系

`model-service` 内部可以依赖 `transport`，但不应让 `transport` 反过来承载 provider 语义。

关系应保持为：

- `model-service` -> `transport`
- provider-aware 业务 -> `model-service`
- provider-agnostic 业务 -> `transport`

## 对外能力

对外建议收敛为两类能力：

1. `ModelClientApi`
   - 面向模型调用
2. provider-aware HTTP 能力
   - 面向 provider 生态里的非模型 HTTP 请求

这样可以保持一个核心事实：

- `model-service` 是 provider owner
- 而不是整个系统的通用网络总入口
