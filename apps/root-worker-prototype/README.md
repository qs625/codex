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

The macOS app packaging builds release `app-server`, bundles it into the app
resources at `Contents/Resources/bin/app-server`, and bundles the default
compact prompt seed at `Contents/Resources/default-config/compact/COMPACT.md`.
It does not bundle a repository source snapshot into the `.app` or `.dmg`.
When launched from Finder or Dock, the app prepares `MORPHEUS_HOME`, preserves
the desktop process environment with an enhanced `PATH`, and creates
`~/.morpheus/compact/COMPACT.md` only if that file is missing.
Model-visible instructions also include ordinary, non-hidden files directly
under `MORPHEUS_HOME/instructions/`, loaded in stable filename order and subject
to the normal instruction byte budget.
When a packaged app starts without `ROOT_WORKER_WORKSPACE`, it uses
`rtk git clone git@github.com:qs625/codex.git ~/.morpheus/source_workspace` the
first time that writable workspace is missing, then starts `app-server` with
that real git workspace as both `cwd` and `ROOT_WORKER_WORKSPACE`.
Existing `source_workspace` contents are never overwritten, pulled, reset, or
otherwise changed automatically by launch.
The packaged app maintains
`~/.morpheus/instructions/morpheus-source-workspace.md` with the effective
source workspace path and the reminder to build/test Morpheus code changes
before calling `request_runtime_restart`. That generated file is updated only
while it still carries the Morpheus managed marker; user-managed replacement
content is left intact.
It also maintains `~/.morpheus/self-project.json` as a hidden/system `/self`
project record whose workspace is the same Morpheus source workspace. The
Electron IPC contract exposes `getSelfProject` and `startSelfCommand` for a
dedicated self command surface; `startSelfCommand` always creates a `/self`
project thread in that workspace and submits the provided task text there.

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
