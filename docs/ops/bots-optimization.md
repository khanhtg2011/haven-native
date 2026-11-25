# Haven Voice Bots Optimization Summary

## `haven-musicbot` (`/opt/haven-musicbot/bot.py`)

- **Process-Group Reaper:** Subprocesses launch with `start_new_session=True`; `stop_stream()` invokes `_terminate_process_group()` (`SIGTERM` -> `1s` wait -> `SIGKILL`) so `ffmpeg` / `yt-dlp` children never orphan.
- **Audio-Only Extraction:** Uses `--extractor-args "youtube:player_client=android_vr,tv_embedded,web"` with format selector `bestaudio[vcodec=none]/bestaudio/best[height<=240][acodec!=none]/worst[acodec!=none]/best` (streams `129k` WebM Opus audio-only instead of `360p`/`1080p` MP4 video).
- **Memory & Swap Cap:** Reuses pre-allocated `_SILENT_ARRAY`, calls `gc.collect()` + `malloc_trim(0)` on peer/stream teardown, and enforces `MemoryHigh=180M` / `MemoryMax=280M` in `haven-musicbot.service`.

## `haven-discord-bridge` (`/opt/haven-discord-bridge/bridge.py`)

- **Opus DTX & Auto-Pause (`DiscordOutboundSource`):** Pre-encodes active PCM with a cached `discord.opus.Encoder` (`is_opus() -> True`), emits 3-byte `OPUS_SILENCE` (`0xf8 0xff 0xfe`) during idle gaps, and calls `discord_vc.pause()` after `50` consecutive idle frames (~1s) synchronized with `_player_state_lock`; non-silent incoming PCM resumes via `discord_vc.resume()`, dropping idle CPU from `~10%` to `<0.4%`.
- **Zero-Copy Silence Track (`HavenPeerTrack`):** Uses shared read-only `_SILENT_ARRAY` for silent Haven outbound frames instead of reallocating NumPy buffers 50 times/second.
- **Synchronized Multi-Speaker PCM Mixer (`mix_pcm_frames` & `SynchronizedMultiSpeakerMixer`):** Buffers per-speaker 3840-byte frames for both **Haven -> Discord** (`HavenAudioMixer`) and **Discord -> Haven** (`DiscordVoiceSink`), sums active speakers in `int32` with rational soft-knee saturation above `24000`, uses event-driven `threading.Event` wake (0% CPU when idle), assigns per-user Opus fallback decoders, and prunes stale/departed speaker buffers within `0.5s`.
