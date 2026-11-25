# Haven VPS OS, Kernel & Swap Tuning

Applied on Debian 11 (`1 vCPU`, `964 MB` RAM) to prevent disk-swap thrashing and reduce TCP/UDP latency.

## Compressed Swap (`zram-tools`)

File: `/etc/default/zramswap`

```ini
ALGO=zstd
PERCENT=50
PRIORITY=100
```

- `/dev/zram0`: `482.3M` `zstd` compressed RAM swap at priority `100` (takes precedence over disk `/swapfile` priority `-2`).

## Kernel VM & Network Parameters

File: `/etc/sysctl.d/99-haven.conf`

- `vm.swappiness = 10`: Keep Node.js, SQLite, and active voice state in physical RAM; page cold pages to compressed `zram`.
- `vm.vfs_cache_pressure = 100`: Balanced dentry/inode reclaim for `/uploads/`.
- `vm.watermark_scale_factor = 200`: Early background `kswapd` reclaim to prevent allocation stalls.
- `net.core.default_qdisc = fq` & `net.ipv4.tcp_congestion_control = bbr` with `tcp_slow_start_after_idle = 0`.
- `net.core.rmem_max = 4194304` & `net.core.wmem_max = 4194304` for WebRTC UDP voice streams.

## Single-TLS Reverse Proxy (`FORCE_HTTP=true` & Caddy)

- `haven-chat` runs with `FORCE_HTTP=true` (both in `/data/.env` and Docker container env so the built-in healthcheck probes `http://127.0.0.1:3000/api/health`) and `PUBLIC_URL=https://haven.khanhs.io.vn` on `127.0.0.1:3000`.
- Caddy terminates external TLS (`:443` + `Strict-Transport-Security`), skips dynamic `zstd`/`gzip` on pre-compressed media extensions, and reuses upstream HTTP connections (`keepalive 60s`, `keepalive_idle_conns 64`, `flush_interval -1`).
- Local VPS bots (`haven-musicbot`, `haven-discord-bridge`) connect directly over `http://127.0.0.1:3000`.

## Coturn STUN/TURN Relay Hardening

File: `/etc/turnserver.conf` + UFW `49152:49300/udp`

- Expanded relay ports from `49160..49200` (41 ports) to `49152..49300` (149 ports).
- Added `no-multicast-peers`, `stale-nonce=600`, `user-quota=16`, `total-quota=120`.


