# Haven Native (Pure Rust Client)

**Haven Native** is a 100% Pure Rust desktop client for [Haven](https://github.com/khanhtg2011/haven-desktop), built with **Slint**, **Tokio**, **Reqwest**, and native **Win32/WASAPI** APIs with zero Chromium, Electron, or WebView2 runtime overhead.

> **Pre-Release Notice:** All builds of `haven-native` (`0.2.0-pre.*`) are published exclusively as GitHub Pre-Releases while voice/media parity features continue to evolve.

---

## Architecture & Key Features

- **Hardware-Accelerated Slint UI (`ui/`):**
  - Uses native GPU rendering (`FemtoVG` / `winit`) by default; supports software fallback via `HAVEN_SW_RENDER=1`.
  - Virtualized `ListView` message history paired with persistent `Rc<VecModel<T>>` incremental updates (`O(1)` optimistic message sends and deduplication).
- **Non-Blocking Off-Thread Image & Avatar Cache (`src/cache.rs`):**
  - OS-local cache (`%LOCALAPPDATA%\HavenNative\cache`) with `128`-entry / `48 MB` RAM LRU and `256 MB` disk LRU caps.
  - Bounded concurrent downloads (`Semaphore(6)`), `tokio::task::spawn_blocking` image decode/resize, in-flight deduplication, animated GIF avatar preservation & frame stepping, and corrupt disk entry auto-recovery.
- **Resilient Networking & Security (`src/net/`):**
  - Full-duplex Socket.IO / WebSocket client with exponential backoff reconnection (`1s..30s`), `10s` handshake timeouts, and rejection of `44` auth failures.
  - Incremental `online-users-delta` presence merging and live `ping-check`/`pong-check` RTT telemetry.
  - Strict TLS certificate validation enabled by default (override only via `HAVEN_INSECURE_TLS=1` for local self-signed dev servers).
- **On-Demand WASAPI Audio Engine (`src/audio/device.rs`):**
  - Initializes lazily (`AudioEngine::new_idle()`) with `0%` idle audio overhead until entering voice, using atomic stereo ring-buffer pairs (`ringbuf`) and real-time resampling to 48 kHz stereo.
- **Native Windows Integrations (`src/overlay/`, `src/tray/`, `src/hotkey/`):**
  - Win32 click-through game HUD overlay, Windows System Tray icon, and global hotkeys (`CapsLock` PTT, `Ctrl+Shift+M` Mute, `Ctrl+Shift+D` Deafen, `Ctrl+Shift+O` Overlay).
- **Ecosystem Operations Docs (`docs/ops/`):**
  - Tuning guides for VPS `zram`/sysctl/Coturn ([vps-tuning.md](docs/ops/vps-tuning.md)), voice bots ([bots-optimization.md](docs/ops/bots-optimization.md)), and `haven-chat` ([haven-chat-tuning.md](docs/ops/haven-chat-tuning.md)).

---

## Companion Haven Repositories

- [`haven-desktop`](https://github.com/khanhtg2011/haven-desktop) — Electron Desktop client (`main`)
- [`haven-desktop-tauri`](https://github.com/khanhtg2011/haven-desktop-tauri) — Tauri v2 + WebView2 Desktop client with 240 FPS Win32 GDI+ HUD overlay
- [`haven-mobile`](https://github.com/khanhtg2011/haven-mobile) — Tauri v2 Android client with Mobile Focus View UI

---

## Development & Testing

```powershell
# Run unit cache tests and live integration checks
cargo test -- --nocapture

# Build size-optimized LTO release executable (~10 MB)
cargo build --release

# Run client (optional software renderer / insecure local TLS overrides)
cargo run --release
# HAVEN_SW_RENDER=1 cargo run
```

## License

AGPL-3.0
