use crate::lock_or_recover;
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 6;
const DEFAULT_MAX_MEM_ENTRIES: usize = 128;
const DEFAULT_MAX_MEM_BYTES: usize = 48 * 1024 * 1024;
const DEFAULT_MAX_DISK_BYTES: u64 = 256 * 1024 * 1024;
const AVATAR_MAX_EDGE: u32 = 128;
const IMAGE_MAX_EDGE: u32 = 1280;
const MAX_GIF_FRAMES: usize = 24;

#[derive(Clone)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub delay_ms: u64,
}

#[derive(Clone)]
pub struct DecodedImageEntry {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub frames: Arc<Vec<DecodedFrame>>,
    pub total_duration_ms: u64,
    pub is_avatar: bool,
    pub url: String,
}

impl DecodedImageEntry {
    pub fn from_single(url: String, is_avatar: bool, width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let arc_rgba = Arc::new(rgba);
        let frame = DecodedFrame {
            width,
            height,
            rgba: arc_rgba.clone(),
            delay_ms: 100,
        };
        Self {
            width,
            height,
            rgba: arc_rgba,
            frames: Arc::new(vec![frame]),
            total_duration_ms: 100,
            is_avatar,
            url,
        }
    }

    pub fn from_frames(url: String, is_avatar: bool, frames: Vec<DecodedFrame>) -> Option<Self> {
        let first = frames.first()?.clone();
        let total_duration_ms: u64 = frames.iter().map(|f| f.delay_ms.max(20)).sum();
        Some(Self {
            width: first.width,
            height: first.height,
            rgba: first.rgba.clone(),
            frames: Arc::new(frames),
            total_duration_ms: total_duration_ms.max(100),
            is_avatar,
            url,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn to_slint_image(&self) -> Image {
        let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&self.rgba, self.width, self.height);
        Image::from_rgba8(buf)
    }

    pub fn to_slint_frame(&self, elapsed_ms: u64) -> Image {
        if self.frames.len() <= 1 {
            return self.to_slint_image();
        }
        let target = elapsed_ms % self.total_duration_ms;
        let mut acc = 0u64;
        for f in self.frames.iter() {
            acc += f.delay_ms.max(20);
            if target < acc {
                let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&f.rgba, f.width, f.height);
                return Image::from_rgba8(buf);
            }
        }
        self.to_slint_image()
    }

    pub fn byte_size(&self) -> usize {
        if self.frames.len() <= 1 {
            self.rgba.len()
        } else {
            self.frames.iter().map(|f| f.rgba.len()).sum()
        }
    }
}

struct CachedMemSlot {
    entry: DecodedImageEntry,
    bytes: usize,
}

struct MemoryLru {
    entries: HashMap<String, CachedMemSlot>,
    order: VecDeque<String>,
    total_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl MemoryLru {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1024),
        }
    }

    fn get(&mut self, key: &str, promote: bool) -> Option<DecodedImageEntry> {
        let entry = self.entries.get(key)?.entry.clone();
        if promote {
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
            }
            self.order.push_back(key.to_string());
        }
        Some(entry)
    }

    fn insert(&mut self, key: String, entry: DecodedImageEntry) -> bool {
        let bytes = entry.byte_size();
        // Never allow a single oversized asset to exceed total memory budget and flush all other items
        if bytes > self.max_bytes {
            return false;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.bytes);
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        }
        while !self.order.is_empty()
            && (self.entries.len() >= self.max_entries || self.total_bytes + bytes > self.max_bytes)
        {
            if let Some(oldest_key) = self.order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest_key) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.bytes);
                }
            }
        }
        self.total_bytes += bytes;
        self.entries.insert(key.clone(), CachedMemSlot { entry, bytes });
        self.order.push_back(key);
        true
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn animated_avatar_urls(&self) -> Vec<String> {
        self.entries
            .values()
            .filter_map(|slot| {
                (slot.entry.is_avatar && slot.entry.frame_count() > 1).then(|| slot.entry.url.clone())
            })
            .collect()
    }
}

struct InflightGuard {
    key: String,
    inflight: Arc<Mutex<HashSet<String>>>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        lock_or_recover(&self.inflight).remove(&self.key);
    }
}

pub fn default_cache_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if !local.is_empty() {
            return PathBuf::from(local).join("HavenNative").join("cache");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("HavenNative").join("cache");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".cache").join("HavenNative");
        }
    }
    std::env::temp_dir().join("HavenNative").join("cache")
}

fn memory_key(url: &str, is_avatar: bool) -> String {
    if is_avatar {
        format!("a:{url}")
    } else {
        format!("i:{url}")
    }
}

pub struct ImageCacheManager {
    base_url: String,
    http_client: reqwest::Client,
    cache_dir: PathBuf,
    max_disk_bytes: u64,
    memory: Arc<Mutex<MemoryLru>>,
    inflight: Arc<Mutex<HashSet<String>>>,
    download_sem: Arc<Semaphore>,
    on_ready: Arc<dyn Fn(String) + Send + Sync>,
    decode_counter: Arc<AtomicUsize>,
}

impl ImageCacheManager {
    pub fn new(
        base_url: String,
        http_client: reqwest::Client,
        on_ready: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        Self::with_limits(
            base_url,
            http_client,
            default_cache_dir(),
            DEFAULT_MAX_MEM_ENTRIES,
            DEFAULT_MAX_MEM_BYTES,
            DEFAULT_MAX_DISK_BYTES,
            on_ready,
        )
    }

    pub fn with_limits(
        base_url: String,
        http_client: reqwest::Client,
        cache_dir: PathBuf,
        max_mem_entries: usize,
        max_mem_bytes: usize,
        max_disk_bytes: u64,
        on_ready: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        Self {
            base_url,
            http_client,
            cache_dir,
            max_disk_bytes,
            memory: Arc::new(Mutex::new(MemoryLru::new(max_mem_entries, max_mem_bytes))),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            download_sem: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_DOWNLOADS)),
            on_ready,
            decode_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn decode_count(&self) -> usize {
        self.decode_counter.load(Ordering::SeqCst)
    }

    pub fn memory_entry_count(&self) -> usize {
        lock_or_recover(&self.memory).len()
    }

    pub fn memory_bytes(&self) -> usize {
        lock_or_recover(&self.memory).total_bytes
    }

    pub fn disk_cache_bytes(&self) -> u64 {
        collect_cache_files(&self.cache_dir)
            .into_iter()
            .map(|(_, len, _)| len)
            .sum()
    }

    pub fn frame_count(&self, url: &str, is_avatar: bool) -> usize {
        let key = memory_key(url.trim(), is_avatar);
        lock_or_recover(&self.memory)
            .get(&key, false)
            .map(|e| e.frame_count())
            .unwrap_or(0)
    }

    pub fn animated_avatar_urls(&self) -> Vec<String> {
        lock_or_recover(&self.memory).animated_avatar_urls()
    }

    pub fn get_cached_only(&self, url: &str, is_avatar: bool) -> Option<DecodedImageEntry> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return None;
        }
        let key = memory_key(trimmed, is_avatar);
        lock_or_recover(&self.memory).get(&key, false)
    }

    pub fn get_or_fetch(&self, url: &str, is_avatar: bool) -> Option<DecodedImageEntry> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return None;
        }
        let cache_key = memory_key(trimmed, is_avatar);

        if let Some(hit) = lock_or_recover(&self.memory).get(&cache_key, true) {
            return Some(hit);
        }

        {
            let mut inflight = lock_or_recover(&self.inflight);
            if !inflight.insert(cache_key.clone()) {
                return None;
            }
        }

        let url_key = trimmed.to_string();
        let full_url = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            trimmed.to_string()
        } else {
            format!("{}{}", self.base_url, trimmed)
        };
        let disk_path = cache_file_path(&self.cache_dir, trimmed, is_avatar);
        let cache_root = self.cache_dir.clone();
        let max_disk_bytes = self.max_disk_bytes;
        let client = self.http_client.clone();
        let memory = self.memory.clone();
        let inflight = self.inflight.clone();
        let sem = self.download_sem.clone();
        let on_ready = self.on_ready.clone();
        let decode_counter = self.decode_counter.clone();

        tokio::spawn(async move {
            let _guard = InflightGuard {
                key: cache_key.clone(),
                inflight,
            };

            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let (raw_bytes, from_disk) = match tokio::fs::read(&disk_path).await {
                Ok(bytes) if !bytes.is_empty() => (Some(bytes), true),
                _ => (None, false),
            };

            let bytes = match raw_bytes {
                Some(b) => b,
                None => {
                    let Ok(resp) = client.get(&full_url).timeout(Duration::from_secs(15)).send().await else {
                        return;
                    };
                    if !resp.status().is_success() {
                        return;
                    }
                    let Ok(fetched) = resp.bytes().await else {
                        return;
                    };
                    let vec_bytes = fetched.to_vec();
                    if vec_bytes.is_empty() {
                        return;
                    }
                    if let Some(parent) = disk_path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&disk_path, &vec_bytes).await.is_ok() {
                        let root_for_prune = cache_root.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            prune_disk_cache(&root_for_prune, max_disk_bytes);
                        })
                        .await;
                    }
                    vec_bytes
                }
            };

            let max_edge = if is_avatar { AVATAR_MAX_EDGE } else { IMAGE_MAX_EDGE };
            let url_for_decode = url_key.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                decode_and_resize(&bytes, &url_for_decode, is_avatar, max_edge)
            })
            .await
            .ok()
            .flatten();

            match decoded {
                Some(entry) => {
                    if lock_or_recover(&memory).insert(cache_key, entry) {
                        decode_counter.fetch_add(1, Ordering::SeqCst);
                        (on_ready)(url_key);
                    }
                }
                None if from_disk => {
                    // Remove corrupted disk file so subsequent requests recover cleanly
                    let _ = tokio::fs::remove_file(&disk_path).await;
                }
                None => {}
            }
        });

        None
    }
}

pub fn cache_file_path(root: &Path, url: &str, is_avatar: bool) -> PathBuf {
    let clean = url.split('?').next().unwrap_or(url);
    let path_obj = Path::new(clean);
    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
        .unwrap_or_else(|| "img".to_string());
    let raw_stem = path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("asset");
    let safe_stem: String = raw_stem
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    let stem = if safe_stem.is_empty() { "asset" } else { &safe_stem };
    let hash = fnv1a_64(url.as_bytes());
    let bucket = if is_avatar { "avatars" } else { "images" };
    root.join(bucket).join(format!("{hash:016x}_{stem}.{ext}"))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn resize_rgba(img: DynamicImage, max_edge: u32) -> (u32, u32, Vec<u8>) {
    let resized = if img.width() > max_edge || img.height() > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img
    };
    let rgba = resized.to_rgba8();
    let (w, h) = rgba.dimensions();
    (w, h, rgba.into_raw())
}

fn decode_and_resize(
    bytes: &[u8],
    url_hint: &str,
    is_avatar: bool,
    max_edge: u32,
) -> Option<DecodedImageEntry> {
    let lower = url_hint.split('?').next().unwrap_or(url_hint).to_ascii_lowercase();
    let looks_like_gif = lower.ends_with(".gif") || bytes.starts_with(b"GIF8");
    if is_avatar && looks_like_gif {
        if let Ok(decoder) = GifDecoder::new(Cursor::new(bytes)) {
            let mut decoded_frames = Vec::new();
            for frame_res in decoder.into_frames().take(MAX_GIF_FRAMES) {
                if let Ok(frame) = frame_res {
                    let (numer, denom) = frame.delay().numer_denom_ms();
                    let delay_ms = if denom > 0 {
                        (numer / denom).clamp(30, 1000) as u64
                    } else {
                        100
                    };
                    let dyn_img = DynamicImage::ImageRgba8(frame.into_buffer());
                    let (w, h, rgba) = resize_rgba(dyn_img, max_edge);
                    decoded_frames.push(DecodedFrame {
                        width: w,
                        height: h,
                        rgba: Arc::new(rgba),
                        delay_ms,
                    });
                }
            }
            if !decoded_frames.is_empty() {
                return DecodedImageEntry::from_frames(url_hint.to_string(), is_avatar, decoded_frames);
            }
        }
    }

    let dyn_img = image::load_from_memory(bytes).ok()?;
    let (w, h, rgba) = resize_rgba(dyn_img, max_edge);
    Some(DecodedImageEntry::from_single(
        url_hint.to_string(),
        is_avatar,
        w,
        h,
        rgba,
    ))
}

fn collect_cache_files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(collect_cache_files(&p));
        } else if let Ok(meta) = entry.metadata() {
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            out.push((p, meta.len(), mtime));
        }
    }
    out
}

fn prune_disk_cache(root: &Path, max_bytes: u64) {
    let mut files = collect_cache_files(root);
    let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, mtime)| *mtime);
    for (path, len, _) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}
