#[cfg(windows)]
pub mod imp {
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{GetStockObject, UpdateWindow, BLACK_BRUSH};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, PostQuitMessage, RegisterClassExW,
        SetLayeredWindowAttributes, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HCURSOR,
        HICON, HWND_TOPMOST, LWA_ALPHA, LWA_COLORKEY, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
        SW_SHOWNOACTIVATE, WM_DESTROY, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    pub struct GameOverlay {
        hwnd: HWND,
        is_visible: Arc<AtomicBool>,
    }

    unsafe extern "system" fn overlay_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    impl GameOverlay {
        pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            unsafe {
                let hinstance = GetModuleHandleW(std::ptr::null());
                let class_name = encode_wide("HavenGameOverlayClass\0");

                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(overlay_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0 as HICON,
                    hCursor: 0 as HCURSOR,
                    hbrBackground: GetStockObject(BLACK_BRUSH) as isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: class_name.as_ptr(),
                    hIconSm: 0 as HICON,
                };

                RegisterClassExW(&wc);

                // Window styles: Click-through + Layered + NoActivate + Topmost
                let ex_style = WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW;
                let style = WS_POPUP;

                let window_title = encode_wide("Haven Overlay\0");

                let hwnd = CreateWindowExW(
                    ex_style,
                    class_name.as_ptr(),
                    window_title.as_ptr(),
                    style,
                    40,  // x position
                    40,  // y position
                    300, // width
                    200, // height
                    0 as HWND,
                    0 as _,
                    hinstance,
                    null_mut(),
                );

                if hwnd == 0 {
                    return Err("Failed to create Win32 Game Overlay window".into());
                }

                // Transparent background (ColorKey: Black RGB(0,0,0) is rendered invisible)
                SetLayeredWindowAttributes(hwnd, 0x00000000 as COLORREF, 220, LWA_COLORKEY | LWA_ALPHA);

                log::info!("Haven Win32 Game Click-Through Overlay created successfully (HWND: {:?})", hwnd);

                Ok(Self {
                    hwnd,
                    is_visible: Arc::new(AtomicBool::new(false)),
                })
            }
        }

        pub fn show(&self) {
            unsafe {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE,
                );
                UpdateWindow(self.hwnd);
                self.is_visible.store(true, Ordering::SeqCst);
                log::info!("Game Overlay visible: True (Click-through activated)");
            }
        }

        pub fn hide(&self) {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
                self.is_visible.store(false, Ordering::SeqCst);
                log::info!("Game Overlay hidden");
            }
        }

        pub fn is_visible(&self) -> bool {
            self.is_visible.load(Ordering::Relaxed)
        }
    }

    fn encode_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
}

#[cfg(not(windows))]
pub mod imp {
    pub struct GameOverlay;
    impl GameOverlay {
        pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Self)
        }
        pub fn show(&self) {}
        pub fn hide(&self) {}
        pub fn is_visible(&self) -> bool { false }
    }
}

pub use imp::GameOverlay;
