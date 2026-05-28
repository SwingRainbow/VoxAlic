use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteDC,
    DeleteObject, GetDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, GetDC, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PW_RENDERFULLCONTENT,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
};

// PrintWindow is not exposed in the windows crate for GDI.
// Declare it directly from user32.dll.
// BOOL PrintWindow(HWND hwnd, HDC hdcBlt, UINT nFlags);
extern "system" {
    fn PrintWindow(
        hwnd: windows::Win32::Foundation::HWND,
        hdc: windows::Win32::Graphics::Gdi::HDC,
        nFlags: u32,
    ) -> i32;
}

#[derive(Debug, Clone)]
pub struct ROIConfig {
    pub x: f64, // relative 0.0-1.0
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Capture Warframe window, return BGR pixel bytes + dimensions of the ROI region.
/// Returns None if window not found or any GDI call fails.
pub fn capture_roi(roi: &ROIConfig) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        let class_name = to_utf16("Warframe");
        let hwnd = FindWindowW(PCWSTR::null(), PCWSTR(class_name.as_ptr())).ok()?;
        if hwnd.is_invalid() {
            return None;
        }

        // Get true window bounds via DWM (DPI-aware)
        let mut rect = windows::Win32::Foundation::RECT::default();
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<windows::Win32::Foundation::RECT>() as u32,
        );

        let win_w = (rect.right - rect.left).max(1);
        let win_h = (rect.bottom - rect.top).max(1);

        // Calculate ROI in absolute pixels
        let roi_x = (win_w as f64 * roi.x) as i32;
        let roi_y = (win_h as f64 * roi.y) as i32;
        let roi_w = (win_w as f64 * roi.w) as i32;
        let roi_h = (win_h as f64 * roi.h) as i32;

        if roi_w <= 0 || roi_h <= 0 {
            return None;
        }

        // Get window DC and create compatible DC + bitmap
        let hdc_window = GetDC(hwnd);
        if hdc_window.is_invalid() {
            return None;
        }

        let hdc_mem = CreateCompatibleDC(hdc_window);
        if hdc_mem.is_invalid() {
            let _ = ReleaseDC(hwnd, hdc_window);
            return None;
        }

        let hbitmap = CreateCompatibleBitmap(hdc_window, win_w, win_h);
        if hbitmap.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd, hdc_window);
            return None;
        }

        let old_bmp = SelectObject(hdc_mem, hbitmap);

        // PrintWindow with PW_RENDERFULLCONTENT (2), fallback without
        let pw_result = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT);
        if pw_result == 0 {
            let _ = PrintWindow(hwnd, hdc_mem, 0);
        }

        // Extract full window pixel data (BGRA, 4 bytes per pixel)
        let full_pixel_count = (win_w * win_h) as usize;
        let mut pixels: Vec<u8> = vec![0u8; full_pixel_count * 4];

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: win_w,
                biHeight: -win_h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };

        let result = GetDIBits(
            hdc_mem,
            hbitmap,
            0,
            win_h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Cleanup GDI
        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hbitmap);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(hwnd, hdc_window);

        if result == 0 {
            return None;
        }

        // Crop ROI from full window pixels (32-bit BGRA, row stride = win_w * 4)
        let mut roi_pixels = vec![0u8; (roi_w * roi_h * 3) as usize]; // BGR only
        for row in 0..roi_h {
            let src_start = ((roi_y + row) * win_w + roi_x) as usize * 4;
            let dst_start = (row * roi_w) as usize * 3;
            for col in 0..roi_w as usize {
                roi_pixels[dst_start + col * 3] = pixels[src_start + col * 4];         // B
                roi_pixels[dst_start + col * 3 + 1] = pixels[src_start + col * 4 + 1]; // G
                roi_pixels[dst_start + col * 3 + 2] = pixels[src_start + col * 4 + 2]; // R
            }
        }

        Some((roi_pixels, roi_w as u32, roi_h as u32))
    }
}

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
