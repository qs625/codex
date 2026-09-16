# Root Worker Prototype

Electron product shell for a root-agent plus worker-agent workspace.

## Run

```bash
pnpm install
pnpm --filter @my-codex/root-worker-prototype dev
```

This starts:

- Vite on `http://localhost:5173` for the Electron renderer
- Electron, which starts a bundled `app-server` when running from a packaged app; source-tree runs fall back to `../../codex-rs/target/debug/app-server`, `../../codex-rs/target/release/app-server`, then `app-server` from `PATH`
- the prototype defaults `MORPHEUS_HOME` to `~/.morpheus`

Use this only when you specifically want the Vite dev server flow.

You can override the app-server command, Morpheus home, or workspace with:

```bash
MORPHEUS_HOME=/tmp/my-codex-root-worker-ui-env/morpheus-home \
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace \
APP_SERVER_CMD="app-server --listen stdio://" \
pnpm --filter @my-codex/root-worker-prototype dev
```

## Build

```bash
pnpm --filter @my-codex/root-worker-prototype build
```

## Package macOS

```bash
pnpm --filter @my-codex/root-worker-prototype package:mac
```

Windows and Linux package scripts run on their native CI runners:

```bash
pnpm --filter @my-codex/root-worker-prototype package:win
pnpm --filter @my-codex/root-worker-prototype package:linux
```

The macOS package is a stable, read-only outer app whose entrypoint is
`Contents/MacOS/MorpheusLauncher`. The outer app also contains a complete,
signed Seed Runtime Capsule under `Contents/Resources/seed-capsule`; that
Capsule carries the full `Root Worker Runtime.app`, including Electron
main/preload/renderer, `app-server`, and default config. Packaging does not
bundle a repository source snapshot into the `.app` or `.dmg`.
When launched from Finder or Dock, the app prepares `MORPHEUS_HOME`, passes a
bounded desktop environment allowlist with `PATH`, and creates
`~/.morpheus/compact/COMPACT.md` only if that file is missing.
Model-visible instructions also include ordinary, non-hidden files directly
under `MORPHEUS_HOME/instructions/`, loaded in stable filename order and subject
to the normal instruction byte budget.
The optional source workspace is `~/.morpheus/source_workspace` unless
`ROOT_WORKER_WORKSPACE` is set. Packaged startup never clones, pulls, resets, or
otherwise creates source code automatically. Without a valid source workspace,
the Seed or current external Capsule still runs, while producing a new
candidate is reported as unsupported.
When a valid source workspace exists, the packaged app maintains
`~/.morpheus/instructions/morpheus-source-workspace.md` with its path and a
reminder to run relevant tests before calling `request_runtime_restart`.
Runtime Capsule 更新模型（中文说明）：

- Electron 完整产出候选 Capsule 后，只调用一次 Launcher
  `select-candidate`。Launcher 在 state lock 内导入、验证并原子更新
  `external_previous <- external_current` 与
  `external_current/selected <- candidate`。
- 选择成功后旧 Runtime 以普通退出码 `0` 退出；不再存在 ready marker、
  token、ack、payload self-registration、prepare/cancel/rollback 等多阶段
  握手。Launcher 直接监督 payload child，并在其退出后按最新 `selected`
  循环启动。
- 已选择的 external Capsule 无法加载或 spawn 时，Launcher 在本地回退到
  `external_previous`；没有 previous 时回退只读 Seed，并持久化 failure
  evidence。Capsule schema v2 是唯一支持的 Capsule 格式，旧 v1
  Capsule 不再兼容。
- Runtime Capsule 是 cooperative best-effort contract，不是 hostile
  same-UID sandbox：manifest 禁止 daemonize、double-fork、`setsid` 和
  process-group escape，但 Launcher 不宣称 kernel-contained proof。若
  Launcher 异常退出，下一次启动会只在 PID、start identity 与 process
  group 都可精确归属时清理残留；身份不符、未跟踪 group member 或其他
  归属歧义绝不发信号，只记录诊断并释放旧记录，必要时由用户手工关闭残留。
  `spawn()` 成功但 identity 尚未来得及持久化时仍有一个有界的 best-effort
  残留窗口；不会用 sidecar、parent-death helper 或隐藏监控器扩大协议。
That generated instruction file is updated only while it still carries the
Morpheus managed marker; user-managed replacement content is left intact.
It also maintains `~/.morpheus/self-project.json` as a system `/self` project
record whose workspace is the same Morpheus source workspace. The Electron IPC
contract exposes `getSelfProject` and `startSelfCommand` for a dedicated self
command surface. Press Cmd+P in the desktop app to open that `/self` input;
submitting it sends the provided task text to a `/self` project thread in the
source workspace. App startup and thread-list refresh ensure a real `/self` root
exists, and that root is shown in the ordinary project tree so it can be
selected from the normal sidebar.

Desktop release automation lives in `.github/workflows/desktop-release.yml`.
Pushing a tag like `desktop-v1.2.3` builds and uploads GitHub Release assets:

- `Root Worker Prototype-arm64.dmg` from macOS
- `Root Worker Prototype-win32-x64.zip` from Windows, containing the packaged
  app bundle and `.exe`
- `Root Worker Prototype-linux-x64.tar.gz` from Linux, containing the packaged
  app bundle

These artifacts do not include a repository source snapshot. They rely on the
installed app's origin-clone workspace setup described above. The workflow does
not perform Apple notarization or Windows code signing; macOS is ad-hoc signed
by the local `package:mac:app` script and Windows/Linux artifacts are unsigned.
DMG staging preserves Electron Framework bundle-relative symlinks so the app
does not depend on the build checkout after being dragged to `/Applications`.
CI DMG creation skips Finder-driven icon layout so macOS runners do not depend
on GUI AppleEvents. GitHub Release upload uses the workflow `GITHUB_TOKEN`, so
CI execution does not depend on local `gh` login; local `gh` authentication is
only useful for manual release inspection or creation from this machine.

The packaged app uses the stable bundle id
`com.openai.root-worker-prototype.dev` for macOS privacy prompts. `Info.plist`
declares microphone, screen capture, and app automation usage descriptions.
Screen Recording and Accessibility approval are still granted by macOS to the
installed, signed app identity in System Settings; they are not replaced by
external computer-use permissions or by the cloned source workspace.

The agent-facing Computer Use entrypoint is the repository CLI, not the
renderer/preload IPC surface:

```bash
node scripts/morpheus-computer-use.mjs run \
  --app com.apple.finder \
  --json \
  --actions '[{"type":"start"},{"type":"observe"},{"type":"move","x":420,"y":360},{"type":"stop"}]'
```

The CLI keeps a Computer Use session inside a single `run` process and reuses
the Electron ComputerUseManager safety gates, target preflight, trace evidence,
and native macOS bridge. CLI v1 supports `start`, `observe`, `move`, and `stop`;
`move` only updates the agent cursor/path evidence and does not move the macOS
system cursor. `click`, `type`, `key`, and `drag` are reported as blocked until a
future confirmation boundary exists. Screenshot evidence includes a bounded data
URL by default and omits the temporary capture path because the CLI cleans up
that file before returning; pass `--omit-screenshot-data` for metadata-only
output.

## Electron

```bash
pnpm --filter @my-codex/root-worker-prototype start
```

`start` now loads the built renderer from `dist/index.html` even when running from the source tree. If the build output is missing, Electron exits with an error telling you to run the build first.

For manual prototype iteration without Vite hot reload, use:

```bash
pnpm --filter @my-codex/root-worker-prototype build
pnpm --filter @my-codex/root-worker-prototype start
```
