# 信息架构

## 页面结构

本次不改变页面结构。现有 root-worker prototype 可以继续保持：

- Thread 列表：显示可切换的 thread、运行状态、最近更新时间。
- Thread 内容区：显示当前 thread 的 `ThreadItem` 列表。
- Live 状态区：显示订阅、运行、重连、错误等状态。
- Composer / 操作区：保留现有输入和控制入口。

## 信息层级

### 一级信息

- 当前选中的 thread。
- 可见 `ThreadItem` 内容。
- 正在运行或已完成的 turn 状态。

### 二级信息

- Live subscription 状态。
- Thread metadata，例如 last active、loaded 状态、resume cursor。
- 错误与恢复入口。

### 三级信息

- 调试用 reducer 状态。
- snapshot/read 来源诊断。
- item lifecycle 异常记录。

## 状态模型

建议将 thread 本地状态拆成四类语义：

- `cold`：本地没有 thread 内容，需要 `thread/read` 初始化。
- `loadedLive`：已有 live cache，切换展示只读 cache。
- `recovering`：用户或系统显式恢复，允许受控 read/reconcile。
- `subscriptionOnly`：只更新订阅状态，不触碰 items。

## 响应式策略

本次策略与布局断点无关。桌面和窄屏都应遵守相同数据权威规则：

- 内容区来自 typed `ThreadItem`。
- loaded thread 切换不做 snapshot merge。
- subscription metadata 与 display items 分离更新。

