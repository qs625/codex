# Review

## 当前状态

未完成独立 review。

本次根据上游要求立即收敛 handoff，尚未等待 `@ui-ue-reviewer` 完成正式设计审查。进入开发前仍需独立 review 检查：

- output/exit notification 是否与 command cell live tail 清晰区分。
- typed data contract 是否足够支持 session 参数展示。
- RightPanel failed retention 是否需要明确 dismiss 或时间窗口。
- 虚拟列表定位是否满足键盘和屏幕阅读器预期。
