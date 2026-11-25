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

/// Safely formats an ISO-8601 timestamp into `HH:MM` without slicing inside UTF-8 code points.
pub fn format_utc_hhmm(created_at: &str) -> String {
    if let Some(hhmm) = created_at.get(11..16) {
        let bytes = hhmm.as_bytes();
        if bytes.len() == 5
            && bytes[2] == b':'
            && bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && bytes[3].is_ascii_digit()
            && bytes[4].is_ascii_digit()
        {
            return hhmm.to_string();
        }
    }
    created_at.to_string()
}

/// Scans message rows in reverse to find a matching pending optimistic bubble.
/// Prefers matching an echoed `client_nonce`, falling back to author+content only when needed.
pub fn find_reconcile_optimistic_index(
    rows: &[(String, String, String)],
    client_nonce: Option<&str>,
    cur_user: &str,
    incoming_author: &str,
    incoming_content: &str,
) -> Option<usize> {
    if let Some(nonce) = client_nonce.filter(|n| !n.is_empty()) {
        if let Some(idx) = rows.iter().rposition(|(id, _, _)| id == nonce) {
            return Some(idx);
        }
    }
    if cur_user.is_empty() || incoming_author != cur_user {
        return None;
    }
    rows.iter().rposition(|(id, author, content)| {
        (id.starts_with("opt-") || id.starts_with("opt_"))
            && author == cur_user
            && content == incoming_content
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

    pub fn last_cloned(&self, code: &str) -> Option<T> {
        self.entries.get(code)?.last().cloned()
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

    pub fn append_or_update_by_id<F>(&mut self, code: &str, msg: T, mut id_of: F)
    where
        F: FnMut(&T) -> String,
    {
        if let Some(list) = self.entries.get_mut(code) {
            let new_id = id_of(&msg);
            if let Some(pos) = list.iter().position(|existing| id_of(existing) == new_id) {
                list[pos] = msg;
            } else {
                list.push(msg);
                if list.len() > self.max_messages {
                    let drain = list.len() - self.max_messages;
                    list.drain(0..drain);
                }
            }
            self.touch(code);
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    fn touch(&mut self, code: &str) {
        if let Some(pos) = self.order.iter().position(|c| c == code) {
            self.order.remove(pos);
        }
        self.order.push_back(code.to_string());
    }
}
