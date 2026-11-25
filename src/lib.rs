pub mod audio;
pub mod cache;
pub mod dialog;
pub mod hotkey;
pub mod net;
pub mod overlay;
pub mod tray;

/// Returns `"software"` only when `SLINT_BACKEND` is unset and `HAVEN_SW_RENDER`
/// explicitly opts into software rendering (`1`/`true`/`yes`). Otherwise returns
/// `None` so Slint auto-selects hardware GPU acceleration.
pub fn resolve_slint_backend(
    slint_backend: Option<&str>,
    sw_render: Option<&str>,
) -> Option<&'static str> {
    if slint_backend.is_some_and(|v| !v.trim().is_empty()) {
        return None;
    }
    match sw_render.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("1" | "true" | "yes") => Some("software"),
        _ => None,
    }
}

/// Acquires a mutex guard while recovering gracefully if a prior holder panicked.
pub fn lock_or_recover<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
