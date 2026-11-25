use haven_native::cache::{cache_file_path, default_cache_dir, ImageCacheManager};
use image::{ImageBuffer, Rgba};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn wait_until<F: Fn() -> bool>(timeout: Duration, pred: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    pred()
}

fn make_test_png(width: u32, height: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(width, height, Rgba([r, g, b, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn make_test_gif(frames: usize) -> Vec<u8> {
    use image::codecs::gif::GifEncoder;
    use image::Frame;
    let mut out = Cursor::new(Vec::new());
    {
        let mut enc = GifEncoder::new(&mut out);
        for i in 0..frames {
            let color = (i as u8).saturating_mul(60);
            let buf = ImageBuffer::from_pixel(16, 16, Rgba([color, 128, 255 - color, 255]));
            enc.encode_frame(Frame::new(buf)).unwrap();
        }
    }
    out.into_inner()
}

async fn spawn_mock_media_server(
    png_bytes: Vec<u8>,
    gif_bytes: Vec<u8>,
    hits: Arc<AtomicUsize>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let png = png_bytes.clone();
            let gif = gif_bytes.clone();
            let hits = hits.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                hits.fetch_add(1, Ordering::SeqCst);
                let (ctype, body) = if path.ends_with(".gif") {
                    ("image/gif", gif)
                } else {
                    ("image/png", png)
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    format!("http://{}", addr)
}

#[test]
fn test_cache_dir_resolves_under_local_appdata() {
    let dir = default_cache_dir();
    let s = dir.to_string_lossy().to_lowercase();
    assert!(s.contains("havennative"), "Expected HavenNative in cache path, got {dir:?}");
    assert!(!dir.is_relative(), "Cache path must be absolute, got {dir:?}");
}

#[tokio::test]
async fn test_dedup_80_lookups_single_decode_and_gif_preservation() {
    let tmp_dir = std::env::temp_dir().join(format!("haven_cache_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let hits = Arc::new(AtomicUsize::new(0));
    let png = make_test_png(256, 256, 88, 101, 242);
    let gif = make_test_gif(3);
    let base_url = spawn_mock_media_server(png, gif, hits.clone()).await;

    let ready_urls = Arc::new(Mutex::new(Vec::<String>::new()));
    let ready_clone = ready_urls.clone();
    let mgr = ImageCacheManager::with_limits(
        base_url,
        reqwest::Client::new(),
        tmp_dir.clone(),
        128,
        48 * 1024 * 1024,
        256 * 1024 * 1024,
        Arc::new(move |url| {
            ready_clone.lock().unwrap().push(url);
        }),
    );

    // Trigger 80 concurrent requests for the same avatar URL
    for _ in 0..80 {
        assert!(mgr.get_or_fetch("/uploads/same-avatar.png", true).is_none());
    }

    assert!(wait_until(Duration::from_secs(2), || mgr.decode_count() == 1).await);

    assert_eq!(hits.load(Ordering::SeqCst), 1, "80 concurrent lookups must hit HTTP once");
    assert_eq!(mgr.decode_count(), 1, "80 concurrent lookups must decode once");

    let entry = mgr
        .get_or_fetch("/uploads/same-avatar.png", true)
        .expect("Decoded entry should be cached in RAM");
    assert!(entry.width <= 128 && entry.height <= 128, "Avatar should resize <=128px");
    assert!(!ready_urls.lock().unwrap().is_empty());

    // Fetch animated GIF avatar and ensure .gif frame bytes remain preserved on disk
    assert!(mgr.get_or_fetch("/uploads/anim-avatar.gif", true).is_none());
    assert!(wait_until(Duration::from_secs(2), || mgr.frame_count("/uploads/anim-avatar.gif", true) >= 2).await);
    assert!(mgr.animated_avatar_urls().contains(&"/uploads/anim-avatar.gif".to_string()));

    // Inline GIF attachment must NOT animate or blow memory budget
    assert!(mgr.get_or_fetch("/uploads/anim-avatar.gif", false).is_none());
    assert!(wait_until(Duration::from_secs(2), || mgr.get_cached_only("/uploads/anim-avatar.gif", false).is_some()).await);
    assert_eq!(mgr.frame_count("/uploads/anim-avatar.gif", false), 1, "Inline image attachments stay single-frame");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_lru_eviction_and_corrupt_disk_cleanup() {
    let tmp_dir = std::env::temp_dir().join(format!("haven_lru_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let hits = Arc::new(AtomicUsize::new(0));
    let png = make_test_png(256, 256, 35, 165, 90);
    let base_url = spawn_mock_media_server(png.clone(), make_test_gif(2), hits).await;

    let mgr = ImageCacheManager::with_limits(
        base_url,
        reqwest::Client::new(),
        tmp_dir.clone(),
        3,           // max 3 entries in RAM
        512 * 1024,  // 512 KB RAM limit
        1800,        // tiny disk cap (~1-2 files max)
        Arc::new(|_| {}),
    );

    // Plant corrupt disk file and verify it is removed instead of remaining sticky
    let corrupt_path = cache_file_path(&tmp_dir, "/uploads/corrupt.png", true);
    std::fs::create_dir_all(corrupt_path.parent().unwrap()).unwrap();
    std::fs::write(&corrupt_path, b"not-a-valid-png").unwrap();
    assert!(mgr.get_or_fetch("/uploads/corrupt.png", true).is_none());
    assert!(wait_until(Duration::from_secs(2), || !corrupt_path.exists()).await, "Corrupt disk cache must be evicted");

    for i in 0..6 {
        let url = format!("/uploads/avatar-{i}.png");
        let _ = mgr.get_or_fetch(&url, true);
        assert!(wait_until(Duration::from_secs(2), || mgr.get_cached_only(&url, true).is_some()).await);
    }

    assert!(mgr.memory_entry_count() <= 3, "Memory LRU exceeded cap: {}", mgr.memory_entry_count());
    assert!(mgr.memory_bytes() <= 512 * 1024, "Memory bytes exceeded cap: {}", mgr.memory_bytes());
    assert!(mgr.disk_cache_bytes() <= 1800, "Disk LRU exceeded cap: {}", mgr.disk_cache_bytes());

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
