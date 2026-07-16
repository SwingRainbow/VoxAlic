use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow,
    SetWindowPos, ShowWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE,
};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};

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
///
/// The search is two-tiered:
/// 1. Title match AND process-name match — prevents browser tabs with
///    "Warframe" in the title from being mistaken for the game.
/// 2. If (1) yields nothing, fall back to process-name-only search (the game
///    window may have a non-standard title).
pub fn resolve_hwnd(keyword: &str) -> isize {
    let keyword_lower = keyword.to_lowercase();
    // Tier 1: title match + process-name verification.
    for w in list_windows(keyword) {
        if exe_name_for_pid(w.pid)
            .map(|n| n.to_lowercase().contains(&keyword_lower))
            .unwrap_or(false)
        {
            return w.hwnd as isize;
        }
    }
    // Tier 2: process-name-only fallback.
    find_window_by_process(keyword)
}

/// Look up the executable name for a process id. Returns `None` if the process
/// is no longer alive or the snapshot fails.
fn exe_name_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID == pid {
                    let name = String::from_utf16_lossy(&entry.szExeFile);
                    let nul = name.find('\0').unwrap_or(name.len());
                    return Some(name[..nul].to_string());
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
    }
    None
}

/// Enumerate visible top-level windows and return the first one whose owning
/// process's executable name contains `keyword` (case-insensitive). Returns 0
/// if no match.
fn find_window_by_process(keyword: &str) -> isize {
    // (found_hwnd, own_pid, keyword_lower)
    let mut ctx: (isize, u32, String) = (0, std::process::id(), keyword.to_lowercase());
    unsafe {
        let _ = EnumWindows(
            Some(enum_callback_by_process),
            LPARAM(&mut ctx as *mut (isize, u32, String) as isize),
        );
    }
    ctx.0
}

unsafe extern "system" fn enum_callback_by_process(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx: &mut (isize, u32, String) = &mut *(lparam.0 as *mut (isize, u32, String));
    // Already found a match — stop enumerating.
    if ctx.0 != 0 { return BOOL::from(true); }
    if !IsWindowVisible(hwnd).as_bool() { return BOOL::from(true); }
    let len = GetWindowTextLengthW(hwnd);
    if len == 0 { return BOOL::from(true); }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == ctx.1 { return BOOL::from(true); }
    if let Some(name) = exe_name_for_pid(pid) {
        if name.to_lowercase().contains(&ctx.2) {
            ctx.0 = hwnd.0 as isize;
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
pub fn strip_frame<'a>(
    pixels: &'a [u8],
    width: u32,
    height: u32,
    target_aspect: f64,
) -> (&'a [u8], u32, u32) {
    let current_aspect = width as f64 / height as f64;
    if current_aspect < target_aspect * 0.95 {
        return (pixels, width, height);
    }

    let expected_height = (width as f64 / target_aspect) as u32;
    if expected_height >= height {
        return (pixels, width, height);
    }
    let strip = (height - expected_height) / 2;
    let bpp = (pixels.len() / (width * height) as usize).max(3);
    let start = (strip * width) as usize * bpp;
    let len = (expected_height * width) as usize * bpp;
    (&pixels[start..start + len], width, expected_height)
}
