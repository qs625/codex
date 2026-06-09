# 轻量调研

## 调研范围

这是对现有 `RunConfigPicker` 的轻量增量设计，不是新产品或大页面改造。调研重点放在现有代码、相邻设计产物和常见工具型 picker 模式。

## 现有产品基线

- `RunConfigPicker` 已是 header 中的 popover 入口，包含 model radio list、reasoning radio group、加载/错误/空结果、取消/应用。
- `model/list` 当前由 Electron IPC `codex:listModels` 调用 app-server v2 `model/list`，入参为 `{ includeHidden: false }`。
- 当前前端 `RunModel` 类型只包含 `id`、`model`、`displayName`、`description`、`hidden`、`supportedReasoningEfforts`、`defaultReasoningEffort`、`isDefault`，尚未表达 provider/source。
- 当前 normalization 会按默认项优先、label 字母排序；若当前 model 不在返回列表中，会 fallback 到 default/首项，这正是本次需要修正的风险点。

## 同类模式

- 工具型 model picker 通常需要区分“可选择全集”和“当前已配置值”。当当前值不在列表中，推荐保留当前值并给出来源/未知状态，而不是隐式替换。
- Provider 信息适合放在次级 meta 行或右侧短标签，不应压过模型名。
- 自定义/配置来源适合用中性 `Configured` 标记，避免误导为推荐或警告。
- 错误恢复文案要强调“不影响当前配置”，降低用户对打开 picker 的风险感知。

## 设计约束推导

- 不新增全局管理入口：这次只解决 picker 中的可见性和选择安全。
- 不把 custom model 做成独立 tab：模型数量通常有限，分组/排序足够；tab 会增加寻找成本。
- 不在打开时自动写入默认模型：打开 picker 是查看行为，不应触发配置变更。
- 不使用高饱和警示色标记 custom model：custom 是正常配置状态，不是异常。

## 参考文件

- `apps/root-worker-prototype/src/components/RunConfigPicker.tsx`
- `apps/root-worker-prototype/src/lib/runConfig.ts`
- `apps/root-worker-prototype/src/types.ts`
- `apps/root-worker-prototype/electron/main.cjs`
- `codex-rs/app-server-protocol/src/protocol/v2/model.rs`
- `ui-design/model-reasoning-switcher/`
