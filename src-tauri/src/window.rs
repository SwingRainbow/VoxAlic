use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow,
    SetWindowPos, ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE,
};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub hwnd: usize,
    pub pid: u32,
}

/// Enumerate all top-level visible windows whose title contains `keyword`
/// (case-insensitive). Excludes windows owned by the current process.
/// Results are sorted so exact title matches come first,
/// then partial matches by alphabetical order.
pub fn list_windows(keyword: &str) -> Vec<WindowInfo> {
    let keyword_lower = keyword.to_lowercase();
    let mut windows: Vec<WindowInfo> = Vec::new();
    let own_pid = std::process::id();

    unsafe {
        let _ = EnumWindows(
            Some(enum_callback),
            LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
        );
    }

    // Filter by keyword AND exclude own process
    windows.retain(|w| {
        w.title.to_lowercase().contains(&keyword_lower) && w.pid != own_pid
    });

    // Sort: exact matches first, then by title
    windows.sort_by(|a, b| {
        let a_exact = a.title.to_lowercase() == keyword_lower;
        let b_exact = b.title.to_lowercase() == keyword_lower;
        b_exact.cmp(&a_exact).then_with(|| a.title.cmp(&b.title))
    });

    windows
}

unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows: &mut Vec<WindowInfo> = &mut *(lparam.0 as *mut Vec<WindowInfo>);

    if IsWindowVisible(hwnd).as_bool() {
        let len = GetWindowTextLengthW(hwnd);
        if len > 0 {
            let mut buf = vec![0u16; (len + 1) as usize];
            let actual = GetWindowTextW(hwnd, &mut buf);
            if actual > 0 {
                buf.truncate(actual as usize);
                if let Ok(title) = String::from_utf16(&buf) {
                    let mut pid: u32 = 0;
                    GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    windows.push(WindowInfo {
                        title,
                        hwnd: hwnd.0 as usize,
                        pid,
                    });
                }
            }
        }
    }
    BOOL::from(true)
}

/// First visible window whose title contains `keyword`, or 0 if none. Shared by
/// the OCR thread and the calibration/test commands so they resolve the game
/// window identically.
pub fn resolve_hwnd(keyword: &str) -> isize {
    list_windows(keyword).first().map(|w| w.hwnd as isize).unwrap_or(0)
}

/// Check whether the given window is minimised (iconic).
pub fn is_minimized(hwnd: isize) -> bool {
    unsafe { IsIconic(HWND(hwnd as *mut _)).as_bool() }
}

/// Check whether the given window handle is still valid.
pub fn is_valid(hwnd: isize) -> bool {
    unsafe { IsWindow(HWND(hwnd as *mut _)).as_bool() }
}

/// Restore (if minimised), bring to top, and set foreground on a window.
pub fn bring_to_front(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let _ = ShowWindow(h, SW_RESTORE);
        let _ = SetWindowPos(
            h,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        let _ = SetWindowPos(
            h,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
    }
}

/// Strip extra rows from a borderless / non-16:9 window capture so that the
/// result matches the given `target_aspect` ratio. Returns the cropped BGR
/// pixel buffer together with the new width and height.
pub fn strip_frame(
    pixels: &[u8],
    width: u32,
    height: u32,
    target_aspect: f64,
) -> (Vec<u8>, u32, u32) {
    let current_aspect = width as f64 / height as f64;
    if current_aspect < target_aspect * 0.95 {
        return (pixels.to_vec(), width, height);
    }

    let expected_height = (width as f64 / target_aspect) as u32;
    if expected_height >= height {
        return (pixels.to_vec(), width, height);
    }
    let strip = (height - expected_height) / 2;
    let start = (strip * width * 3) as usize;
    let len = (expected_height * width * 3) as usize;
    (pixels[start..start + len].to_vec(), width, expected_height)
}
