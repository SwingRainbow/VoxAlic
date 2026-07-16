use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteDC,
    DeleteObject, GetDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, GetDC, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, PW_RENDERFULLCONTENT,
};

use base64::Engine;
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
/// `bpp` is bytes-per-pixel (4 for BGRA, 3 for BGR).
fn is_black_frame(buf: &[u8], bpp: usize) -> bool {
    let total = buf.len() / bpp;
    let mut non_black = 0usize;
    for px in buf.chunks_exact(bpp) {
        if px[0] > BLACK_LEVEL || px[1] > BLACK_LEVEL || px[2] > BLACK_LEVEL {
            non_black += 1;
            if non_black * 100 > total {
                return false;
            }
        }
    }
    true
}

/// Capture the full window client area as a top-down **BGRA** buffer.
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

    // Keep BGRA (4 bytes/pixel) — conversion to BGR is deferred to ROI
    // extraction (crop_roi_bgra), avoiding a full-frame copy here.
    if is_black_frame(&pixels, 4) {
        return None;
    }

    Some((pixels, win_w as u32, win_h as u32))
}

/// Crop a fractional ROI out of a full-frame BGRA buffer, producing a BGR
/// ROI (3 bytes/pixel). Coordinates `x/y/w/h` are fractions of `w`/`h` in
/// 0.0–1.0. Returns `None` if the ROI is out of bounds.
fn crop_roi(frame: &[u8], w: u32, h: u32, roi: &ROIConfig) -> Option<(Vec<u8>, u32, u32)> {
    crop_roi_bgra(frame, w, h, roi, 4)
}

/// Generic ROI crop: reads `src_bpp`-byte pixels from `frame`, writes 3-byte
/// BGR pixels to output. For BGRA source (bpp=4), alpha is dropped.
fn crop_roi_bgra(frame: &[u8], w: u32, h: u32, roi: &ROIConfig, src_bpp: u32) -> Option<(Vec<u8>, u32, u32)> {
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
    let src_bpp = src_bpp as usize;
    for row in 0..roi_h {
        let src_row = ((roi_y + row) * w + roi_x) as usize * src_bpp;
        let dst_row = row as usize * row_bytes;
        for col in 0..roi_w {
            let src = src_row + col as usize * src_bpp;
            let dst = dst_row + col as usize * 3;
            out[dst] = frame[src];         // B
            out[dst + 1] = frame[src + 1]; // G
            out[dst + 2] = frame[src + 2]; // R
        }
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
        (&*frame, w, h)
    };
    crop_roi(content, cw, ch, roi)
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
        (&*frame, w, h)
    };

    // BGRA → RGB for the image crate.
    let px_count = (cw * ch) as usize;
    let mut rgb = vec![0u8; px_count * 3];
    let src_bpp = 4;
    for i in 0..px_count {
        rgb[i * 3] = content[i * src_bpp + 2];     // R
        rgb[i * 3 + 1] = content[i * src_bpp + 1]; // G
        rgb[i * 3 + 2] = content[i * src_bpp];     // B
    }

    let img = image::RgbImage::from_raw(cw, ch, rgb)?;
    let mut png: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;

    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)))
}
