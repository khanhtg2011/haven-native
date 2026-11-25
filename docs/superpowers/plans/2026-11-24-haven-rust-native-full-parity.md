# Haven Pure Rust Native Full Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port all remaining core and advanced features from the original Haven (Electron/Tauri) client into the 100% Pure Rust + Slint Desktop Client (`haven-native`), achieving complete desktop feature parity without any WebView2 or Chromium dependencies.

**Architecture:** Extend the pure Rust modular architecture (`crates/haven-native`) with Slint declarative UI components, asynchronous non-blocking Tokio networking, WASAPI native audio engine, Win32 DXGI/GDI screen capture, and persistent local caching.

**Tech Stack:** Rust 2021, Slint 1.8, Tokio 1.38, CPAL / WASAPI, Tungstenite, Reqwest, Windows-sys 0.52.

**Spec:** Original Haven Tauri / Electron client implementation (`src/tauri-bridge/haven-bridge.js`, `src-tauri/src/commands.rs`, `src/renderer/`).

---

## Global Constraints

* **100% Pure Rust & Slint:** Strict prohibition on Chromium, CEF, Electron, or WebView2 runtime dependencies.
* **Pre-Release Release Policy:** Every artifact, workflow build, and Git tag must remain strictly flagged as `prerelease: true` until explicit user sign-off.
* **100% English Code & Documentation:** All commit messages, code comments, and documentation must be in English.
* **Non-Blocking Architecture:** Zero network I/O (`.await`) or blocking operations inside the WebSocket receiver loop or Slint UI thread.
* **Target OS:** Windows 10/11 x64 (MSVC).

## Review Focus

1. **Direct Messaging (DMs) Switching:** Switching from guild channels to private 1-on-1 conversations must preserve scroll position and message history without state leakage.
2. **Lightbox Image Viewer & Memory Safety:** High-resolution multi-megabyte image viewing must zoom and pan smoothly without memory leaks or pixel decoding panics.
3. **WASAPI Audio Device Hot-Swapping:** Unplugging or switching default USB headset / microphone during an active voice session must seamlessly reconnect without crashing CPAL stream.
4. **Screen Share Frame Throttling:** Desktop screen capture must throttle to target FPS (30/60 fps) and not saturate CPU or block the Slint render loop.
5. **Localization (i18n) Dynamic Fallback:** Missing keys in language files must gracefully fallback to English without raw formatting identifiers.

---

## Task 1: Direct Messages (DMs) & Home Navigation View

**Goal:** Enable private 1-on-1 conversations with friends, DM list under Home tab, and real-time private message notifications.

**Files:**
- Create: `crates/haven-native/ui/components/dm_list.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/net/models.rs`
- Modify: `crates/haven-native/src/net/client.rs`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Write failing test in `tests/e2e_native_test.rs` for DM event serialization (`get-dms`, `dm-message`).
- [ ] Define `DmItemData` struct in `crates/haven-native/ui/components/dm_list.slint` with friend avatar, username, last message, and online indicator.
- [ ] Implement `DmList` component in Slint with search filter and unread notification badges.
- [ ] Connect Home guild icon (`home` in `guild_sidebar.slint`) to toggle between Server Channel view and DMs list view.
- [ ] Add `HavenSocketClient::get_dms` and event handlers for `HavenEvent::DmList` and `HavenEvent::NewDmMessage`.
- [ ] Verify DM channel switching with `agy-computer-use` on Windows desktop.
- [ ] Commit: `feat(native): implement direct messages and home view navigation`.

---

## Task 2: Fullscreen Lightbox Image Viewer & Media Gallery

**Goal:** Provide an interactive modal lightbox for images and stickers with zoom, pan, download to disk, and copy to clipboard.

**Files:**
- Create: `crates/haven-native/ui/components/lightbox_modal.slint`
- Modify: `crates/haven-native/ui/components/chat_area.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Write unit test in `tests/e2e_native_test.rs` validating image save dialog and clipboard bitmap export.
- [ ] Create `LightboxModal` component in `crates/haven-native/ui/components/lightbox_modal.slint` with dark translucent backdrop, zoom in/out buttons, download button, and close (Esc / backdrop click) triggers.
- [ ] Wire inline image clicks in `chat_area.slint` to open `LightboxModal` with the cached full-resolution bitmap.
- [ ] Implement Windows `OpenClipboard` / `SetClipboardData(CF_DIB)` in `main.rs` for 1-click image copying.
- [ ] Implement file save picker using native Win32 `GetSaveFileNameW` to save downloaded images directly to user's Pictures / Downloads directory.
- [ ] Verify image lightbox interaction using `agy-computer-use` click and capture.
- [ ] Commit: `feat(native): add interactive fullscreen lightbox image viewer and clipboard integration`.

---

## Task 3: Interactive User Profile Popout & Member Card

**Goal:** Display detailed user profiles upon clicking any avatar or member in chat and the member sidebar (Bio, joined date, badges, full avatar preview, roles).

**Files:**
- Create: `crates/haven-native/ui/components/user_popout.slint`
- Modify: `crates/haven-native/ui/components/members_sidebar.slint`
- Modify: `crates/haven-native/ui/components/chat_area.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Create `UserPopout` Slint modal component featuring banner background, circular avatar with status dot, display name, username, bio/about me text, and role tags.
- [ ] Wire member item clicks in `members_sidebar.slint` and message avatar clicks in `chat_area.slint` to trigger `show-user-profile(username)`.
- [ ] Add `fetch_user_profile` endpoint call in `HavenApiClient` to fetch full user bio and statistics.
- [ ] Add click-outside dismiss logic and Esc keyboard accelerator.
- [ ] Verify popout rendering and dismissal with `agy-computer-use`.
- [ ] Commit: `feat(native): add interactive user profile popout card`.

---

## Task 4: Dynamic Audio Device Selection & Sound FX Engine

**Goal:** Enable choosing specific microphone and speaker/headset devices, input volume slider, mic test playback, and sound effects for join/leave/mute.

**Files:**
- Modify: `crates/haven-native/src/audio/device.rs`
- Modify: `crates/haven-native/src/audio/engine.rs`
- Create: `crates/haven-native/ui/components/audio_settings.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Write tests in `tests/e2e_native_test.rs` for CPAL WASAPI input and output device enumeration.
- [ ] Implement device enumeration in `crates/haven-native/src/audio/device.rs` (`enumerate_audio_devices() -> (Vec<AudioDevice>, Vec<AudioDevice>)`).
- [ ] Implement audio device switching in `AudioEngine` without stopping active socket connections.
- [ ] Create `AudioSettings` Slint view with Output Device dropdown, Input Device dropdown, Input Volume slider, and Output Volume slider.
- [ ] Integrate lightweight WAV sound effects for `voice-join.wav`, `voice-leave.wav`, `mute.wav`, `unmute.wav` using native ringbuffer mixing.
- [ ] Verify device selection and audio engine stability.
- [ ] Commit: `feat(native): add dynamic audio device selection and audio feedback cues`.

---

## Task 5: Screen Sharing & Video Feed (Native Win32 Desktop Capture)

**Goal:** Stream desktop screen or application window into Haven voice channels using native Windows Desktop Duplication API (DXGI / GDI).

**Files:**
- Create: `crates/haven-native/src/video/capture.rs`
- Create: `crates/haven-native/src/video/pipeline.rs`
- Create: `crates/haven-native/ui/components/screen_picker.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Write integration test verifying desktop frame capture and JPEG/WebP frame encoding.
- [ ] Implement `capture_monitor_frame` using Win32 GDI / DXGI Desktop Duplication in `crates/haven-native/src/video/capture.rs`.
- [ ] Create `ScreenPicker` modal in Slint showing available monitors and top-level application windows.
- [ ] Wire screen share toggle button in the active voice bar.
- [ ] Transmit screen video frames over Haven WebRTC video track or binary socket frame channel.
- [ ] Verify screen sharing start/stop sequence with `agy-computer-use`.
- [ ] Commit: `feat(native): implement native win32 screen sharing and video streaming`.

---

## Task 6: Server & Channel Management Dialogs

**Goal:** Allow users to create new text/voice channels, delete channels, join servers via invite links, and create custom servers.

**Files:**
- Create: `crates/haven-native/ui/components/create_channel_modal.slint`
- Create: `crates/haven-native/ui/components/invite_modal.slint`
- Modify: `crates/haven-native/ui/components/channel_list.slint`
- Modify: `crates/haven-native/src/net/client.rs`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Write unit tests for `create-channel` and `join-invite` socket command packets.
- [ ] Build `CreateChannelModal` in Slint with radio toggle for "Kênh Văn bản" (Text) vs "Kênh Thoại" (Voice) and channel name input.
- [ ] Build `InviteModal` with input for Haven server invite code / URL.
- [ ] Wire `+` button in `channel_list.slint` to open `CreateChannelModal`.
- [ ] Wire `+` button in `guild_sidebar.slint` to open `InviteModal` or Server Creation modal.
- [ ] Handle server responses and update the active channel list with zero delay.
- [ ] Commit: `feat(native): add channel creation and server invite management dialogs`.

---

## Task 7: Full Internationalization (i18n) & Settings Dialog Parity

**Goal:** Implement comprehensive Vietnamese (`vi.json`) and English (`en.json`) localization across all UI dialogs, plus theme personalization.

**Files:**
- Create: `crates/haven-native/src/i18n.rs`
- Modify: `crates/haven-native/ui/theme.slint`
- Create: `crates/haven-native/ui/components/settings_dialog.slint`
- Modify: `crates/haven-native/ui/app.slint`
- Modify: `crates/haven-native/src/main.rs`
- Test: `tests/e2e_native_test.rs`

### Steps:
- [ ] Load and parse `src/i18n/locales/vi.json` and `en.json` in `crates/haven-native/src/i18n.rs`.
- [ ] Implement `I18nManager` providing key lookups with parameter substitution (e.g., `{count}`, `{username}`).
- [ ] Build comprehensive `SettingsDialog` in Slint with tabs: "Tài khoản", "Giọng nói & Video", "Phím tắt", "Giao diện", "Ngôn ngữ".
- [ ] Allow switching language dynamically between Vietnamese and English.
- [ ] Add custom color accent selection (Blurple, Emerald Green, Ruby Red, Amethyst Purple).
- [ ] Verify localized strings and theme switching with `agy-computer-use`.
- [ ] Commit: `feat(native): implement full i18n localization and settings dialog parity`.

---

## Verification & Pre-Release Pipeline

- Run all unit and integration tests:
  ```powershell
  cargo test --manifest-path crates/haven-native/Cargo.toml
  ```
- Run automated security audit:
  ```powershell
  cargo audit
  ```
- Build production release binary:
  ```powershell
  cargo build --release --manifest-path crates/haven-native/Cargo.toml
  ```
- Validate GUI transitions via `agy-computer-use` on Windows Desktop:
  ```powershell
  python -m agy_computer_use.cli bg-capture --title "Haven Native"
  ```
- Deploy Git Tag `v0.2.0-pre.X` with `prerelease: true` via GitHub Actions.
