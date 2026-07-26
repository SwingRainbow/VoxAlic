"""Offline diagnostic: run NCC + same-row filter + CNN on 609 labeled frames.

Separates localization errors (NCC) from classification errors (CNN).
Outputs per-digit recall/precision and end-to-end accuracy.
"""

import os, sys, json
import numpy as np
from pathlib import Path
from collections import defaultdict
from PIL import Image

import torch
import torch.nn as nn
import torch.nn.functional as F

# ── Paths ───────────────────────────────────────────────────
SCRIPT_DIR = Path(__file__).parent
TEMPLATE_DIR = Path("src-tauri/resources/digit_templates")
APPDATA = Path.home() / "AppData" / "Roaming" / "com.voxalic.app"
TRAINING_DIR = APPDATA / "training_frames"
LOW_SCORE_DIR = APPDATA / "low_score_frames"

MATCH_THRESHOLD = 0.70
NMS_IOU = 0.3
INPUT_SIZE = 24

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")


# ── 1. Load templates ───────────────────────────────────────
def load_templates():
    """Load 10 digit templates from PNG files."""
    templates = []
    for d in range(10):
        path = TEMPLATE_DIR / f"{d}.png"
        img = Image.open(path).convert("L")
        arr = np.array(img, dtype=np.float32) / 255.0
        h, w = arr.shape
        templates.append({
            "digit": d,
            "pixels": arr.flatten().tolist(),
            "width": w,
            "height": h,
        })
    return templates


# ── 2. NCC matching (Python replica of ocr.rs) ───────────────
def ncc_match(image, template, threshold):
    """Normalized cross-correlation. Returns [(score, x, y), ...]."""
    img_h, img_w = image.shape
    tpl = np.array(template["pixels"], dtype=np.float32).reshape(template["height"], template["width"])
    tpl_h, tpl_w = tpl.shape
    n = tpl_w * tpl_h
    tpl_mean = tpl.mean()
    tpl_c = tpl - tpl_mean
    tpl_l2 = np.sqrt((tpl_c ** 2).sum())
    if tpl_l2 < 1e-6:
        return []

    results = []
    max_y = img_h - tpl_h
    max_x = img_w - tpl_w
    for y in range(0, max_y, 2):  # stride 2 for speed
        for x in range(0, max_x, 2):
            patch = image[y:y + tpl_h, x:x + tpl_w]
            patch_mean = patch.mean()
            patch_c = patch - patch_mean
            patch_l2 = np.sqrt((patch_c ** 2).sum())
            if patch_l2 < 1e-6:
                continue
            score = (tpl_c * patch_c).sum() / (tpl_l2 * patch_l2)
            if score > threshold:
                results.append((float(score), x, y))
    return results


def iou(a, b):
    """Intersection over min area (matching Rust ocr.rs nms)."""
    ax1, ay1, ax2, ay2 = a[1], a[2], a[1] + a[4], a[2] + a[5]
    bx1, by1, bx2, by2 = b[1], b[2], b[1] + b[4], b[2] + b[5]
    ix1, iy1 = max(ax1, bx1), max(ay1, by1)
    ix2, iy2 = min(ax2, bx2), min(ay2, by2)
    if ix1 >= ix2 or iy1 >= iy2:
        return 0.0
    inter = (ix2 - ix1) * (iy2 - iy1)
    area_a = (ax2 - ax1) * (ay2 - ay1)
    area_b = (bx2 - bx1) * (by2 - by1)
    return inter / min(area_a, area_b)


def nms(detections, iou_threshold):
    """Non-maximum suppression."""
    if len(detections) <= 1:
        return detections.copy()
    sorted_dets = sorted(detections, key=lambda d: -d[0])
    kept = []
    for det in sorted_dets:
        overlap = False
        for k in kept:
            if iou(det, k) > iou_threshold:
                overlap = True
                break
        if not overlap:
            kept.append(det)
    return kept


def filter_same_row_py(detections):
    """Keep only digits in the same horizontal row (largest group by score)."""
    if len(detections) <= 1:
        return detections

    sorted_dets = sorted(detections, key=lambda d: d[2])  # sort by Y
    groups = []
    current = [sorted_dets[0]]
    last_y = sorted_dets[0][2]

    for det in sorted_dets[1:]:
        threshold = det[5] // 2  # half template height
        if abs(det[2] - last_y) <= threshold:
            current.append(det)
            last_y = det[2]
        else:
            groups.append(current)
            current = [det]
            last_y = det[2]
    groups.append(current)

    # Pick group with highest total score
    groups.sort(key=lambda g: -sum(d[0] for d in g))
    return groups[0]


# ── 3. CNN model ────────────────────────────────────────────
class DigitCNN(nn.Module):
    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.bn1 = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.bn2 = nn.BatchNorm2d(16)
        self.conv3 = nn.Conv2d(16, 32, 3, padding=1)
        self.bn3 = nn.BatchNorm2d(32)
        self.pool = nn.MaxPool2d(2, 2)
        self.fc1 = nn.Linear(3 * 3 * 32, 48)
        self.fc2 = nn.Linear(48, 11)
        self.dropout = nn.Dropout(0.5)

    def forward(self, x):
        x = self.pool(F.relu(self.bn1(self.conv1(x))))
        x = self.pool(F.relu(self.bn2(self.conv2(x))))
        x = self.pool(F.relu(self.bn3(self.conv3(x))))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        return self.fc2(x)


def load_cnn():
    path = SCRIPT_DIR / "best_cnn_v2.pth"
    model = DigitCNN().to(DEVICE)
    model.load_state_dict(torch.load(path, map_location=DEVICE))
    model.eval()
    return model


def crop_cnn_patch(gray, cx, cy, w, h):
    """Extract 24x24 patch centered on NCC detection for CNN input."""
    patch = np.zeros((INPUT_SIZE, INPUT_SIZE), dtype=np.float32)
    ox = (INPUT_SIZE - w) // 2
    oy = (INPUT_SIZE - h) // 2
    for dy in range(h):
        iy = cy + dy
        if iy >= gray.shape[0]:
            break
        py = oy + dy
        if py >= INPUT_SIZE:
            break
        for dx in range(w):
            ix = cx + dx
            if ix >= gray.shape[1]:
                break
            px = ox + dx
            if px >= INPUT_SIZE:
                break
            patch[py, px] = gray[iy, ix] / 255.0
    return patch


# ── 4. Load labeled data ────────────────────────────────────
def load_annotations():
    """Load all annotation CSVs."""
    merged = {}
    for csv_path in sorted(SCRIPT_DIR.glob("annotations_v*.csv")):
        with open(csv_path, "rb") as f:
            raw = f.read()
        if b"\n" not in raw:
            raw = raw.replace(bytes([0x5c, 0x6e]), b"\n")
        text = raw.decode("utf-8")
        for line in text.strip().split("\n"):
            line = line.strip()
            if not line or "filename" in line:
                continue
            parts = line.split(",")
            if len(parts) < 2:
                continue
            fname, label = parts[0].strip(), parts[1].strip()
            if len(label) != 4 or not label.isdigit():
                continue
            for d in [TRAINING_DIR, LOW_SCORE_DIR]:
                p = d / fname
                if p.exists():
                    merged[fname] = (label, p)
                    break
    return merged


# ── 5. Main diagnostic ──────────────────────────────────────
def main():
    print("Loading templates...")
    templates = load_templates()
    print(f"  {len(templates)} templates loaded")

    print("Loading CNN...")
    cnn = load_cnn()
    print("  CNN loaded")

    print("Loading annotations...")
    annotated = load_annotations()
    print(f"  {len(annotated)} labeled frames")

    # Stats
    total = 0
    ncc_found = 0       # NCC found all 4 label digits
    ncc_any_found = 0   # NCC found at least some digits
    ncc_extra = 0       # NCC found extra non-timer digits
    cnn_correct = 0     # CNN classified correctly (of found digits)
    cnn_total = 0       # Total digits classified by CNN
    exact_match = 0     # All 4 digits correct end-to-end
    per_digit_ok = [0, 0, 0, 0]  # Per-position correct count
    per_digit_total = [0, 0, 0, 0]

    errors_ncc_miss = 0     # Label digit not found by NCC
    errors_ncc_extra = 0    # NCC found digit not in label (at wrong position)
    errors_cnn_wrong = 0    # NCC found correctly but CNN misclassified

    sample_errors = []  # Collect error examples

    for fname, (label_str, path) in annotated.items():
        gray = np.array(Image.open(path).convert("L"), dtype=np.float32)
        label = [int(c) for c in label_str]

        # Run NCC on all 10 templates
        all_dets = []
        for tpl in templates:
            dets = ncc_match(gray, tpl, MATCH_THRESHOLD)
            for score, x, y in dets:
                all_dets.append((score, x, y, tpl["digit"], tpl["width"], tpl["height"]))

        if not all_dets:
            errors_ncc_miss += 4
            ncc_any_found += 0
            total += 1
            continue

        # NMS
        best_score = max(d[0] for d in all_dets)
        kept = nms(all_dets, NMS_IOU)

        # Same-row filter
        kept = filter_same_row_py(kept)

        # Sort by X
        kept.sort(key=lambda d: d[1])

        # Match to label: assume sorted detections correspond to label digits
        ncc_digits = [d[3] for d in kept]
        ncc_positions = [(d[1], d[2], d[4], d[5]) for d in kept]

        ncc_any_found += 1

        # Check how many label digits were found
        matched = 0
        for i, expected_digit in enumerate(label):
            found = False
            for j, ncc_d in enumerate(ncc_digits):
                if ncc_d == expected_digit and j < len(ncc_positions):
                    # Check if position is plausible (roughly in order)
                    found = True
                    matched += 1
                    break
            if not found:
                errors_ncc_miss += 1
            per_digit_total[i] += 1

        if matched == 4:
            ncc_found += 1

        if len(ncc_digits) > 4:
            ncc_extra += 1
            errors_ncc_extra += len(ncc_digits) - 4

        # CNN classification on each NCC detection
        frame_cnn_correct = 0
        frame_cnn_total = 0
        for j, (det_digit, (cx, cy, dw, dh)) in enumerate(zip(ncc_digits, ncc_positions)):
            patch = crop_cnn_patch(gray, cx, cy, dw, dh)
            tensor = torch.from_numpy(patch).float().unsqueeze(0).unsqueeze(0).to(DEVICE)
            with torch.no_grad():
                logits = cnn(tensor)
                pred = torch.argmax(logits, dim=1).item()
            frame_cnn_total += 1
            cnn_total += 1

            # Find which label digit this corresponds to (by position order)
            if j < len(label):
                true_digit = label[j]
                per_digit_total[j] += 1
                if pred == true_digit:
                    cnn_correct += 1
                    frame_cnn_correct += 1
                    per_digit_ok[j] += 1
                else:
                    errors_cnn_wrong += 1
                    if len(sample_errors) < 10:
                        sample_errors.append({
                            "fname": fname[:50],
                            "label": label_str,
                            "pos": j,
                            "true": true_digit,
                            "pred": pred,
                            "ncc_detected": ncc_digits,
                        })
            elif pred != 10:  # non-digit predicted as digit = extra error
                errors_ncc_extra += 1

        if frame_cnn_correct == 4 and matched == 4:
            exact_match += 1

        total += 1
        if total % 100 == 0:
            print(f"  processed {total}/{len(annotated)}")

    # ── Report ──
    print(f"\n{'='*60}")
    print(f"DIAGNOSTIC RESULTS ({total} frames)")
    print(f"{'='*60}")
    print(f"\nNCC LOCALIZATION:")
    print(f"  Frames with any NCC detection: {ncc_any_found}/{total} ({ncc_any_found/total*100:.1f}%)")
    print(f"  Frames with all 4 digits found: {ncc_found}/{total} ({ncc_found/total*100:.1f}%)")
    print(f"  Frames with extra detections: {ncc_extra}/{total} ({ncc_extra/total*100:.1f}%)")
    print(f"  Label digits missed by NCC: {errors_ncc_miss}")
    print(f"  Extra (non-timer) detections: {errors_ncc_extra}")

    print(f"\nCNN CLASSIFICATION (of NCC-detected digits):")
    print(f"  Overall: {cnn_correct}/{cnn_total} ({cnn_correct/cnn_total*100:.1f}%)" if cnn_total > 0 else "  N/A")

    HEAD_NAMES = ["分十", "分个", "秒十", "秒个"]
    for i in range(4):
        if per_digit_total[i] > 0:
            print(f"  {HEAD_NAMES[i]}: {per_digit_ok[i]}/{per_digit_total[i]} ({per_digit_ok[i]/per_digit_total[i]*100:.1f}%)")

    print(f"\nEND-TO-END:")
    print(f"  Exact match (4/4): {exact_match}/{total} ({exact_match/total*100:.1f}%)")

    print(f"\nERROR BREAKDOWN:")
    print(f"  NCC miss (digit not found): {errors_ncc_miss}")
    print(f"  NCC extra (non-timer): {errors_ncc_extra}")
    print(f"  CNN misclassification: {errors_cnn_wrong}")

    if sample_errors:
        print(f"\nSample CNN errors:")
        for e in sample_errors[:8]:
            print(f"  {e['fname']} label={e['label']} pos={e['pos']}({HEAD_NAMES[e['pos']]}) true={e['true']} pred={e['pred']} ncc={e['ncc_detected']}")


if __name__ == "__main__":
    main()
