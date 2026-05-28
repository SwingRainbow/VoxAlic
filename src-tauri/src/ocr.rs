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
            let pixels: Vec<f32> = img
                .pixels()
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
    roi_pixels: &[u8], // BGR
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
            if gray_val > 160.0 {
                1.0
            } else {
                0.0
            }
        })
        .collect();

    let img_w = roi_w as usize;
    let img_h = roi_h as usize;

    let mut all_detections: Vec<(f32, usize, usize, u8)> = Vec::new();

    for tpl in &templates.templates {
        let dets = match_template(
            &gray,
            img_w,
            img_h,
            &tpl.pixels,
            tpl.width,
            tpl.height,
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
    let kept = nms(
        &all_detections,
        templates.templates[0].width,
        templates.templates[0].height,
        0.3,
    );

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
    detections: &[(f32, usize, usize, u8)],
    box_w: usize,
    box_h: usize,
    iou_thresh: f32,
) -> Vec<(f32, usize, usize, u8)> {
    let mut sorted = detections.to_vec();
    sorted.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
