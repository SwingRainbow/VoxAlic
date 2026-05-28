# Mission Timer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port Python OCR mission timer (screen capture + template matching + life support detection + 5min checkpoint) to Tauri v2.

**Architecture:** Capture Warframe window via Win32 PrintWindow in a dedicated std::thread. Match digit templates with hand-written normalized cross-correlation (no opencv). Timer state flows through Arc<RwLock> shared with the existing 1s tokio tick which pushes to frontend via Tauri events.

**Tech Stack:** Rust `windows` crate (Win32 GDI), `image` crate (PNG decode only), std::thread + std::sync, TypeScript frontend.

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src-tauri/Cargo.toml` | Modify | Add `windows`, `image` deps |
| `src-tauri/resources/digit_templates/*.png` | Create | 10 digit template images copied from Python project |
| `src-tauri/src/capture.rs` | Create | Win32 PrintWindow screen capture + ROI crop |
| `src-tauri/src/ocr.rs` | Create | Template matching, NMS, digit recognition |
| `src-tauri/src/mission_timer.rs` | Create | Timer state machine, OCR polling thread, life support |
| `src-tauri/src/models.rs` | Modify | Add MissionTimerPayload struct |
| `src-tauri/src/config.rs` | Modify | Add mission timer config fields |
| `src-tauri/src/lib.rs` | Modify | Wire up module, merge payload, add commands |
| `index.html` | Modify | Add mission timer tab + settings section |
| `src/styles.css` | Modify | Add mission timer tab styles |
| `src/main.ts` | Modify | Add mission timer rendering + config |

---

### Task 1: Dependencies and Assets

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/resources/digit_templates/0.png` through `9.png`

- [ ] **Step 1: Add `windows` and `image` crates to Cargo.toml**

Edit `src-tauri/Cargo.toml`, add under `[dependencies]`:

```toml
windows = { version = "0.58", features = [
    "Win32_Graphics_Gdi",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Dwm",
] }
image = { version = "0.25", default-features = false, features = ["png"] }
```

Full file after edit:

```toml
[package]
name = "tauri-warframe-monitor"
version = "0.1.0"
description = "Warframe Fissure Monitor"
authors = ["you"]
edition = "2021"

[lib]
name = "tauri_warframe_monitor_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-opener = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
windows = { version = "0.58", features = [
    "Win32_Graphics_Gdi",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Dwm",
] }
image = { version = "0.25", default-features = false, features = ["png"] }
```

- [ ] **Step 2: Copy digit template PNGs from Python project**

Run in PowerShell:

```powershell
New-Item -ItemType Directory -Force -Path "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\0.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\1.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\2.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\3.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\4.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\5.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\6.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\7.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\8.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
Copy-Item "C:\Users\TDD\Desktop\warframe_monitor\assets\digit_templates\9.png" -Destination "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\resources\digit_templates\"
```

- [ ] **Step 3: Verify cargo check with new deps**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check
```

Expected: new deps download and compile without errors (unused import warnings are OK).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/resources/
git commit -m "feat: add windows/image deps and digit templates for OCR"
```

---

### Task 2: Screen Capture Module

**Files:**
- Create: `src-tauri/src/capture.rs`

- [ ] **Step 1: Create capture.rs with PrintWindow capture function**

Write `src-tauri/src/capture.rs`:

```rust
use windows::core::{Result as WinResult, PCWSTR};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteDC,
    DeleteObject, BitBlt, GetDIBits, PrintWindow, SRCCOPY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HBITMAP, PW_RENDERFULLCONTENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowRect, GetDC, ReleaseDC, GetClientRect,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
};

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
        let hwnd = FindWindowW(PCWSTR::null(), Some(PCWSTR(class_name.as_ptr())));
        if hwnd.0 == 0 {
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
        if hdc_window.0 == 0 {
            return None;
        }

        let hdc_mem = CreateCompatibleDC(hdc_window);
        if hdc_mem.0 == 0 {
            let _ = ReleaseDC(hwnd, hdc_window);
            return None;
        }

        let hbitmap = CreateCompatibleBitmap(hdc_window, win_w, win_h);
        if hbitmap.0 == 0 {
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd, hdc_window);
            return None;
        }

        let old_bmp = SelectObject(hdc_mem, hbitmap);

        // PrintWindow with fallback to full content
        let pw_result = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT.0);
        if pw_result.is_err() {
            // Fallback: plain PrintWindow without PW_RENDERFULLCONTENT
            let _ = PrintWindow(hwnd, hdc_mem, 0);
        }

        // Extract pixel data from the ROI region
        let mut pixels = vec![0u8; (roi_w * roi_h * 4) as usize]; // 4 bytes per pixel (BGRA)

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
                roi_pixels[dst_start + col * 3] = pixels[src_start + col * 4];     // B
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
```

- [ ] **Step 2: Compile check capture.rs**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check
```

Expected: capture.rs compiles. Fix any API name issues by checking `windows` crate docs.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/capture.rs
git commit -m "feat: add Win32 PrintWindow screen capture module"
```

---

### Task 3: OCR Template Matching Module

**Files:**
- Create: `src-tauri/src/ocr.rs`

- [ ] **Step 1: Create ocr.rs with template loading and matching**

Write `src-tauri/src/ocr.rs`:

```rust
use image::GenericImageView;

pub struct DigitTemplate {
    pub digit: u8,
    pub pixels: Vec<f32>, // grayscale, normalized 0.0-1.0
    pub width: usize,
    pub height: usize,
}

pub struct DigitTemplates {
    pub templates: Vec<DigitTemplate>,
}

impl DigitTemplates {
    /// Load all 10 digit templates embedded at compile time.
    pub fn load() -> Self {
        // Embed all 10 template PNGs
        let pngs: [&[u8]; 10] = [
            include_bytes!("../resources/digit_templates/0.png"),
            include_bytes!("../resources/digit_templates/1.png"),
            include_bytes!("../resources/digit_templates/2.png"),
            include_bytes!("../resources/digit_templates/3.png"),
            include_bytes!("../resources/digit_templates/4.png"),
            include_bytes!("../resources/digit_templates/5.png"),
            include_bytes!("../resources/digit_templates/6.png"),
            include_bytes!("../resources/digit_templates/7.png"),
            include_bytes!("../resources/digit_templates/8.png"),
            include_bytes!("../resources/digit_templates/9.png"),
        ];

        let mut templates = Vec::new();
        for (digit, png_bytes) in pngs.iter().enumerate() {
            let img = image::load_from_memory(png_bytes)
                .expect("Failed to decode digit template")
                .into_luma8();
            let (w, h) = img.dimensions();
            let pixels: Vec<f32> = img.pixels()
                .map(|p| p.0[0] as f32 / 255.0)
                .collect();
            templates.push(DigitTemplate {
                digit: digit as u8,
                pixels,
                width: w as usize,
                height: h as usize,
            });
        }

        Self { templates }
    }
}

/// Run template matching and return recognized time string like "4:32" or "12:05".
/// Returns None if no confident match found.
pub fn recognize_digits(
    roi_pixels: &[u8],    // BGR
    roi_w: u32,
    roi_h: u32,
    templates: &DigitTemplates,
    match_threshold: f32,
) -> Option<String> {
    // Convert BGR to grayscale + threshold at 160
    let gray: Vec<f32> = roi_pixels
        .chunks(3)
        .map(|rgb| {
            let b = rgb[0] as f32;
            let g = rgb[1] as f32;
            let r = rgb[2] as f32;
            let gray_val = 0.299 * r + 0.587 * g + 0.114 * b;
            if gray_val > 160.0 { 1.0 } else { 0.0 }
        })
        .collect();

    let img_w = roi_w as usize;
    let img_h = roi_h as usize;

    let mut all_detections: Vec<(f32, usize, usize, u8)> = Vec::new();

    for tpl in &templates.templates {
        let dets = match_template(
            &gray, img_w, img_h,
            &tpl.pixels, tpl.width, tpl.height,
            match_threshold,
        );
        for (score, x, y) in dets {
            all_detections.push((score, x, y, tpl.digit));
        }
    }

    if all_detections.is_empty() {
        return None;
    }

    // NMS merge overlapping detections (IoU > 0.3)
    let kept = nms(&all_detections, templates.templates[0].width, templates.templates[0].height, 0.3);

    // Sort by x coordinate, join digits
    let mut sorted = kept.clone();
    sorted.sort_by(|a, b| a.1.cmp(&b.1));

    let digits: String = sorted.iter().map(|(_, _, _, d)| (d + b'0') as char).collect();

    // Parse expected format: "M:SS" or "MM:SS"
    let len = digits.len();
    if len < 3 {
        return None;
    }
    let minutes = &digits[..len - 2];
    let seconds = &digits[len - 2..];
    // Validate seconds < 60
    if let Ok(sec) = seconds.parse::<u32>() {
        if sec >= 60 {
            return None;
        }
    }
    Some(format!("{}:{}", minutes, seconds))
}

fn match_template(
    image: &[f32], img_w: usize, img_h: usize,
    template: &[f32], tpl_w: usize, tpl_h: usize,
    threshold: f32,
) -> Vec<(f32, usize, usize)> {
    let n = (tpl_w * tpl_h) as f32;
    let tpl_mean = template.iter().sum::<f32>() / n;
    let tpl_centered: Vec<f32> = template.iter().map(|v| v - tpl_mean).collect();
    let tpl_l2 = tpl_centered.iter().map(|v| v * v).sum::<f32>().sqrt();

    if tpl_l2 < 1e-6 {
        return Vec::new();
    }

    let mut results = Vec::new();
    let max_y = img_h.saturating_sub(tpl_h);
    let max_x = img_w.saturating_sub(tpl_w);

    for y in 0..max_y {
        for x in 0..max_x {
            let mut patch_mean = 0.0f32;
            for dy in 0..tpl_h {
                for dx in 0..tpl_w {
                    patch_mean += image[(y + dy) * img_w + (x + dx)];
                }
            }
            patch_mean /= n;

            let mut numerator = 0.0f32;
            let mut patch_sq = 0.0f32;
            for dy in 0..tpl_h {
                for dx in 0..tpl_w {
                    let p_centered = image[(y + dy) * img_w + (x + dx)] - patch_mean;
                    numerator += tpl_centered[dy * tpl_w + dx] * p_centered;
                    patch_sq += p_centered * p_centered;
                }
            }

            let denom = tpl_l2 * patch_sq.sqrt();
            let score = if denom > 1e-6 { numerator / denom } else { 0.0 };

            if score > threshold {
                results.push((score, x, y));
            }
        }
    }

    results
}

fn nms(
    detections: &[(f32, usize, usize, u8)],
    box_w: usize,
    box_h: usize,
    iou_thresh: f32,
) -> Vec<(f32, usize, usize, u8)> {
    let mut sorted = detections.to_vec();
    sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep: Vec<(f32, usize, usize, u8)> = Vec::new();
    for det in &sorted {
        let (score, x, y, digit) = *det;
        let mut overlap = false;
        for k in &keep {
            let x1 = x.max(k.1) as f32;
            let y1 = y.max(k.2) as f32;
            let x2 = (x + box_w).min(k.1 + box_w) as f32;
            let y2 = (y + box_h).min(k.2 + box_h) as f32;
            if x2 > x1 && y2 > y1 {
                let inter = (x2 - x1) * (y2 - y1);
                let union = 2.0 * (box_w * box_h) as f32 - inter;
                if inter / union > iou_thresh {
                    overlap = true;
                    break;
                }
            }
        }
        if !overlap {
            keep.push((score, x, y, digit));
        }
    }
    keep
}
```

- [ ] **Step 2: Compile check ocr.rs**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check
```

Expected: compiles (no capture.rs or lib.rs references yet — standalone module, OK if unused).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/ocr.rs
git commit -m "feat: add template matching OCR module"
```

---

### Task 4: Mission Timer State Machine

**Files:**
- Create: `src-tauri/src/mission_timer.rs`

- [ ] **Step 1: Create mission_timer.rs**

Write `src-tauri/src/mission_timer.rs`:

```rust
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use crate::capture::{self, ROIConfig};
use crate::config::{AppConfig, MissionTimerConfig};
use crate::ocr::{DigitTemplates, recognize_digits};

const OCR_INTERVAL_MS: u64 = 2000;
const MATCH_THRESHOLD: f32 = 0.55;
const MAX_REJECT: u32 = 3;
const CHECKPOINT_STALE_SECS: u64 = 30;
const DANGER_RED_PCT: f32 = 20.0;
const WARN_RED_PCT: f32 = 10.0;

#[derive(Debug, Clone)]
pub enum TimerCommand {
    Start,
    Stop,
    Reset,
    SetMode(String),
}

#[derive(Debug, Clone)]
pub struct MissionTimerPayload {
    pub elapsed_secs: u32,
    pub elapsed_str: String,
    pub state: String,          // "idle" | "running" | "paused" | "checkpoint"
    pub mode: String,           // "normal" | "fissure"
    pub life_support_pct: f32,
    pub life_support_level: String, // "normal" | "warning" | "danger"
    pub status_text: String,
}

impl Default for MissionTimerPayload {
    fn default() -> Self {
        Self {
            elapsed_secs: 0,
            elapsed_str: "0:00".into(),
            state: "idle".into(),
            mode: "normal".into(),
            life_support_pct: 100.0,
            life_support_level: "normal".into(),
            status_text: "未启动".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TimerState {
    Idle,
    Running,
    Paused,
    Checkpoint,
}

pub struct MissionTimerState {
    pub payload: MissionTimerPayload,
    // internal
    inner_state: TimerState,
    start_instant: Option<Instant>,
    paused_elapsed: Duration,
    last_valid_ocr: Option<String>,
    last_valid_instant: Option<Instant>,
    consecutive_rejects: u32,
    last_mode: String,
}

impl MissionTimerState {
    pub fn new() -> Self {
        Self {
            payload: MissionTimerPayload::default(),
            inner_state: TimerState::Idle,
            start_instant: None,
            paused_elapsed: Duration::ZERO,
            last_valid_ocr: None,
            last_valid_instant: None,
            consecutive_rejects: 0,
            last_mode: "normal".into(),
        }
    }

    fn handle_command(&mut self, cmd: &TimerCommand) {
        match cmd {
            TimerCommand::Start => {
                if self.inner_state == TimerState::Idle || self.inner_state == TimerState::Paused || self.inner_state == TimerState::Checkpoint {
                    self.inner_state = TimerState::Running;
                    if self.start_instant.is_none() {
                        self.start_instant = Some(Instant::now());
                        self.paused_elapsed = Duration::ZERO;
                    }
                    self.payload.status_text = "运行中".into();
                }
            }
            TimerCommand::Stop => {
                if self.inner_state == TimerState::Running || self.inner_state == TimerState::Checkpoint {
                    self.inner_state = TimerState::Paused;
                    if let Some(start) = self.start_instant {
                        self.paused_elapsed += start.elapsed();
                    }
                    self.start_instant = None;
                    self.payload.status_text = "已暂停".into();
                }
            }
            TimerCommand::Reset => {
                self.inner_state = TimerState::Idle;
                self.start_instant = None;
                self.paused_elapsed = Duration::ZERO;
                self.last_valid_ocr = None;
                self.last_valid_instant = None;
                self.consecutive_rejects = 0;
                self.payload = MissionTimerPayload::default();
            }
            TimerCommand::SetMode(mode) => {
                self.last_mode = mode.clone();
                self.payload.mode = mode.clone();
            }
        }
        self.payload.state = state_str(self.inner_state);
    }

    fn update_elapsed(&mut self) {
        let elapsed = if let Some(start) = self.start_instant {
            if self.inner_state == TimerState::Running {
                start.elapsed().as_secs() as u32
            } else {
                self.paused_elapsed.as_secs() as u32
            }
        } else {
            self.paused_elapsed.as_secs() as u32
        };

        self.payload.elapsed_secs = elapsed;
        let m = elapsed / 60;
        let s = elapsed % 60;
        self.payload.elapsed_str = format!("{}:{:02}", m, s);
    }

    fn apply_ocr(&mut self, ocr_result: Option<String>) {
        if self.inner_state != TimerState::Running {
            return;
        }

        if let Some(ref result) = ocr_result {
            // Parse OCR time to seconds
            let ocr_secs = parse_time_to_secs(result);
            let current_elapsed = self.payload.elapsed_secs;

            // Validate jump range: -10 to +30 seconds
            let valid = if let (Some(last_ocr), Some(last_inst)) = (&self.last_valid_ocr, self.last_valid_instant) {
                let last_secs = parse_time_to_secs(last_ocr);
                let wall_delta = last_inst.elapsed().as_secs() as i64;
                let ocr_delta = ocr_secs as i64 - last_secs as i64;
                (ocr_delta - wall_delta).abs() <= 30
            } else {
                // First reading: accept if close-ish to wall time
                let wall = self.payload.elapsed_secs;
                (ocr_secs as i64 - wall as i64).abs() <= 60
            };

            if valid {
                self.consecutive_rejects = 0;
                // Check if OCR unchanged for checkpoint detection
                if let (Some(last_ocr), Some(last_inst)) = (&self.last_valid_ocr, self.last_valid_instant) {
                    if result == last_ocr && last_inst.elapsed().as_secs() > CHECKPOINT_STALE_SECS {
                        self.inner_state = TimerState::Checkpoint;
                        self.payload.state = "checkpoint".into();
                        self.payload.status_text = "5分钟截点 — 请切回游戏".into();
                        return;
                    }
                }
                self.last_valid_ocr = Some(result.clone());
                self.last_valid_instant = Some(Instant::now());
            } else {
                self.consecutive_rejects += 1;
                if self.consecutive_rejects >= MAX_REJECT {
                    // Too many rejections — possible game state change, reset
                    self.inner_state = TimerState::Idle;
                    self.payload = MissionTimerPayload::default();
                }
            }
        } else {
            // OCR failed — keep internal timer, do nothing
        }
    }
}

fn state_str(s: TimerState) -> String {
    match s {
        TimerState::Idle => "idle".into(),
        TimerState::Running => "running".into(),
        TimerState::Paused => "paused".into(),
        TimerState::Checkpoint => "checkpoint".into(),
    }
}

fn parse_time_to_secs(s: &str) -> u32 {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 {
        let m: u32 = parts[0].parse().unwrap_or(0);
        let sec: u32 = parts[1].parse().unwrap_or(0);
        m * 60 + sec
    } else {
        0
    }
}

/// Detect life support percentage from a separate ROI (HSV red detection).
pub fn detect_life_support(pixels: &[u8], width: u32, height: u32) -> f32 {
    let total = (width * height) as usize;
    if total == 0 {
        return 100.0;
    }

    let mut red_pixels = 0usize;
    for chunk in pixels.chunks(3) {
        let b = chunk[0] as f32 / 255.0;
        let g = chunk[1] as f32 / 255.0;
        let r = chunk[2] as f32 / 255.0;

        // BGR to HSV approximation
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let h = if delta < 1e-4 {
            0.0
        } else if (max - r).abs() < 1e-4 {
            60.0 * ((g - b) / delta % 6.0)
        } else if (max - g).abs() < 1e-4 {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };

        let s = if max < 1e-4 { 0.0 } else { delta / max };
        let v = max;

        // Red hue ranges
        let is_red = (h >= 0.0 && h <= 10.0) || (h >= 160.0 && h <= 180.0);
        if is_red && s > 0.4 && v > 0.3 {
            red_pixels += 1;
        }
    }

    // Red% in ROI: high red = life support bar is red = danger (<20%)
    pct.min(100.0)
}

/// Spawn the OCR polling thread. Returns command sender.
pub fn start_timer_thread(
    shared: Arc<std::sync::RwLock<MissionTimerState>>,
    config: Arc<std::sync::RwLock<AppConfig>>,
    templates: &'static DigitTemplates,
) -> mpsc::Sender<TimerCommand> {
    let (tx, rx) = mpsc::channel::<TimerCommand>();

    std::thread::spawn(move || {
        loop {
            // Process any pending commands (non-blocking)
            while let Ok(cmd) = rx.try_recv() {
                let mut state = shared.write().unwrap();
                state.handle_command(&cmd);
            }

            // OCR capture cycle
            {
                let state = shared.read().unwrap();
                if state.inner_state == TimerState::Running {
                    let mode = state.payload.mode.clone();
                    drop(state);

                    let cfg = config.read().unwrap();
                    let roi = if mode == "fissure" {
                        &cfg.mission_timer.fissure_roi
                    } else {
                        &cfg.mission_timer.normal_roi
                    };

                    let roi_config = ROIConfig {
                        x: roi.x,
                        y: roi.y,
                        w: roi.w,
                        h: roi.h,
                    };

                    // Capture main timer ROI
                    if let Some((pixels, w, h)) = capture::capture_roi(&roi_config) {
                        let ocr_result = recognize_digits(&pixels, w, h, templates, MATCH_THRESHOLD);
                        let mut state = shared.write().unwrap();
                        state.apply_ocr(ocr_result);
                    }

                    // Capture life support ROI
                    if let Some((ls_pixels, ls_w, ls_h)) = capture::capture_roi(&ROIConfig {
                        x: cfg.mission_timer.life_support_roi.x,
                        y: cfg.mission_timer.life_support_roi.y,
                        w: cfg.mission_timer.life_support_roi.w,
                        h: cfg.mission_timer.life_support_roi.h,
                    }) {
                        let ls_pct = detect_life_support(&ls_pixels, ls_w, ls_h);
                        let mut state = shared.write().unwrap();
                        state.payload.life_support_pct = ls_pct;
                        state.payload.life_support_level = if ls_pct > DANGER_RED_PCT {
                            "danger".into()
                        } else if ls_pct > WARN_RED_PCT {
                            "warning".into()
                        } else {
                            "normal".into()
                        };
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(OCR_INTERVAL_MS));
        }
    });

    tx
}
```

- [ ] **Step 2: Compile check**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check
```

Expected: may fail on missing `MissionTimerConfig` in config.rs — expected, will add in Task 6. If other errors, fix then proceed.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/mission_timer.rs
git commit -m "feat: add mission timer state machine and OCR polling thread"
```

---

### Task 5: Extend Models with Timer Payload

**Files:**
- Modify: `src-tauri/src/models.rs`

- [ ] **Step 1: Add MissionTimerPayload to models.rs**

Edit `src-tauri/src/models.rs`, add after line 27 (after CycleInfo struct):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTimerPayload {
    pub elapsed_secs: u32,
    pub elapsed_str: String,
    pub state: String,
    pub mode: String,
    pub life_support_pct: f32,
    pub life_support_level: String,
    pub status_text: String,
}
```

Then add the field to `AppStatePayload` (after the `cycles` field):

```rust
pub struct AppStatePayload {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
    pub mission_timer: MissionTimerPayload,
}
```

Full file after edits:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fissure {
    pub node_key: String,
    pub node_name: String,
    pub planet: String,
    pub mission_type: String,
    pub tier_key: String,
    pub tier_label: String,
    pub expiry_ms: i64,
    pub is_hard: bool,
    pub is_storm: bool,
    pub remain_ms: i64,
    pub remain_str: String,
    pub is_expiring: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleInfo {
    pub name: String,
    pub state: String,
    pub state_icon: String,
    pub remain_ms: i64,
    pub is_day: bool,
    pub remain_str: String,
    pub expiry_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTimerPayload {
    pub elapsed_secs: u32,
    pub elapsed_str: String,
    pub state: String,
    pub mode: String,
    pub life_support_pct: f32,
    pub life_support_level: String,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatePayload {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
    pub mission_timer: MissionTimerPayload,
}
```

- [ ] **Step 2: Compile check — expect errors in lib.rs due to new field**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check 2>&1 | Select-Object -First 20
```

Expected: errors about missing `mission_timer` field in `AppStatePayload` construction in `lib.rs`. This is expected — will fix in Task 7.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/models.rs
git commit -m "feat: add MissionTimerPayload to data models"
```

---

### Task 6: Extend Config with Timer Settings

**Files:**
- Modify: `src-tauri/src/config.rs`

- [ ] **Step 1: Add MissionTimerConfig and ROISettings to config.rs**

Edit `src-tauri/src/config.rs`, replace entire content:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROISettings {
    #[serde(default = "default_roi_x")]
    pub x: f64,
    #[serde(default = "default_roi_y")]
    pub y: f64,
    #[serde(default = "default_roi_w")]
    pub w: f64,
    #[serde(default = "default_roi_h")]
    pub h: f64,
}

fn default_roi_x() -> f64 { 0.005 }
fn default_roi_y() -> f64 { 0.395 }
fn default_roi_w() -> f64 { 0.06 }
fn default_roi_h() -> f64 { 0.025 }

fn fissure_roi_y() -> f64 { 0.405 }
fn life_support_y() -> f64 { 0.43 }
fn life_support_w() -> f64 { 0.15 }
fn life_support_h() -> f64 { 0.03 }

impl Default for ROISettings {
    fn default() -> Self {
        Self {
            x: default_roi_x(),
            y: default_roi_y(),
            w: default_roi_w(),
            h: default_roi_h(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTimerConfig {
    #[serde(default = "default_timer_mode")]
    pub mode: String,
    #[serde(default)]
    pub normal_roi: ROISettings,
    #[serde(default = "default_fissure_roi")]
    pub fissure_roi: ROISettings,
    #[serde(default = "default_life_support_roi")]
    pub life_support_roi: ROISettings,
}

fn default_timer_mode() -> String { "normal".into() }
fn default_fissure_roi() -> ROISettings {
    ROISettings { y: fissure_roi_y(), ..Default::default() }
}
fn default_life_support_roi() -> ROISettings {
    ROISettings { y: life_support_y(), w: life_support_w(), h: life_support_h(), ..Default::default() }
}

impl Default for MissionTimerConfig {
    fn default() -> Self {
        Self {
            mode: "normal".into(),
            normal_roi: ROISettings::default(),
            fissure_roi: default_fissure_roi(),
            life_support_roi: default_life_support_roi(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub mission_timer: MissionTimerConfig,
}

fn default_close_to_tray() -> bool { true }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            mission_timer: MissionTimerConfig::default(),
        }
    }
}

pub fn config_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("config.json")
}

pub fn load_config(app_data_dir: &PathBuf) -> AppConfig {
    let path = config_path(app_data_dir);
    if path.exists() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&json) {
                return cfg;
            }
        }
    }
    let cfg = AppConfig::default();
    let _ = std::fs::create_dir_all(app_data_dir);
    if let Ok(json) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(&path, json);
    }
    cfg
}

pub fn save_config(app_data_dir: &PathBuf, config: &AppConfig) -> Result<(), String> {
    let _ = std::fs::create_dir_all(app_data_dir);
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(app_data_dir), json).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Compile check**

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri"; cargo check
```

Expected: mission_timer.rs should now compile. lib.rs will still have errors from missing `mission_timer` field (fix in next task).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "feat: add MissionTimerConfig with ROI settings to AppConfig"
```

---

### Task 7: Wire Everything in lib.rs

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Rewrite lib.rs to integrate all modules**

Write `src-tauri/src/lib.rs` (full rewrite):

```rust
mod api;
mod capture;
mod config;
mod mission_timer;
mod models;
mod ocr;
mod state;

use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::sync::mpsc;
use std::time::Duration;
use state::{AppState, SharedState};
use models::{AppStatePayload, MissionTimerPayload};
use config::{AppConfig, load_config, save_config};
use mission_timer::{MissionTimerState, TimerCommand, start_timer_thread};
use api::{fetch_worldstate, parse_fissures, parse_cycles, fmt_remain, now_ms};
use ocr::DigitTemplates;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Emitter,
};

const REFRESH_SEC: u32 = 1800;

type SharedConfig = Arc<StdRwLock<AppConfig>>;
type MissionTimerShared = Arc<StdRwLock<MissionTimerState>>;

fn build_payload(state: &AppState, timer_state: &MissionTimerState) -> AppStatePayload {
    let mut normal = state.normal_fissures.clone();
    let mut hard = state.hard_fissures.clone();
    let mut storms = state.storm_fissures.clone();
    let mut cycles = state.cycles.clone();

    let now = now_ms();
    for f in normal.iter_mut().chain(hard.iter_mut()).chain(storms.iter_mut()) {
        let remain = f.expiry_ms - now;
        f.remain_ms = remain;
        f.remain_str = fmt_remain(remain);
        f.is_expiring = remain > 0 && remain < 300_000;
    }
    for c in &mut cycles {
        let remain = c.expiry_ms - now;
        c.remain_ms = remain;
        c.remain_str = fmt_remain(remain);
    }

    AppStatePayload {
        normal_fissures: normal,
        hard_fissures: hard,
        storm_fissures: storms,
        cycles,
        last_update: state.last_update.clone(),
        countdown_secs: state.countdown_secs,
        mission_timer: timer_state.payload.clone(),
    }
}

#[tauri::command]
async fn refresh_now(state: tauri::State<'_, SharedState>, timer: tauri::State<'_, MissionTimerShared>, app: tauri::AppHandle) -> Result<(), String> {
    let data = fetch_worldstate().await?;
    let (normal, hard, storms) = parse_fissures(&data);
    let cycles = parse_cycles(&data);
    let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
    {
        let mut s = state.write().await;
        s.normal_fissures = normal;
        s.hard_fissures = hard;
        s.storm_fissures = storms;
        s.cycles = cycles;
        s.last_update = now_str;
        s.countdown_secs = REFRESH_SEC;
    }
    let payload = {
        let s = state.read().await;
        let t = timer.read().unwrap();
        build_payload(&s, &t)
    };
    let _ = app.emit("worldstate-update", payload);
    Ok(())
}

#[tauri::command]
fn get_config(cfg: tauri::State<'_, SharedConfig>) -> AppConfig {
    cfg.read().unwrap().clone()
}

#[tauri::command]
fn set_config(cfg: tauri::State<'_, SharedConfig>, config: AppConfig, app: tauri::AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    save_config(&app_data_dir, &config)?;
    *cfg.write().unwrap() = config;
    Ok(())
}

#[tauri::command]
fn timer_command(
    cmd_tx: tauri::State<'_, mpsc::Sender<TimerCommand>>,
    command: String,
    mode: Option<String>,
) -> Result<(), String> {
    let cmd = match command.as_str() {
        "start" => TimerCommand::Start,
        "stop" => TimerCommand::Stop,
        "reset" => TimerCommand::Reset,
        "set_mode" => TimerCommand::SetMode(mode.unwrap_or_else(|| "normal".into())),
        _ => return Err(format!("Unknown command: {}", command)),
    };
    cmd_tx.send(cmd).map_err(|e| e.to_string())
}

// Leak digit templates so they live for 'static lifetime
static mut TEMPLATES: Option<DigitTemplates> = None;

fn get_templates() -> &'static DigitTemplates {
    unsafe {
        if TEMPLATES.is_none() {
            TEMPLATES = Some(DigitTemplates::load());
        }
        TEMPLATES.as_ref().unwrap()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let templates: &'static DigitTemplates = get_templates();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let state: SharedState = SharedState::default();

            // Load config
            let app_data_dir = app.path().app_data_dir().expect("app data dir");
            let config: SharedConfig = Arc::new(StdRwLock::new(load_config(&app_data_dir)));

            // Mission timer shared state + command channel
            let timer_state: MissionTimerShared = Arc::new(StdRwLock::new(MissionTimerState::new()));
            let timer_config = config.clone();
            let timer_shared = timer_state.clone();
            let cmd_tx = start_timer_thread(timer_shared, timer_config, templates);

            // Background fetch loop (every REFRESH_SEC)
            let fetch_state = state.clone();
            let fetch_handle = app.handle().clone();
            let fetch_timer = timer_state.clone();
            tauri::async_runtime::spawn(async move {
                {
                    if let Ok(data) = fetch_worldstate().await {
                        let (normal, hard, storms) = parse_fissures(&data);
                        let cycles = parse_cycles(&data);
                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                        {
                            let mut s = fetch_state.write().await;
                            s.normal_fissures = normal;
                            s.hard_fissures = hard;
                            s.storm_fissures = storms;
                            s.cycles = cycles;
                            s.last_update = now_str;
                            s.countdown_secs = REFRESH_SEC;
                        }
                    }
                    let payload = {
                        let s = fetch_state.read().await;
                        let t = fetch_timer.read().unwrap();
                        build_payload(&s, &t)
                    };
                    let _ = fetch_handle.emit("worldstate-update", payload);
                }
                let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_SEC as u64));
                loop {
                    interval.tick().await;
                    if let Ok(data) = fetch_worldstate().await {
                        let (normal, hard, storms) = parse_fissures(&data);
                        let cycles = parse_cycles(&data);
                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                        {
                            let mut s = fetch_state.write().await;
                            s.normal_fissures = normal;
                            s.hard_fissures = hard;
                            s.storm_fissures = storms;
                            s.cycles = cycles;
                            s.last_update = now_str;
                            s.countdown_secs = REFRESH_SEC;
                        }
                    }
                    let payload = {
                        let s = fetch_state.read().await;
                        let t = fetch_timer.read().unwrap();
                        build_payload(&s, &t)
                    };
                    let _ = fetch_handle.emit("worldstate-update", payload);
                }
            });

            // Per-second tick (now also reads timer state)
            let tick_state = state.clone();
            let tick_handle = app.handle().clone();
            let tick_timer = timer_state.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    let payload = {
                        let mut s = tick_state.write().await;
                        s.countdown_secs = s.countdown_secs.saturating_sub(1);
                        let mut t = tick_timer.write().unwrap();
                        t.update_elapsed();
                        build_payload(&s, &t)
                    };
                    let _ = tick_handle.emit("tick-update", payload);
                }
            });

            // System tray
            let show_item = MenuItemBuilder::with_id("show", "显示").build(app)?;
            let refresh_item = MenuItemBuilder::with_id("refresh", "立即刷新").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&refresh_item)
                .item(&quit_item)
                .build()?;

            let tray_state = state.clone();
            let tray_timer = timer_state.clone();
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "refresh" => {
                        let state = tray_state.clone();
                        let timer = tray_timer.clone();
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Ok(data) = fetch_worldstate().await {
                                let (normal, hard, storms) = parse_fissures(&data);
                                let cycles = parse_cycles(&data);
                                let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                                {
                                    let mut s = state.write().await;
                                    s.normal_fissures = normal;
                                    s.hard_fissures = hard;
                                    s.storm_fissures = storms;
                                    s.cycles = cycles;
                                    s.last_update = now_str;
                                    s.countdown_secs = REFRESH_SEC;
                                }
                                let payload = {
                                    let s = state.read().await;
                                    let t = timer.read().unwrap();
                                    build_payload(&s, &t)
                                };
                                let _ = handle.emit("worldstate-update", payload);
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up, ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Window close -> hide to tray or exit based on config
            let close_config = config.clone();
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let cfg = close_config.read().unwrap();
                        if cfg.close_to_tray {
                            api.prevent_close();
                            let _ = window_clone.hide();
                        }
                    }
                });
            }

            app.manage(state);
            app.manage(config);
            app.manage(timer_state);
            app.manage(cmd_tx);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![refresh_now, get_config, set_config, timer_command])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 2: Build check**

```powershell
Stop-Process -Name "tauri-warframe-monitor" -Force -ErrorAction SilentlyContinue
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor"; npm run tauri build 2>&1 | Select-Object -Last 30
```

Expected: build succeeds. If windows crate API names need adjustment, fix and re-build.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: integrate mission timer into lib.rs — thread, tick, commands"
```

---

### Task 8: Frontend — HTML Mission Timer Tab

**Files:**
- Modify: `index.html`

- [ ] **Step 1: Add mission timer tab button and content**

Edit `index.html`, add the timer tab button in the `.tabs` div (after settings button):

```html
<button class="tab-btn" data-tab="timer">任务计时</button>
```

Add timer tab content before `</body>` (after tab-settings div):

```html
<div id="tab-timer" class="tab-content">
  <div class="timer-container">
    <div class="timer-display">
      <div class="timer-digits" id="timer-digits">0:00</div>
      <div class="timer-status" id="timer-status">未启动</div>
    </div>

    <div class="timer-controls">
      <label class="timer-mode-label">模式:</label>
      <label class="timer-radio">
        <input type="radio" name="timer-mode" value="normal" checked />
        <span>普通</span>
      </label>
      <label class="timer-radio">
        <input type="radio" name="timer-mode" value="fissure" />
        <span>裂缝</span>
      </label>
    </div>

    <div class="timer-buttons">
      <button class="timer-btn start" id="btn-timer-start">开始</button>
      <button class="timer-btn stop" id="btn-timer-stop">暂停</button>
      <button class="timer-btn reset" id="btn-timer-reset">重置</button>
    </div>

    <div class="life-support-bar">
      <div class="ls-label">维生系统</div>
      <div class="ls-bar-track">
        <div class="ls-bar-fill" id="ls-bar-fill" style="width:100%"></div>
      </div>
      <div class="ls-pct" id="ls-pct">--</div>
    </div>
  </div>
</div>
```

Full `.tabs` div after edit:

```html
<div class="tabs">
  <button class="tab-btn active" data-tab="cycles">世界时间</button>
  <button class="tab-btn" data-tab="fissures">虚空裂缝</button>
  <button class="tab-btn" data-tab="timer">任务计时</button>
  <button class="tab-btn" data-tab="settings">设置</button>
</div>
```

- [ ] **Step 2: Verify HTML structure visually**

Build and run the app, check the tab appears:

```powershell
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor"; npm run tauri dev
```

Open the app, click "任务计时" tab — should see placeholder content (no styles yet).

- [ ] **Step 3: Commit**

```bash
git add index.html
git commit -m "feat: add mission timer tab to HTML"
```

---

### Task 9: Frontend — CSS Timer Styles

**Files:**
- Modify: `src/styles.css`

- [ ] **Step 1: Append timer styles to styles.css**

Append to `src/styles.css`:

```css
/* ── Mission Timer ── */
.timer-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding: 20px 0;
}

.timer-display {
  text-align: center;
}

.timer-digits {
  font-size: 64px;
  font-weight: bold;
  font-family: 'Consolas', 'Courier New', monospace;
  color: #e0e0e0;
  letter-spacing: 4px;
}

.timer-status {
  font-size: 14px;
  color: var(--dim);
  margin-top: 4px;
}

.timer-controls {
  display: flex;
  align-items: center;
  gap: 12px;
}

.timer-mode-label {
  font-size: 13px;
  color: var(--dim);
}

.timer-radio {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 13px;
  color: var(--text);
  cursor: pointer;
}

.timer-buttons {
  display: flex;
  gap: 10px;
}

.timer-btn {
  padding: 8px 24px;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  font-size: 14px;
  color: #fff;
  transition: background 0.15s;
}
.timer-btn.start { background: #388E3C; }
.timer-btn.start:hover { background: #43A047; }
.timer-btn.stop { background: #F57C00; }
.timer-btn.stop:hover { background: #FB8C00; }
.timer-btn.reset { background: #555; }
.timer-btn.reset:hover { background: #666; }

/* Life Support */
.life-support-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  max-width: 360px;
}

.ls-label {
  font-size: 12px;
  color: var(--dim);
  min-width: 48px;
}

.ls-bar-track {
  flex: 1;
  height: 14px;
  background: #333;
  border-radius: 7px;
  overflow: hidden;
}

.ls-bar-fill {
  height: 100%;
  border-radius: 7px;
  transition: width 1s ease, background 0.5s;
}
.ls-bar-fill.normal { background: #4CAF50; }
.ls-bar-fill.warning { background: #FF9800; }
.ls-bar-fill.danger { background: #F44336; }

.ls-pct {
  font-size: 14px;
  font-weight: bold;
  color: var(--text);
  min-width: 36px;
  text-align: right;
}

/* Timer state: checkpoint pulse */
#timer-status.checkpoint {
  color: var(--warn);
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/styles.css
git commit -m "feat: add mission timer CSS styles"
```

---

### Task 10: Frontend — TypeScript Timer Logic

**Files:**
- Modify: `src/main.ts`

- [ ] **Step 1: Add mission timer rendering and event handling**

Edit `src/main.ts`, add the `MissionTimerPayload` interface after `AppStatePayload`:

```typescript
interface MissionTimerPayload {
  elapsed_secs: number;
  elapsed_str: string;
  state: string;
  mode: string;
  life_support_pct: number;
  life_support_level: string;
  status_text: string;
}
```

Add to `AppStatePayload` interface:

```typescript
interface AppStatePayload {
  normal_fissures: Fissure[];
  hard_fissures: Fissure[];
  storm_fissures: Fissure[];
  cycles: CycleInfo[];
  last_update: string;
  countdown_secs: number;
  mission_timer: MissionTimerPayload;
}
```

Add render function after `handleUpdate`:

```typescript
function renderTimer(t: MissionTimerPayload) {
  document.getElementById('timer-digits')!.textContent = t.elapsed_str;
  const statusEl = document.getElementById('timer-status')!;
  statusEl.textContent = t.status_text;
  statusEl.className = 'timer-status';
  if (t.state === 'checkpoint') {
    statusEl.classList.add('checkpoint');
  }

  // Life support
  const lsBar = document.getElementById('ls-bar-fill')!;
  lsBar.style.width = `${t.life_support_pct}%`;
  lsBar.className = 'ls-bar-fill ' + t.life_support_level;
  document.getElementById('ls-pct')!.textContent =
    t.life_support_pct > 0 ? `${t.life_support_pct.toFixed(0)}%` : '--';

  // Update mode radio to match state
  const modeRadios = document.getElementsByName('timer-mode') as NodeListOf<HTMLInputElement>;
  modeRadios.forEach(r => { r.checked = r.value === t.mode; });
}
```

Update `handleUpdate` to also render timer:

```typescript
function handleUpdate(payload: AppStatePayload) {
  currentData = payload;
  document.getElementById('status-text')!.textContent =
    `更新于 ${payload.last_update}  下次刷新 ${payload.countdown_secs}s`;
  renderCycles(payload.cycles);
  updateFilters();
  renderFissures();
  renderTimer(payload.mission_timer);
}
```

Add timer UI event listeners inside `DOMContentLoaded` (after the settings block):

```typescript
// Timer: start/stop/reset buttons
document.getElementById('btn-timer-start')!.addEventListener('click', () => {
  invoke('timer_command', { command: 'start' });
});
document.getElementById('btn-timer-stop')!.addEventListener('click', () => {
  invoke('timer_command', { command: 'stop' });
});
document.getElementById('btn-timer-reset')!.addEventListener('click', () => {
  invoke('timer_command', { command: 'reset' });
});

// Timer: mode radio
document.querySelectorAll('input[name="timer-mode"]').forEach(radio => {
  radio.addEventListener('change', () => {
    if ((radio as HTMLInputElement).checked) {
      invoke('timer_command', { command: 'set_mode', mode: (radio as HTMLInputElement).value });
    }
  });
});
```

- [ ] **Step 2: Build and test**

```powershell
Stop-Process -Name "tauri-warframe-monitor" -Force -ErrorAction SilentlyContinue
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor"; npm run tauri build 2>&1 | Select-Object -Last 20
```

Expected: build succeeds.

- [ ] **Step 3: Commit**

```bash
git add src/main.ts
git commit -m "feat: add mission timer frontend rendering and controls"
```

---

### Task 11: Final Integration Build

**Files:**
- None new

- [ ] **Step 1: Full release build**

```powershell
Stop-Process -Name "tauri-warframe-monitor" -Force -ErrorAction SilentlyContinue
cd "C:\Users\TDD\Desktop\tauri-warframe-monitor"; npm run tauri build
```

- [ ] **Step 2: Verify exe exists and size**

```powershell
Get-Item "C:\Users\TDD\Desktop\tauri-warframe-monitor\src-tauri\target\release\tauri-warframe-monitor.exe" | Select-Object Name, Length
```

- [ ] **Step 3: Run the app and smoke test**

Launch the exe, verify:
- 任务计时 tab renders with "0:00" and "未启动"
- 普通/裂缝 radio buttons work
- 开始/暂停/重置 buttons send commands (no visible error)
- 世界时间 and 裂缝 tabs still work
- 设置 tab still works

- [ ] **Step 4: Commit and tag**

```bash
git add -A
git commit -m "feat: mission timer with OCR — v0.3.0"
git tag -a v0.3.0 -m "v0.3.0: mission timer with OCR screen capture"
```
