# `haven-chat` Backend Tuning Summary

Applied in-place inside container `haven-chat` (with idempotent patch scripts archived under `/opt/haven-patches/` on the VPS host) so existing runtime patches and the live `haven_data` volume remain intact.

## SQLite WAL Checkpointing & Planner Maintenance (`/app/src/db/sqlite.js`)

- Sets `PRAGMA journal_size_limit = 8388608` (`8 MB` cap) on WAL connections.
- Adds `SqliteDatabase.startMaintenanceLoop(intervalMs = 300000)` and `stopMaintenanceLoop()` (auto-started with `.unref()` for file-backed databases, cleared in `close()`):
  - Runs `PRAGMA wal_checkpoint(PASSIVE)` every 5 minutes and escalates to `PRAGMA wal_checkpoint(TRUNCATE)` if `haven.db-wal` exceeds `4 MB` with `busy === 0`.
  - Runs `PRAGMA optimize` every 6 hours so query planner index statistics remain current.

## `/app` Shell Caching & Socket.IO Frame Tuning (`/app/server.js`)

- Pre-renders `CACHED_APP_HTML` and `CACHED_APP_ETAG` (`W/"<version>"`) once at server startup instead of blocking the event loop with synchronous `fs.readFileSync` + regex replacement on every `/app` load.
- Evaluates `If-None-Match` and serves HTTP `304 Not Modified` immediately on matching ETags while preserving `Cache-Control: no-cache`.
- Sets `perMessageDeflate: false` on `new Server(server, ...)` to eliminate per-socket zlib compression arenas (~300 KB/socket) and deflate CPU overhead for small JSON signaling payloads.

## Idle `exiftool` Reaping & V8 Heap Ceiling (`/app/src/mediaTools.js`, `/entrypoint.sh`)

- Wraps the `-stay_open True` `exiftool` worker inside `/app/src/mediaTools.js` (the module invoked by `src/privacyPass.js`) with an `.unref()` idle timer (`EXIFTOOL_IDLE_MS`, default `60000` ms) that issues `-stay_open\nFalse\n` (plus a `2s` `SIGKILL` fallback) after idle periods and spawns lazily on the next metadata request, reclaiming ~`19.5 MB` RSS whenever media uploads are idle.
- Exports `NODE_OPTIONS="--max-old-space-size=256 --expose-gc"` in `/entrypoint.sh` (honoring any existing override) so V8 compacts heap cleanly below the `450 MB` container cap and `server.js` memory watchdog's `global.gc()` call is active.


