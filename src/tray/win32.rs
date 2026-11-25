#[cfg(windows)]
pub mod imp {
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW,
        PostQuitMessage, RegisterClassExW, SetForegroundWindow, TrackPopupMenu,
        CS_HREDRAW, CS_VREDRAW, HCURSOR, HICON, HMENU, IDI_APPLICATION, MF_SEPARATOR,
        MF_STRING, TPM_RIGHTBUTTON, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK,
        WM_LBUTTONUP, WM_RBUTTONUP, WM_USER, WNDCLASSEXW,
    };

    pub const WM_TRAYICON: u32 = WM_USER + 100;
    pub const IDM_SHOW: usize = 1001;
    pub const IDM_TOGGLE_MUTE: usize = 1002;
    pub const IDM_TOGGLE_OVERLAY: usize = 1003;
    pub const IDM_EXIT: usize = 1004;

    pub struct TrayManager {
        helper_hwnd: HWND,
        is_running: Arc<AtomicBool>,
    }

    pub struct TrayCallbacks {
        pub on_show: Box<dyn Fn() + Send + Sync>,
        pub on_toggle_mute: Box<dyn Fn() + Send + Sync>,
        pub on_toggle_overlay: Box<dyn Fn() + Send + Sync>,
        pub on_exit: Box<dyn Fn() + Send + Sync>,
    }

    static TRAY_CALLBACKS: OnceLock<Mutex<Option<TrayCallbacks>>> = OnceLock::new();

    fn get_callbacks() -> &'static Mutex<Option<TrayCallbacks>> {
        TRAY_CALLBACKS.get_or_init(|| Mutex::new(None))
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TRAYICON => {
                let event = lparam as u32;
                match event {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                        if let Ok(guard) = get_callbacks().lock() {
                            if let Some(cb) = guard.as_ref() {
                                (cb.on_show)();
                            }
                        }
                    }
                    WM_RBUTTONUP => {
                        let mut pt: POINT = std::mem::zeroed();
                        GetCursorPos(&mut pt);
                        SetForegroundWindow(hwnd);

                        let hmenu = CreatePopupMenu();
                        let show_str = encode_wide("Show Haven\0");
                        let mute_str = encode_wide("Toggle Mute (Mic)\0");
                        let overlay_str = encode_wide("Toggle In-game HUD\0");
                        let exit_str = encode_wide("Exit Haven\0");

                        AppendMenuW(hmenu, MF_STRING, IDM_SHOW, show_str.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, IDM_TOGGLE_MUTE, mute_str.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, IDM_TOGGLE_OVERLAY, overlay_str.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());
                        AppendMenuW(hmenu, MF_STRING, IDM_EXIT, exit_str.as_ptr());

                        TrackPopupMenu(
                            hmenu,
                            TPM_RIGHTBUTTON,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        DestroyMenu(hmenu);
                    }
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                let id = wparam;
                if let Ok(guard) = get_callbacks().lock() {
                    if let Some(cb) = guard.as_ref() {
                        match id {
                            IDM_SHOW => (cb.on_show)(),
                            IDM_TOGGLE_MUTE => (cb.on_toggle_mute)(),
                            IDM_TOGGLE_OVERLAY => (cb.on_toggle_overlay)(),
                            IDM_EXIT => (cb.on_exit)(),
                            _ => {}
                        }
                    }
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    impl TrayManager {
        pub fn new(callbacks: TrayCallbacks) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            if let Ok(mut guard) = get_callbacks().lock() {
                *guard = Some(callbacks);
            }

            unsafe {
                let hinstance = GetModuleHandleW(std::ptr::null());
                let class_name = encode_wide("HavenTrayHelperClass\0");

                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(tray_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0 as HICON,
                    hCursor: 0 as HCURSOR,
                    hbrBackground: 0_isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: class_name.as_ptr(),
                    hIconSm: 0 as HICON,
                };

                RegisterClassExW(&wc);

                let window_title = encode_wide("Haven Tray Helper\0");
                let hwnd = CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    window_title.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0 as HWND,
                    0 as HMENU,
                    hinstance,
                    null_mut(),
                );

                if hwnd == 0 {
                    return Err("Failed to create Win32 Tray helper window".into());
                }

                let icon = LoadIconW(0 as _, IDI_APPLICATION);
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                nid.uCallbackMessage = WM_TRAYICON;
                nid.hIcon = icon;

                let tip = encode_wide("Haven Native Client\0");
                for (i, &ch) in tip.iter().enumerate().take(127) {
                    nid.szTip[i] = ch;
                }

                Shell_NotifyIconW(NIM_ADD, &nid);
                log::info!("Haven System Tray Icon successfully registered");

                let is_running = Arc::new(AtomicBool::new(true));
                let is_running_clone = is_running.clone();

                std::thread::spawn(move || {
                    let mut msg = std::mem::zeroed();
                    while is_running_clone.load(Ordering::Relaxed) {
                        if GetMessageW(&mut msg, hwnd, 0, 0) > 0 {
                            DispatchMessageW(&msg);
                        } else {
                            break;
                        }
                    }
                });

                Ok(Self {
                    helper_hwnd: hwnd,
                    is_running,
                })
            }
        }

        pub fn remove(&self) {
            unsafe {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = self.helper_hwnd;
                nid.uID = 1;
                Shell_NotifyIconW(NIM_DELETE, &nid);
                self.is_running.store(false, Ordering::SeqCst);
                DestroyWindow(self.helper_hwnd);
                log::info!("Haven System Tray Icon removed");
            }
        }
    }

    impl Drop for TrayManager {
        fn drop(&mut self) {
            self.remove();
        }
    }

    fn encode_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
}

#[cfg(not(windows))]
pub mod imp {
    pub struct TrayCallbacks;
    pub struct TrayManager;
    impl TrayManager {
        pub fn new(_: TrayCallbacks) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Self)
        }
        pub fn remove(&self) {}
    }
}

pub use imp::{TrayCallbacks, TrayManager};
