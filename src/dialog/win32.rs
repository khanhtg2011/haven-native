#[cfg(windows)]
pub mod imp {
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};

    #[repr(C)]
    #[allow(non_snake_case, clippy::upper_case_acronyms)]
    struct OPENFILENAMEW {
        lStructSize: u32,
        hwndOwner: HWND,
        hInstance: isize,
        lpstrFilter: *const u16,
        lpstrCustomFilter: *mut u16,
        nMaxCustFilter: u32,
        nFilterIndex: u32,
        lpstrFile: *mut u16,
        nMaxFile: u32,
        lpstrFileTitle: *mut u16,
        nMaxFileTitle: u32,
        lpstrInitialDir: *const u16,
        lpstrTitle: *const u16,
        Flags: u32,
        nFileOffset: u16,
        nFileExtension: u16,
        lpstrDefExt: *const u16,
        lCustData: LPARAM,
        lpfnHook: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> usize>,
        lpTemplateName: *const u16,
        pvReserved: *mut std::ffi::c_void,
        dwReserved: u32,
        FlagsEx: u32,
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(lpofn: *mut OPENFILENAMEW) -> BOOL;
    }

    const OFN_FILEMUSTEXIST: u32 = 0x00001000;
    const OFN_PATHMUSTEXIST: u32 = 0x00000800;

    pub fn pick_file() -> Option<PathBuf> {
        let mut file_buf = vec![0u16; 1024];

        let filter_str = "All Files (*.*)\0*.*\0Images (*.png;*.jpg;*.jpeg;*.gif;*.webp)\0*.png;*.jpg;*.jpeg;*.gif;*.webp\0\0";
        let filter_wide: Vec<u16> = filter_str.encode_utf16().collect();
        let title_wide: Vec<u16> = "Select File to Send in Haven\0".encode_utf16().collect();

        let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFilter = filter_wide.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = file_buf.as_mut_ptr();
        ofn.nMaxFile = file_buf.len() as u32;
        ofn.lpstrTitle = title_wide.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;

        let ok = unsafe { GetOpenFileNameW(&mut ofn) };
        if ok != 0 {
            let nul_pos = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
            let path_str = String::from_utf16_lossy(&file_buf[..nul_pos]);
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
        None
    }
}

#[cfg(not(windows))]
pub mod imp {
    use std::path::PathBuf;
    pub fn pick_file() -> Option<PathBuf> {
        None
    }
}

pub use imp::pick_file;
