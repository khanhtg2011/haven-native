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

/// Scans message rows in reverse to find a matching pending optimistic bubble (`opt-*`)
/// for the current user even if interleaved messages arrived first.
pub fn find_reconcile_optimistic_index(
    rows: &[(String, String, String)],
    cur_user: &str,
    incoming_author: &str,
    incoming_content: &str,
) -> Option<usize> {
    if cur_user.is_empty() || incoming_author != cur_user {
        return None;
    }
    rows.iter()
        .rposition(|(id, author, content)| {
            id.starts_with("opt-") && author == cur_user && content == incoming_content
        })
}

/// Bounded LRU in-memory channel history cache for zero-latency channel switching.
pub struct ChannelHistoryLru<T: Clone> {
    max_channels: usize,
    max_messages: usize,
    order: std::collections::VecDeque<String>,
    entries: std::collections::HashMap<String, Vec<T>>,
}

impl<T: Clone> ChannelHistoryLru<T> {
    pub fn new(max_channels: usize, max_messages: usize) -> Self {
        Self {
            max_channels: max_channels.max(1),
            max_messages: max_messages.max(1),
            order: std::collections::VecDeque::new(),
            entries: std::collections::HashMap::new(),
        }
    }

    pub fn get(&mut self, code: &str) -> Option<Vec<T>> {
        let val = self.entries.get(code)?.clone();
        self.touch(code);
        Some(val)
    }

    pub fn put(&mut self, code: String, mut messages: Vec<T>) {
        if messages.len() > self.max_messages {
            let drain = messages.len() - self.max_messages;
            messages.drain(0..drain);
        }
        self.touch(&code);
        self.entries.insert(code, messages);
        while self.entries.len() > self.max_channels {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
    }

    pub fn append_if_present(&mut self, code: &str, msg: T) {
        if let Some(list) = self.entries.get_mut(code) {
            list.push(msg);
            if list.len() > self.max_messages {
                let drain = list.len() - self.max_messages;
                list.drain(0..drain);
            }
            self.touch(code);
        }
    }

    fn touch(&mut self, code: &str) {
        if let Some(pos) = self.order.iter().position(|c| c == code) {
            self.order.remove(pos);
        }
        self.order.push_back(code.to_string());
    }
}
