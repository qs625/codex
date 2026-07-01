# codex-rs Crate 聚合优先级计划

## 目标

在不重新打乱当前 service 重构成果的前提下，优先处理那些：

- owner 明显
- 迁移边界清楚
- crate 本身只是中间壳
- 收益高于风险

的聚合项。

这份计划是对 [workspace-crate-consolidation-blueprint.md](/Users/bytedance/Projects/my-codex/spec/workspace-crate-consolidation-blueprint.md:1) 的执行版拆解。

## 优先级定义

### P0

立即可做，且应优先完成：

- 中间壳特征明显
- 不需要先重画大范围 domain 边界
- 完成后能直接降低 workspace 噪音

### P1

可以紧接着做，但需要先确认 family 内部边界：

- crate 之间存在真实复用
- 需要决定 DTO/API 最终归属
- 迁移后会影响较多引用点

### P2

暂不直接动代码，先做架构整理：

- family 太大
- 现在的 crate 组织已经混合 domain、实现细节和历史兼容层
- 如果直接合并，容易做成新的混乱状态

## P0 任务

### 1. `runtime-capability-api -> thread-service-api`

#### 判断

- `runtime-capability-api` 不是独立 domain
- 它表达的是 thread/session/turn 运行期 capability
- 这类 contract 的 owner 本来就应该是 `thread-service-api`

#### 建议动作

- 把 `ThreadCapability` 及相关能力定义迁入 `thread-service-api`
- 全局删除对 `runtime-capability-api` 的依赖
- 删除该 crate

#### 影响范围

- `thread-service-api`
- `tool-service-api`
- `workflow-api`
- 其他引用 capability 的 service/api crate

#### 风险

- 低

#### 收益

- 去掉一个明显“不是 domain 却占顶层 crate 名额”的中间层
- capability owner 更清晰

---

### 2. `process-exec -> command-service`

#### 判断

- `process-exec` 本质是 command execution implementation
- 不是独立业务域
- 对外 contract 已经由 `command-service-api` 表达

#### 建议动作

- 把 `process-exec` 的实现迁入 `command-service`
- `command-service` 内部模块化承接 process manager / child process / wait / stdin 等逻辑
- 删除 `process-exec` crate

#### 影响范围

- `command-service`
- `command-service-api`
- 直接依赖 `process-exec` 的调用方

#### 风险

- 中低

#### 收益

- command family owner 清晰
- 进一步收口 shell/exec/process 相关实现

---

### 3. `shell-utils -> command-service`

#### 判断

- shell 解析、argv 构造、展示辅助，本质属于 command domain
- 不值得作为平级顶层 crate 存在

#### 建议动作

- 把 shell parsing / formatting / command helper 并入 `command-service`
- 仅在确实存在跨 domain 通用逻辑时，保留 crate-private 或内部公共模块
- 删除 `shell-utils` crate

#### 影响范围

- `command-service`
- `cli`
- `app-server`
- 其他直接依赖 shell helper 的 crate

#### 风险

- 中低

#### 收益

- command family 进一步完整
- 降低“工具型小 crate”噪音

---

### 4. `connectors-types -> connectors-api`

#### 判断

- 目前 `connectors-types` 大多是 connectors API DTO
- 独立存在会让 owner 看起来分成三层：`connectors / connectors-api / connectors-types`

#### 建议动作

- 把 `AppInfo`、`AppMetadata`、`AppSummary` 等 DTO 并回 `connectors-api`
- `connectors` 只依赖 `connectors-api`
- 删除 `connectors-types`

#### 影响范围

- `connectors`
- `app-server`
- `tui`
- `plugin-service` 或其他使用 connector DTO 的模块

#### 风险

- 低

#### 收益

- 一个 family 直接从 3 层收成 2 层

## P1 任务

### 5. `mcp-tool-types -> mcp-service-api` 或 `tool-service-api`

#### 判断

- 这是交叉域 DTO，不适合长期独立
- 但归属需要先判断：更偏 MCP 边界，还是更偏 tool dispatch 边界

#### 建议决策标准

如果这些类型主要表达：

- MCP server/tool metadata
- MCP tool invocation contract

则归 `mcp-service-api`

如果这些类型主要表达：

- tool assembly
- tool dispatch 中对 MCP tool 的统一建模

则归 `tool-service-api`

#### 建议动作

- 先列出现有 public type 和引用方
- 判定 owner
- 迁移后删除 `mcp-tool-types`

#### 风险

- 中

#### 收益

- 去掉一个明显的“交叉层壳 crate”

---

### 6. `thread-service-sample` 移出核心 workspace

#### 判断

- 它不是架构 owner
- 它只是 sample / smoke / demo 性质

#### 建议动作

- 改为 `examples/`
- 或移入 `devtools/`
- 或单独保留但不纳入主 workspace members

#### 风险

- 低

#### 收益

- 降低主 workspace 清单噪音

---

### 7. `debug-client` / `app-server-test-client` 移出核心叙事

#### 判断

- 这类 crate 不是产品架构的 owner
- 它们属于开发/测试辅助

#### 建议动作

- 保留代码能力
- 但从架构视角单独分组
- 如条件允许，减少直接出现在主 members 清单中的程度

#### 风险

- 低

#### 收益

- workspace 列表更接近产品架构，而不是工具集合

## P2 任务

### 8. API client family 重构

当前相关 crate：

```text
api-runtime-api
api-provider
api-auth
api-types
codex-api
```

#### 问题

- 有的在表达 transport contract
- 有的在表达 provider / auth / DTO
- 有的在表达 concrete runtime

现在 family 的切分轴不统一。

#### 当前不建议直接合并的原因

- 需要先判断最终 owner
- 需要区分“跨 provider 稳定 contract”和“OpenAI/Codex 具体实现”

#### 先做什么

- 先画 API client family 依赖图
- 再决定是压成 2 个 crate，还是保留 3 个 crate

---

### 9. Telemetry / Metrics family 重构

当前相关 crate：

```text
metrics-api
session-telemetry-api
otel
otel-init
```

#### 问题

- contract 粒度偏碎
- telemetry 结构是按实现和上下文混着拆

#### 当前不建议直接合并的原因

- 会牵连较多 runtime owner
- 需要先明确：metrics contract、session telemetry contract、otel implementation 的边界

---

### 10. Config family 重构

这是整个 workspace 里最需要收口，但也最不能草率动刀的 family：

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

#### 问题

- crate 太多
- 有些表达存储格式
- 有些表达装载来源
- 有些表达校验/约束
- 有些只是实现细节

#### 当前不建议直接合并的原因

- 很容易把不同职责强行塞回一个超大 `config` crate
- 会制造新的 owner 模糊

#### 先做什么

- 先把 config family 分为：
  - config model
  - config load
  - config validation
  - config editing/tooling
- 然后再决定收成几层

---

### 11. App-server family 重构

当前相关 crate：

```text
app-server
app-server-client
app-server-daemon
app-server-protocol
app-server-transport
app-server-test-client
```

#### 问题

- 有 transport 边界
- 有可执行边界
- 有 client helper
- 有 test tool

它们不应该都平铺为同一级别的“核心架构 crate”。

#### 先做什么

- 先区分：
  - 真正 protocol boundary
  - executable boundary
  - client SDK/helper
  - test/dev tool

## 推荐执行顺序

### 第 1 批

建议先做：

1. `runtime-capability-api -> thread-service-api`
2. `connectors-types -> connectors-api`

原因：

- 改动边界最清楚
- 风险最低
- 能快速验证聚合路线没有问题

### 第 2 批

然后做：

3. `process-exec -> command-service`
4. `shell-utils -> command-service`

原因：

- 它们都属于 command family
- 连续处理，能避免 family 内来回改依赖

### 第 3 批

之后做：

5. `mcp-tool-types` owner 判定并迁移

原因：

- 需要一点 owner 判断
- 但规模仍可控

### 第 4 批

最后清理：

6. sample / debug / test client 类 crate 的 workspace 归位

## 完成标准

每个聚合项完成时，都要满足：

1. 原 crate 不再承担生产 owner 角色
2. 引用点全部切到目标 owner crate
3. 没有留下新的 facade/bridge 壳 crate
4. workspace members 更少，且语义更清楚
5. family 结构更接近：

```text
service
service-api
可选 types
```

而不是：

```text
service
api
types
runtime
runtime-api
host
bridge
adapter
sample
```

## 结论

按当前仓库状态，最适合的推进方式不是“大范围同时聚合”，而是优先吃掉最明显的中间壳 crate。

也就是：

```text
runtime-capability-api
connectors-types
process-exec
shell-utils
mcp-tool-types
```

这几项处理完之后，workspace 的层次会先明显清楚一截。后面再去动 `config-*`、`api-*`、`app-server-*` 这些大 family，风险会小很多。
