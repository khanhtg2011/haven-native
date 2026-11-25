slint::include_modules!();

mod audio;
mod dialog;
mod hotkey;
mod net;
mod overlay;
mod tray;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use haven_native::cache::{DecodedImageEntry, ImageCacheManager};
use haven_native::lock_or_recover;
use serde::{Deserialize, Serialize};
use slint::{Color, Model, VecModel};
use tokio::sync::mpsc;

use crate::audio::{AudioChannels, AudioEngine};
use crate::hotkey::{HotkeyCallbacks, HotkeyManager};
use crate::net::api::HavenApiClient;
use crate::net::client::HavenSocketClient;
use crate::net::models::{ChannelItem, ChatMessageItem, HavenEvent, OnlineUserItem};
use crate::overlay::GameOverlay;
use crate::tray::{TrayCallbacks, TrayManager};

const BASE_URL: &str = "https://haven.khanhs.io.vn";
const WS_URL: &str = "wss://haven.khanhs.io.vn/socket.io/?EIO=4&transport=websocket";
const SESSION_FILE: &str = "haven_session.json";

#[derive(Serialize, Deserialize)]
struct SavedSession {
    username: String,
    token: String,
}

fn mutate_vec_model<T: Clone + 'static>(
    current: slint::ModelRc<T>,
    bind: impl FnOnce(slint::ModelRc<T>),
    f: impl FnOnce(&VecModel<T>),
) {
    if let Some(m) = current.as_any().downcast_ref::<VecModel<T>>() {
        f(m);
    } else {
        let fresh = Rc::new(VecModel::from(current.iter().collect::<Vec<_>>()));
        f(&fresh);
        bind(fresh.into());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(backend) = haven_native::resolve_slint_backend(
        std::env::var("SLINT_BACKEND").ok().as_deref(),
        std::env::var("HAVEN_SW_RENDER").ok().as_deref(),
    ) {
        std::env::set_var("SLINT_BACKEND", backend);
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("🚀 Khởi chạy Haven Native Client 100% Rust (Zero WebView2)...");

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::StationsAndDesktops::{OpenDesktopW, SetThreadDesktop};
        let default_name: Vec<u16> = "Default\0".encode_utf16().collect();
        let h_desk = OpenDesktopW(default_name.as_ptr(), 0, 0, 0x000F01FF); // DESKTOP_ALL_ACCESS
        if h_desk != 0 {
            let res = SetThreadDesktop(h_desk);
            log::info!("Attached to interactive Default desktop: res={}", res);
        }
    }

    let main_window = MainWindow::new()?;
    main_window.set_messages(Rc::new(VecModel::<MessageData>::default()).into());
    main_window.set_online_members(Rc::new(VecModel::<MemberData>::default()).into());
    main_window.set_offline_members(Rc::new(VecModel::<MemberData>::default()).into());
    main_window.set_text_channels(Rc::new(VecModel::<ChannelItemData>::default()).into());
    main_window.set_voice_channels(Rc::new(VecModel::<ChannelItemData>::default()).into());

    // State chia sẻ giữa các luồng
    let active_channel_code = Arc::new(Mutex::new(String::new()));
    let active_voice_code = Arc::new(Mutex::new(String::new()));
    let channels_cache = Arc::new(Mutex::new(Vec::<ChannelItem>::new()));
    let socket_client_holder = Arc::new(Mutex::new(Option::<Arc<HavenSocketClient>>::None));
    let current_token = Arc::new(Mutex::new(String::new()));
    let api_client = Arc::new(HavenApiClient::new(BASE_URL));
    let user_avatars = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let pending_cache_urls = Arc::new(Mutex::new(HashSet::<String>::new()));
    let refresh_scheduled = Arc::new(AtomicBool::new(false));
    let anim_clock = Instant::now();

    let cache_mgr: Arc<ImageCacheManager> = Arc::new_cyclic(|weak_cache| {
        let pending_urls = pending_cache_urls.clone();
        let refresh_flag = refresh_scheduled.clone();
        let window_weak = main_window.as_weak();
        let user_avatars = user_avatars.clone();
        let cache_weak = weak_cache.clone();

        ImageCacheManager::new(
            BASE_URL.to_string(),
            api_client.client(),
            Arc::new(move |ready_url| {
                lock_or_recover(&pending_urls).insert(ready_url);
                if refresh_flag.swap(true, Ordering::SeqCst) {
                    return;
                }
                let pending = pending_urls.clone();
                let flag = refresh_flag.clone();
                let window_weak = window_weak.clone();
                let user_avatars = user_avatars.clone();
                let cache_weak = cache_weak.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    flag.store(false, Ordering::SeqCst);
                    let batch: HashSet<String> = {
                        let mut guard = lock_or_recover(&pending);
                        guard.drain().collect()
                    };
                    if batch.is_empty() {
                        return;
                    }
                    let Some(cache_arc) = cache_weak.upgrade() else {
                        return;
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak.upgrade() {
                            refresh_cached_ui(
                                &window,
                                &cache_arc,
                                &user_avatars,
                                Some(&batch),
                                false,
                                anim_clock.elapsed().as_millis() as u64,
                            );
                        }
                    });
                });
            }),
        )
    });

    // Lightweight timer driving animated GIF avatars when present in memory cache
    let _gif_anim_timer = {
        let timer = slint::Timer::default();
        let window_weak = main_window.as_weak();
        let cache_mgr = cache_mgr.clone();
        let user_avatars = user_avatars.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
            let anim_urls = cache_mgr.animated_avatar_urls();
            if anim_urls.is_empty() {
                return;
            }
            if let Some(window) = window_weak.upgrade() {
                let set: HashSet<String> = anim_urls.into_iter().collect();
                refresh_cached_ui(
                    &window,
                    &cache_mgr,
                    &user_avatars,
                    Some(&set),
                    true,
                    anim_clock.elapsed().as_millis() as u64,
                );
            }
        });
        timer
    };

    // 0. Lazy Audio Engine (WASAPI streams stay closed until user joins voice)
    let audio_engine = Rc::new(RefCell::new(AudioEngine::new_idle()));
    let audio_channels: Rc<RefCell<Option<AudioChannels>>> = Rc::new(RefCell::new(None));
    let audio_ctrl = Some(audio_engine.borrow().controller());
    log::info!("WASAPI audio engine initialized in idle mode (0% idle CPU until voice join)");

    // Callback mute mic
    {
        let audio_ctrl = audio_ctrl.clone();
        let window_weak = main_window.as_weak();
        let socket_holder = socket_client_holder.clone();
        let vcode_lock = active_voice_code.clone();

        main_window.on_toggle_mute(move || {
            if let Some(window) = window_weak.upgrade() {
                let new_state = !window.get_is_muted();
                window.set_is_muted(new_state);
                log::info!("Trạng thái Mute Micro: {}", new_state);
                if let Some(ctrl) = &audio_ctrl {
                    ctrl.set_muted(new_state);
                }

                let vcode = lock_or_recover(&vcode_lock).clone();
                if !vcode.is_empty() {
                    let holder = lock_or_recover(&socket_holder);
                    if let Some(client) = holder.as_ref() {
                        client.send_voice_mute_state(&vcode, new_state);
                    }
                }
            }
        });
    }

    // Callback deafen tai nghe
    {
        let audio_ctrl = audio_ctrl.clone();
        let window_weak = main_window.as_weak();
        main_window.on_toggle_deafen(move || {
            if let Some(window) = window_weak.upgrade() {
                let new_state = !window.get_is_deafened();
                window.set_is_deafened(new_state);
                log::info!("Trạng thái Deafen Audio: {}", new_state);
                if let Some(ctrl) = &audio_ctrl {
                    ctrl.set_deafened(new_state);
                }
            }
        });
    }

    // 0.2. Khởi tạo Win32 Game Click-Through Overlay
    let game_overlay = match GameOverlay::new() {
        Ok(overlay) => {
            log::info!("Game Click-Through Overlay khởi tạo thành công");
            Some(Arc::new(overlay))
        }
        Err(e) => {
            log::warn!("Không thể tạo Game Overlay: {:?}", e);
            None
        }
    };

    // Callback cài đặt / Bật tắt Game Overlay HUD
    {
        let game_overlay = game_overlay.clone();
        main_window.on_open_settings(move || {
            if let Some(overlay) = &game_overlay {
                if overlay.is_visible() {
                    overlay.hide();
                } else {
                    overlay.show();
                }
            }
        });
    }

    // 0.3. Khởi tạo Windows System Tray Icon
    let _tray_manager = {
        let window_weak = main_window.as_weak();
        let audio_ctrl = audio_ctrl.clone();
        let game_overlay = game_overlay.clone();

        let tray_callbacks = TrayCallbacks {
            on_show: Box::new({
                let window_weak = window_weak.clone();
                move || {
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.show().unwrap_or_default();
                            }
                        }
                    });
                }
            }),
            on_toggle_mute: Box::new({
                let window_weak = window_weak.clone();
                let audio_ctrl = audio_ctrl.clone();
                move || {
                    let audio_ctrl = audio_ctrl.clone();
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                let new_state = !window.get_is_muted();
                                window.set_is_muted(new_state);
                                if let Some(ctrl) = &audio_ctrl {
                                    ctrl.set_muted(new_state);
                                }
                            }
                        }
                    });
                }
            }),
            on_toggle_overlay: Box::new({
                let game_overlay = game_overlay.clone();
                move || {
                    if let Some(overlay) = &game_overlay {
                        if overlay.is_visible() {
                            overlay.hide();
                        } else {
                            overlay.show();
                        }
                    }
                }
            }),
            on_exit: Box::new(|| {
                log::info!("Thoát Haven Native Client...");
                std::process::exit(0);
            }),
        };

        match TrayManager::new(tray_callbacks) {
            Ok(tray) => {
                log::info!("System Tray Manager đã kích hoạt");
                Some(tray)
            }
            Err(e) => {
                log::warn!("Không thể tạo System Tray: {:?}", e);
                None
            }
        }
    };

    // 0.4. Khởi tạo Global Hotkey Manager (PTT CapsLock, Ctrl+Shift+M, Ctrl+Shift+D, Ctrl+Shift+O)
    let _hotkey_manager = {
        let window_weak = main_window.as_weak();
        let audio_ctrl = audio_ctrl.clone();
        let game_overlay = game_overlay.clone();

        let hotkey_callbacks = HotkeyCallbacks {
            on_toggle_mute: Box::new({
                let window_weak = window_weak.clone();
                let audio_ctrl = audio_ctrl.clone();
                move || {
                    let audio_ctrl = audio_ctrl.clone();
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                let new_state = !window.get_is_muted();
                                window.set_is_muted(new_state);
                                if let Some(ctrl) = &audio_ctrl {
                                    ctrl.set_muted(new_state);
                                }
                                log::info!("Hotkey: Toggle Mute -> {}", new_state);
                            }
                        }
                    });
                }
            }),
            on_toggle_deafen: Box::new({
                let window_weak = window_weak.clone();
                let audio_ctrl = audio_ctrl.clone();
                move || {
                    let audio_ctrl = audio_ctrl.clone();
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                let new_state = !window.get_is_deafened();
                                window.set_is_deafened(new_state);
                                if let Some(ctrl) = &audio_ctrl {
                                    ctrl.set_deafened(new_state);
                                }
                                log::info!("Hotkey: Toggle Deafen -> {}", new_state);
                            }
                        }
                    });
                }
            }),
            on_toggle_overlay: Box::new({
                let game_overlay = game_overlay.clone();
                move || {
                    if let Some(overlay) = &game_overlay {
                        if overlay.is_visible() {
                            overlay.hide();
                        } else {
                            overlay.show();
                        }
                        log::info!("Hotkey: Toggle In-game Overlay HUD");
                    }
                }
            }),
            on_ptt_down: Box::new({
                let audio_ctrl = audio_ctrl.clone();
                let window_weak = window_weak.clone();
                let socket_holder = socket_client_holder.clone();
                let vcode_lock = active_voice_code.clone();
                move || {
                    if let Some(ctrl) = &audio_ctrl {
                        ctrl.set_muted(false);
                    }
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_is_muted(false);
                            }
                        }
                    });

                    let vcode = lock_or_recover(&vcode_lock).clone();
                    if !vcode.is_empty() {
                        let holder = lock_or_recover(&socket_holder);
                        if let Some(client) = holder.as_ref() {
                            client.send_voice_speaking(true);
                        }
                    }
                    log::info!("PTT Pressed (Transmitting Audio)");
                }
            }),
            on_ptt_up: Box::new({
                let audio_ctrl = audio_ctrl.clone();
                let window_weak = window_weak.clone();
                let socket_holder = socket_client_holder.clone();
                let vcode_lock = active_voice_code.clone();
                move || {
                    if let Some(ctrl) = &audio_ctrl {
                        ctrl.set_muted(true);
                    }
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak.clone();
                        move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_is_muted(true);
                            }
                        }
                    });

                    let vcode = lock_or_recover(&vcode_lock).clone();
                    if !vcode.is_empty() {
                        let holder = lock_or_recover(&socket_holder);
                        if let Some(client) = holder.as_ref() {
                            client.send_voice_speaking(false);
                        }
                    }
                    log::info!("PTT Released (Microphone Muted)");
                }
            }),
        };

        Some(HotkeyManager::start(hotkey_callbacks))
    };

    // 1. Fetch server public config (e.g. server title/name)
    {
        let api_client = api_client.clone();
        let window_weak = main_window.as_weak();
        tokio::spawn(async move {
            if let Ok(config) = api_client.get_public_config().await {
                if let Some(sname) = config.server_name {
                    log::info!("Server name từ VPS: {}", sname);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.set_server_name(sname.into());
                        }
                    });
                }
            }
        });
    }

    // Callback xử lý đăng nhập từ UI Slint
    {
        let api_client = api_client.clone();
        let window_weak = main_window.as_weak();
        let socket_client_holder = socket_client_holder.clone();
        let active_channel_code = active_channel_code.clone();
        let active_voice_code = active_voice_code.clone();
        let channels_cache = channels_cache.clone();
        let current_token = current_token.clone();
        let user_avatars = user_avatars.clone();
        let cache_mgr = cache_mgr.clone();

        main_window.on_do_login(move |username, password| {
            let u = username.to_string();
            let p = password.to_string();
            let api_client = api_client.clone();
            let window_weak = window_weak.clone();
            let socket_client_holder = socket_client_holder.clone();
            let active_channel_code = active_channel_code.clone();
            let active_voice_code = active_voice_code.clone();
            let channels_cache = channels_cache.clone();
            let current_token = current_token.clone();
            let user_avatars = user_avatars.clone();
            let cache_mgr = cache_mgr.clone();

            log::info!("Nhận yêu cầu đăng nhập từ tài khoản: {}", u);

            if let Some(window) = window_weak.upgrade() {
                window.set_login_loading(true);
                window.set_login_error("".into());
            }

            tokio::spawn(async move {
                match api_client.login(&u, &p).await {
                    Ok(res) => {
                        log::info!("Đăng nhập thành công! User: {}", res.user.username);
                        *lock_or_recover(&current_token) = res.token.clone();

                        let session = SavedSession {
                            username: res.user.username.clone(),
                            token: res.token.clone(),
                        };
                        if let Ok(json_data) = serde_json::to_string(&session) {
                            let _ = fs::write(SESSION_FILE, json_data);
                        }

                        start_socket_session(
                            &res.token,
                            &res.user.username,
                            window_weak.clone(),
                            socket_client_holder,
                            active_channel_code,
                            active_voice_code,
                            channels_cache,
                            user_avatars,
                            cache_mgr,
                        ).await;
                    }
                    Err(e) => {
                        let err_msg = format!("Đăng nhập thất bại: {}", e);
                        log::warn!("{}", err_msg);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_login_loading(false);
                                window.set_login_error("Sai tài khoản, mật khẩu hoặc lỗi máy chủ!".into());
                            }
                        });
                    }
                }
            });
        });
    }

    // Callback khi gửi tin nhắn từ Slint UI (Tức thì 0ms Optimistic UI)
    {
        let socket_client_holder = socket_client_holder.clone();
        let active_channel_code = active_channel_code.clone();
        let window_weak = main_window.as_weak();

        main_window.on_send_message(move |msg_text| {
            let text = msg_text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let code = lock_or_recover(&active_channel_code).clone();
            log::info!("Gửi tin nhắn tức thì (Optimistic 0ms) tới channel [{}]: {}", code, text);

            // 1. Gửi qua WebSocket
            let holder = lock_or_recover(&socket_client_holder);
            if let Some(client) = holder.as_ref() {
                if !code.is_empty() {
                    client.send_message(&code, &text);
                }
            }

            // 2. Hiển thị ngay lập tức trên UI (0ms delay)
            if let Some(window) = window_weak.upgrade() {
                let cur_user = window.get_current_username().to_string();
                let cur_av_text = window.get_current_user_avatar_text().to_string();
                let cur_av_color = window.get_current_user_avatar_color();
                let has_av = window.get_has_current_user_avatar();
                let cur_av_img = window.get_current_user_avatar();

                let now = chrono::Local::now();
                let time_str = now.format("%H:%M").to_string();

                mutate_vec_model(
                    window.get_messages(),
                    |m| window.set_messages(m),
                    |m| {
                        let count = m.row_count();
                        let is_grouped = if count > 0 {
                            m.row_data(count - 1)
                                .is_some_and(|last| last.author == cur_user && !cur_user.is_empty())
                        } else {
                            false
                        };

                        let opt_msg = MessageData {
                            id: format!("opt-{}", now.timestamp_millis()).into(),
                            author: cur_user.into(),
                            avatar_text: cur_av_text.into(),
                            avatar_color: cur_av_color,
                            avatar_image: cur_av_img,
                            has_avatar: has_av,
                            timestamp: time_str.into(),
                            content: text.into(),
                            is_bot: false,
                            is_attachment: false,
                            attachment_url: "".into(),
                            attachment_name: "".into(),
                            is_image: false,
                            image_data: slint::Image::default(),
                            has_image: false,
                            is_grouped,
                        };

                        m.push(opt_msg);
                    },
                );
            }
        });
    }

    // Callback toggle voice (từ top bar hoặc sidebar)
    {
        let socket_client_holder = socket_client_holder.clone();
        let active_voice_code = active_voice_code.clone();
        let active_channel_code = active_channel_code.clone();
        let channels_cache = channels_cache.clone();
        let audio_engine = audio_engine.clone();
        let audio_channels = audio_channels.clone();
        let window_weak = main_window.as_weak();

        main_window.on_toggle_voice(move || {
            if let Some(window) = window_weak.upgrade() {
                if window.get_is_voice_connected() {
                    disconnect_voice_session(
                        &window,
                        &socket_client_holder,
                        &active_voice_code,
                        &audio_engine,
                        &audio_channels,
                    );
                } else {
                    let code = lock_or_recover(&active_channel_code).clone();
                    connect_voice_session(
                        &window,
                        &code,
                        &socket_client_holder,
                        &active_voice_code,
                        &channels_cache,
                        &audio_engine,
                        &audio_channels,
                    );
                }
            }
        });
    }

    // Callback đính kèm file / ảnh
    {
        let api_client = api_client.clone();
        let current_token = current_token.clone();
        let socket_client_holder = socket_client_holder.clone();
        let active_channel_code = active_channel_code.clone();

        main_window.on_pick_attachment(move || {
            let api_client = api_client.clone();
            let current_token = current_token.clone();
            let socket_client_holder = socket_client_holder.clone();
            let active_channel_code = active_channel_code.clone();

            tokio::task::spawn_blocking(move || {
                if let Some(file_path) = dialog::pick_file() {
                    log::info!("Đã chọn file gửi: {:?}", file_path);
                    if let Ok(bytes) = std::fs::read(&file_path) {
                        let file_name = file_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                            .to_string();

                        let token = lock_or_recover(&current_token).clone();
                        let code = lock_or_recover(&active_channel_code).clone();

                        tokio::spawn(async move {
                            match api_client.upload_file(&token, &file_name, bytes).await {
                                Ok(res) => {
                                    log::info!("Upload thành công: {:?}", res);
                                    let msg_payload = if res.is_image.unwrap_or(false) {
                                        res.url
                                    } else {
                                        let size_str = format!("{} KB", res.file_size.unwrap_or(0) / 1024);
                                        format!("[file:{}]({}|{})", res.original_name.as_deref().unwrap_or(&file_name), res.url, size_str)
                                    };

                                    let holder = lock_or_recover(&socket_client_holder);
                                    if let Some(client) = holder.as_ref() {
                                        client.send_message(&code, &msg_payload);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Upload thất bại: {:?}", e);
                                }
                            }
                        });
                    }
                }
            });
        });
    }

    // Callback mở attachment trên trình duyệt
    main_window.on_open_attachment(|url| {
        let u = url.to_string();
        log::info!("Mở tệp đính kèm: {}", u);
        open_url(&u);
    });

    // Callback khi chuyển đổi channel (Text hoặc Voice)
    {
        let socket_client_holder = socket_client_holder.clone();
        let active_channel_code = active_channel_code.clone();
        let active_voice_code = active_voice_code.clone();
        let channels_cache = channels_cache.clone();
        let audio_engine = audio_engine.clone();
        let audio_channels = audio_channels.clone();
        let window_weak = main_window.as_weak();

        main_window.on_channel_selected(move |channel_id, is_voice| {
            let code = channel_id.to_string();
            log::info!("Đã chọn channel: {} (voice: {})", code, is_voice);

            if is_voice {
                if let Some(window) = window_weak.upgrade() {
                    connect_voice_session(
                        &window,
                        &code,
                        &socket_client_holder,
                        &active_voice_code,
                        &channels_cache,
                        &audio_engine,
                        &audio_channels,
                    );
                }
            } else {
                *lock_or_recover(&active_channel_code) = code.clone();

                let ch_name = {
                    let cache = lock_or_recover(&channels_cache);
                    cache.iter().find(|c| c.code == code).map(|c| (c.name.clone(), c.topic.clone().unwrap_or_default()))
                };

                if let Some((name, topic)) = ch_name {
                    let window_weak = window_weak.clone();
                    let code_clone = code.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak.upgrade() {
                            window.set_active_channel_code(code_clone.into());
                            window.set_active_channel_name(name.into());
                            window.set_channel_topic(topic.into());
                        }
                    });
                }

                let holder = lock_or_recover(&socket_client_holder);
                if let Some(client) = holder.as_ref() {
                    client.enter_channel(&code);
                }
            }
        });
    }

    // Callback ngắt kết nối Voice
    {
        let socket_client_holder = socket_client_holder.clone();
        let active_voice_code = active_voice_code.clone();
        let audio_engine = audio_engine.clone();
        let audio_channels = audio_channels.clone();
        let window_weak = main_window.as_weak();

        main_window.on_disconnect_voice(move || {
            if let Some(window) = window_weak.upgrade() {
                disconnect_voice_session(
                    &window,
                    &socket_client_holder,
                    &active_voice_code,
                    &audio_engine,
                    &audio_channels,
                );
            }
        });
    }

    // Callback khi chọn server/guild
    main_window.on_guild_selected(|guild_id| {
        log::info!("Đã chọn guild: {}", guild_id);
    });

    // 2. Tự động kiểm tra file session.json đã lưu trước đó
    if Path::new(SESSION_FILE).exists() {
        if let Ok(content) = fs::read_to_string(SESSION_FILE) {
            if let Ok(session) = serde_json::from_str::<SavedSession>(&content) {
                log::info!("Tìm thấy phiên đăng nhập đã lưu cho: {}", session.username);
                *lock_or_recover(&current_token) = session.token.clone();
                let window_weak = main_window.as_weak();
                let socket_client_holder = socket_client_holder.clone();
                let active_channel_code = active_channel_code.clone();
                let active_voice_code = active_voice_code.clone();
                let channels_cache = channels_cache.clone();
                let user_avatars = user_avatars.clone();
                let cache_mgr = cache_mgr.clone();

                tokio::spawn(async move {
                    start_socket_session(
                        &session.token,
                        &session.username,
                        window_weak,
                        socket_client_holder,
                        active_channel_code,
                        active_voice_code,
                        channels_cache,
                        user_avatars,
                        cache_mgr,
                    ).await;
                });
            }
        }
    }

    log::info!("Haven Native sẵn sàng. Hiển thị cửa sổ...");
    main_window.run()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_socket_session(
    token: &str,
    username: &str,
    window_weak: slint::Weak<MainWindow>,
    socket_client_holder: Arc<Mutex<Option<Arc<HavenSocketClient>>>>,
    active_channel_code: Arc<Mutex<String>>,
    active_voice_code: Arc<Mutex<String>>,
    channels_cache: Arc<Mutex<Vec<ChannelItem>>>,
    user_avatars: Arc<Mutex<HashMap<String, String>>>,
    cache_mgr: Arc<ImageCacheManager>,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<HavenEvent>();

    match HavenSocketClient::connect(WS_URL, token, event_tx).await {
        Ok(client) => {
            let client_arc = Arc::new(client);
            *lock_or_recover(&socket_client_holder) = Some(client_arc.clone());
            let mut members_by_channel: HashMap<String, Vec<OnlineUserItem>> = HashMap::new();

            let u_name = username.to_string();
            let w_init = window_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = w_init.upgrade() {
                    window.set_is_logged_in(true);
                    window.set_login_loading(false);
                    window.set_current_username(u_name.into());
                }
            });

            // Lắng nghe sự kiện WebSocket realtime
            while let Some(event) = event_rx.recv().await {
                match event {
                    HavenEvent::Connected(sid) => {
                        log::info!("WebSocket đã kết nối. SID: {}", sid);
                    }
                    HavenEvent::SessionInfo(user_info) => {
                        log::info!("Nhận SessionInfo: id={} username={} avatar={:?} status={:?}",
                            user_info.id, user_info.username, user_info.avatar, user_info.status_text);
                        if let Some(av) = &user_info.avatar {
                            lock_or_recover(&user_avatars).insert(user_info.username.clone(), av.clone());
                        }

                        let avatar_entry = user_info.avatar.as_deref().and_then(|av| cache_mgr.get_or_fetch(av, true));
                        let status_text = user_info.status_text.as_deref().unwrap_or("Online").to_string();

                        let u_name = user_info.username.clone();
                        let initials = if u_name.len() >= 2 {
                            u_name[..2].to_uppercase()
                        } else {
                            u_name.to_uppercase()
                        };

                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_current_username(u_name.into());
                                window.set_user_status(status_text.into());
                                window.set_current_user_avatar_text(initials.into());
                                if let Some(entry) = avatar_entry {
                                    window.set_current_user_avatar(entry.to_slint_image());
                                    window.set_has_current_user_avatar(true);
                                }
                            }
                        });
                    }
                    HavenEvent::ChannelsList(channels) => {
                        log::info!("Nhận danh sách {} kênh từ máy chủ", channels.len());
                        *lock_or_recover(&channels_cache) = channels.clone();

                        let existing_code = lock_or_recover(&active_channel_code).clone();
                        let first_text = channels.iter().find(|c| c.text_enabled == 1);
                        let target_code = if !existing_code.is_empty() {
                            existing_code
                        } else if let Some(ch) = first_text {
                            *lock_or_recover(&active_channel_code) = ch.code.clone();
                            client_arc.enter_channel(&ch.code);
                            ch.code.clone()
                        } else {
                            String::new()
                        };

                        let mut text_items = Vec::new();
                        let mut voice_items = Vec::new();
                        let mut target_name = String::new();
                        let mut target_topic = String::new();

                        for ch in &channels {
                            let is_selected = !target_code.is_empty() && ch.code == target_code;
                            if is_selected {
                                target_name = ch.name.clone();
                                target_topic = ch.topic.clone().unwrap_or_default();
                            }

                            let item = ChannelItemData {
                                id: ch.code.clone().into(),
                                name: ch.name.clone().into(),
                                is_voice: ch.voice_enabled == 1 && ch.text_enabled == 0,
                                is_selected,
                                unread: false,
                            };

                            if ch.voice_enabled == 1 && ch.text_enabled == 0 {
                                voice_items.push(item);
                            } else {
                                text_items.push(item);
                            }
                        }

                        let window_weak = window_weak.clone();
                        let target_code_clone = target_code.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                mutate_vec_model(window.get_text_channels(), |m| window.set_text_channels(m), |m| m.set_vec(text_items));
                                mutate_vec_model(window.get_voice_channels(), |m| window.set_voice_channels(m), |m| m.set_vec(voice_items));
                                if !target_name.is_empty() {
                                    window.set_active_channel_code(target_code_clone.into());
                                    window.set_active_channel_name(target_name.into());
                                    window.set_channel_topic(target_topic.into());
                                }
                            }
                        });
                    }
                    HavenEvent::OnlineUsers(online_evt) => {
                        let cur_code = lock_or_recover(&active_channel_code).clone();
                        log::info!("Nhận danh sách online-users cho channel {}: {} users (kênh hiện tại: {})", online_evt.channel_code, online_evt.users.len(), cur_code);
                        let is_active = online_evt.channel_code == cur_code || cur_code.is_empty();
                        let mut users = online_evt.users;
                        sort_online_users(&mut users);
                        members_by_channel.insert(online_evt.channel_code, users.clone());
                        if is_active {
                            sync_members_ui(&users, &window_weak, &cache_mgr, &user_avatars);
                        }
                    }
                    HavenEvent::OnlineUsersDelta(delta) => {
                        let cur_code = lock_or_recover(&active_channel_code).clone();
                        if let Some(list) = members_by_channel.get_mut(&delta.channel_code) {
                            let drop_ids: HashSet<i64> = delta
                                .remove
                                .iter()
                                .copied()
                                .chain(delta.upsert.iter().map(|u| u.id))
                                .collect();
                            list.retain(|u| !drop_ids.contains(&u.id));
                            list.extend(delta.upsert);
                            sort_online_users(list);
                            if delta.channel_code == cur_code || cur_code.is_empty() {
                                sync_members_ui(list, &window_weak, &cache_mgr, &user_avatars);
                            }
                        }
                    }
                    HavenEvent::PingRtt(rtt) => {
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_voice_ping_ms(rtt as i32);
                            }
                        });
                    }
                    HavenEvent::MessageHistory(history) => {
                        let cur_code = lock_or_recover(&active_channel_code).clone();
                        if history.channel_code == cur_code {
                            log::info!("Nạp {} tin nhắn lịch sử (0ms) cho channel {}", history.messages.len(), cur_code);
                            let mut processed = Vec::new();
                            let mut prev_author: Option<String> = None;
                            for m in history.messages {
                                let author = m.username.clone().unwrap_or_default();
                                let is_grouped = prev_author.as_ref() == Some(&author) && !author.is_empty();
                                prev_author = Some(author);
                                let mut pm = process_message(m, &cache_mgr, &user_avatars);
                                pm.is_grouped = is_grouped;
                                processed.push(pm);
                            }
                            let window_weak = window_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    let slint_msgs: Vec<MessageData> = processed.into_iter().map(to_slint_message).collect();
                                    mutate_vec_model(window.get_messages(), |m| window.set_messages(m), |m| m.set_vec(slint_msgs));
                                }
                            });
                        }
                    }
                    HavenEvent::NewMessage(new_msg) => {
                        let cur_code = lock_or_recover(&active_channel_code).clone();
                        if new_msg.channel_code == cur_code {
                            let u = new_msg.message.username.clone().unwrap_or_else(|| "Unknown".to_string());
                            let c = new_msg.message.content.clone().unwrap_or_default();
                            log::info!("Tin nhắn mới từ [{}]: {}", u, c);
                            let mut processed = process_message(new_msg.message, &cache_mgr, &user_avatars);
                            let window_weak = window_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    let cur_user = window.get_current_username().to_string();
                                    mutate_vec_model(
                                        window.get_messages(),
                                        |m| window.set_messages(m),
                                        |m| {
                                            let count = m.row_count();
                                            if count > 0 {
                                                let last_idx = count - 1;
                                                if let Some(mut last) = m.row_data(last_idx) {
                                                    if u == cur_user && last.id.starts_with("opt-") && last.content == c {
                                                        last.id = processed.id.into();
                                                        m.set_row_data(last_idx, last);
                                                        return;
                                                    }
                                                    if last.author == u && !u.is_empty() {
                                                        processed.is_grouped = true;
                                                    }
                                                }
                                            }
                                            m.push(to_slint_message(processed));
                                        },
                                    );
                                }
                            });
                        }
                    }
                    HavenEvent::VoiceExistingUsers { channel_code, users } => {
                        log::info!("Phòng thoại [{}] có {} thành viên đang tham gia", channel_code, users.len());
                        *lock_or_recover(&active_voice_code) = channel_code.clone();
                        let window_weak = window_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_is_voice_connected(true);
                                window.set_active_voice_channel(channel_code.into());
                            }
                        });
                    }
                    HavenEvent::VoiceUsersUpdate { channel_code, users } => {
                        log::info!("Cập nhật thành viên phòng thoại [{}] ({} users)", channel_code, users.len());
                        let cur_voice = lock_or_recover(&active_voice_code).clone();
                        if channel_code == cur_voice {
                            let window_weak = window_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    window.set_is_voice_connected(true);
                                }
                            });
                        }
                    }
                    HavenEvent::VoiceUserJoined { channel_code, user } => {
                        log::info!("Người dùng mới vào voice [{:?}]: {:?}", channel_code, user);
                    }
                    HavenEvent::VoiceUserLeft { user_id } => {
                        log::info!("Người dùng rời voice: {:?}", user_id);
                    }
                    HavenEvent::VoiceSpeaking { user_id, speaking } => {
                        log::debug!("Trạng thái nói: user {:?} -> {}", user_id, speaking);
                    }
                    HavenEvent::Disconnected(reason) => {
                        log::warn!("Mất kết nối WebSocket: {}", reason);
                    }
                }
            }
        }
        Err(e) => {
            log::error!("Không thể kết nối WebSocket: {:?}", e);
            let w_err = window_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = w_err.upgrade() {
                    window.set_is_logged_in(false);
                    window.set_login_error("Mất kết nối đến máy chủ WebSocket!".into());
                }
            });
        }
    }
}

fn parse_attachment_content(content: &str) -> (String, bool, bool, String, String) {
    let trimmed = content.trim();

    // Check if [file:originalName](url|size)
    if trimmed.starts_with("[file:") && trimmed.contains("](") && trimmed.ends_with(')') {
        if let Some(close_bracket) = trimmed.find("](") {
            let name = &trimmed[6..close_bracket];
            let inner = &trimmed[close_bracket + 2..trimmed.len() - 1];
            let (url, _) = if let Some(sep) = inner.find('|') {
                (&inner[..sep], &inner[sep + 1..])
            } else {
                (inner, "")
            };
            return (String::new(), true, false, url.to_string(), name.to_string());
        }
    }

    // Check if raw upload URL or image URL
    let is_upload = trimmed.starts_with("/uploads/") || (trimmed.starts_with("http") && trimmed.contains("/uploads/"));
    let lower = trimmed.to_lowercase();
    let is_img = lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".gif") || lower.ends_with(".webp");

    if is_upload || is_img {
        let name = Path::new(trimmed)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_string();
        return (String::new(), true, is_img, trimmed.to_string(), name);
    }

    (content.to_string(), false, false, String::new(), String::new())
}

struct ProcessedMember {
    name: String,
    role: String,
    avatar_text: String,
    avatar_color: Color,
    avatar_entry: Option<DecodedImageEntry>,
    status_color: Color,
    status_text: String,
    is_bot: bool,
    online: bool,
}

struct ProcessedMessage {
    id: String,
    author: String,
    avatar_text: String,
    avatar_color: Color,
    avatar_entry: Option<DecodedImageEntry>,
    timestamp: String,
    content: String,
    is_bot: bool,
    is_attachment: bool,
    attachment_url: String,
    attachment_name: String,
    is_image: bool,
    image_entry: Option<DecodedImageEntry>,
    is_grouped: bool,
}

fn sort_online_users(users: &mut [OnlineUserItem]) {
    users.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.username.to_ascii_lowercase().cmp(&b.username.to_ascii_lowercase()))
    });
}

fn sync_members_ui(
    users: &[OnlineUserItem],
    window_weak: &slint::Weak<MainWindow>,
    cache_mgr: &Arc<ImageCacheManager>,
    user_avatars: &Arc<Mutex<HashMap<String, String>>>,
) {
    let mut processed_members: Vec<ProcessedMember> = Vec::with_capacity(users.len());
    for u in users {
        if let Some(av) = &u.avatar {
            lock_or_recover(user_avatars).insert(u.username.clone(), av.clone());
        }

        let is_bot = u.username.to_lowercase().contains("bot") || u.username.to_lowercase().contains("bridge");
        let role_name = u
            .role
            .as_ref()
            .and_then(|r| r.name.clone())
            .unwrap_or_else(|| "Member".to_string());

        let avatar_color = u
            .role
            .as_ref()
            .and_then(|role| role.color.as_deref())
            .and_then(parse_hex_color)
            .unwrap_or_else(|| Color::from_rgb_u8(88, 101, 242));

        let status_color = if u.online {
            match u.status.as_deref() {
                Some("away") => Color::from_rgb_u8(241, 196, 15),
                Some("dnd") => Color::from_rgb_u8(231, 76, 60),
                _ => Color::from_rgb_u8(35, 165, 90),
            }
        } else {
            Color::from_rgb_u8(116, 127, 141)
        };

        let initials = if u.username.len() >= 2 {
            u.username[..2].to_uppercase()
        } else {
            u.username.to_uppercase()
        };

        let avatar_entry = u.avatar.as_deref().and_then(|av| cache_mgr.get_or_fetch(av, true));

        processed_members.push(ProcessedMember {
            name: u.username.clone(),
            role: role_name,
            avatar_text: initials,
            avatar_color,
            avatar_entry,
            status_color,
            status_text: u.status_text.clone().unwrap_or_default(),
            is_bot,
            online: u.online,
        });
    }

    let window_weak = window_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window_weak.upgrade() {
            let mut online_list: Vec<MemberData> = Vec::new();
            let mut offline_list: Vec<MemberData> = Vec::new();

            for pm in processed_members {
                let (avatar_image, has_avatar) = if let Some(entry) = &pm.avatar_entry {
                    (entry.to_slint_image(), true)
                } else {
                    (slint::Image::default(), false)
                };

                let member = MemberData {
                    name: pm.name.into(),
                    role: pm.role.into(),
                    avatar_text: pm.avatar_text.into(),
                    avatar_color: pm.avatar_color,
                    avatar_image,
                    has_avatar,
                    status_color: pm.status_color,
                    status_text: pm.status_text.into(),
                    is_bot: pm.is_bot,
                };

                if pm.online {
                    online_list.push(member);
                } else {
                    offline_list.push(member);
                }
            }

            mutate_vec_model(window.get_online_members(), |m| window.set_online_members(m), |m| m.set_vec(online_list));
            mutate_vec_model(window.get_offline_members(), |m| window.set_offline_members(m), |m| m.set_vec(offline_list));
        }
    });
}

fn connect_voice_session(
    window: &MainWindow,
    code: &str,
    socket_client_holder: &Arc<Mutex<Option<Arc<HavenSocketClient>>>>,
    active_voice_code: &Arc<Mutex<String>>,
    channels_cache: &Arc<Mutex<Vec<ChannelItem>>>,
    audio_engine: &Rc<RefCell<AudioEngine>>,
    audio_channels: &Rc<RefCell<Option<AudioChannels>>>,
) {
    if !code.is_empty() {
        log::info!("Joining voice channel [{}]", code);
    }
    match audio_engine.borrow_mut().start_streams() {
        Ok(ch) => *audio_channels.borrow_mut() = Some(ch),
        Err(e) => log::warn!("Failed to open WASAPI audio streams on voice join: {:?}", e),
    }
    if !code.is_empty() {
        let client_opt = lock_or_recover(socket_client_holder).clone();
        if let Some(client) = client_opt {
            client.join_voice(code);
        }
    }
    *lock_or_recover(active_voice_code) = code.to_string();
    let ch_name = {
        let cache = lock_or_recover(channels_cache);
        cache
            .iter()
            .find(|c| c.code == code)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| code.to_string())
    };
    window.set_is_voice_connected(true);
    window.set_active_voice_channel(ch_name.into());
}

fn disconnect_voice_session(
    window: &MainWindow,
    socket_client_holder: &Arc<Mutex<Option<Arc<HavenSocketClient>>>>,
    active_voice_code: &Arc<Mutex<String>>,
    audio_engine: &Rc<RefCell<AudioEngine>>,
    audio_channels: &Rc<RefCell<Option<AudioChannels>>>,
) {
    let code = {
        let mut guard = lock_or_recover(active_voice_code);
        std::mem::take(&mut *guard)
    };
    log::info!("Disconnecting from voice channel [{}]", code);
    if !code.is_empty() {
        let client_opt = lock_or_recover(socket_client_holder).clone();
        if let Some(client) = client_opt {
            client.leave_voice(&code);
        }
    }
    audio_engine.borrow_mut().stop_streams();
    *audio_channels.borrow_mut() = None;
    window.set_is_voice_connected(false);
    window.set_active_voice_channel("".into());
}

fn refresh_cached_ui(
    window: &MainWindow,
    cache_mgr: &ImageCacheManager,
    user_avatars: &Arc<Mutex<HashMap<String, String>>>,
    filter_urls: Option<&HashSet<String>>,
    anim_only: bool,
    elapsed_ms: u64,
) {
    let avatars_snapshot = lock_or_recover(user_avatars).clone();
    let matches_filter = |url: &str| filter_urls.is_none_or(|set| set.contains(url));

    let cur_user = window.get_current_username().to_string();
    if let Some(self_url) = avatars_snapshot.get(&cur_user) {
        if matches_filter(self_url) {
            if let Some(entry) = cache_mgr.get_cached_only(self_url, true) {
                if !anim_only || entry.frame_count() > 1 {
                    window.set_current_user_avatar(entry.to_slint_frame(elapsed_ms));
                    window.set_has_current_user_avatar(true);
                }
            }
        }
    }

    let refresh_members = |model_rc: slint::ModelRc<MemberData>| {
        if let Some(m) = model_rc.as_any().downcast_ref::<VecModel<MemberData>>() {
            for idx in 0..m.row_count() {
                if let Some(mut row) = m.row_data(idx) {
                    if let Some(url) = avatars_snapshot.get(row.name.as_str()) {
                        let should_update = if anim_only {
                            matches_filter(url)
                        } else {
                            !row.has_avatar || matches_filter(url)
                        };
                        if should_update {
                            if let Some(entry) = cache_mgr.get_cached_only(url, true) {
                                row.avatar_image = entry.to_slint_frame(elapsed_ms);
                                row.has_avatar = true;
                                m.set_row_data(idx, row);
                            }
                        }
                    }
                }
            }
        }
    };
    refresh_members(window.get_online_members());
    refresh_members(window.get_offline_members());

    if let Some(m) = window.get_messages().as_any().downcast_ref::<VecModel<MessageData>>() {
        for idx in 0..m.row_count() {
            if let Some(mut row) = m.row_data(idx) {
                let mut changed = false;
                if !row.is_grouped {
                    if let Some(url) = avatars_snapshot.get(row.author.as_str()) {
                        let should_update = if anim_only {
                            matches_filter(url)
                        } else {
                            !row.has_avatar || matches_filter(url)
                        };
                        if should_update {
                            if let Some(entry) = cache_mgr.get_cached_only(url, true) {
                                row.avatar_image = entry.to_slint_frame(elapsed_ms);
                                row.has_avatar = true;
                                changed = true;
                            }
                        }
                    }
                }
                if !anim_only && row.is_image && !row.attachment_url.is_empty() {
                    let att_url = row.attachment_url.to_string();
                    if !row.has_image || matches_filter(&att_url) {
                        if let Some(entry) = cache_mgr.get_cached_only(&att_url, false) {
                            row.image_data = entry.to_slint_image();
                            row.has_image = true;
                            changed = true;
                        }
                    }
                }
                if changed {
                    m.set_row_data(idx, row);
                }
            }
        }
    }
}

fn process_message(
    m: ChatMessageItem,
    cache_mgr: &Arc<ImageCacheManager>,
    user_avatars: &Arc<Mutex<HashMap<String, String>>>,
) -> ProcessedMessage {
    let username = m.username.unwrap_or_else(|| "Unknown".to_string());
    let raw_content = m.content.unwrap_or_default();
    let created_at = m.created_at.unwrap_or_default();

    let (content, is_attachment, is_image, attachment_url, attachment_name) = parse_attachment_content(&raw_content);

    let initials = if username.len() >= 2 {
        username[..2].to_uppercase()
    } else {
        username.to_uppercase()
    };

    let is_bot = username.to_lowercase().contains("bot") || username.to_lowercase().contains("bridge");
    let color = if is_bot {
        Color::from_rgb_u8(88, 101, 242) // Blurple
    } else {
        Color::from_rgb_u8(35, 165, 90) // Discord Green
    };

    let time_str = if created_at.len() >= 16 {
        created_at[11..16].to_string()
    } else {
        created_at
    };

    let avatar_url = if let Some(av) = m.avatar {
        lock_or_recover(&user_avatars).insert(username.clone(), av.clone());
        Some(av)
    } else {
        let map = lock_or_recover(&user_avatars);
        map.get(&username).cloned()
    };

    let avatar_entry = avatar_url.and_then(|av| cache_mgr.get_or_fetch(&av, true));

    let image_entry = if is_image && !attachment_url.is_empty() {
        cache_mgr.get_or_fetch(&attachment_url, false)
    } else {
        None
    };

    ProcessedMessage {
        id: format!("{}", m.id),
        author: username,
        avatar_text: initials,
        avatar_color: color,
        avatar_entry,
        timestamp: time_str,
        content,
        is_bot,
        is_attachment,
        attachment_url,
        attachment_name,
        is_image,
        image_entry,
        is_grouped: false,
    }
}

fn to_slint_message(pm: ProcessedMessage) -> MessageData {
    let (avatar_image, has_avatar) = if let Some(entry) = &pm.avatar_entry {
        (entry.to_slint_image(), true)
    } else {
        (slint::Image::default(), false)
    };

    let (image_data, has_image) = if let Some(entry) = &pm.image_entry {
        (entry.to_slint_image(), true)
    } else {
        (slint::Image::default(), false)
    };

    MessageData {
        id: pm.id.into(),
        author: pm.author.into(),
        avatar_text: pm.avatar_text.into(),
        avatar_color: pm.avatar_color,
        avatar_image,
        has_avatar,
        timestamp: pm.timestamp.into(),
        content: pm.content.into(),
        is_bot: pm.is_bot,
        is_attachment: pm.is_attachment,
        attachment_url: pm.attachment_url.into(),
        attachment_name: pm.attachment_name.into(),
        is_image: pm.is_image,
        image_data,
        has_image,
        is_grouped: pm.is_grouped,
    }
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some(Color::from_rgb_u8(r, g, b))
    } else {
        None
    }
}

fn open_url(url: &str) {
    let full_url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("{}{}", BASE_URL, url)
    };

    #[cfg(windows)]
    unsafe {
        let wide: Vec<u16> = full_url.encode_utf16().chain(std::iter::once(0)).collect();
        let op: Vec<u16> = "open\0".encode_utf16().collect();
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            0,
            op.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }

    #[cfg(not(windows))]
    {
        let _ = open::that(full_url);
    }
}
