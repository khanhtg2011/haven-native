# Haven Ecosystem Comprehensive Full-Stack Optimization Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate event-loop freezes, swap thrashing, idle CPU waste, memory leaks, and UI/network latency across the entire Haven stack (VPS OS/Caddy/Coturn, `haven-chat` Node.js server + SQLite, `haven-musicbot` + `haven-discord-bridge` bots, `haven-native` Pure Rust client, and `src-tauri`/Electron desktop bridge).

**Architecture:** Apply targeted, benchmark-driven optimizations per layer in dependency order: stabilize VPS memory (`zram`, kill double-TLS via `FORCE_HTTP=true`, widen Coturn ports), cut `haven-chat` disk/CPU pressure (SQLite WAL checkpoint scheduler, cached `/app` shell, idle `exiftool` reaping), harden Python voice bots (reap orphaned `ffmpeg` children, stop 994 MB MusicBot swap leak, gate 50 Hz idle silence encoding in Discord Bridge, add true multi-speaker PCM mixing), and remove `haven-native` CPU rendering and $O(N)$ message cloning bottlenecks. Each task verifies measurably via live profiling, unit/E2E tests, and service telemetry.

**Tech Stack:** Debian 11 (1 vCPU, 964 MB RAM), Caddy, Coturn, Docker + Node.js v22 (`express`, `socket.io`, `better-sqlite3`), Python 3.9 (`aiortc`, `av`, `numpy`, `discord.py`, `yt-dlp`, `ffmpeg`), Rust 2021 (`slint 1.8`, `tokio`, `tokio-tungstenite`, `reqwest`, `cpal`, `ringbuf`), Tauri v2.

**Spec:** Live VPS & Repository Forensic Audit (2026-11-25) across `vpssieutoc.khanhs` (`103.74.100.97`) and `c:\Users\Administrator\Downloads\haven-desktop`.

---

## Global Constraints

* **Sequential VPS Operations (Haven Rule 1):** Never open concurrent background SSH sessions to `$HAVEN_VPS_HOST` (`MaxStartups` drops parallel Paramiko connections with `WinError 10054`). Execute VPS commands strictly sequentially via a single SSH channel/script.
* **Live SQLite Safety (Haven Rule 4):** Never overwrite `haven.db` while `haven-chat` is running; all DB maintenance runs inside `SqliteDatabase` or via `sqlite3` with `PRAGMA busy_timeout = 5000`.
* **100% English Repository Artifacts:** All source code, documentation, plans, comments, and Git commit messages must be strictly in English.
* **Pre-Release Policy:** Every `haven-native` Rust build, workflow run, and Git tag must keep `prerelease: true` in `.github/workflows/release-pure-rust.yml` until explicit user approval.
* **i18n Template Integrity (Haven Rule 3):** Never modify or translate `{{...}}` interpolation keys in localization files (`vi.json`, `en.json`).
* **Non-Blocking Event Loops:** Zero synchronous HTTP calls, disk image decodes, or blocking process waits inside Tokio WS receiver loops, Slint UI callbacks, or Node.js Socket.IO handlers.

## Baseline Telemetry vs. Target KPIs

| Layer / Component | Measured Baseline (2026-11-25) | Root Cause Verified | Target After Optimization |
| :--- | :--- | :--- | :--- |
| **VPS Swap & Kernel** | `1,151 MB` disk swap used on `964 MB` RAM (`vm.swappiness=20`, no `zram`) | Uncompressed disk swap storms on `/dev/vda1` (1 vCPU) | `< 150 MB` swap used, compressed `zram` active, `0` swap stalls |
| **`haven-chat` Node.js** | `crash.log` event-loop lag up to `1,012,120 ms` (16.8 min); `37.7 GB` Block Read I/O; `4.0 MB` WAL on `1.3 MB` DB; internal TLS on `127.0.0.1:3000` | Disk swap thrashing + double-TLS (`Caddy -> https://127.0.0.1:3000`) + synchronous `fs.readFileSync('/app')` + permanent `exiftool` Perl daemon | `< 50 ms` max loop lag; `FORCE_HTTP=true`; `0` double-TLS tax; WAL `< 1 MB` |
| **Coturn** | `41` relay ports (`49160..49200`) | Narrow range risks exhaustion under multi-peer voice + screen share + bots | `149` ports (`49152..49300`) + `stale-nonce` & `no-multicast-peers` |
| **`haven-musicbot`** | **`994 MB` VmSwap + `205 MB` RSS (`2.57 GB` VSZ)**; orphaned `ffmpeg` (`PID 107117`) alive `19h32m`; downloads `itag=18` MP4 video+audio | `stop_stream()` leaves child `ffmpeg`/`sh` unreaped; `itag=18` pulls full MP4 video; unbounded `np`/`av` frame allocations | `< 95 MB` RSS, `< 10 MB` Swap, `0` zombie `ffmpeg` children, audio-only streams |
| **`haven-discord-bridge`** | **`10.1%` continuous idle CPU** (`1h49m` CPU in `18h`); no multi-speaker mixing | Encodes `50` silent PCM frames/s via NumPy/PyAV/`libopus` 24/7; FIFO appends overwrite/interleave concurrent speakers | `< 1.5%` idle CPU (silence gating/DTX); smooth multi-speaker soft-knee PCM mixer |
| **`haven-native` (Rust)** | Forced `SLINT_BACKEND=software`; clones full `Vec<MessageData>` per message; decodes all 80 avatars/images synchronously on UI thread; skips `online-users-delta`; hardcoded `voice-ping-ms: 18`; `danger_accept_invalid_certs(true)` | CPU software rasterizer; no `Rc<VecModel>` mutation; uncached `slint::Image::load_from_path`; no WS auto-reconnect or live ping | GPU-accelerated Slint UI (`< 35 MB` RAM idle); $O(1)$ message appends + `ListView` virtualization; LRU image cache; TLS verified + auto-reconnect |
| **Tauri / Electron** | `src-tauri/Cargo.toml` missing `[profile.release]` LTO/size strip; `src/main/main.js` 235 KB monolith; unthrottled DOM observer in `haven-bridge.js` | Unoptimized Tauri release binary & eager main-process module loading | Shrunk stripped Tauri binary + debounced bridge observer |

## Review Focus

1. **Multi-User Simultaneous Voice on Discord Bridge:** When 2+ users speak concurrently on either Haven or Discord while a user joins/leaves mid-sentence, verify per-speaker mix buffers align cleanly without sample clipping distortion or memory leak (owned by **Task 9**).
2. **Swap Starvation Under Peak Burst (`yt-dlp` + `ffmpeg` + WebRTC + Docker):** On a 1-vCPU / 964 MB RAM VPS, launching a playlist extraction while a user connects must stay within `zram` + cgroup limits without OOM-killing `haven-chat` (owned by **Task 1, Task 7**).
3. **Orphaned Subprocess Tree Termination:** Canceling or skipping a track mid-handshake in `haven-musicbot` (both direct `ffmpeg` and `yt-dlp | ffmpeg` shell fallback) must kill the entire process group with zero lingering descriptors or zombie children (owned by **Task 7**).
4. **Slint Image Cache Memory Ceiling & Background Refresh:** Switching rapidly across 5 channels containing high-res images and animated GIFs must never decode on the Slint UI thread, exceed cache budget, or leave blank avatars after background fetch finishes (owned by **Task 11**).
5. **Reverse Proxy & WebSocket Origin Preservation Behind `FORCE_HTTP=true`:** Switching `haven-chat` from internal TLS to plain HTTP behind Caddy must preserve `X-Forwarded-Proto`, real client IP (`clientIp.js` / rate limits), WebSocket upgrades, and `/uploads` CORS headers (owned by **Task 2, Task 5**).

---

## Subsystem A: VPS Infrastructure, Caddy, Coturn & OS Tuning

### Task 1: Configure `zram` Compressed Swap & Kernel Memory Tuning on VPS

**Files:**
- Create (VPS): `/etc/default/zramswap`
- Modify (VPS): `/etc/sysctl.d/99-haven.conf`
- Test: Sequential VPS check via `zramctl`, `swapon --show`, `free -m`, `sysctl`

**Interfaces:**
- Produces: Kernel compressed RAM swap (`zram0`, priority `100` above disk `/swapfile` priority `-2`) + tuned `vm.swappiness` & `vm.vfs_cache_pressure`.

- [ ] **Step 1: Record baseline swap and zram state on VPS**

Run sequentially via `run_vps.py`:
```bash
free -m && swapon --show && lsmod | grep zram || echo "NO_ZRAM"
```
Expected: `Swap: 2047 1151 ...` on `/swapfile` priority `-2`, `NO_ZRAM`.

- [ ] **Step 2: Install `zram-tools` and configure 512 MB LZ4/ZSTD compressed zram**

Write `/etc/default/zramswap` (`ALGO=zstd`, `PERCENT=50`, `PRIORITY=100`), enable `zramswap.service`, and tune `/etc/sysctl.d/99-haven.conf`:
- `vm.swappiness = 10` (prefer keeping active Node.js/SQLite/bot pages in physical RAM, compress infrequently used Python objects to `zram` rather than `/swapfile`)
- `vm.vfs_cache_pressure = 100` (balanced inode/dentry reclaim for uploads directory)
- `vm.watermark_scale_factor = 200` (proactive kswapd reclaim prevents sudden allocation stalls)
Apply via `sysctl -p /etc/sysctl.d/99-haven.conf` and refresh disk `/swapfile` (`swapoff /swapfile && swapon -p -2 /swapfile` once `haven-musicbot` swap is cleared in Task 7).

- [ ] **Step 3: Verify `zram` active and sysctl values applied**

Run:
```bash
zramctl && swapon --show && sysctl vm.swappiness vm.vfs_cache_pressure vm.watermark_scale_factor
```
Expected: `/dev/zram0` active with `PRIORITY=100` (`zstd` compression), sysctl values match config.

- [ ] **Step 4: Document VPS sysctl/zram baseline in repo ops docs**

```bash
git add docs/
git commit -m "docs(ops): document VPS zram and vm memory tuning parameters"
```

---

### Task 2: Eliminate Double-TLS Overhead (`FORCE_HTTP=true`) & Optimize Caddy Reverse Proxy

**Files:**
- Modify (VPS): `/var/lib/docker/volumes/haven_data/_data/.env` (set `FORCE_HTTP=true`)
- Modify (VPS): `/etc/caddy/Caddyfile`
- Test: `curl -sI https://haven.khanhs.io.vn/api/health`, Socket.IO WSS handshake test, Review Focus #5 test (`X-Forwarded-Proto` & CORS headers on `/uploads/*`)

**Interfaces:**
- Consumes: `haven-chat` built-in `forceHttp` flag (`/app/server.js:585, 2995`).
- Produces: Single TLS termination at Caddy (`:443`) forwarding over plain HTTP (`http://127.0.0.1:3000`), skipping dynamic zstd/gzip on pre-compressed media.

- [ ] **Step 1: Verify current double-TLS hop and Caddy headers**

Run sequentially:
```bash
curl -k -s -o /dev/null -w "upstream_https:%{http_code}\n" https://127.0.0.1:3000/api/health
curl -sI https://haven.khanhs.io.vn/fonts/inter.woff2 2>&1 | grep -iE "HTTP/|content-encoding|cache-control" || true
```
Expected: `upstream_https:200` confirming `haven-chat` currently encrypts local `127.0.0.1:3000` traffic.

- [ ] **Step 2: Configure `FORCE_HTTP=true` in Haven `.env` and optimize `/etc/caddy/Caddyfile`**

1. Set `FORCE_HTTP=true` and verify `PUBLIC_URL=https://haven.khanhs.io.vn` in `/var/lib/docker/volumes/haven_data/_data/.env`.
2. Update `/etc/caddy/Caddyfile` to:
   - Reverse proxy to `http://127.0.0.1:3000` (remove `tls_insecure_skip_verify`).
   - Add `@compressible` matcher excluding already-compressed extensions (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.woff2`, `.mp4`, `.webm`, `.mp3`, `.ogg`, `.opus`, `.aac`, `.zip`, `.exe`) before `encode @compressible zstd gzip` so CPU is never wasted re-compressing media.
   - Set `keepalive 60s`, `keepalive_idle_conns 64`, `flush_interval -1`.
3. Update internal bot `.env` `HAVEN_URL=http://127.0.0.1:3000` in `/opt/haven-musicbot/.env` and `/opt/haven-discord-bridge/.env` (removing HTTPS loopback overhead for bots too).
4. Recreate/restart `haven-chat` (`docker restart haven-chat`), reload Caddy (`caddy validate --config /etc/caddy/Caddyfile && systemctl reload caddy`), and restart bots sequentially.

- [ ] **Step 3: Verify Review Focus #5 (HTTP->HTTPS headers, WSS, `/uploads` CORS, rate-limit client IP)**

Run:
```bash
curl -s -o /dev/null -w "health:%{http_code} ttfb:%{time_starttransfer}\n" https://haven.khanhs.io.vn/api/health
curl -sI -H "Origin: https://haven.khanhs.io.vn" https://haven.khanhs.io.vn/uploads/1795187300140-b1e75a03497c6fa8.gif | grep -iE "HTTP/|access-control|cache-control|content-encoding"
docker logs haven-chat --tail 15 | grep "FORCE_HTTP"
```
Expected: `⚡ FORCE_HTTP=true, running plain HTTP (reverse proxy mode)` in container logs, `health:200`, `/uploads/...` has `Access-Control-Allow-Origin: *` and `Cache-Control: public, max-age=604800, immutable` without wasteful gzip re-compression.

---

### Task 3: Expand Coturn UDP Relay Port Range & Harden STUN/TURN Configuration

**Files:**
- Modify (VPS): `/etc/turnserver.conf`
- Test: `systemctl status coturn`, `ss -lunp | grep turnserver`, ICE server allocation verification via `/api/ice-servers`

**Interfaces:**
- Produces: 149 relay UDP ports (`49152..49300`) + hardened TURN limits (`stale-nonce`, `no-multicast-peers`, `bps-capacity=0`, `user-quota=12`, `total-quota=120`).

- [ ] **Step 1: Verify current Coturn port exhaustion risk**

Run:
```bash
grep -E "min-port|max-port" /etc/turnserver.conf && ss -u -a | grep -c "491[6-9][0-9]\|49200" || true
```
Expected: `min-port=49160` and `max-port=49200` (only 41 ports available).

- [ ] **Step 2: Expand Coturn relay range and apply hardening in `/etc/turnserver.conf`**

Update `/etc/turnserver.conf`:
- `min-port=49152`
- `max-port=49300`
- Add `no-multicast-peers`, `stale-nonce=600`, `user-quota=16`, `total-quota=120`.
- Open UDP `49152:49300` on host firewall (`ufw allow 49152:49300/udp` if `ufw status` is active), then `systemctl restart coturn`.

- [ ] **Step 3: Verify Coturn listening and active TURN allocation**

Run:
```bash
systemctl is-active coturn && grep -E "min-port|max-port|quota" /etc/turnserver.conf
```
Expected: `active` and updated port range `49152..49300`.

---

## Subsystem B: `haven-chat` Backend Server (Node.js v22 + Express + Socket.IO + SQLite)

> **Note:** `haven-chat` runs from image `ghcr.io/ancsemi/haven:latest`. Track server-side patches cleanly as an idempotent patch script + systemd/docker entrypoint hook or forked container build so container recreation preserves every optimization.

### Task 4: SQLite WAL Checkpoint Scheduler & Query Maintenance

**Files:**
- Modify (Container/Patch): `/app/src/db/sqlite.js` (`SqliteDatabase` constructor & cleanup interval)
- Test: `node --test test/*.test.js` inside `haven-chat` container + live `haven.db-wal` file size check

**Interfaces:**
- Consumes: `SqliteDatabase` in `/app/src/db/sqlite.js:88-165`.
- Produces: `SqliteDatabase.startMaintenanceLoop(intervalMs = 300000)` + `stopMaintenanceLoop()` invoked on graceful shutdown.

- [ ] **Step 1: Baseline WAL bloat measurement**

Run:
```bash
ls -lh /var/lib/docker/volumes/haven_data/_data/haven.db* && sqlite3 /var/lib/docker/volumes/haven_data/_data/haven.db "PRAGMA wal_checkpoint(PASSIVE);"
```
Expected: Observe `haven.db-wal` currently ~`4.0M` (> 3x the `1.3M` main DB) before periodic passive/truncate checkpointing is added.

- [ ] **Step 2: Add non-blocking periodic WAL checkpoint & `PRAGMA optimize` in `/app/src/db/sqlite.js`**

Add `startMaintenanceLoop(intervalMs = 300000)` to `SqliteDatabase`:
1. Set `this.pragma('journal_size_limit = 8388608');` (8 MB cap on WAL size after checkpoint).
2. Every 5 minutes (`.unref()` timer), check WAL size; run `this.pragma('wal_checkpoint(PASSIVE)')` (falls back to `TRUNCATE` only when no readers active and WAL > 4 MB).
3. Every 6 hours, run `this.pragma('optimize')` so SQLite planner updates index statistics cleanly.
4. Clear interval inside `SqliteDatabase.close()`.

- [ ] **Step 3: Verify WAL size shrinks and unit tests pass**

Run:
```bash
docker exec haven-chat node -e "const { SqliteDatabase } = require('./src/db/sqlite'); const assert = require('assert'); const db = new SqliteDatabase(':memory:'); assert.strictEqual(typeof db.startMaintenanceLoop, 'function'); db.close(); console.log('PASS');"
```
Expected: `PASS`, and after restart live `haven.db-wal` stays bounded under `1 MB` during normal chat load.

---

### Task 5: Cache Versioned `/app` Shell in RAM & Tune Socket.IO Frame Overhead

**Files:**
- Modify (Container/Patch): `/app/server.js:1154-1165` (`/app` route) and `/app/server.js:3066-3074` (`io = new Server(...)`)
- Test: HTTP latency benchmark on `https://haven.khanhs.io.vn/app` + E2E WebSocket connection test

**Interfaces:**
- Consumes: `public/app.html` and `package.json` version in `/app/server.js`.
- Produces: Cached `renderedAppHtml` buffer + static weak `ETag` (`W/"<ver>"`), plus `perMessageDeflate: false` on `Socket.IO` server.

- [ ] **Step 1: Benchmark baseline `/app` HTML disk read & compression overhead**

Run:
```bash
docker exec haven-chat node -e "const fs = require('fs'); const t0 = performance.now(); for(let i=0;i<100;i++) fs.readFileSync('/app/public/app.html','utf8'); console.log('100x sync read ms:', (performance.now()-t0).toFixed(2));"
```
Expected: Measurable sync read latency blocking the single-threaded Node event loop on every `/app` page load.

- [ ] **Step 2: Pre-render `app.html` once at boot & configure Socket.IO `perMessageDeflate: false`**

1. In `/app/server.js` outside `app.get('/app', ...)`: read and cache `const CACHED_APP_HTML = fs.readFileSync(path.join(__dirname, 'public', 'app.html'), 'utf8').replace(/(\?v=)[^"']*/g, \`$1${ver}\`);` and `const CACHED_APP_ETAG = \`W/"${ver}"\`;` (with `fs.watchFile` or explicit reload in dev only).
2. Inside `app.get('/app', ...)`: compare `req.headers['if-none-match'] === CACHED_APP_ETAG` -> return `304`, else send pre-built `CACHED_APP_HTML` with `ETag`.
3. In `new Server(server, { ... })` (`/app/server.js:3066`): set `perMessageDeflate: false` (JSON chat/voice signaling payloads are small `< 4 KB` and already run over BBR TCP; disabling zlib per-frame compression eliminates 1-vCPU zlib memory arenas of ~300 KB per connected socket + per-frame CPU overhead).

- [ ] **Step 3: Verify Review Focus #5 (`/app` HTTP 304 ETag & WebSocket frame latency)**

Run:
```bash
curl -s -D - -o /dev/null -H 'If-None-Match: W/"5.7.0"' https://haven.khanhs.io.vn/app | grep -E "HTTP/|etag|cache-control"
```
Expected: `304 Not Modified` on cached hit, and `haven-native` E2E WebSocket suite connects and passes cleanly.

---

### Task 6: Idle Subprocess Reaping (`exiftool`) & Event-Loop Lag Hardening

**Files:**
- Modify (Container/Patch): `/app/src/privacyPass.js` (lazy spawn + 60s idle auto-close for `exiftool`) and container Docker config (`NODE_OPTIONS=--max-old-space-size=256 --expose-gc`)
- Test: `ps -eo pid,rss,comm | grep exiftool` after idle window + upload privacy strip smoke check

**Interfaces:**
- Produces: Zero idle RAM waste from dormant Perl `exiftool` process when no uploads happen; explicit V8 heap ceiling fitting 450 MB Docker limit.

- [ ] **Step 1: Verify permanent idle `exiftool` daemon RAM footprint**

Run:
```bash
ps -eo pid,rss,etime,cmd | grep exiftool | grep -v grep
```
Expected: Persistent `perl /usr/bin/exiftool -stay_open True` consuming ~10 MB RSS continuously even with zero image uploads.

- [ ] **Step 2: Add idle timeout reaping to `exiftool` manager & set `--max-old-space-size=256`**

1. Wrap `exiftool` stdin/stdout worker in `privacyPass.js` with an idle timer (`EXIFTOOL_IDLE_MS = 60000`) that writes `-stay_open\nFalse\n` when idle for 60s and spawns lazily on the next image strip request.
2. Add `NODE_OPTIONS=--max-old-space-size=256 --expose-gc` in container environment so the `/app/server.js:3533` watchdog `global.gc()` nudge works and V8 compacts heap before hitting the 450 MB Docker cgroup cap.

- [ ] **Step 3: Verify `exiftool` exits cleanly after idle and restarts on upload**

Run upload test via `rust_native_bot`, wait 65s, and inspect `ps`:
Expected: `exiftool` runs during upload, reaps to `0 MB` RSS after 65s idle, and `haven-chat` logs `NODE_OPTIONS` with `global.gc` enabled.

---

## Subsystem C: Voice Bots (`haven-musicbot` & `haven-discord-bridge`)

### Task 7: Eliminate `haven-musicbot` 994 MB Swap Leak & Orphaned `ffmpeg` Subprocesses

**Files:**
- Modify (VPS): `/opt/haven-musicbot/bot.py` (`AudioBroadcaster`, `BroadcasterPeerTrack`, `YtDlpExtractor`)
- Modify (VPS): `/etc/systemd/system/haven-musicbot.service` (`MemoryHigh=180M`, `MemoryMax=280M`)
- Test (VPS): `/tmp/test_musicbot_lifecycle.py` + `ps` child check + `/proc/<pid>/status` `VmSwap` check

**Interfaces:**
- Consumes: `AudioBroadcaster` (`/opt/haven-musicbot/bot.py:107-230`).
- Produces: Deterministic `start_new_session=True` + `os.killpg` process-group teardown in `stop_stream()`, audio-only `yt-dlp` extraction (`ba[acodec!=none]/bestaudio[ext=webm]/bestaudio/best`), shared static `SILENT_AUDIO_FRAME` cache (eliminates creating 50 NumPy/PyAV `AudioFrame` allocations/sec when idle/paused).

- [ ] **Step 1: Verify baseline leak & orphaned `ffmpeg` child (`PID 107117`)**

Run:
```bash
ps -ef | grep -E "ffmpeg|yt-dlp" | grep -v grep && grep -E "VmRSS|VmSwap" /proc/$(pgrep -f "/opt/haven-musicbot/bot.py")/status
```
Expected: Confirms orphaned `ffmpeg` downloading `itag=18` MP4 video+audio and massive `VmSwap` (~`994 MB`).

- [ ] **Step 2: Write verification test `/tmp/test_musicbot_lifecycle.py` for Review Focus #3 (Process-Group Teardown & Audio-Only Format)**

Assert:
1. `YtDlpExtractor.extract_info("ytsearch1:lofi")` selects audio-only `acodec != none` / `vcodec == none` stream when available (bandwidth reduction >= 3x vs `itag=18`).
2. Starting and immediately stopping `AudioBroadcaster.start_stream()` / `stop_stream()` leaves `0` child `ffmpeg` or `yt-dlp` PIDs alive (`pgrep -P <pid>` is empty within 1.5s).

- [ ] **Step 3: Implement leak fixes & process-group reaper in `/opt/haven-musicbot/bot.py`**

1. **Process-Group Subprocess Isolation & Teardown:**
   - Pass `start_new_session=True` to `asyncio.create_subprocess_exec` and `create_subprocess_shell`.
   - Implement `async def _terminate_process_group(proc)`: send `os.killpg(proc.pid, signal.SIGTERM)`, `await asyncio.wait_for(proc.wait(), timeout=1.0)`, fallback to `os.killpg(proc.pid, signal.SIGKILL)` + `await proc.wait()` to prevent zombie/hanging `ffmpeg` PIDs forever.
   - Avoid self-cancellation inside `_producer_loop` `finally`: only call process cleanup cleanly and schedule `gc.collect()` on track finish / peer disconnect.
2. **Audio-Only `yt-dlp` Selector:**
   - Change `-f "bestaudio/best"` to `-f "bestaudio[vcodec=none]/bestaudio/best"` in both `YtDlpExtractor` and piped fallback so YouTube streams Opus/AAC audio directly instead of downloading 360p MP4 video (`itag=18`).
3. **Pre-allocated Idle Silence Frame + `MALLOC_TRIM` Guard:**
   - Pre-build immutable silence bytes; reuse cached `av.AudioFrame` properties safely and drop `peer_queues` immediately on peer disconnect.
4. **Tighten Systemd Caps & Clear Stale Swap:**
   - Update `haven-musicbot.service`: `MemoryHigh=180M`, `MemoryMax=280M`, reload daemon, restart `haven-musicbot`.

- [ ] **Step 4: Run verification test & verify RSS/Swap stays minimal**

Run:
```bash
/opt/haven-musicbot/venv/bin/python /tmp/test_musicbot_lifecycle.py && sleep 2 && grep -E "VmRSS|VmSwap" /proc/$(pgrep -f "/opt/haven-musicbot/bot.py")/status && ps -ef | grep ffmpeg | grep -v grep || echo "ZERO_ORPHAN_FFMPEG"
```
Expected: `ZERO_ORPHAN_FFMPEG`, `VmRSS < 80 MB`, `VmSwap == 0 kB`, and `!play lofi` + `!stop` cleanly terminates `ffmpeg`.

---

### Task 8: Eliminate `haven-discord-bridge` 10.1% Idle CPU Burn via Silence DTX & Pre-Encoded Silence

**Files:**
- Modify (VPS): `/opt/haven-discord-bridge/bridge.py` (`HavenPeerTrack.recv`, `DiscordOutboundSource`)
- Test (VPS): 15-second idle CPU telemetry (`pidstat -p <pid> 1 5` or `/proc/<pid>/stat` delta) + E2E audio bridge check (`test_bridge_e2e.py`)

**Interfaces:**
- Consumes: `HavenPeerTrack` (`/opt/haven-discord-bridge/bridge.py:86-165`) and `DiscordOutboundSource` (`/opt/haven-discord-bridge/bridge.py:171-228`).
- Produces: `SilenceGatedOpus` / pre-allocated silence optimization avoiding 50 Hz `np.frombuffer` + `av.AudioFrame` recreation on empty queues and pausing Discord `AudioSource` silence emission after Discord's 5-frame Opus DTX silence tail.

- [ ] **Step 1: Measure baseline idle CPU usage of `haven-discord-bridge`**

Run while voice rooms have zero active speech:
```bash
ps -p $(pgrep -f "/opt/haven-discord-bridge/bridge.py") -o pid,pcpu,pmem,rss,cputime
```
Expected: ~`10%` continuous CPU despite zero active speech.

- [ ] **Step 2: Optimize idle frames in `DiscordOutboundSource` and `HavenPeerTrack`**

1. **Discord Outbound Silence DTX Gate (`DiscordOutboundSource`):** Pre-encode a single 3-byte Opus silence frame `OPUS_SILENCE = b"\xf8\xff\xfe"` and return `is_opus() -> True` using a persistent `discord.opus.Encoder` cached on the source: when `queue` is active, encode incoming 3840-byte PCM frames via the cached encoder; once `is_speaking == False` after the 5-frame (100ms) hangover tail, return `OPUS_SILENCE` zero-cost directly without invoking `libopus.encode()` or NumPy 50 times/sec.
2. **Haven Outbound Pre-Allocated Silence Frame (`HavenPeerTrack`):** Cache a single pre-built `av.AudioFrame` (`SILENT_AV_FRAME`) and clone/update only `pts` when `queue` is empty and hangover has ended, eliminating 50 Hz `np.frombuffer` and `AudioFrame.from_ndarray` allocations per idle peer.
3. Cache `asyncio` clock alignment cleanly when `queue` is drained so idle peers consume near-zero Python scheduler time.

- [ ] **Step 3: Restart bridge and verify idle CPU drops sharply**

Run:
```bash
systemctl restart haven-discord-bridge && sleep 10 && ps -p $(pgrep -f "/opt/haven-discord-bridge/bridge.py") -o pid,pcpu,pmem,rss
```
Expected: Idle `%CPU` drops from `~10.1%` down below `2.0%`, while `/opt/haven-musicbot/venv/bin/python /tmp/test_bridge_e2e.py` continues passing 100%.

---

### Task 9: Implement True Multi-Speaker Digital PCM Mixer (Soft-Knee Saturation) in Discord Bridge

**Files:**
- Modify (VPS): `/opt/haven-discord-bridge/bridge.py` (`HavenAudioMixer`, `DiscordVoiceSink`, `HavenPeerTrack`)
- Test (VPS): `/tmp/test_multi_speaker_mix.py` (unit test mixing 2 concurrent 440Hz + 880Hz stereo PCM streams with `int32` sum + soft-knee saturation; plus Review Focus #1 mid-sentence peer leave test)

**Interfaces:**
- Consumes: `DiscordOutboundSource` and `HavenPeerTrack`.
- Produces: `mix_pcm_frames(frames: list[bytes]) -> bytes` pure helper + `SynchronizedMultiSpeakerMixer` feeding both **Haven ➔ Discord** (`HavenAudioMixer._mix_loop`) and **Discord ➔ Haven** (`DiscordVoiceSink` per-Discord-user ringbuffers mixed cleanly per 20ms frame).

- [ ] **Step 1: Write failing unit test `/tmp/test_multi_speaker_mix.py`**

```python
import numpy as np
from bridge import mix_pcm_frames, FRAME_SIZE

def test_mix_two_speakers_no_clipping_distortion():
    t = np.linspace(0, 0.02, 960, endpoint=False)
    f1 = (np.sin(2 * np.pi * 440 * t) * 20000).astype(np.int16)
    f2 = (np.sin(2 * np.pi * 880 * t) * 20000).astype(np.int16)
    b1 = np.column_stack((f1, f1)).tobytes()
    b2 = np.column_stack((f2, f2)).tobytes()
    out = mix_pcm_frames([b1, b2])
    assert len(out) == FRAME_SIZE
    arr = np.frombuffer(out, dtype=np.int16)
    # Must not wrap around integer overflow; soft-saturated within [-32767, 32767]
    assert arr.max() <= 32767 and arr.min() >= -32767
    # Energy must contain BOTH speakers (RMS higher than single tone after soft saturation)
    assert np.sqrt(np.mean(arr.astype(np.float64)**2)) > 14000
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
PYTHONPATH=/opt/haven-discord-bridge /opt/haven-musicbot/venv/bin/python -m pytest /tmp/test_multi_speaker_mix.py -v
```
Expected: `FAIL` with `ImportError: cannot import name 'mix_pcm_frames'`.

- [ ] **Step 3: Implement `mix_pcm_frames()` and per-user mix buffers for both bridge directions in `/opt/haven-discord-bridge/bridge.py`**

1. Implement `mix_pcm_frames(frames)`:
   - Fast path `len(frames) == 1`: return `frames[0]` zero-copy.
   - Multi-speaker path: sum as `np.int32`, apply smooth soft-knee saturation `tanh`/rational compressor above `24000` (`0.73` headroom) before casting back to `np.int16`, preventing harsh digital clipping when 2–4 loud speakers overlap.
2. **Haven ➔ Discord (`HavenAudioMixer`):** Buffer per-`user_id` (`collections.deque(maxlen=12)`); only spin the 20ms mixer loop when at least one `user_buffer` is active (event-driven wake so idle CPU stays near `0%`!).
3. **Discord ➔ Haven (`DiscordVoiceSink`):** Key buffers by `discord_user_id` (`discord_user_buffers`); mix active Discord speakers per 20ms window before calling `track.push_pcm(mixed)` so 2 Discord speakers no longer interleave sequentially.
4. Review Focus #1 guard: automatically prune stale `user_buffers` after 500ms of silence / user leave.

- [ ] **Step 4: Run test & live E2E two-speaker verification**

Run:
```bash
PYTHONPATH=/opt/haven-discord-bridge /opt/haven-musicbot/venv/bin/python -m pytest /tmp/test_multi_speaker_mix.py -v && systemctl restart haven-discord-bridge
```
Expected: `PASS`, and `Haven Peer` logs show smooth frame forwarding during concurrent speech.

---

## Subsystem D: Pure Rust Native Client (`crates/haven-native`) & Legacy Desktop (`src-tauri/`, `src/`)

### Task 10: GPU-Accelerated Slint Rendering, `ListView` Virtualization & $O(1)$ `VecModel` Mutations

**Files:**
- Modify: `crates/haven-native/src/main.rs:39-41, 465-493, 959-998`
- Modify: `crates/haven-native/ui/components/chat_area.slint:1-5, 152-270`
- Test: `crates/haven-native/tests/e2e_native_test.rs` + release build benchmark (`cargo test --manifest-path crates/haven-native/Cargo.toml`)

**Interfaces:**
- Consumes: `ChatArea` (`chat_area.slint`), `MainWindow` (`app.slint`).
- Produces: Persistent `Rc<VecModel<MessageData>>`, `Rc<VecModel<MemberData>>`, `Rc<VecModel<ChannelItemData>>` stored cleanly via `main_window.as_weak()`, plus virtualized `ListView` in `chat_area.slint`.

- [ ] **Step 1: Write unit benchmark test in `crates/haven-native/tests/e2e_native_test.rs` asserting incremental model update invariants**

Test creating a `VecModel` of 500 `MessageData`-equivalent items and pushing 100 messages via incremental `push()` vs full clone-rebuild (asserting $O(1)$ push latency `< 1 ms`).

- [ ] **Step 2: Replace software-forced renderer, full-Vec cloning, and eager `Flickable` layout**

1. **GPU Backend Selection (`crates/haven-native/src/main.rs:39-41`):** Remove forced `std::env::set_var("SLINT_BACKEND", "software");`. Let Slint pick hardware-accelerated OpenGL/FemtoVG/Skia (`winit`) automatically on Windows 10/11 GPUs, setting an opt-in fallback flag `HAVEN_SW_RENDER=1` only for headless/RDP environments.
2. **Persistent `Rc<VecModel<T>>` (`main.rs`):** Bind `let msg_model = Rc::new(VecModel::<MessageData>::default()); main_window.set_messages(msg_model.clone().into());` once on startup. In `on_send_message` and `HavenEvent::NewMessage`: mutate `msg_model.push(...)` and `msg_model.set_row_data(last_idx, ...)` directly instead of `window.get_messages().iter().collect()`! On `MessageHistory`: call `msg_model.set_vec(slint_msgs)`. Same for `online_members` / `offline_members`.
3. **Virtualized `ListView` (`crates/haven-native/ui/components/chat_area.slint:152-162`):** Replace eager `Flickable { VerticalLayout { for msg in root.messages: ... } }` with Slint standard `ListView { for msg in root.messages: Rectangle { ... } }` so only visible viewport rows are laid out and rendered.

- [ ] **Step 3: Compile & run tests**

Run:
```powershell
cargo test --manifest-path crates/haven-native/Cargo.toml -- --nocapture
```
Expected: Compiles cleanly (`0` warnings/errors) and `PASS`. Verify app launches with `agy-computer-use` on Windows desktop or `HAVEN_SW_RENDER=1` fallback check.

- [ ] **Step 4: Commit**

```bash
git add crates/haven-native/src/main.rs crates/haven-native/ui/components/chat_area.slint crates/haven-native/tests/e2e_native_test.rs
git commit -m "perf(native): enable GPU backend, ListView virtualization, and O(1) VecModel updates"
```

---

### Task 11: Decoded `slint::Image` LRU Cache, Off-Thread Decoding & Reactive Avatar Refresh

**Files:**
- Create: `crates/haven-native/src/cache.rs` (`ImageCacheManager`, `DecodedImageEntry`)
- Modify: `crates/haven-native/src/main.rs:66, 771, 888, 1116-1351`
- Test: `crates/haven-native/tests/cache_unit_test.rs` (covers Review Focus #4: rapid multi-channel GIF/PNG switch, max LRU memory ceiling, and UI refresh callback trigger)

**Interfaces:**
- Produces: `ImageCacheManager::new(base_url: String, http: reqwest::Client, on_ready: Box<dyn Fn(String) + Send + Sync>)`, `get_or_fetch(&self, url: &str) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>`.

- [ ] **Step 1: Write failing test in `crates/haven-native/tests/cache_unit_test.rs`**

Assert:
1. Cache directory resolves under OS app-local data (`LOCALAPPDATA\HavenNative\cache`), NOT CWD `.cache/`.
2. Decoding same avatar URL across 80 messages triggers **exactly 1** decode operation (`decode_count == 1`).
3. Eviction policy bounds on-disk cache size (`<= 256 MB`) and in-memory pixel buffer cache (`<= 128` entries / `<= 48 MB`).

- [ ] **Step 2: Run test to verify failure**

Run:
```powershell
cargo test --test cache_unit_test --manifest-path crates/haven-native/Cargo.toml
```
Expected: FAIL (`cache.rs` not yet extracted / LRU bounds missing).

- [ ] **Step 3: Implement `crates/haven-native/src/cache.rs` & wire reactive refresh into `main.rs`**

1. Share a single pooled `reqwest::Client` across `HavenApiClient` and `ImageCacheManager`.
2. Perform image network fetch in Tokio (bounded by `tokio::sync::Semaphore` max `6` concurrent downloads so scrolling doesn't flood connections), then run `image::load_from_memory` + resize (`<= 128x128` for avatars, `<= 1280px` max edge for inline thumbnails) inside **`tokio::task::spawn_blocking`**, returning a `SharedPixelBuffer<Rgba8Pixel>` ready to wrap into `slint::Image::from_rgba8(...)` in `invoke_from_event_loop` with **zero disk decode on the UI thread**!
3. Deduplicate in-flight URLs (`inflight: Mutex<HashSet<String>>`) so 80 history messages from the same 3 authors issue 3 fetches instead of 80!
4. When a background download+decode completes, trigger a debounced `CacheReady(url)` event to refresh visible avatar/image slots in `msg_model` and `member_model` automatically.
5. Preserve `.gif` frame bytes on disk (add lightweight GIF frame stepper or first-frame instant decode + animated tick for active viewport avatars).

- [ ] **Step 4: Run test & commit**

Run:
```powershell
cargo test --manifest-path crates/haven-native/Cargo.toml
```
Expected: `PASS`.

```bash
git add crates/haven-native/src/cache.rs crates/haven-native/src/main.rs crates/haven-native/tests/cache_unit_test.rs
git commit -m "perf(native): add bounded LRU pixel-buffer cache with off-thread decoding and reactive refresh"
```

---

### Task 12: WebSocket Reconnection, `online-users-delta`, Live Ping RTT & Strict TLS

**Files:**
- Modify: `crates/haven-native/src/net/client.rs:30-238`
- Modify: `crates/haven-native/src/net/api.rs:12-22`
- Modify: `crates/haven-native/src/net/models.rs` (`OnlineUsersDeltaEvent`)
- Modify: `crates/haven-native/src/main.rs` (handle `HavenEvent::OnlineUsersDelta`, `HavenEvent::PingRtt`, replace `Mutex::lock().unwrap()` with safe recovery helper)
- Test: `crates/haven-native/tests/e2e_native_test.rs`

**Interfaces:**
- Produces: `HavenEvent::OnlineUsersDelta`, `HavenEvent::PingRtt(u32)`, automatic reconnect loop (`1s -> 2s -> 4s -> max 30s` + jitter) inside `HavenSocketClient`.

- [ ] **Step 1: Add failing test for `online-users-delta` deserialization and TLS verification**

Assert `serde_json::from_str::<OnlineUsersDeltaEvent>` parses `{ "channelCode": "f5b41ec5", "upsert": [...], "remove": [17] }` and `HavenApiClient::new("https://haven.khanhs.io.vn")` succeeds with valid Let's Encrypt cert verification (`danger_accept_invalid_certs(false)`).

- [ ] **Step 2: Implement WebSocket resilience, delta handling, RTT probe, and TLS security**

1. **Handle `online-users-delta` (`client.rs`):** Since `client.rs:45` sends `"presenceDeltas": 1`, parse `"online-users-delta"` events and patch `online_members` / `offline_members` incrementally without full list rebuilds.
2. **Live RTT Ping (`client.rs` + `main.rs` + `app.slint`):** Emit `["ping-check", {"t": now_ms}]` every 10s, measure `pong-check` delta from echoed payload timestamp `t` (immune to FIFO timer drift — Haven Postmortem #8), and update `window.set_voice_ping_ms(...)` instead of hardcoded `18`.
3. **Auto-Reconnect Loop:** Wrap WebSocket reader/writer in a supervised loop with exponential backoff (`1s..30s`) that preserves token and automatically re-emits `get-channels` & `enter-channel` on reconnect.
4. **TLS Hardening (`api.rs` & `cache.rs`):** Remove `.danger_accept_invalid_certs(true)` by default (allow override only if `HAVEN_INSECURE_TLS=1` for local self-signed dev servers).

- [ ] **Step 3: Run E2E native test & commit**

Run:
```powershell
cargo test --test e2e_native_test --manifest-path crates/haven-native/Cargo.toml -- --nocapture
```
Expected: Real `voice-ping-ms` measured against `https://haven.khanhs.io.vn`, TLS verified cleanly, `PASS`.

```bash
git add crates/haven-native/src/
git commit -m "feat(native): add online-users-delta, WS auto-reconnect, live RTT ping, and strict TLS"
```

---

### Task 13: On-Demand WASAPI Audio Lifecycle & WebRTC Opus Pipeline Readiness

**Files:**
- Modify: `crates/haven-native/src/audio/device.rs:15-120`
- Modify: `crates/haven-native/src/main.rs:68-123, 505-539, 613-632, 666-686`
- Test: `crates/haven-native/tests/e2e_native_test.rs` (verify lazy `AudioEngine::start_voice()` / `stop_voice()` lifecycle)

**Interfaces:**
- Produces: `AudioEngine::new_idle()`, `start_streams(&mut self) -> Result<AudioChannels>`, `stop_streams(&mut self)`.

- [ ] **Step 1: Update `AudioEngine` so WASAPI streams run ONLY while connected to voice**

1. Currently `AudioEngine::new()` opens active 48kHz stereo input+output CPAL hardware streams at login even when the user is only reading text chat, wasting CPU/audio interrupts.
2. Refactor `AudioEngine` to initialize lazily (`start_streams()` invoked on `on_toggle_voice` / voice join; `stop_streams()` dropped cleanly on `on_disconnect_voice`), exposing producer/consumer `ringbuf` handles for the Rust WebRTC Opus media task (`webrtc` / `str0m` + `opus`) while respecting device sample rate / channel layout mismatches cleanly (resampling/mono-to-stereo downmix instead of silent failure).

- [ ] **Step 2: Run E2E audio lifecycle test & verify zero idle CPAL threads**

Run:
```powershell
cargo test --test e2e_native_test --manifest-path crates/haven-native/Cargo.toml -- --nocapture
```
Expected: `AudioEngine` initializes without opening hardware streams until `start_streams()` is called; `stop_streams()` drops CPAL cleanly; `PASS`.

```bash
git add crates/haven-native/src/audio/ crates/haven-native/src/main.rs crates/haven-native/tests/e2e_native_test.rs
git commit -m "perf(native): switch WASAPI audio streams to on-demand voice lifecycle"
```

---

### Task 14: Tauri v2 Binary Size/LTO Tuning, Electron Bridge Throttling & Pre-Release CI Gate

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `[profile.release]`)
- Modify: `src/tauri-bridge/haven-bridge.js` (debounce/scope DOM `MutationObserver` if active)
- Verify: `.github/workflows/release-pure-rust.yml` (`prerelease: true` gate)
- Test: `npm test` + `cargo check --manifest-path src-tauri/Cargo.toml` + `cargo test --manifest-path crates/haven-native/Cargo.toml`

**Interfaces:**
- Produces: Optimized Tauri release profile (`opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`), low-overhead DOM bridge observer, and intact `prerelease: true` CI gate.

- [ ] **Step 1: Add `[profile.release]` optimization to `src-tauri/Cargo.toml` & debounce `haven-bridge.js` observers**

1. Append `[profile.release]` (`opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`) to `src-tauri/Cargo.toml` (matching `crates/haven-native/Cargo.toml`).
2. Audit `src/tauri-bridge/haven-bridge.js` for high-frequency DOM polling or unthrottled `MutationObserver` callbacks; add `requestAnimationFrame` coalescing (`16ms` batching) on voice speaking overlay updates (`report_voice_overlay`) so heavy chat scroll doesn't flood Tauri IPC.
3. Verify `.github/workflows/release-pure-rust.yml` retains `prerelease: true` and passes `cargo test` + release compilation.

- [ ] **Step 2: Run full client verification checks**

Run:
```powershell
npm test
cargo test --manifest-path crates/haven-native/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: `npm test`, `haven-native` tests, and `src-tauri` check all `PASS` cleanly.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src/tauri-bridge/haven-bridge.js
git commit -m "perf(desktop): add Tauri release LTO profile and coalesce bridge overlay IPC"
```

---

## Verification & Rollout Sequence

1. **Wave 1 (Zero-Downtime VPS Foundation):** Tasks 1 ➔ 2 ➔ 3 (zram + single TLS `FORCE_HTTP=true` + Coturn ports). Immediate reduction in VPS CPU, disk I/O, and elimination of swap freezes.
2. **Wave 2 (Python Bots Reliability & Audio Quality):** Tasks 7 ➔ 8 ➔ 9 (MusicBot 994 MB swap/zombie fix ➔ Bridge 10% idle CPU fix ➔ Multi-speaker digital PCM mixer). Test live via `test_bridge_e2e.py` and `!play`/`!stop`.
3. **Wave 3 (`haven-chat` Container Tuning):** Tasks 4 ➔ 5 ➔ 6 (WAL maintenance + cached `/app` + `perMessageDeflate: false` + idle `exiftool` reaping). Monitor `crash.log` for zero `Event loop lag` entries over 24 hours.
4. **Wave 4 (Clients & Pre-Release CI):** Tasks 10 ➔ 11 ➔ 12 ➔ 13 ➔ 14. Verify with `cargo test`, `npm test`, live desktop verification via `agy-computer-use`, and publish `haven-native` exclusively as a GitHub **Pre-Release** (`v0.2.0-pre.x`).
