# 组件拆分与开发 Handoff

## RunConfigTrigger

保持现有入口。

展示规则：

- 主文案：`运行配置`
- Meta：`{modelLabel} · {reasoningLabel}`
- 如果当前模型来自 configured provider，可保持 trigger 简短，不强制展示 provider；provider 在 popover 内解释。
- 当前 model label 取值：`displayName || model || id`，不要因 catalog 缺失显示空。

## ModelOption 数据字段

建议前端归一化出 picker 专用类型：

```ts
type RunModelSource = "catalog" | "configured" | "current";

type RunModelOption = {
  id: string;
  model: string;
  displayName: string;
  description: string;
  source: RunModelSource;
  hidden: boolean;
  supportedReasoningEfforts: RunReasoningEffortOption[];
  defaultReasoningEffort: string;
  isDefault: boolean;
};

type RunConfigSelection = {
  model: string;
  reasoningEffort: string;
};
```

字段说明：

- `catalog`：来自 `model/list` 的模型。
- `configured`：来自当前有效配置中的自定义 provider/model，可被选择。
- `current`：当前 thread model 无法在 catalog/configured 中匹配时构造的 synthetic item。
- 本次实现不扩展 v2 `Model` 字段；configured model 由 app-server 返回 `configured:<provider>:<model>` synthetic id，并把 provider 友好名写入 `description`。
- 前端 normalize 阶段从 synthetic id 或 `当前配置` 描述派生 `source === "configured"`；current missing item 由 renderer 本地构造。

## ModelList

每项结构：

- radio indicator
- model name
- meta：`Catalog · 默认 {effort}`、`Configured · 默认 {effort}` 或 `当前配置 · 未出现在 model/list`
- badge：仅 configured custom model 显示 `Configured`；synthetic current item 显示 `Current`

排序：

1. `source === "current"`
2. `source === "catalog" && isDefault`
3. `source === "configured"`
4. `source === "catalog"`

同组内：

- catalog：按 label 排序。
- configured：按 label 排序。

去重：

- key 使用 `id`；configured synthetic id 形如 `configured:<provider>:<model>`。
- 若 catalog 中已有同 `model` 条目，app-server 复用 catalog 条目并在 description 中追加当前配置来源，避免重复。
- 当前 apply API 不携带 provider，因此本次不支持同名不同 provider 的精确切换。

应用契约：

- 本次 `onApply` / IPC / thread runtime 仍只携带 `model` 与 `reasoningEffort`。
- 同名不同 provider 的精确选择留作后续 provider-aware selection；本次只保证当前 configured model 可见且不会被 picker 打开动作覆盖。

## CurrentMissingModelOption

触发条件：

- `selectedThread.model` 有值。
- 归一化后的 catalog/configured options 中没有同 provider/model 可匹配项。

行为：

- 固定显示为第一项并选中。
- 不调用 `resolveSelectionForModel` 自动改成默认 catalog model。
- Reasoning 使用当前 thread 的 `reasoningEffort`；如果为空，显示 `未知` 或仅隐藏 reasoning options，不主动补默认。
- 用户选择其它项前，`应用` 禁用，因为草稿未改变。
- `current` synthetic item 不能单独让 `应用` 进入可用状态；只有用户选择 catalog/configured item 后才允许应用，避免把无法验证来源的未知模型重新写回。

推荐文案：

- Meta：`当前配置 · 未出现在 model/list`
- 辅助说明：`继续显示；只有用户选择其它项才改变。`

## ReasoningSelector

规则：

- catalog/configured item：仅展示 `supportedReasoningEfforts` 中支持项。
- current synthetic item：如果缺少支持项，不展示可切换 reasoning；保留当前 reasoning label。
- 切换到支持项不包含当前 effort 的 model 时，自动选择 `defaultReasoningEffort`，并显示 `已回退到该模型默认 reasoning`。

## 状态文案

- Loading：`正在加载模型...`
- Error：`模型列表加载失败，当前配置未受影响。`
- Error detail：具体错误单独放在下一行或次级文本，例如 `错误详情：{message}`，不要直接拼在中文句号后。
- Retry：`重试`
- Empty：`暂无可用模型，当前配置未受影响。`
- Current missing：`当前配置 · 未出现在 model/list`
- Current missing helper：`继续显示；只有用户选择其它项才改变。`
- Running：`当前 turn 正在运行，结束后可应用切换`
- Fallback：`已回退到该模型默认 reasoning`
- Badge：`Configured`、`Current`

## 空状态规则

- model/list 为空但当前 thread 有 model：显示 synthetic current item + 空状态说明。
- model/list 为空且当前 thread 无 model：只显示空状态，`应用` 禁用。
- configured custom model 为空：不单独显示“暂无自定义模型”，避免制造无关信息。

## 可访问性验收

- Trigger 使用原生 button，`aria-haspopup="dialog"`，`aria-expanded` 跟随打开状态。
- Popover 打开后 focus 落到 dialog 容器或第一个可交互项；关闭、取消、Escape、外部点击后 focus 返回 trigger。
- Model list 与 Reasoning selector 使用 radio group 语义；如果保留 button + `role="radio"` 实现，需要支持方向键在同组内移动和选择，Tab 只在组之间移动。
- 每个 model option 的 accessible label 至少包含：模型名、来源（Catalog / Configured / Current）、默认 reasoning、是否当前选中。
- `Configured` / `Current` badge 不能只作为视觉标签，必须进入 accessible label。
- loading、error、fallback、running 状态使用可朗读文本；error/fallback/running 建议放在 `aria-live="polite"` 区域。
- 禁用 `应用` 时必须有可见原因文本，例如 running、未改变、模型列表错误或 current synthetic item 不可写回。
- 颜色不能作为唯一状态表达；选中、configured、current、error 都要有文本或形状辅助。

## 交互风险

- 现有 `normalizeModelListResponse` 会在当前 model 不存在时 fallback 到 default/首项；需要改为先构造 options，再保留当前项。
- 本次不扩展 v2 `Model` 协议字段；configured model 来源由 synthetic id 和 description 表达，前端只派生本地 `configured/current` 展示状态。
- 当前 apply selection 仍只有 `model` 与 `reasoningEffort`，所以不能支持同名不同 provider 的精确选择；后续若要支持，需要扩展 provider-aware selection。
- 打开 picker 是查看行为，不能调用 `onApply` 或更新 local override。
- Synthetic current item 只用于保留可见性，不应被当成已验证可选模型写回。
- 应用失败后必须回滚 optimistic update，现有 App 已有回滚逻辑，需覆盖 custom model 场景。
- custom provider 的 reasoning 支持项可能未知；未知时不要展示不可验证的 Low/Medium/High 全量切换。

## 开发入口

- UI：`apps/root-worker-prototype/src/components/RunConfigPicker.tsx`
- 归一化：`apps/root-worker-prototype/src/lib/runConfig.ts`
- 类型：`apps/root-worker-prototype/src/types.ts`
- IPC：`apps/root-worker-prototype/electron/main.cjs`
- app-server 协议：`codex-rs/app-server-protocol/src/protocol/v2/model.rs`

建议测试：

- 当前 catalog model 正常选中。
- configured custom model 显示 provider 与 `Configured`。
- 当前 model 不在 catalog 时不会 fallback。
- 打开后取消不改变 thread。
- loading/error/empty 不覆盖当前配置。
- running 时 `应用` 禁用。
