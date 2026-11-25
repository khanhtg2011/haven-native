# Haven Full-Stack VPS & End-to-End UX Optimization Plan (Wave 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut VPS RAM/network hop overhead (`docker-proxy`, Caddy heap, static streaming) and eliminate frontend cold-load waterfalls, text flashing, channel-switch blanking, send lag, and mobile/desktop UI stutter across `haven-chat` (Web/Electron/Tauri/Android) and `haven-native`.

**Architecture:** Build directly on the stabilized Wave 1 baseline (`zram0`, `FORCE_HTTP=true`, SQLite WAL scheduler, `exiftool` reaping, and bot fixes) in three cohesive layers: (A) remove userspace `docker-proxy` overhead and offload `/uploads/*` + static caching to Caddy `file_server`; (B) eliminate synchronous `/api/plugins` disk reads, idle relay timers, cost-12 pure-JS `bcryptjs` spikes, and 140+ unbundled frontend HTTP roundtrips via `esbuild` CSS/JS bundling; (C) deliver instant perceived UX via `localStorage`-cached zero-FOUC i18n, in-memory LRU channel history + shimmer skeletons, optimistic message bubbles, `content-visibility: auto` + zero-CLS image sizing, and `document.hidden` / mobile `touch-action` frame guards across Web/Tauri/Mobile and `haven-native`.

**Tech Stack:** Debian 11 (`1 vCPU`, `964 MB` RAM, `20 GB` disk), Docker, Caddy (`GOMEMLIMIT`), Node.js v22 (`express`, `socket.io`, `better-sqlite3`, `bcryptjs`, `esbuild 0.28.2`), Vanilla ESM (`115` files under `/app/public/js/modules/`), CSS (`33` `@import` sheets under `/app/public/css/`), Tauri v2 (`haven-desktop-tauri`, `haven-mobile`), Electron (`haven-desktop`), Rust 2021 + Slint 1.8 (`haven-native`).

**Spec:** Live VPS & UX Forensic Audit (2026-11-25 10:58 UTC+7) on `103.74.100.97` (`haven-chat-patched:perf-20261125`) and the four standalone client repositories (`haven-desktop`, `haven-desktop-tauri`, `haven-mobile`, `haven-native`).

---

## Global Constraints

* **Sequential VPS Operations (`haven` Rule 1):** Never open concurrent background SSH sessions to `$HAVEN_VPS_HOST`; execute every remote command sequentially through a single SSH channel.
* **Live SQLite Safety (`haven` Rule 4):** Never overwrite `haven.db` while `haven-chat` is running; inspect with `SELECT` preview and set `PRAGMA busy_timeout = 5000` before modifying `server_settings` or tables.
* **i18n & WebRTC Parity (`haven` Rules 2 & 3):** Never translate or corrupt `{{...}}` template variables in `public/locales/*.json`; keep `48kHz` stereo 20ms WebRTC frames and 200-400ms jitter buffers intact.
* **Discord Layout & Mobile DOM Contract (`haven` Rule 6):** Keep `<aside class="sidebar">` `<div>` nesting balanced at depth `0`, preserve single `+` (`#upload-btn`) inside `#message-input-area`, keep double-tap (`80-450ms`, `dx <= 12`) & `dblclick` voice join working, and verify against `haven-mobile/test/discord-ui.test.js` (`15/15` pass).
* **Idempotent Container Patches:** Record every server-side `/app` modification as an idempotent Node/shell patch under `/opt/haven-patches/` on the VPS and commit client/ops updates to the corresponding repository cleanly.
* **100% English Codebase Artifacts (`AGENTS.md` Rule 1) & Pre-Release Gate:** All code, tests, docs, plans, and commits are English; retain `prerelease: true` in CI release workflows (`build-tauri.yml`, `build-android.yml`, `release-pure-rust.yml`).

## Baseline Telemetry vs. Target KPIs

| Layer / Bottleneck | Measured Baseline (2026-11-25 10:58) | Target After Optimization |
| :--- | :--- | :--- |
| **Docker Networking (`docker-proxy`)** | 3 `docker-proxy` PIDs (`43,104 KB` / `42.1 MiB` RSS) proxying `127.0.0.1:3000` & unused `0.0.0.0:40000` (`voice_relay_mode = off`); no `/etc/docker/daemon.json` | `0` `docker-proxy` processes (`--network host` + `"userland-proxy": false`), port `40000` unexposed while relay is `off` |
| **Caddy & Journald** | Caddy `87.3 MB` RSS (`GOMEMLIMIT=100MiB`); proxies all `/uploads/*` (`58M`) & `/public/*` (`13M`); `/var/log/journal` `59M` | Caddy `<= 58 MB` RSS (`GOMEMLIMIT=56MiB`); Caddy directly serves `/uploads/*` + cached bundles; journald capped at `25M` |
| **`haven-chat` Backend** | `/api/plugins` runs sync `readdirSync` + `readFileSync` per hit; `bcryptjs` cost `12` (`250-450ms` blocking hash); `voiceRelay` 5s timer runs while `off`; `pollLastfm` active; `cleanup_max_uploads_mb = 0` | Cached plugin metadata (`< 1 ms`); `bcryptjs` cost `10`; idle relay timer skipped; unused poller disabled; `cleanup_max_uploads_mb` bounded |
| **Frontend Cold Load (`/app`)** | ~`160+` HTTP roundtrips (`app.html` `453 KB`, `33` cascading CSS `@import`s `892 KB`, `15+` `<script>`s, `109` ESM imports across `115` files `3.1 MB`, `maxAge: 0` revalidation, `376 KB` `vi.json` fetch) | `<= 12` requests cold, `0` revalidation bursts (`1y, immutable` on `?v=`), single CSS bundle + single ESM bundle, zero-FOUC cached i18n (`0 ms` warm translate) |
| **Perceived Interaction UX** | `switchChannel()` sets `messagesEl.innerHTML = ''` with no cache/skeleton; `_sendMessage()` waits for server echo; images lack aspect/decode hints (triggering `setTimeout(..., 300)` scroll jumps); `theme.js` RAF loops lack `document.hidden` pause | `< 16 ms` instant channel switch (LRU cache + shimmer skeleton); `< 16 ms` optimistic send bubble; `content-visibility: auto` + `0` CLS; paused RAF on hidden tabs + `touch-action: manipulation` |

## Review Focus

1. **Direct Caddy `/uploads/*` Security Parity (`src/storage/serve.js`):** Serving `/uploads/*` via Caddy `file_server` must return HTTP `404` for `/uploads/deleted-attachments/...`, `/uploads/deleted%2Dattachments/...`, `/uploads/bot-audio/...`, dotfiles, and Windows shortname tricks while keeping `Access-Control-Allow-Origin: *`, `Cross-Origin-Resource-Policy: cross-origin`, `Vary: Origin`, and `X-Content-Type-Options: nosniff` (owned by **Task 2**).
2. **Host-Network Loopback Isolation:** Moving `haven-chat` to `--network host` must preserve strict `HOST=127.0.0.1` binding so external traffic cannot reach port `3000` directly, while Caddy and host bots reach `http://127.0.0.1:3000` reliably (owned by **Task 1**).
3. **Bundled Assets vs. Runtime Plugins & `discord-ui.test.js`:** Bundling 33 CSS files (`style.css` -> `discord-theme.css` last) and 109 ESM modules via `esbuild` must keep `window.HavenApi` / `BraidLayout.plugin.js` intact and pass every DOM/CSS check in `haven-mobile/test/discord-ui.test.js` (owned by **Task 5**).
4. **LRU Channel Cache & Optimistic Send Reconciliation:** Rapidly switching `A -> B -> A` while `new-message`, `edit-message`, `delete-message`, or E2E DM decryption events fire must never leave duplicate, stale, or undecrypted bubbles (owned by **Task 7**).
5. **Versioned Cache Invalidation & Pre-Release Guard:** Enabling `Cache-Control: public, max-age=31536000, immutable` on `?v=` URLs must never serve stale JS/CSS/locales after a container update (`app.html` stays `no-cache` + ETag), and client CI workflows preserve `prerelease: true` (owned by **Task 5, Task 6, Task 8**).

---

## Subsystem A: VPS Host, Docker & Caddy Static Offload

### Task 1: Eliminate `docker-proxy` Processes, Apply `--network host`, and Cap Caddy/Journald Memory

**Files:**
- Create (VPS): `/etc/docker/daemon.json`
- Modify (VPS): `haven-chat` container run spec (`--network host`, `HOST=127.0.0.1`, remove `-p 40000` while `voice_relay_mode = off`, retain `--memory=450m`, `--env-file /data/.env`, `haven_data:/data`)
- Modify (VPS): Caddy systemd drop-in (`GOMEMLIMIT=56MiB`), `/etc/systemd/journald.conf` (`SystemMaxUse=25M`)
- Test: Sequential VPS check via `ps`, `ss`, `curl`

**Interfaces:**
- Produces: Zero `docker-proxy` userspace forwarders (`~42.1 MiB` RAM freed), direct kernel loopback `127.0.0.1:3000`, Caddy RSS `< 60 MB`, journald `<= 25 MB`.

- [ ] **Step 1: Verify baseline `docker-proxy`, container inspection spec, and journald usage**

Run sequentially on VPS:
```bash
ps -eo pid,rss,comm,args | grep -E "docker-proxy|caddy" | grep -v grep
docker inspect haven-chat --format '{{json .HostConfig}}'
journalctl --disk-usage
```
Expected: 3 `docker-proxy` processes (`~43,104 KB` total), Caddy `~87 MB` RSS, journal `~59 MB`.

- [ ] **Step 2: Configure Docker `daemon.json` and limit systemd journal size**

Write `/etc/docker/daemon.json`:
```json
{
  "userland-proxy": false,
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  }
}
```
Update `/etc/systemd/journald.conf` (`SystemMaxUse=25M`), reload journald, and run `journalctl --vacuum-size=25M`.

- [ ] **Step 3: Lower Caddy `GOMEMLIMIT=56MiB` and recreate `haven-chat` with `--network host`**

1. Update Caddy systemd override (`Environment="GOMEMLIMIT=56MiB"`), `systemctl daemon-reload && systemctl reload caddy`.
2. Verify `/var/lib/docker/volumes/haven_data/_data/.env` contains `HOST=127.0.0.1` and `FORCE_HTTP=true`.
3. Recreate `haven-chat` using saved inspect env/volumes (`--network host`, `--memory=450m`, `--restart unless-stopped`, image `haven-chat-patched:perf-20261125`) without publishing `40000` while `voice_relay_mode = off`. Restart `docker` cleanly if `daemon.json` is applied first.

- [ ] **Step 4: Verify Review Focus #2 (loopback isolation, zero `docker-proxy`, healthy service)**

Run:
```bash
ps -ef | grep docker-proxy | grep -v grep || echo "ZERO_DOCKER_PROXY"
ss -tulnp | grep ":3000"
curl -s -o /dev/null -w "local:%{http_code} ext:%{http_code}\n" http://127.0.0.1:3000/api/health
curl -m 2 -s http://103.74.100.97:3000/api/health || echo "EXTERNAL_3000_BLOCKED"
curl -s -o /dev/null -w "https_health:%{http_code}\n" https://haven.khanhs.io.vn/api/health
```
Expected: `ZERO_DOCKER_PROXY`, `ss` shows `127.0.0.1:3000` only, `EXTERNAL_3000_BLOCKED`, and `https_health:200`.

---

### Task 2: Offload `/uploads/*` Serving to Caddy `file_server` with Full Security Parity

**Files:**
- Modify (VPS): `/etc/caddy/Caddyfile`
- Create (VPS): `/opt/haven-patches/verify_uploads_caddy.sh`
- Test: HTTP status, security blocklist, and CORS/cache headers verification

**Interfaces:**
- Consumes: Direct filesystem path `/var/lib/docker/volumes/haven_data/_data/uploads` and Caddy `:443` listener (`caddy` user read permission via group/ACL check).
- Produces: Zero Node.js/Express streaming overhead for uploaded images, GIFs, avatars, stickers, and media attachments (`58 MB` corpus).

- [ ] **Step 1: Verify `caddy` user filesystem traverse permissions on `haven_data/_data/uploads`**

Run:
```bash
sudo -u caddy test -r /var/lib/docker/volumes/haven_data/_data/uploads && echo "CADDY_CAN_READ" || echo "NEEDS_ACL"
```
If `NEEDS_ACL`, grant narrow read+traverse ACL (`setfacl -m u:caddy:rx ...` without changing UID `1000` ownership of SQLite/uploads) or fall back cleanly so future Docker uploads remain readable (`setfacl -d -m u:caddy:rx /var/lib/docker/volumes/haven_data/_data/uploads`).

- [ ] **Step 2: Write security & header parity test `/opt/haven-patches/verify_uploads_caddy.sh` (Review Focus #1)**

Assert:
1. `GET https://haven.khanhs.io.vn/uploads/1795187300140-b1e75a03497c6fa8.gif` with `Origin: https://haven.khanhs.io.vn` returns `200`, `Access-Control-Allow-Origin: *`, `Cross-Origin-Resource-Policy: cross-origin`, `Vary: Origin`, `Cache-Control: public, max-age=604800, immutable`, and `X-Content-Type-Options: nosniff`.
2. `/uploads/deleted-attachments/test`, `/uploads/deleted%2Dattachments/test`, `/uploads/bot-audio/test`, `/uploads/.hidden` all return `404` (never `200` or directory listing).

- [ ] **Step 3: Configure Caddy `handle_path /uploads/*` block with security guards**

Update `/etc/caddy/Caddyfile`:
1. Match internal paths (`@blockedUploads path /uploads/deleted-attachments* /uploads/deleted%2Dattachments* /uploads/bot-audio* /uploads/.* /uploads/*~* /uploads/*:*`) and immediately `respond @blockedUploads 404`.
2. In `handle_path /uploads/*`: `root * /var/lib/docker/volumes/haven_data/_data/uploads`, set image CORS headers (`Access-Control-Allow-Origin: *`, `Cross-Origin-Resource-Policy: cross-origin`, `Vary: Origin`) on `*.png *.jpg *.jpeg *.gif *.webp`, `X-Content-Type-Options: nosniff`, `Cache-Control: public, max-age=604800, immutable`, and `file_server` (no `browse`).
3. Validate with `caddy validate --config /etc/caddy/Caddyfile` and `systemctl reload caddy`.

- [ ] **Step 4: Run `/opt/haven-patches/verify_uploads_caddy.sh`**

Run: `bash /opt/haven-patches/verify_uploads_caddy.sh`
Expected: All assertions pass (`200` from Caddy directly on valid image, `404` on all blocked internal/encoded paths).

---

### Task 3: Configure Upload Retention Ceiling & Disable Unnecessary Server Background Pollers

**Files:**
- Modify (VPS SQLite): `/var/lib/docker/volumes/haven_data/_data/haven.db` (`server_settings`: `cleanup_max_uploads_mb`, `unicode_emoji_auto_update`)
- Modify (VPS Env): `/var/lib/docker/volumes/haven_data/_data/.env` (`LASTFM_API_KEY` toggle if confirmed unused by user)
- Test: SQLite `SELECT` preview + container health log check

**Interfaces:**
- Produces: Bounded upload disk growth (preventing `20 GB` VPS disk exhaustion) and stopped external periodic HTTP polling when unused.

- [ ] **Step 1: Preview current `server_settings` values safely (Haven Rule 4)**

Run:
```bash
docker exec haven-chat node -e "const D=require('better-sqlite3');const db=new D('/data/haven.db');db.pragma('busy_timeout=5000');console.log(db.prepare(\"SELECT key,value FROM server_settings WHERE key IN ('cleanup_max_uploads_mb','unicode_emoji_auto_update','max_upload_mb')\").all());db.close();"
```

- [ ] **Step 2: Apply safe `cleanup_max_uploads_mb = '1024'` and disable periodic `unicode_emoji_auto_update = 'false'`**

Run transactional update with `busy_timeout = 5000`; optionally comment out `LASTFM_API_KEY` in `.env` if confirmed unused and restart `haven-chat`.

- [ ] **Step 3: Verify settings persisted and startup logs show zero errors**

Expected: `cleanup_max_uploads_mb = '1024'`, `unicode_emoji_auto_update = 'false'`, `haven-chat` `(healthy)`.

---

## Subsystem B: `haven-chat` Backend Event-Loop & Asset Bundling (`/app`)

### Task 4: Cache `/api/plugins` Metadata, Skip Idle `voiceRelay` Timer, and Lower `bcryptjs` Cost to `10`

**Files:**
- Create (VPS): `/opt/haven-patches/task4b_server_hotpaths.js`
- Modify (Container): `/app/server.js:140-158` (`/api/plugins` metadata cache), `/app/src/socketHandlers/index.js:252` (`voiceRelay.mode() === 'off'` guard), `/app/src/auth.js:153,650,1404` (`bcryptjs` cost `10` + lazy async dummy hash)
- Test: Node benchmark + container `npm test` (`node --test --test-concurrency=1`) + live `/api/plugins` & auth smoke check

**Interfaces:**
- Produces: Cached `getPluginListSnapshot()` invalidated when plugins directory mtime changes (no per-request sync `readFileSync`), zero wasted 5s timer wakeups when `voice_relay_mode = 'off'`, ~4x faster non-blocking password hashing (`~70 ms` vs `~320 ms` on 1 vCPU).

- [ ] **Step 1: Benchmark baseline `/api/plugins` latency and verify sync calls**

Run:
```bash
docker exec haven-chat node -e "const t0=performance.now(); const fs=require('fs'),p='/app/plugins'; for(let i=0;i<50;i++){for(const f of fs.readdirSync(p).filter(x=>x.endsWith('.plugin.js'))) fs.readFileSync(p+'/'+f,'utf8');} console.log('50x sync scan ms:', (performance.now()-t0).toFixed(2));"
```

- [ ] **Step 2: Write idempotent patch `/opt/haven-patches/task4b_server_hotpaths.js`**

1. In `/app/server.js`: track `_pluginListCache = { mtimeMs: 0, plugins: [] }`, refresh only when `fs.statSync(PLUGINS_DIR).mtimeMs !== _pluginListCache.mtimeMs` (or invalidate on plugin install/delete routes).
2. In `/app/src/socketHandlers/index.js`: guard the 5s timer body at line ~252 with `if (state.voiceRelay.mode() !== 'builtin') return;`.
3. In `/app/src/auth.js`: replace blocking `bcrypt.hashSync(..., 12)` at line `153` with lazy precomputed async hash (cost `10`) and lower registration/password-change cost `12 -> 10` at lines `650` and `1404`.
4. Copy patched files into container (commit patched layer or persist patch bootstrap in `/entrypoint.sh`) and restart `haven-chat`.

- [ ] **Step 3: Verify `/api/plugins` latency, auth flow, and container tests pass**

Run live auth & `/api/plugins` smoke test:
Expected: `/api/plugins` responds in `< 2 ms` cached, login & user verification succeed cleanly, `crash.log` records zero event-loop lag warnings.

---

### Task 5: Bundle CSS (33 -> 1) and ESM Modules (109 -> 1) via `esbuild` + Immutable `?v=` Caching

**Files:**
- Create (Container + `/opt/haven-patches/`): `/app/scripts/build-web-bundles.js`, `/app/public/css/bundle.min.css`, `/app/public/js/app.bundle.min.js`
- Modify (Container): `/app/server.js` (immutable `1y` cache headers when request query `v` matches `pkg.version`; `CACHED_APP_HTML` stays `no-cache` + ETag), `/app/public/app.html` (load `/css/bundle.min.css?v=5.8.0` and `<script type="module" src="/js/app.bundle.min.js?v=5.8.0">` with `defer` on classic scripts and `<link rel="preload">` for font + locale)
- Test: `haven-mobile` `test/discord-ui.test.js` + cold load request counter via Playwright/CDP + Review Focus #2 & #5

**Interfaces:**
- Consumes: Already-installed `/app/node_modules/esbuild` (`0.28.2`), `public/css/style.css`, `public/js/app.js`.
- Produces: Single concatenated/minified CSS preserving exact cascade order (ending with `discord-theme.css`), single bundled ESM entry `app.bundle.min.js`, and version-gated `Cache-Control: public, max-age=31536000, immutable`.

- [ ] **Step 1: Verify baseline cold-load request count on `https://haven.khanhs.io.vn/app`**

Run a headless CDP network counter checking total requests & transfer size on fresh load of `/app`.
Expected: Observe **~150–170 HTTP requests** (33 CSS + ~110 JS ESM + classic JS + locales + fonts + plugins).

- [ ] **Step 2: Create `/app/scripts/build-web-bundles.js` using installed `esbuild`**

1. Bundle `/app/public/css/style.css` -> `/app/public/css/bundle.min.css` (`bundle: true, minify: true, external: ['/fonts/*', '/emoji/*', '/img/*', '/uploads/*']`), verifying `discord-theme.css` tokens appear at the tail of `bundle.min.css` to preserve CSS specificity order.
2. Bundle `/app/public/js/app.js` -> `/app/public/js/app.bundle.min.js` (`bundle: true, format: 'esm', minify: true, target: ['chrome108', 'safari15']`).
3. Keep original unbundled files untouched on disk as fallback and rewrite `<link>`/`<script>` in `app.html` (`?v=5.8.0` versioned), removing dead `.bak` exposure (`modmode*.bak`).

- [ ] **Step 3: Update `/app/server.js` static middleware for `?v=<ver>` immutable caching**

Inside `express.static(path.join(__dirname, 'public'), { setHeaders })`: when `req.query.v === CURRENT_VER` (or `/fonts/`, `/locales/*.json?v=...`), set `Cache-Control: public, max-age=31536000, immutable`; otherwise keep `maxAge: 0`. Keep `/app` shell `Cache-Control: no-cache` + `ETag` (`CACHED_APP_HTML` regenerated after bundle).

- [ ] **Step 4: Run Review Focus #2 & #5 verification**

1. Sync updated `app.html` / `bundle.min.css` checks against `haven-mobile/test/discord-ui.test.js` (`node --test`).
2. Re-run headless CDP cold load and warm reload audit on `https://haven.khanhs.io.vn/app`.
Expected: Cold HTTP requests drop from `160+` to `<= 15`; warm reload transfers `0` JS/CSS bytes (`disk/memory cache`); `BraidLayout.plugin.js` and Discord theme load cleanly with `0` console errors.

---

## Subsystem C: End-to-End Frontend, Mobile & Native Client UX Acceleration

### Task 6: Zero-FOUC Synchronous `i18n` Boot Cache & Font Preload

**Files:**
- Modify (Container): `/app/public/js/i18n.js`, `/app/public/app.html`
- Test: Headless CDP cache-hit & interpolation test (`{{...}}` integrity per `haven` Rule 3)

**Interfaces:**
- Consumes: `package.json` / `?v=` version embedded on `<script src="/js/i18n.js?v=...">`.
- Produces: `localStorage` locale payload (`haven_i18n_cache_<locale>` storing `{ v: ver, data }`) applied synchronously before first render frame + `<link rel="preload" href="/locales/vi.json?v=..." as="fetch" crossorigin>`.

- [ ] **Step 1: Add failing warm-load i18n flash assertion**

Assert in Playwright/CDP test that on second visit to `/app`, localized labels (`data-i18n="app.sidebar.join_btn"`) are already Vietnamese on `DOMContentLoaded` (`t('app.sidebar.join_btn') !== 'Join'`) with zero network wait.

- [ ] **Step 2: Implement versioned synchronous cache in `/app/public/js/i18n.js`**

1. Extract active asset version `ver` from `document.currentScript?.src` (`?v=`) and load `/locales/${locale}.json?v=${ver}` (triggering immutable CDN/browser HTTP cache).
2. Read `localStorage.getItem('haven_i18n_cache_' + locale)` before async fetch; if `.v === ver` and `typeof .data === 'object'`, assign `_translations` immediately and execute `translatePage()` on `DOMContentLoaded` without waiting on `/api/public-config` (unless preference is `'auto'` and never resolved before).
3. On cold first visit, add `<link rel="preload">` + avoid sequential `await /api/public-config` -> `await fetch(locale)` waterfall by firing `/locales/${guess}.json?v=${ver}` in parallel.

- [ ] **Step 3: Verify `vi.json` integrity, `0 ms` warm translate, and zero `{{...}}` breakage**

Run Playwright check + `scripts/validate-locales.js` inside container.
Expected: Warm load renders 100% Vietnamese before paint (`window.__i18nCacheHit === true`), `validate-locales.js` passes cleanly.

---

### Task 7: Instant Channel Switching (LRU Cache + Shimmer Skeleton) & Optimistic Message Sending

**Files:**
- Modify (Container): `/app/src/socketHandlers/messages.js` (echo optional `clientNonce` on sender ACK/`new-message`), `/app/public/js/modules/app-channels.js:100-140`, `/app/public/js/modules/app-messages.js`, `/app/public/js/modules/app-composer.js`, `/app/public/js/modules/app-socket-channels.js`, `/app/public/css/style-messages.css`
- Test: Automated Socket + Playwright channel-switch & send timing test (Review Focus #3)

**Interfaces:**
- Consumes: `switchChannel(code)`, `message-history`, `new-message`, `edit-message`, `delete-message`, `_sendMessage()`.
- Produces: Bounded 12-channel `_channelHistoryCache` (Map storing `{ messages, scrollTop, isAtBottom, cachedAt }`), `.channel-skeleton` CSS placeholder for uncached channels, and `clientNonce`-tagged optimistic `.message-pending` bubbles reconciled on server echo.

- [ ] **Step 1: Write Playwright/Socket integration test for Review Focus #3**

Assert:
1. Switching from `#TEMP` back to `#Yapping` renders cached messages within `< 20 ms` (before `message-history` arrives) with zero duplicate DOM rows after `message-history` reconciles.
2. Sending text via `_sendMessage()` appends `.message-pending` within `< 16 ms`, clears composer immediately, and promotes the exact node (matching `clientNonce`) upon `new-message` without flickering or duplicating.
3. Receiving `new-message` / `delete-message` while viewing channel `B` cleanly updates `A`'s LRU entry (or invalidates it cleanly).

- [ ] **Step 2: Implement bounded LRU `_channelHistoryCache` & skeleton in `app-channels.js` / `app-socket-channels.js`**

1. On `switchChannel(code)`: instead of bare `messagesEl.innerHTML = ''`, check `_channelHistoryCache.get(code)`. If hit (non-DM or already decrypted messages), render immediately (`renderMessages`) and restore scroll position, then emit `get-messages` silently to sync any missed newer messages; if miss, render 6 lightweight `.message-skeleton` shimmer rows.
2. On `message-history`, `new-message`, `message-edited`, `message-deleted`: update `_channelHistoryCache` (capped at `12` channels x `80` messages, LRU eviction on insert) and invalidate on reconnect/wake.

- [ ] **Step 3: Implement optimistic send with `clientNonce` correlation**

In `_sendMessage()` (`app-composer.js` / `app-messages.js`) for standard channel/DM text messages (excluding slash commands and self-destruct prompts): generate `clientNonce = crypto.randomUUID?.() || (Date.now() + '-' + Math.random())`, append temporary optimistic DOM row with `data-client-nonce`, include `clientNonce` in `socket.emit('send-message', payload)` (server echoes `clientNonce` cleanly on `new-message` or falls back to timestamp+content+user match on same-client socket), and mark `failed` with inline Retry if ACK/echo times out (`8s`).

- [ ] **Step 4: Re-bundle frontend assets (`Task 5`) & verify tests pass**

Run `node /app/scripts/build-web-bundles.js` and re-run the Playwright/Socket suite.
Expected: Instant `< 16 ms` channel revisit & send feedback, `0` duplicates, E2E DMs decrypt cleanly.

---

### Task 8: Zero-CLS Message Viewport, Battery/Idle Canvas Throttling, Mobile Touch Polish & Client Shell UX

**Files:**
- Modify (Container): `/app/public/css/style-messages.css`, `/app/public/css/style-mobile-discord.css`, `/app/public/js/modules/app-message-content.js:368`, `/app/public/js/modules/app-messages.js:1234-1420`, `/app/public/js/theme.js`
- Modify (`haven-desktop-tauri` & `haven-mobile` if needed): `src/tauri-bridge/haven-bridge.js`
- Modify (`haven-native`): `src/main.rs`, `src/net/models.rs`, `src/net/client.rs`, `src/cache.rs`, `ui/app.slint` (skeleton shimmer / optimistic send / `clientNonce` UI feedback parity)
- Test: `haven-mobile` `npm test`, `haven-desktop-tauri` `npm test`, `haven-native` `cargo test`, Playwright mobile/desktop scroll & CLS check

**Interfaces:**
- Produces: `content-visibility: auto` message containment, `decoding="async"` + aspect-ratio wrappers (eliminating jump-scrolls on image load), paused animated backgrounds when tab hidden, `touch-action: manipulation` across mobile interactive elements, and matching optimistic UX in `haven-native`.

- [ ] **Step 1: Add CSS containment, zero-CLS image boxes, and mobile `touch-action` rules**

1. In `style-messages.css`: add `.message, .message-compact { content-visibility: auto; contain-intrinsic-size: 0 64px; }` and constrain `.chat-image-wrap` / `.chat-image` (`aspect-ratio` placeholder + `background: rgba(255,255,255,0.03)` while decoding) so `#messages` scroll height doesn't jump mid-read.
2. In `app-message-content.js` and `app-messages.js`: add `decoding="async"` on avatars and `.chat-image` tags; ensure mobile `.channel-item, .nav-btn, button` get `touch-action: manipulation` (`style-mobile-discord.css`) without altering `test/discord-ui.test.js` selectors or touch-handler timings.

- [ ] **Step 2: Pause heavy background `requestAnimationFrame` canvas animations when hidden or reduced-motion**

In `/app/public/js/theme.js`: listen to `visibilitychange` (`document.hidden`) and `window.matchMedia('(prefers-reduced-motion: reduce)')` so `_rgbRAF`, `_matrixRAF`, `_emberRAF`, `_graceRAF`, and `_snowRAF` stop scheduling GPU draws when the window/tab/app is minimized or backgrounded.

- [ ] **Step 3: Bring optimistic send + channel-loading shimmer parity to `haven-native` (`C:\Users\Administrator\Downloads\haven-native`)**

Add non-blocking optimistic pending bubble (`is_pending: true`, matching `app-composer.js`) and subtle loading indicator during cold channel fetch in `src/main.rs`, `src/net/models.rs`, and `ui/app.slint`, verified by `cargo test`.

- [ ] **Step 4: Run full cross-repo verification & headless mobile/desktop audit**

Run:
```bash
npm --prefix C:\Users\Administrator\Downloads\haven-desktop test
npm --prefix C:\Users\Administrator\Downloads\haven-desktop-tauri test
npm --prefix C:\Users\Administrator\Downloads\haven-mobile test
cargo test --manifest-path C:\Users\Administrator\Downloads\haven-native\Cargo.toml
```
Plus headless Playwright mobile (`390x844`) & desktop (`1280x800`) audit against `https://haven.khanhs.io.vn/app`:
Expected: All unit/E2E suites pass (`0` failures), `aside.sidebar` tag depth = `0`, single `+` composer intact, `<= 15` HTTP requests on cold load, `0` console errors, `0` idle `ffmpeg`/swap/event-loop lag regressions.

---

## Plan Self-Review Checklist
- **Spec Coverage:** Every measured bottleneck across VPS networking (`docker-proxy`, port `40000`), Caddy (`/uploads/*` offload, `GOMEMLIMIT`, `journald`), `haven-chat` (`/api/plugins` cache, timer gating, `bcryptjs` cost, `cleanup_max_uploads_mb`, `esbuild` CSS/JS bundling, immutable `?v=` headers), and end-to-end UX (`i18n` cache, instant `switchChannel`, optimistic `_sendMessage`, `content-visibility: auto`, image `decoding="async"`, `theme.js` RAF pause, `touch-action: manipulation`, and `haven-native` optimistic parity) has an owning task with measurable KPIs.
- **Type & Interface Consistency:** `clientNonce`, `_channelHistoryCache`, `build-web-bundles.js`, `bundle.min.css`, and `app.bundle.min.js` match across Tasks 5–8 cleanly.
- **Safety:** `haven` Rules 1–6 and all 5 Review Focus failure modes have dedicated tests before deployment.
