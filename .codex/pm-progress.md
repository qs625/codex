# PM Progress

## Index
- [Current Goal](#current-goal)
- [Active Work](#active-work)
- [Recent Completed](#recent-completed)
- [PM Progress Archive](pm-progress-archive/index.md)
- [Known Issues](#known-issues)

## Current Goal
Current installed Runtime Capsule is `sha256:3e6269c708c358b7665c900aa8dd77166dd5c4ae257382e16053ee6d758fc76b`, selected/externalCurrent/activeLaunch in runtime-launcher control state, installed by recovered restart request `call_74YbmRKaOr3SkSr4SJYNhgXn` (do not repeat the same restart request). Artifact manifest metadata sourceCommit is `05e9b669eab720945f9d000a2d5557b2f14d94bd`. Previous Runtime Capsule is `sha256:bb7f0bab0d56b639995f6ead7d38441c26d3f212dcb590e9fdfca40ed6a033f3`. `split-init-context-into-items` is merged into main and pending Runtime Capsule delivery from main commit `6abcc5a6fee0c6dfe9355eee07390e7c36a85830` plus this progress-record commit. Next action: build capsule-only Runtime Capsule, request a new full Runtime restart, and self-debug the installed UI.

## Active Work
No active product implementation work. `split-init-context-into-items` is merged and pending Runtime Capsule delivery. `source_workspace-dev` is idle/synced and available; `source_workspace-dev-3` has delivered branch `fix/init-context-multiple-items` at `decbd58e4` and should be synced after main delivery/progress settle; `source_workspace-dev-2` remains dirty/unavailable and should not receive new tasks until cleaned or explicitly reconciled.

## Recent Completed
- id: split-init-context-into-items
  status: merged_pending_capsule_delivery
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/init-context-multiple-items
  commit: owner `decbd58e4`; merge `6abcc5a6`
  summary: Typed `injectedContext.sections` now project as multiple adjacent Init Context context/tool items (`ctx-id:section:N`) instead of one item with `toolDetailSections`; compact replacement history uses the same section-item projection and context entries are prevented from being re-merged by compact cell grouping.
  validation: owner/reviewer approved; PM design验收 passed against the brief (typed projection, not CSS-only; no regex details splitting; persisted/model-visible history unchanged); main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts src/lib/thread.test.ts` 358/358. Pending capsule-only Runtime delivery and installed self-debug.

- id: compact-marker-non-expandable-ui
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `c64e67639`; merge `33aaff9cc`
  summary: Compact rows now render as non-expandable `Context compacted` markers; no compact summary `<details>`, summary preview/body, or “Summary available/View summary” appears in the active chat UI.
  validation: owner/reviewer approved; PM design验收 passed; main focused UI/lib tests 114/114 and then combined Init Context tests 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug DOM confirmed compact summary details/body count 0 and console 0 errors/warnings.

- id: split-init-context-display
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `ac04bd218`; merge `924c1a598`
  summary: Init Context remains one row/event but carries typed `toolDetailSections`; ToolRow details render section labels/text separately while preserving `toolDetails` as compatibility/search fallback.
  validation: owner/reviewer approved after virtualization height double-count fix; PM design验收 passed; main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts` 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug confirmed new renderer bundle and zero console errors/warnings.

- id: capsule-only-runtime-delivery
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  commit: owner `572d158d305b5a1d6a2f39aa7db90733e0168b7f`; merge `71e628861`
  summary: Added explicit `package:root-worker-prototype:mac:capsule` path that builds a complete Runtime Capsule into `runtime-launcher/incoming/<activationId>` without rebuilding the outer Launcher or DMG; full mac packaging still reuses the capsule stage.
  validation: owner/reviewer approved; PM design验收 passed; main script tests 9/9; actual delivery used `pnpm package:root-worker-prototype:mac:capsule`, produced `sha256:3e6269...`, and recovered restart `call_74YbmRKaOr3SkSr4SJYNhgXn` selected it as externalCurrent/activeLaunch. No DMG/full outer app packaging was used for this delivery.

- id: file-tree-open-normal-preview
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/file-tree-opens-file-preview
  commit: owner `d3a5ab0c8b8a9e14b1e8f9226f3fdec57fcd2cbf`; merge `53717f8991d08022f6db1d14c4f1a0b3e21c1bc3`
  summary: File tree file clicks now open normal file preview; Git panel staged/unstaged/commit file rows remain the diff-entry points. Removed file-tree-to-Git-diff resolver/status lookup.
  validation: owner/reviewer approved; PM design验收 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/RightPanel.test.tsx` 59/59; `git diff --check HEAD~1..HEAD` pass. Delivered in Runtime Capsule `sha256:bb7f0bab0d56b639995f6ead7d38441c26d3f212dcb590e9fdfca40ed6a033f3`; self-debug installed renderer URL points to `bb7f0bab...`, console 0 errors/warnings.

- id: computer-use-helper-signed-equivalence
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/computer-use-helper-signed-equivalence
  commit: owner `c108505c732a66328b4af4fb4974e204dcb3f0cd`; merge `e79fb2ecc26e2fa39d96a2bcc11b2356c5b6237b`
  summary: Stable Computer Use helper equivalence now normalizes Mach-O code-signature-only differences instead of raw hashing signed executable bytes, while still detecting real code/resource changes.
  validation: owner/reviewer approved; PM design验收 passed; main `node --test apps/root-worker-prototype/electron/installedArtifactUpdate.test.cjs apps/root-worker-prototype/electron/appServerClient.test.cjs` 85/85; package/restart installed `sha256:c1ea6473...`; explicit current-release materialization dry-check returned `status:"unchanged"` without changing inode/mtime. After user authorization, MCP `computer.permissions_status(includeObservation:true, includePerception:false)` reported Screen Recording granted, Accessibility true, stablePermissionSubject true. Subsequent UI Capsule delivery did not rewrite the helper.

- id: readme-project-comparison
  status: merged_no_capsule_required
  commit: `17bf5645c Document Morpheus project comparison`
  summary: README rewritten in Chinese with Morpheus overview and comparison against OpenAI Codex, Claude Code Best, and DeepSeek Harness/dsh; stable reference source locations documented. Pure documentation change; no Runtime Capsule needed.

## Known Issues
- 2026-09-19 Computer Use stable helper no-rewrite keeps `Contents/Resources/payload-electron-path` unchanged by design to avoid touching the stable helper bundle. After the `bb7f0bab...` delivery, MCP server diagnostics still show `execPath` from previous artifact `c1ea6473...` while the main Runtime is `bb7f0bab...`. This is currently safe because `c1ea6473...` is externalPrevious, but future artifact GC could break the stable helper MCP server if the referenced previous payload is removed. Need a follow-up design for a stable current-runtime indirection or non-TCC-breaking payload pointer update.
- 2026-09-19 `.morpheus/config.toml` remains dirty from user/runtime config changes and must not be committed accidentally.
- Older completed entries and stale known issues are archived in [PM Progress Archive](pm-progress-archive/index.md).
