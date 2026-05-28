use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow,
    IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub hwnd: usize, // isize as usize for serialization
}

/// Enumerate all top-level visible windows whose title contains `keyword`
/// (case-insensitive). Results are sorted so exact title matches come first,
/// then partial matches by alphabetical order.
pub fn list_windows(keyword: &str) -> Vec<WindowInfo> {
    let keyword_lower = keyword.to_lowercase();
    let mut windows: Vec<WindowInfo> = Vec::new();

    unsafe {
        let _ = EnumWindows(
            Some(enum_callback),
            LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
        );
    }

    // Filter by keyword
    windows.retain(|w| w.title.to_lowercase().contains(&keyword_lower));

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
                    windows.push(WindowInfo {
                        title,
                        hwnd: hwnd.0 as usize,
                    });
                }
            }
        }
    }
    BOOL::from(true)
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
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
    }
}

/// Strip extra rows from a borderless / non-16:9 window capture so that the
/// result matches the given `target_aspect` ratio. Returns the cropped BGR
/// pixel buffer together with the new width and height.
pub fn strip_frame(pixels: &[u8], width: u32, height: u32, target_aspect: f64) -> (Vec<u8>, u32, u32) {
    let expected_height = (width as f64 / target_aspect) as u32;
    if expected_height >= height {
        return (pixels.to_vec(), width, height);
    }
    let strip = (height - expected_height) / 2;
    let start = (strip * width * 3) as usize;
    let len = (expected_height * width * 3) as usize;
    (pixels[start..start + len].to_vec(), width, expected_height)
}
