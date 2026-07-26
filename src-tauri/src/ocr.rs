use crate::cnn::Cnn;
use image::ImageEncoder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

            // Warframe HUD scaling changes the absolute digit size even when
            // ROI fractions line up. Keep the original templates for the
            // 2304x1440 samples and add a smaller scale for 1728x1080/HUD 140.
            push_template(&mut templates, digit as u8, &img);
            let scaled = image::imageops::resize(
                &img,
                ((img.width() as f32) * 0.85).round().max(1.0) as u32,
                ((img.height() as f32) * 0.85).round().max(1.0) as u32,
                image::imageops::FilterType::Triangle,
            );
            push_template(&mut templates, digit as u8, &scaled);
        }

        Self { templates }
    }
}

fn push_template(templates: &mut Vec<DigitTemplate>, digit: u8, img: &image::GrayImage) {
    let (w, h) = img.dimensions();
    let pixels: Vec<f32> = img.pixels().map(|p| p.0[0] as f32 / 255.0).collect();
    templates.push(DigitTemplate {
        digit,
        pixels,
        width: w as usize,
        height: h as usize,
    });
}

/// Compute Otsu's threshold — finds the gray level that best separates
/// dark (background) and bright (digit) pixels by maximizing between-class
/// variance.  Falls back to 160 when the histogram is degenerate.
fn otsu_threshold(gray_vals: &[f32]) -> f32 {
    let mut hist = [0u32; 256];
    let mut sum_all = 0.0f64;
    for &g in gray_vals {
        let bin = (g.clamp(0.0, 255.0)) as usize;
        hist[bin.min(255)] += 1;
        sum_all += g as f64;
    }

    let total = gray_vals.len() as f64;
    let mut w0 = 0.0f64;
    let mut sum0 = 0.0f64;
    let mut max_var = 0.0f64;
    let mut best_t = 160.0f64;

    for t in 0..256 {
        let cnt = hist[t] as f64;
        w0 += cnt;
        if w0 == 0.0 {
            continue;
        }
        let w1 = total - w0;
        if w1 == 0.0 {
            break;
        }
        sum0 += t as f64 * cnt;
        let mean0 = sum0 / w0;
        let mean1 = (sum_all - sum0) / w1;
        let var = w0 * w1 * (mean0 - mean1) * (mean0 - mean1);
        if var > max_var {
            max_var = var;
            best_t = t as f64;
        }
    }

    best_t as f32
}

/// Run template matching and return recognized time string like "4:32" or "12:05".
/// Returns None if no confident match found.
///
/// High-score frames (best NCC ≥ 0.80) are saved to `training_frames/` and
/// low-score frames (< 0.75) to `low_score_frames/`, both under
/// `%APPDATA%/com.voxalic.app/`.
/// Filter NCC detections to keep only digits in the same horizontal row.
/// Timer digits share a Y coordinate; life-support / buff digits sit elsewhere.
fn filter_same_row(
    detections: &[(f32, usize, usize, u8, usize, usize)],
) -> Vec<(f32, usize, usize, u8, usize, usize)> {
    if detections.len() <= 1 {
        return detections.to_vec();
    }

    // Sort by Y coordinate
    let mut sorted = detections.to_vec();
    sorted.sort_by_key(|d| d.2);

    // Greedy clustering: group detections within ~half template height in Y
    let mut groups: Vec<Vec<(f32, usize, usize, u8, usize, usize)>> = Vec::new();
    let mut current: Vec<(f32, usize, usize, u8, usize, usize)> = Vec::new();
    let mut last_y = 0usize;

    for det in &sorted {
        if current.is_empty() {
            current.push(*det);
            last_y = det.2;
        } else {
            // Row tolerance: half the avg template height
            let threshold = (det.5 as i32) / 2;
            if (det.2 as i32 - last_y as i32).abs() <= threshold {
                current.push(*det);
                last_y = det.2; // moving average anchoring
            } else {
                groups.push(current);
                current = vec![*det];
                last_y = det.2;
            }
        }
    }
    groups.push(current);

    // Pick best group: by total match score (prefer confident timer row)
    groups.sort_by(|a, b| {
        let score_a: f32 = a.iter().map(|d| d.0).sum();
        let score_b: f32 = b.iter().map(|d| d.0).sum();
        score_b.partial_cmp(&score_a).unwrap()
    });

    groups.into_iter().next().unwrap_or_default()
}

pub fn recognize_digits(
    roi_pixels: &[u8], // BGR
    roi_w: u32,
    roi_h: u32,
    templates: &DigitTemplates,
    match_threshold: f32,
    mut cnn: Option<&mut Cnn>,
) -> Option<String> {
    // BGR → grayscale, then Otsu → binary
    let gray_vals: Vec<f32> = roi_pixels
        .chunks(3)
        .map(|rgb| {
            let b = rgb[0] as f32;
            let g = rgb[1] as f32;
            let r = rgb[2] as f32;
            0.299 * r + 0.587 * g + 0.114 * b
        })
        .collect();

    let thresh = otsu_threshold(&gray_vals);

    let binary: Vec<f32> = gray_vals
        .iter()
        .map(|&g| if g > thresh { 1.0 } else { 0.0 })
        .collect();

    let img_w = roi_w as usize;
    let img_h = roi_h as usize;

    let mut all_detections: Vec<(f32, usize, usize, u8, usize, usize)> = Vec::new();

    for tpl in &templates.templates {
        let dets = match_template(
            &binary,
            img_w,
            img_h,
            &tpl.pixels,
            tpl.width,
            tpl.height,
            match_threshold,
        );
        for (score, x, y) in dets {
            all_detections.push((score, x, y, tpl.digit, tpl.width, tpl.height));
        }
    }

    // Save low-score frames for training data collection
    let best_score = all_detections
        .iter()
        .map(|d| d.0)
        .fold(0.0f32, f32::max);

    if all_detections.is_empty() {
        eprintln!("[OCR] 0 NCC candidates (thresh={match_threshold})");
        save_training_frame(&gray_vals, roi_w, roi_h, 0.0, "NoResult");
        return None;
    }

    let raw_count = all_detections.len();

    // NMS merge overlapping detections (IoU > 0.3)
    let mut kept = nms(&all_detections, 0.3);

    let nms_count = kept.len();

    // Same-row filter: timer digits share a horizontal line.
    // Life-support digits occupy different rows → excluded.
    kept = filter_same_row(&kept);

    let row_count = kept.len();
    eprintln!("[OCR] NCC candidates: raw={raw_count} nms={nms_count} row={row_count}");

    // CNN refinement: re-classify each NCC detection
    if let Some(ref mut cnn) = cnn {
        let mut refined: Vec<(f32, usize, usize, u8, usize, usize)> = Vec::new();
        for &(score, x, y, digit, w, h) in &kept {
            let patch = extract_cnn_patch(&gray_vals, img_w, img_h, x, y, w, h);
            let (cnn_class, conf) = cnn.classify(&patch);

            if cnn_class == 10 {
                continue; // CNN says non-digit → reject
            }

            let final_digit = if conf >= 0.88 {
                cnn_class // trust zone
            } else if conf >= 0.60 {
                if digit == cnn_class { digit } else { continue } // consultation zone
            } else {
                digit // reject zone: keep NCC
            };

            refined.push((score, x, y, final_digit, w, h));
        }
        // Fallback: if CNN rejected everything, keep NCC-only result
        if !refined.is_empty() {
            kept = refined;
        } else {
            eprintln!("[OCR] CNN rejected all {} candidates, fallback to NCC", kept.len());
        }
    }

    // Sort by x coordinate, join digits
    let mut sorted = kept;
    sorted.sort_by_key(|d| d.1);

    let digits: String = sorted
        .iter()
        .map(|(_, _, _, d, ..)| (d + b'0') as char)
        .collect();

    // Save frames for training: high-score → training_frames/, low-score → low_score_frames/
    if best_score >= 0.80 {
        save_training_frame(&gray_vals, roi_w, roi_h, best_score, &digits);
    } else if best_score < 0.75 {
        save_training_frame(&gray_vals, roi_w, roi_h, best_score, &digits);
    }

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
    image: &[f32],
    img_w: usize,
    img_h: usize,
    template: &[f32],
    tpl_w: usize,
    tpl_h: usize,
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
    detections: &[(f32, usize, usize, u8, usize, usize)],
    iou_thresh: f32,
) -> Vec<(f32, usize, usize, u8, usize, usize)> {
    let mut sorted = detections.to_vec();
    sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep: Vec<(f32, usize, usize, u8, usize, usize)> = Vec::new();
    for det in &sorted {
        let (score, x, y, digit, w, h) = *det;
        let mut overlap = false;
        for k in &keep {
            let (_, kx, ky, _, kw, kh) = *k;
            let x1 = x.max(kx) as f32;
            let y1 = y.max(ky) as f32;
            let x2 = (x + w).min(kx + kw) as f32;
            let y2 = (y + h).min(ky + kh) as f32;
            if x2 > x1 && y2 > y1 {
                let inter = (x2 - x1) * (y2 - y1);
                let area_a = (w * h) as f32;
                let area_b = (kw * kh) as f32;
                let min_area = area_a.min(area_b);
                if inter / min_area > iou_thresh {
                    overlap = true;
                    break;
                }
            }
        }
        if !overlap {
            keep.push((score, x, y, digit, w, h));
        }
    }
    keep
}

/// Save a grayscale ROI frame to %APPDATA%/com.voxalic.app/.
/// High-score frames (result="high") go to `training_frames/`, low-score
/// frames go to `low_score_frames/`.  Throttled to ≤1 save per 2 seconds.
fn save_training_frame(
    gray_vals: &[f32],
    w: u32,
    h: u32,
    best_score: f32,
    result: &str,
) {
    static LAST_SAVE: AtomicI64 = AtomicI64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let last = LAST_SAVE.load(Ordering::Relaxed);
    if now - last < 2 {
        return; // throttle
    }
    LAST_SAVE.store(now, Ordering::Relaxed);

    let appdata = match std::env::var("APPDATA") {
        Ok(v) => v,
        Err(_) => return,
    };
    let subdir = if best_score >= 0.80 { "training_frames" } else { "low_score_frames" };
    let dir = PathBuf::from(appdata).join("com.voxalic.app").join(subdir);
    if let Err(_) = std::fs::create_dir_all(&dir) {
        return;
    }

    let pixels: Vec<u8> = gray_vals.iter().map(|&g| g.clamp(0.0, 255.0) as u8).collect();
    let img = image::GrayImage::from_raw(w, h, pixels);
    let img = match img {
        Some(i) => i,
        None => return,
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let safe_result = result.chars().filter(|c| c.is_alphanumeric() || *c == ':').take(20).collect::<String>();
    let fname = format!("{:.2}_{}_{}.png", best_score, ts, safe_result);
    let path = dir.join(&fname);

    // Write PNG to memory first, then to disk (avoid partial writes)
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    if encoder.write_image(img.as_raw(), w, h, image::ExtendedColorType::L8).is_err() {
        return;
    }
    let _ = std::fs::write(&path, &buf);
}

/// Extract a 40×40 crop around the NCC detection centre, then resize to
/// 24×24 via bilinear interpolation — matching the training pipeline.
fn extract_cnn_patch(
    gray_vals: &[f32],
    img_w: usize,
    img_h: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> [f32; 576] {
    let crop_sz = 40usize;
    let half = crop_sz / 2;
    let cx = x + w / 2;
    let cy = y + h / 2;

    // Crop 40×40 centred on detection
    let mut crop = [0f32; 40 * 40];
    let x0 = cx.saturating_sub(half);
    let y0 = cy.saturating_sub(half);
    for dy in 0..crop_sz {
        let iy = y0 + dy;
        if iy >= img_h { break; }
        for dx in 0..crop_sz {
            let ix = x0 + dx;
            if ix >= img_w { break; }
            crop[dy * crop_sz + dx] = gray_vals[iy * img_w + ix] / 255.0;
        }
    }

    // Bilinear resize 40×40 → 24×24
    let mut patch = [0f32; 576];
    let scale_x = crop_sz as f32 / 24.0;
    let scale_y = crop_sz as f32 / 24.0;
    for dy in 0..24 {
        for dx in 0..24 {
            let sx = dx as f32 * scale_x;
            let sy = dy as f32 * scale_y;
            let x0 = (sx as usize).min(crop_sz - 2);
            let y0 = (sy as usize).min(crop_sz - 2);
            let x1 = (x0 + 1).min(crop_sz - 1);
            let y1 = (y0 + 1).min(crop_sz - 1);
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let v00 = crop[y0 * crop_sz + x0];
            let v10 = crop[y0 * crop_sz + x1];
            let v01 = crop[y1 * crop_sz + x0];
            let v11 = crop[y1 * crop_sz + x1];
            patch[dy * 24 + dx] =
                (1.0 - fx) * (1.0 - fy) * v00
                + fx * (1.0 - fy) * v10
                + (1.0 - fx) * fy * v01
                + fx * fy * v11;
        }
    }
    patch
}
