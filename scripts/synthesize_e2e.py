"""Synthetic data generator v2 for end-to-end mission timer CNN.

Improvements over v1:
- Structured HUD-like backgrounds (gradients, panels, simulated UI elements)
- More digit rendering variation (fonts, anti-aliasing, brightness)
- Real-world augmentations (motion blur, compression artifacts, speckle noise)
- Larger dataset with wider time distribution coverage
"""

import os
import random
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter
from pathlib import Path

OUT_DIR = Path(__file__).parent / "synthetic_e2e"
OUT_DIR.mkdir(exist_ok=True)

N_SAMPLES = 3000
ROI_W, ROI_H = 131, 167


def _font_cache():
    """Lazy-load available fonts at various sizes."""
    if hasattr(_font_cache, "_cache"):
        return _font_cache._cache

    fonts_by_size = {}
    font_paths = []
    for name in ["consolab.ttf", "courbd.ttf", "lucon.ttf",
                  "arialbd.ttf", "segoeuib.ttf", "calibrib.ttf",
                  "cambriab.ttf", "timesbd.ttf"]:
        p = os.path.join("C:\\Windows\\Fonts", name)
        if os.path.exists(p):
            font_paths.append(p)

    for sz in [24, 26, 28, 30, 32, 34, 36, 38, 40, 44, 48]:
        sz_fonts = []
        for fp in font_paths:
            try:
                sz_fonts.append(ImageFont.truetype(fp, sz))
            except Exception:
                pass
        if sz_fonts:
            fonts_by_size[sz] = sz_fonts

    _font_cache._cache = fonts_by_size
    return fonts_by_size


def random_font(min_size=24, max_size=48):
    fonts_by_size = _font_cache()
    available = [sz for sz in fonts_by_size if min_size <= sz <= max_size]
    if not available:
        return ImageFont.load_default()
    sz = random.choice(available)
    return random.choice(fonts_by_size[sz])


def generate_background(w, h) -> np.ndarray:
    """Create a HUD-like background with structure."""
    bg_type = random.random()

    if bg_type < 0.4:
        # Dark gradient background (like HUD panels)
        bg = np.zeros((h, w), dtype=np.uint8)
        direction = random.randint(0, 3)
        for y in range(h):
            for x in range(w):
                if direction == 0:  # vertical gradient
                    v = 15 + int(30 * y / h)
                elif direction == 1:  # horizontal gradient
                    v = 15 + int(30 * x / w)
                elif direction == 2:  # radial (dark center)
                    cx, cy = w // 2, h // 2
                    dist = np.sqrt((x - cx) ** 2 + (y - cy) ** 2)
                    v = 10 + int(35 * dist / max(w, h))
                else:  # corner gradient
                    v = 10 + int(35 * (x + y) / (w + h))
                bg[y, x] = np.clip(v, 8, 50)
    else:
        # Flat dark with noise + structural elements
        bg = np.random.randint(10, 45, (h, w), dtype=np.uint8)

    # Structural HUD elements
    for _ in range(random.randint(0, 4)):
        elem = random.random()
        if elem < 0.3:
            # Horizontal line/band
            y = random.randint(4, h - 5)
            thick = random.randint(1, 2)
            val = random.randint(3, 18)
            bg[y: y + thick, :] = np.clip(
                bg[y: y + thick, :].astype(int) + val, 0, 255
            ).astype(np.uint8)
        elif elem < 0.6:
            # Small bright dot cluster (like buff indicators)
            cx = random.randint(5, w - 5)
            cy = random.randint(5, h - 5)
            r = random.randint(2, 5)
            y_vals, x_vals = np.ogrid[:h, :w]
            mask = (x_vals - cx) ** 2 + (y_vals - cy) ** 2 <= r**2
            bg[mask] = np.clip(bg[mask].astype(int) + random.randint(10, 40), 0, 255).astype(np.uint8)
        else:
            # Vertical separator line
            x = random.randint(5, w - 5)
            bg[:, x: x + 1] = np.clip(
                bg[:, x: x + 1].astype(int) + random.randint(5, 15), 0, 255
            ).astype(np.uint8)

    # Gaussian noise
    noise = np.random.normal(0, random.uniform(2, 8), (h, w)).astype(np.int16)
    bg = np.clip(bg.astype(np.int16) + noise, 0, 255).astype(np.uint8)

    return bg


def generate_one(index: int):
    img = Image.fromarray(generate_background(ROI_W, ROI_H), mode="L")

    # Random time
    mins = random.randint(0, 59)
    secs = random.randint(0, 59)
    text = f"{mins}:{secs:02d}"
    label = f"{mins:02d}{secs:02d}"

    font = random_font(26, 42)

    # Measure text
    temp = Image.new("L", (ROI_W, ROI_H))
    temp_draw = ImageDraw.Draw(temp)
    bbox = temp_draw.textbbox((0, 0), text, font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]

    if tw <= 0 or th <= 0 or tw >= ROI_W or th >= ROI_H:
        fname = f"syn_{index:04d}_{label}.png"
        img.save(OUT_DIR / fname)
        return fname, label

    # Position: centered with jitter
    base_x = (ROI_W - tw) // 2
    base_y = (ROI_H - th) // 2
    jitter_x = random.randint(-15, 15)
    jitter_y = random.randint(-10, 10)
    x = max(3, min(ROI_W - tw - 3, base_x + jitter_x))
    y = max(3, min(ROI_H - th - 3, base_y + jitter_y))

    # Dark panel behind digits
    margin = random.randint(3, 8)
    arr = np.array(img)
    x1 = max(0, x - margin)
    y1 = max(0, y - margin)
    x2 = min(ROI_W, x + tw + margin)
    y2 = min(ROI_H, y + th + margin)
    darken = random.randint(15, 40)
    arr[y1:y2, x1:x2] = np.clip(
        arr[y1:y2, x1:x2].astype(int) - darken, 0, 255
    ).astype(np.uint8)
    img = Image.fromarray(arr)

    # Render each character
    draw = ImageDraw.Draw(img)
    char_x = x
    base_brightness = random.randint(190, 250)
    for ch in text:
        brightness = int(np.clip(base_brightness + random.randint(-20, 15), 160, 255))
        draw.text((char_x, y), ch, fill=brightness, font=font)
        c_bbox = draw.textbbox((0, 0), ch, font=font)
        char_x += c_bbox[2] - c_bbox[0] + random.randint(0, 3)

    img_np = np.array(img)

    # Post-rendering augmentations
    r = random.random()

    if r < 0.25:
        # Gaussian blur (motion / effects)
        sigma = random.uniform(0.3, 1.5)
        img_np = np.array(Image.fromarray(img_np).filter(ImageFilter.GaussianBlur(sigma)))
    elif r < 0.40:
        # Slight pixelation / compression
        small = Image.fromarray(img_np).resize(
            (ROI_W // 2, ROI_H // 2), Image.NEAREST
        )
        img_np = np.array(small.resize((ROI_W, ROI_H), Image.BILINEAR))
    elif r < 0.48:
        # Salt and pepper noise
        mask = np.random.random(img_np.shape) < 0.003
        img_np[mask] = random.randint(0, 255)

    # Brightness / contrast jitter
    brightness_shift = random.randint(-30, 30)
    contrast_scale = random.uniform(0.85, 1.15)
    img_np = np.clip(
        (img_np.astype(float) - 128) * contrast_scale + 128 + brightness_shift,
        0, 255,
    ).astype(np.uint8)

    fname = f"syn_{index:04d}_{label}.png"
    Image.fromarray(img_np, mode="L").save(OUT_DIR / fname)
    return fname, label


def main():
    print(f"Generating {N_SAMPLES} synthetic frames to {OUT_DIR}...")
    labels = []
    for i in range(N_SAMPLES):
        fname, label = generate_one(i)
        labels.append(f"{fname},{label}")
        if (i + 1) % 500 == 0:
            print(f"  {i + 1}/{N_SAMPLES}")

    csv_path = OUT_DIR / "labels.csv"
    with open(csv_path, "w") as f:
        f.write("filename,label\n")
        f.write("\n".join(labels))

    # Verify: count per digit position
    from collections import Counter
    # labels list contains "filename,MMSS" - extract just the 4-digit label
    label_digits = []
    for entry in labels:
        parts = entry.split(",")
        if len(parts) == 2 and len(parts[1]) == 4:
            label_digits.append(parts[1])
    for pos, name in enumerate(["分十", "分个", "秒十", "秒个"]):
        c = Counter(d[pos] for d in label_digits)
        print(f"  {name}: {dict(sorted(c.items()))}")

    print(f"Done. {N_SAMPLES} frames + labels.csv in {OUT_DIR}")


if __name__ == "__main__":
    main()
