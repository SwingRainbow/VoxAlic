use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteDC,
    DeleteObject, GetDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, GetDC, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, PW_RENDERFULLCONTENT,
};

use crate::window;
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

/// Threshold below which a BGR channel value counts as "black".
const BLACK_LEVEL: u8 = 8;

/// Return `true` when essentially the whole buffer is black — this happens when
/// `PrintWindow` captures a window that is occluded, mid-render, or not
/// composited yet. Such frames are useless for OCR and should be discarded.
/// A frame is considered black when at most 1% of its pixels are non-black.
fn is_black_frame(bgr: &[u8]) -> bool {
    if bgr.len() < 3 {
        return true;
    }
    let total = bgr.len() / 3;
    let mut non_black = 0usize;
    for px in bgr.chunks_exact(3) {
        if px[0] > BLACK_LEVEL || px[1] > BLACK_LEVEL || px[2] > BLACK_LEVEL {
            non_black += 1;
            // Early exit: > 1% non-black means it's a real frame.
            if non_black * 100 > total {
                return false;
            }
        }
    }
    true
}

/// Capture the full window client area as a top-down **BGR** buffer.
///
/// `hwnd_raw` is the raw window handle (`HWND.0 as isize`). Returns `None` if
/// the window is invalid/minimized, any GDI/DWM call fails, or the captured
/// frame is black (see [`is_black_frame`]).
pub fn capture_full(hwnd_raw: isize) -> Option<(Vec<u8>, u32, u32)> {
    let hwnd = windows::Win32::Foundation::HWND(hwnd_raw as *mut _);
    if !window::is_valid(hwnd_raw) || window::is_minimized(hwnd_raw) {
        return None;
    }

    // Get true window bounds via DWM (DPI-aware), fall back to GetWindowRect
    let mut rect = windows::Win32::Foundation::RECT::default();
    let dwm_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<windows::Win32::Foundation::RECT>() as u32,
        )
    };
    if dwm_result.is_err() {
        // DWM unavailable — fall back to classic GetWindowRect
        unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    }

    let win_w = (rect.right - rect.left).max(1);
    let win_h = (rect.bottom - rect.top).max(1);

    // Get window DC and create compatible DC + bitmap
    let hdc_window = unsafe { GetDC(hwnd) };
    if hdc_window.is_invalid() {
        return None;
    }

    let hdc_mem = unsafe { CreateCompatibleDC(hdc_window) };
    if hdc_mem.is_invalid() {
        unsafe { let _ = ReleaseDC(hwnd, hdc_window); }
        return None;
    }

    let hbitmap = unsafe { CreateCompatibleBitmap(hdc_window, win_w, win_h) };
    if hbitmap.is_invalid() {
        unsafe {
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd, hdc_window);
        }
        return None;
    }

    let old_bmp = unsafe { SelectObject(hdc_mem, hbitmap) };

    // PrintWindow with PW_RENDERFULLCONTENT (2), fallback without
    let pw_result = unsafe { PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT) };
    if pw_result == 0 {
        unsafe { let _ = PrintWindow(hwnd, hdc_mem, 0); }
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
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [Default::default(); 1],
    };

    let result = unsafe {
        GetDIBits(
            hdc_mem,
            hbitmap,
            0,
            win_h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };

    // Cleanup GDI
    unsafe {
        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hbitmap);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(hwnd, hdc_window);
    }

    if result == 0 {
        return None;
    }

    // Convert full window BGRA → BGR (drop alpha)
    let mut bgr = vec![0u8; full_pixel_count * 3];
    for i in 0..full_pixel_count {
        bgr[i * 3] = pixels[i * 4];         // B
        bgr[i * 3 + 1] = pixels[i * 4 + 1]; // G
        bgr[i * 3 + 2] = pixels[i * 4 + 2]; // R
    }

    if is_black_frame(&bgr) {
        return None;
    }

    Some((bgr, win_w as u32, win_h as u32))
}

/// Crop a fractional ROI out of a full BGR frame. Coordinates `x/y/w/h` are
/// fractions of `w`/`h` in 0.0–1.0. Returns `None` if the ROI is out of bounds.
fn crop_roi(frame: &[u8], w: u32, h: u32, roi: &ROIConfig) -> Option<(Vec<u8>, u32, u32)> {
    if roi.x + roi.w > 1.01 || roi.y + roi.h > 1.01 {
        return None;
    }
    let roi_x = (w as f64 * roi.x) as u32;
    let roi_y = (h as f64 * roi.y) as u32;
    let roi_w = (w as f64 * roi.w) as u32;
    let roi_h = (h as f64 * roi.h) as u32;
    if roi_w == 0 || roi_h == 0 || roi_x + roi_w > w || roi_y + roi_h > h {
        return None;
    }

    let row_bytes = (roi_w * 3) as usize;
    let mut out = vec![0u8; row_bytes * roi_h as usize];
    for row in 0..roi_h {
        let src = ((roi_y + row) * w + roi_x) as usize * 3;
        let dst = row as usize * row_bytes;
        out[dst..dst + row_bytes].copy_from_slice(&frame[src..src + row_bytes]);
    }
    Some((out, roi_w, roi_h))
}

/// Capture a fractional ROI from the window (no frame stripping).
/// Returned buffer is **BGR** (3 bytes per pixel).
pub fn capture_roi(hwnd: isize, roi: &ROIConfig) -> Option<(Vec<u8>, u32, u32)> {
    let (frame, w, h) = capture_full(hwnd)?;
    crop_roi(&frame, w, h, roi)
}

/// Capture a fractional ROI, optionally stripping non-16:9 letterbox borders
/// from the **full window** before cropping. Stripping the frame first is
/// essential: the ROI fractions are calibrated against the 16:9 game content,
/// not the raw window (which may carry borders or a title bar).
pub fn capture_roi_stripped(hwnd: isize, roi: &ROIConfig, strip_frame: bool) -> Option<(Vec<u8>, u32, u32)> {
    let (frame, w, h) = capture_full(hwnd)?;
    let (content, cw, ch) = if strip_frame {
        window::strip_frame(&frame, w, h, 16.0 / 9.0)
    } else {
        (frame, w, h)
    };
    crop_roi(&content, cw, ch, roi)
}

/// Capture the full window (optionally frame-stripped to match the OCR pipeline)
/// and return it as a `data:image/png;base64,...` URL string for the calibration
/// preview. The returned image is **the exact frame OCR crops from**, so ROI
/// fractions drawn on it map 1:1 to capture coordinates.
pub fn capture_preview_data_url(hwnd: isize, strip_frame: bool) -> Option<String> {
    let (frame, w, h) = capture_full(hwnd)?;
    let (content, cw, ch) = if strip_frame {
        window::strip_frame(&frame, w, h, 16.0 / 9.0)
    } else {
        (frame, w, h)
    };

    // BGR → RGB for the image crate.
    let px_count = (cw * ch) as usize;
    let mut rgb = vec![0u8; px_count * 3];
    for i in 0..px_count {
        rgb[i * 3] = content[i * 3 + 2];     // R
        rgb[i * 3 + 1] = content[i * 3 + 1]; // G
        rgb[i * 3 + 2] = content[i * 3];     // B
    }

    let img = image::RgbImage::from_raw(cw, ch, rgb)?;
    let mut png: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;

    Some(format!("data:image/png;base64,{}", base64_encode(&png)))
}

/// Minimal standard-base64 encoder (no padding omitted). Avoids pulling in a
/// dependency just for the one-shot calibration preview.
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
