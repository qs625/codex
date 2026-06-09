# 信息架构

## 页面位置

入口保持在 thread header 右侧的 `RunConfigPicker`，不新增页面、不新增导航。

## Popover 结构

1. Header
   - 标题：`运行配置`
   - 说明：`更改后仅影响当前 thread 的后续消息`
2. Model section
   - Label：`模型`
   - Helper：`Catalog 与当前配置`
   - 列表项：模型名、来源 meta、默认 reasoning、状态标记。
3. Reasoning section
   - Label：`Reasoning`
   - Helper：`随所选模型支持项变化`
   - Radio options：仅展示支持项。
4. State message
   - fallback、loading、error、empty、running。
5. Footer actions
   - `取消`
   - `应用`

## 列表分组与排序

推荐不使用可折叠分组，改用稳定排序和短 meta：

1. Synthetic current item：仅当当前 model 不在任何列表中时出现，固定第一。
2. Catalog default model：`isDefault` 为 true 的 catalog model。
3. 其他 catalog models：按 `displayName || model || id` 字母排序。
4. Configured custom models：按 provider 名，再按 `displayName || model` 排序。

如果 configured custom model 与 catalog model 的 `model` 完全相同，本次实现复用 catalog item，并在 meta 中追加当前配置来源；不要显示两个同名可选项。因为当前 apply payload 只保存 `model` + `reasoningEffort`，同名不同 provider 的精确选择留作后续 provider-aware selection。

## 响应式策略

- 目标平台是桌面 Electron，popover 宽度保持约 360-420px。
- 小宽度下模型名单行截断，provider 和标记仍可见；描述最多一行。
- 截断优先级：先截断 description，最后截断 model label；`Configured` / `Current` badge 不截断。
- 不使用嵌套卡片；列表项是紧凑行。
- Header trigger 高度保持稳定，长 model label 省略，不撑开 header。

## 信息层级

- 一级：模型名与当前选中状态。
- 二级：Catalog / Configured / 当前配置、provider 名、默认 reasoning。
- 三级：描述、错误详情。
- 状态标记只辅助扫描，不替代文本说明。
