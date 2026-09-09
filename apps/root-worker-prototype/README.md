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
That request has no mode: it builds and signs one complete Runtime Capsule,
asks the stable Launcher to prepare it, stops the current `app-server`, and
exits with the coordinated restart code. The Launcher proves the old Runtime
tree has stopped before selecting and starting the candidate. Runtime Capsule
v1 is a trusted cooperative supervision contract, not a hostile same-UID
sandbox: every manifest prohibits daemonizing, double-forking, `setsid`, and
process-group escape. The typed completion evidence is
`CooperativeObservedEmpty`; any observed escape, identity mismatch, timeout,
ambiguous ownership, or residual process durably blocks selection, fallback,
commit, and further spawn. It does not claim kernel-contained proof for an
unobserved malicious escape. Readiness or
observation failure rolls back to the previous external Capsule, or to the
read-only Seed when no previous external Capsule exists. Launcher recovery
evidence is recorded on the durable `/self` thread before it is acknowledged.
Preparation failures leave the selected Runtime unchanged.
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
