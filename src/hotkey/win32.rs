#[cfg(windows)]
pub mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CAPITAL, VK_CONTROL, VK_SHIFT,
    };

    pub struct HotkeyCallbacks {
        pub on_toggle_mute: Box<dyn Fn() + Send + Sync>,
        pub on_toggle_deafen: Box<dyn Fn() + Send + Sync>,
        pub on_toggle_overlay: Box<dyn Fn() + Send + Sync>,
        pub on_ptt_down: Box<dyn Fn() + Send + Sync>,
        pub on_ptt_up: Box<dyn Fn() + Send + Sync>,
    }

    pub struct HotkeyManager {
        is_running: Arc<AtomicBool>,
    }

    impl HotkeyManager {
        pub fn start(callbacks: HotkeyCallbacks) -> Self {
            let is_running = Arc::new(AtomicBool::new(true));
            let running = is_running.clone();

            std::thread::spawn(move || {
                let mut ptt_was_down = false;
                let mut m_was_down = false;
                let mut d_was_down = false;
                let mut o_was_down = false;

                // VK Codes
                const VK_M: i32 = 0x4D;
                const VK_D: i32 = 0x44;
                const VK_O: i32 = 0x4F;
                const VK_PTT: i32 = VK_CAPITAL as i32; // Default Push-To-Talk: CapsLock

                log::info!("Global Hotkey Manager started: PTT (CapsLock), Ctrl+Shift+M (Mute), Ctrl+Shift+D (Deafen), Ctrl+Shift+O (Overlay)");

                while running.load(Ordering::Relaxed) {
                    unsafe {
                        let ctrl_down = (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                        let shift_down = (GetAsyncKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;

                        // Check Push-To-Talk
                        let ptt_down = (GetAsyncKeyState(VK_PTT) as u16 & 0x8000) != 0;
                        if ptt_down && !ptt_was_down {
                            ptt_was_down = true;
                            (callbacks.on_ptt_down)();
                        } else if !ptt_down && ptt_was_down {
                            ptt_was_down = false;
                            (callbacks.on_ptt_up)();
                        }

                        // Check Ctrl + Shift combinations
                        if ctrl_down && shift_down {
                            // Ctrl + Shift + M -> Toggle Mute
                            let m_down = (GetAsyncKeyState(VK_M) as u16 & 0x8000) != 0;
                            if m_down && !m_was_down {
                                m_was_down = true;
                                (callbacks.on_toggle_mute)();
                            } else if !m_down {
                                m_was_down = false;
                            }

                            // Ctrl + Shift + D -> Toggle Deafen
                            let d_down = (GetAsyncKeyState(VK_D) as u16 & 0x8000) != 0;
                            if d_down && !d_was_down {
                                d_was_down = true;
                                (callbacks.on_toggle_deafen)();
                            } else if !d_down {
                                d_was_down = false;
                            }

                            // Ctrl + Shift + O -> Toggle Overlay
                            let o_down = (GetAsyncKeyState(VK_O) as u16 & 0x8000) != 0;
                            if o_down && !o_was_down {
                                o_was_down = true;
                                (callbacks.on_toggle_overlay)();
                            } else if !o_down {
                                o_was_down = false;
                            }
                        } else {
                            m_was_down = false;
                            d_was_down = false;
                            o_was_down = false;
                        }
                    }

                    // Poll every 16ms (~60Hz polling for near-zero latency in games)
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }

                log::info!("Global Hotkey Manager stopped");
            });

            Self { is_running }
        }

        pub fn stop(&self) {
            self.is_running.store(false, Ordering::SeqCst);
        }
    }

    impl Drop for HotkeyManager {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(not(windows))]
pub mod imp {
    pub struct HotkeyCallbacks;
    pub struct HotkeyManager;
    impl HotkeyManager {
        pub fn start(_: HotkeyCallbacks) -> Self { Self }
        pub fn stop(&self) {}
    }
}

pub use imp::{HotkeyCallbacks, HotkeyManager};
