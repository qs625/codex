# codex-rs Crate 收缩蓝图

## 背景

`codex-rs` 当前的主要问题，不是 domain 数量太多，而是 crate 被同时当作多种分层手段使用：

- domain owner
- API contract
- DTO/types
- transport/protocol
- client/server
- runtime/capability
- test/sample/devtool

这些切分轴叠在一起，导致 workspace 在目录层面表达出非常强的复杂度，但真实的 owner 边界并不清晰。

当前观察结论：

- workspace 总 crate 数量约 `171`
- 其中 `src` 文件数小于等于 `3` 的 crate 约 `85`
- 命名上存在大量层次型 crate：
  - `-api`：约 `33`
  - `-types`：约 `9`
  - `-client`：约 `10`
  - `-service`：约 `8`
  - `-service-api`：约 `8`

这说明目前不是“少量清晰 service + 必要配套 crate”，而是“过多抽象层以 crate 形式外显”。

## 目标

把 workspace 的主表达方式统一到两类：

```text
1. domain/service owner
2. 稳定跨 owner contract
```

其余内容尽量退回模块层，不再作为顶层 crate 暴露。

目标不是简单减少 crate 数量，而是让人一眼能回答下面几个问题：

- 这个 domain 的 owner crate 是谁？
- 这个 API 属于哪个 service？
- 这个 types crate 是否真的有跨 owner 复用价值？
- 这个 crate 是业务域，还是只是迁移时期抽出来的中间层？

## 收缩原则

### 1. 一个 domain 最多保留 2 到 3 个 crate

推荐形态：

```text
<domain>-service
<domain>-service-api
<domain>-types   (仅在确有跨 owner DTO 时保留)
```

默认不再额外拆出：

- `*-runtime`
- `*-runtime-api`
- `*-host`
- `*-bridge`
- `*-adapter`

这些应该是模块，不该是顶层 crate。

### 2. API crate 只定义“该 service 自己提供的能力”

例如：

- `tool-service-api` 只定义 tool service 对外提供的 API
- `thread-service-api` 只定义 thread/session/turn owner 暴露的 API 和运行期 capability

不要在某个 service 的 API crate 内重新定义它依赖别人的 host/port/caller trait。

### 3. capability 不单独成 domain

运行期 capability 本质上属于 thread/session/turn owner。

因此：

- thread/session/turn capability 统一收进 `thread-service-api`
- 不再单独保留 `runtime-capability-api` 一类 crate

### 4. DTO 默认跟 API 走

只有同时满足下面两个条件，才允许保留独立 `types` crate：

- 多个 owner crate 真正共享
- 这些类型不适合放进任一 owner 的 `service-api`

否则：

- service 对外 request/response/DTO 放入 `service-api`
- service 内部 struct 放入 `service`

### 5. transport/protocol 只保留真实边界

可以保留：

- `app-server-protocol`
- `exec-server-protocol`

因为它们对应真实 wire boundary。

不应保留：

- 仅为内部模块协作而拆出的 protocol/runtime/types 壳 crate

### 6. dev/test/sample 工具不应污染主架构

这类 crate 应尽量移出核心 workspace 叙事：

- sample
- debug client
- test client
- 仅供本地调试的 CLI

它们可以保留，但不应与核心 domain 平级表达。

## 建议的顶层分区

建议未来按语义把 crate 归到四层：

### 1. Services

核心业务 owner：

- `thread-service`
- `tool-service`
- `command-service`
- `approval-service`
- `mcp-service`
- `goal-service`
- `plugin-service`
- `memory-service`
- `workflow`

### 2. Platform

全局平台基础设施：

- `config`
- `state`
- `protocol`
- `thread-store`
- `model-provider`
- `models-manager`
- `login`
- `metrics/otel`

### 3. Adapters

对外入口与 transport：

- `app-server`
- `cli`
- `tui`
- `mcp-server`
- `exec-server`

### 4. Integrations

外部系统/执行环境适配：

- `codex-api`
- `chatgpt`
- `openai-files`
- `connectors`
- `network-proxy`
- `sandboxing`
- `hooks`

### 5. Devtools / Samples

- `thread-service-sample`
- `debug-client`
- `app-server-test-client`
- 其他仅供调试或验证的工具

## 当前应保留的核心家族

这些家族当前基本符合“service + service-api”方向，应作为后续基线继续收口：

### Thread

保留：

```text
thread-service
thread-service-api
```

要求：

- `thread-service-api` 同时承载 thread/session/turn capability
- 不再继续长出新的 runtime-capability crate

### Tool

保留：

```text
tool-service
tool-service-api
tool-types
```

说明：

- `tool-types` 暂时可以保留，因为 tool schema / call / output / discoverable metadata 复用面较广
- 后续如果 `tool-service-api` 已足以承载外部 contract，再评估是否并回

### Command

保留：

```text
command-service
command-service-api
```

要求：

- `process-exec`
- `shell-utils`
- 其他 shell/escalation/process 实现

应继续向 `command-service` 内部收口，不再作为平级 runtime 家族扩散。

### Approval

保留：

```text
approval-service
approval-service-api
```

要求：

- exec approval / network approval / guardian / permission policy 相关逻辑优先归于这里
- 不要再散落到 thread/service 调用方中作为独立 helper 家族

### MCP

保留：

```text
mcp-service
mcp-service-api
mcp-types
```

说明：

- `mcp-types` 暂可保留
- `mcp-tool-types` 不应长期单独存在

### Plugin / Memory / Goal

保留：

```text
plugin-service + plugin-service-api
memory-service + memory-service-api
goal-service + goal-service-api
```

这些方向目前已经比 thread 收口前清晰很多，不需要再引入新的中间层。

## 明确建议合并/删除的 crate

以下不是最终代码删除计划，而是架构方向上的 owner 合并建议。

### A. 直接并回现有 owner

#### `runtime-capability-api`

问题：

- 不是独立 domain
- 只是 thread 运行期 capability 的抽象容器

建议：

- 并入 `thread-service-api`

#### `process-exec`

问题：

- 本质上是 command execution implementation
- 对外不构成独立业务边界

建议：

- 并入 `command-service`

#### `shell-utils`

问题：

- 本质是 command domain 的 shell 解析/构造工具

建议：

- 并入 `command-service`

#### `permissions-runtime`

问题：

- 名称表达的是实现手段，不是业务 owner
- 实际内容横跨 approval policy / network approval / exec permission

建议：

- approval 主逻辑并入 `approval-service`
- sandbox 侧逻辑保留在 `sandboxing`

#### `mcp-tool-types`

问题：

- 是 MCP 与 Tool 的交叉 DTO
- 独立成 crate 会让 owner 不清楚

建议：

- 优先并入 `mcp-service-api`
- 若明显更偏 tool-facing，则并入 `tool-service-api`

#### `connectors-types`

问题：

- 大部分只是 connectors API 的 DTO

建议：

- 并入 `connectors-api`

### B. 重新归并 family，减少层级噪音

#### API runtime family

当前：

```text
api-runtime-api
api-provider
api-auth
api-types
codex-api
```

问题：

- “API client 的实现”
- “provider 选择”
- “transport contract”
- “DTO”

被拆成多层小 crate，表达过细。

建议方向：

```text
codex-api
api-client-api   (或保留 api-runtime-api，但需改名更清晰)
```

其中：

- provider/auth/types 尽量收回 owner family
- 只保留真正有跨 owner 价值的 contract

#### Telemetry / Metrics family

当前：

```text
metrics-api
session-telemetry-api
otel
otel-init
```

问题：

- telemetry contract 过碎
- session telemetry 作为一个单独顶层 crate，层级语义偏重

建议方向：

- metrics contract 与 telemetry contract 重新梳理
- session telemetry 若仅服务少数 runtime，可并回 owner API 或 metrics family

### C. 移出主 workspace 叙事

#### `thread-service-sample`

建议：

- 改为 `examples/` 或单独 dev crate

#### `app-server-test-client`

建议：

- 保留作为测试工具，但从主架构图中剥离

#### `debug-client`

建议：

- 视为 devtool，不参与核心分层叙事

## 当前需要重点收缩的几个家族

### 1. Config family

当前层级过多：

```text
config
config-loader
config-loader-remote
config-local-loader
config-permissions
config-requirements
config-schema
config-state
config-toml
config-types
config-diagnostics
config-edit
```

问题：

- 这是实现细节被 crate 化
- 不是清晰的 domain owner 分层

建议目标：

压缩到 3 到 5 个 crate 以内，例如：

```text
config
config-api/types
config-loader
config-schema   (如确有单独价值)
config-edit     (如确有 CLI/工具边界)
```

其余尽量退回模块。

### 2. App-server family

当前：

```text
app-server
app-server-client
app-server-daemon
app-server-protocol
app-server-transport
app-server-test-client
```

建议：

- 保留 `app-server`
- 保留 `app-server-protocol`
- 其余按“真实可执行边界”收缩

需要重点审查：

- `app-server-transport` 是否必须独立
- `app-server-client` 是否只是调用薄壳
- `app-server-test-client` 是否只属于测试工具

### 3. Connectors family

当前：

```text
connectors
connectors-api
connectors-types
```

建议：

- 优先收缩为：

```text
connectors
connectors-api
```

### 4. State family

当前：

```text
state
state-api
state-cli
```

这组相对合理。

建议：

- 保留 `state`
- 保留 `state-api`
- `state-cli` 视是否真有独立入口价值决定去留

## 具体收缩表

### 保留

```text
thread-service / thread-service-api
tool-service / tool-service-api / tool-types
command-service / command-service-api
approval-service / approval-service-api
mcp-service / mcp-service-api / mcp-types
plugin-service / plugin-service-api
memory-service / memory-service-api
goal-service / goal-service-api
state / state-api
thread-store / thread-store-api
app-server / app-server-protocol
cli
tui
mcp-server
exec-server / exec-server-api / exec-server-protocol
workflow / workflow-api
```

### 合并回 owner

```text
runtime-capability-api -> thread-service-api
process-exec -> command-service
shell-utils -> command-service
permissions-runtime -> approval-service / sandboxing
mcp-tool-types -> mcp-service-api 或 tool-service-api
connectors-types -> connectors-api
```

### 待重新分层

```text
api-runtime-api
api-provider
api-auth
api-types
metrics-api
session-telemetry-api
config-* family
app-server-* family
```

### 移出核心架构叙事

```text
thread-service-sample
debug-client
app-server-test-client
部分 *-cli 调试工具
```

## 执行顺序

建议按下面顺序推进，而不是并行大改：

### 第一阶段：清理明显中间层

- `runtime-capability-api`
- `process-exec`
- `shell-utils`
- `mcp-tool-types`
- `connectors-types`

这一阶段目标是去掉最典型的“名字像层，实际不是 owner”的 crate。

### 第二阶段：清理 dev/test/sample 噪音

- `thread-service-sample`
- `debug-client`
- `app-server-test-client`

目标是让 workspace 主列表更像产品架构，而不是开发辅助工具集合。

### 第三阶段：整理 family

- `config-*`
- `app-server-*`
- `api-*`
- `telemetry/metrics-*`

这阶段不是简单合并，而是先重画 family 内部 owner 边界，再决定保留多少 crate。

### 第四阶段：目录级重分组

等 owner 收敛之后，再考虑按目录重分组：

- `services/`
- `platform/`
- `adapters/`
- `integrations/`
- `devtools/`

不要在 crate 边界还不清楚时先做目录移动，否则只会把噪音换个位置继续存在。

## 判断标准

后续新增或保留一个 crate，至少要满足下面条件之一：

- 有独立 owner
- 有稳定跨 owner contract
- 有真实 transport / executable boundary
- 有明确编译隔离收益，且不损害架构可读性

如果不满足，就应该是模块，不应该是 crate。

## 结论

当前 `codex-rs` 的主要收缩方向不是继续按功能“拆”，而是把过去因重构、解耦、编译隔离和中间抽象而长出的 crate 重新收回到清晰的 owner 结构。

下一步的重点不是继续深拆某个单一 service，而是先把 workspace 级别的 crate 叙事收口成：

```text
少量明确 owner crate
+ 少量必要 contract crate
+ 极少数真实 protocol/client/server crate
```

如果这一步不先做，后续每推进一个 domain，都还会继续长出新的中间壳 crate。
