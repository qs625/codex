# Morpheus Android Companion

First-pass native Android client for connecting a phone to a Morpheus app-server
that is already running on another machine. The app is a remote UI only: the
app-server owns thread runtime, tools, filesystem access, auth, and persistence.

## What Works

- Connects to `ws://...` or `wss://...` app-server endpoints with optional
  `Authorization: Bearer ...`.
- Performs the app-server handshake: `initialize` request followed by the
  `initialized` notification.
- Lists recent threads with `thread/list`.
- Opens a selected thread with `thread/read` using `includeTurns: true`, then
  calls `thread/resume` with `excludeTurns: true` so live notifications stream
  into the selected conversation.
- Starts a new thread with `thread/start`.
- Sends text input with `turn/start`.
- Merges typed `thread/*`, `turn/*`, and `item/*` notifications into the same
  reducer used for read snapshots.

The renderer intentionally uses typed thread item `type` values from
app-server. Unknown future item types are displayed as compact JSON fallback
instead of parsing provider raw output or legacy assistant markers.

## Run The Server

From the repository's Rust workspace:

```sh
rtk cargo build -p app-server --bin app-server
```

For local network access, bind app-server to an address reachable by the phone.
Non-loopback WebSocket listeners require auth:

```sh
rtk ./target/debug/app-server --listen ws://0.0.0.0:8910 --ws-auth capability-token --ws-token-file /absolute/path/to/ws-token.txt
```

Then enter `ws://<machine-lan-ip>:8910` in the Android app and paste the token
from `ws-token.txt`.

For tunnel access, expose that listener with your tunnel provider and enter the
resulting `wss://...` URL in the app. Keep WebSocket auth enabled for tunnel or
public network access.

Loopback-only development can run without WebSocket auth, but a phone cannot
usually reach `127.0.0.1` on the development machine:

```sh
rtk ./target/debug/app-server --listen ws://127.0.0.1:8910
```

## Build The App

This directory is an independent Gradle Android project:

```sh
rtk gradle -p apps/android-companion testDebugUnitTest
rtk gradle -p apps/android-companion assembleDebug
```

If your environment does not have Gradle or the Android SDK installed, open
`apps/android-companion` in Android Studio and let it sync the project. The
core JSON-RPC and thread reducer logic lives outside Compose UI so it can be
covered by JVM unit tests.

## Scope Notes

This first version does not implement settings, file browsing/editing, approval
review UI, attachments, screenshots, or a tunnel service. It consumes the
existing app-server WebSocket JSON-RPC protocol directly and does not embed the
desktop Electron/Root Worker UI in a WebView.
