"""Train single-digit CNN with NCC-localized patch extraction.

V3: Uses NCC template matching + same-row filter to locate digits in each
labeled frame, then crops patches at NCC-detected positions. This fixes the
"position 3 (秒个) trained on background" bug caused by fixed-position cropping.

Architecture unchanged: 3-conv CNN (same as cnn.rs), 11 classes (0-9 + non-digit).
Exports Conv-BN fused cnn_weights.bin.

Usage:  python scripts/train_cnn_v3.py
Output: best_cnn_v3.pth, src-tauri/resources/cnn_weights.bin
"""

import re, random, json
import numpy as np
from pathlib import Path
from collections import Counter
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
BEST_PTH = SCRIPT_DIR / "best_cnn_v3.pth"
OUTPUT_BLOB = Path("src-tauri/resources/cnn_weights.bin")
NCC_CACHE = SCRIPT_DIR / "ncc_detections_v3.json"

INPUT_SIZE = 24
NUM_CLASSES = 11
BATCH_SIZE = 64
MAX_EPOCHS = 50
LR = 1e-3
WEIGHT_DECAY = 1e-4
ES_PATIENCE = 12
NCC_THRESHOLD = 0.65

torch.manual_seed(42)
random.seed(42)
np.random.seed(42)
DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")


# ── 1. Templates ────────────────────────────────────────────
def load_templates():
    templates = []
    for d in range(10):
        img = Image.open(TEMPLATE_DIR / f"{d}.png").convert("L")
        arr = np.array(img, dtype=np.float32) / 255.0
        templates.append({
            "digit": d, "pixels": arr, "width": arr.shape[1], "height": arr.shape[0],
        })
    return templates


# ── 2. NCC matching (Python replica of ocr.rs) ───────────────
def ncc_match_one(image, tpl, threshold, search_region=None):
    """NCC for one template. If search_region=(x0,y0,x1,y1), only search there."""
    img_h, img_w = image.shape
    tpl_arr = tpl["pixels"]
    tpl_h, tpl_w = tpl_arr.shape
    n = tpl_w * tpl_h
    tpl_mean = tpl_arr.mean()
    tpl_c = tpl_arr - tpl_mean
    tpl_l2 = np.sqrt((tpl_c ** 2).sum())
    if tpl_l2 < 1e-6:
        return []

    if search_region:
        x0, y0, x1, y1 = search_region
        x0 = max(0, x0); y0 = max(0, y0)
        x1 = min(img_w - tpl_w, x1); y1 = min(img_h - tpl_h, y1)
    else:
        x0, y0, x1, y1 = 0, 0, img_w - tpl_w, img_h - tpl_h

    results = []
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            patch = image[y:y + tpl_h, x:x + tpl_w]
            patch_mean = patch.mean()
            patch_c = patch - patch_mean
            patch_l2 = np.sqrt((patch_c ** 2).sum())
            if patch_l2 < 1e-6:
                continue
            score = (tpl_c * patch_c).sum() / (tpl_l2 * patch_l2)
            if score > threshold:
                results.append((float(score), x, y, tpl["digit"], tpl_w, tpl_h))
    return results


def iou(a, b):
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


def nms(detections, iou_threshold=0.3):
    if len(detections) <= 1:
        return detections.copy()
    sorted_dets = sorted(detections, key=lambda d: -d[0])
    kept = []
    for det in sorted_dets:
        if not any(iou(det, k) > iou_threshold for k in kept):
            kept.append(det)
    return kept


def filter_same_row(detections):
    if len(detections) <= 1:
        return detections
    sorted_dets = sorted(detections, key=lambda d: d[2])
    groups, current = [], [sorted_dets[0]]
    last_y = sorted_dets[0][2]
    for det in sorted_dets[1:]:
        threshold = det[5] // 2
        if abs(det[2] - last_y) <= threshold:
            current.append(det)
            last_y = det[2]
        else:
            groups.append(current)
            current = [det]
            last_y = det[2]
    groups.append(current)
    groups.sort(key=lambda g: -sum(d[0] for d in g))
    return groups[0]


# ── 3. NCC-based detection ──────────────────────────────────
def otsu_threshold(gray):
    """Otsu binarization — same as Rust ocr.rs."""
    hist = np.zeros(256, dtype=np.int32)
    for v in gray.ravel():
        hist[min(255, max(0, int(v)))] += 1
    total = gray.size
    sum_all = np.dot(np.arange(256), hist)
    w_b = 0
    sum_b = 0.0
    max_var = 0.0
    best_thresh = 160.0
    for t in range(256):
        w_b += hist[t]
        if w_b == 0: continue
        w_f = total - w_b
        if w_f == 0: break
        sum_b += t * hist[t]
        m_b = sum_b / w_b
        m_f = (sum_all - sum_b) / w_f
        var = w_b * w_f * (m_b - m_f) ** 2
        if var > max_var:
            max_var = var
            best_thresh = float(t)
    return best_thresh


# Update detect_digits to use binary image (same as Rust)
def detect_digits(gray, templates):
    """Run NCC with Otsu binarization and same-row filter — matching Rust ocr.rs pipeline."""
    thresh = otsu_threshold(gray)
    binary = (gray > thresh).astype(np.float32)
    h, w = binary.shape
    all_dets = []
    for tpl in templates:
        all_dets.extend(ncc_match_one(binary, tpl, NCC_THRESHOLD))
    if not all_dets:
        return []
    kept = nms(all_dets)
    kept = filter_same_row(kept)
    kept.sort(key=lambda d: d[1])
    return kept


def digit_positions_est(w, h):
    """Estimated positions for search region narrowing."""
    mid_y = h // 2
    spacing = 35
    total_w = 4 * spacing
    start_x = (w - total_w) // 2 + spacing // 2
    return [(start_x + i * spacing, mid_y) for i in range(4)]


# ── 4. Data loading ─────────────────────────────────────────
def load_annotations():
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


def load_gray(path):
    return np.array(Image.open(path).convert("L"), dtype=np.float32)


def crop_cnn_patch(gray, cx, cy, dw, dh):
    """Extract 24x24 patch centred on a detection box for CNN input."""
    patch = np.zeros((INPUT_SIZE, INPUT_SIZE), dtype=np.float32)
    ox = (INPUT_SIZE - dw) // 2
    oy = (INPUT_SIZE - dh) // 2
    h, w = gray.shape
    for dy in range(dh):
        iy = cy + dy
        if iy >= h: break
        py = oy + dy
        if py >= INPUT_SIZE: break
        for dx in range(dw):
            ix = cx + dx
            if ix >= w: break
            px = ox + dx
            if px >= INPUT_SIZE: break
            patch[py, px] = gray[iy, ix] / 255.0
    return patch


def augment_patch(patch):
    patch = patch + random.uniform(-0.20, 0.20)
    scale = random.uniform(0.85, 1.15)
    if abs(scale - 1.0) > 0.02:
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        new_sz = max(8, int(INPUT_SIZE * scale))
        pil = pil.resize((new_sz, new_sz), Image.BILINEAR)
        src = np.array(pil, dtype=np.float32) / 255.0
        patch = np.zeros((INPUT_SIZE, INPUT_SIZE), dtype=np.float32)
        if new_sz <= INPUT_SIZE:
            ox = (INPUT_SIZE - new_sz) // 2; oy = ox
            patch[oy:oy + new_sz, ox:ox + new_sz] = src
        else:
            ox = (new_sz - INPUT_SIZE) // 2; oy = ox
            patch = src[oy:oy + INPUT_SIZE, ox:ox + INPUT_SIZE]
    if random.random() < 0.3:
        angle = random.uniform(-3.0, 3.0)
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        pil = pil.rotate(angle, resample=Image.BILINEAR, fillcolor=0)
        patch = np.array(pil, dtype=np.float32) / 255.0
    if random.random() < 0.2:
        patch = patch + np.random.normal(0, 0.02, patch.shape)
    return patch.clip(0, 1)


# ── 5. Dataset generation ───────────────────────────────────
def generate_dataset(annotated, templates, augment=True):
    """Extract digit patches using NCC detection positions."""
    X_list, y_list = [], []
    jitters = [(0, 0), (-3, 0), (3, 0), (0, -2), (0, 2), (-2, -1), (2, 1)]
    if not augment:
        jitters = [(0, 0)]

    for fname, (label_str, path) in annotated.items():
        gray = load_gray(path)
        h, w = gray.shape
        label = [int(c) for c in label_str]

        # Run NCC detection
        dets = detect_digits(gray, templates)
        ncc_digits = [d[3] for d in dets]
        ncc_boxes = [(d[1], d[2], d[4], d[5]) for d in dets]

        # Match NCC detections to label digits
        # Strategy: if NCC found 4 digits that match the label, use them.
        # Otherwise, fall back one-by-one: for each label digit, find the
        # nearest NCC detection with matching digit class.
        if len(ncc_digits) >= 3 and ncc_digits[:min(4, len(ncc_digits))] == label[:min(4, len(ncc_digits))]:
            # Direct match: NCC order == label order
            for i in range(min(4, len(ncc_boxes))):
                cx, cy, dw, dh = ncc_boxes[i]
                for dx, dy in jitters:
                    patch = crop_cnn_patch(gray, cx + dx, cy + dy, dw, dh)
                    if augment:
                        patch = augment_patch(patch)
                    X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                    y_list.append(label[i])
        else:
            # Partial match: for each label position, find closest NCC detection
            used = set()
            for i, target_digit in enumerate(label):
                best_dist = 9999
                best_j = -1
                est_x, est_y = digit_positions_est(w, h)[i]
                for j, (ncc_d, (cx, cy, dw, dh)) in enumerate(zip(ncc_digits, ncc_boxes)):
                    if j in used:
                        continue
                    if ncc_d != target_digit:
                        continue
                    dist = abs(cx - est_x) + abs(cy - est_y)
                    if dist < best_dist:
                        best_dist = dist
                        best_j = j
                if best_j >= 0:
                    used.add(best_j)
                    cx, cy, dw, dh = ncc_boxes[best_j]
                else:
                    # Fallback: use estimated position
                    cx, cy = digit_positions_est(w, h)[i]
                    dw, dh = 18, 25  # avg template size
                for dx, dy in jitters:
                    patch = crop_cnn_patch(gray, cx + dx, cy + dy, dw, dh)
                    if augment:
                        patch = augment_patch(patch)
                    X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                    y_list.append(target_digit)

    # Non-digit patches (~10% of total)
    non_digit_count = len(X_list) // 10
    rng = random.Random(42)
    entries = list(annotated.items())
    for _ in range(non_digit_count):
        fname, (_, path) = rng.choice(entries)
        gray = load_gray(path)
        h, w = gray.shape
        cx = rng.randint(10, w - INPUT_SIZE - 10)
        cy = rng.randint(10, h - INPUT_SIZE - 10)
        patch = gray[cy:cy + INPUT_SIZE, cx:cx + INPUT_SIZE] / 255.0
        X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
        y_list.append(10)

    X = torch.stack(X_list)
    y = torch.tensor(y_list, dtype=torch.long)
    perm = torch.randperm(len(X))
    return X[perm], y[perm]


# ── 6. Model (same architecture as cnn.rs) ──────────────────
class DigitCNN(nn.Module):
    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1); self.bn1 = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1); self.bn2 = nn.BatchNorm2d(16)
        self.conv3 = nn.Conv2d(16, 32, 3, padding=1); self.bn3 = nn.BatchNorm2d(32)
        self.pool = nn.MaxPool2d(2, 2)
        self.fc1 = nn.Linear(3 * 3 * 32, 48)
        self.fc2 = nn.Linear(48, NUM_CLASSES)
        self.dropout = nn.Dropout(0.5)

    def forward(self, x):
        x = self.pool(F.relu(self.bn1(self.conv1(x))))
        x = self.pool(F.relu(self.bn2(self.conv2(x))))
        x = self.pool(F.relu(self.bn3(self.conv3(x))))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        return self.fc2(self.dropout(x))


# ── 7. Export ───────────────────────────────────────────────
def fuse_conv_bn(conv_w, conv_b, bn_w, bn_b, bn_mean, bn_var, eps=1e-5):
    std = np.sqrt(bn_var + eps)
    gamma_div_std = bn_w / std
    fused_w = conv_w * gamma_div_std[:, np.newaxis, np.newaxis, np.newaxis]
    fused_b = gamma_div_std * (conv_b - bn_mean) + bn_b
    return fused_w, fused_b


def export_weights(model):
    state = {k: v.cpu().numpy() for k, v in model.state_dict().items()}
    blob = b""
    for ci in [1, 2, 3]:
        cw = state[f"conv{ci}.weight"]; cb = state[f"conv{ci}.bias"]
        bw = state[f"bn{ci}.weight"]; bb = state[f"bn{ci}.bias"]
        bm = state[f"bn{ci}.running_mean"]; bv = state[f"bn{ci}.running_var"]
        fw, fb = fuse_conv_bn(cw, cb, bw, bb, bm, bv)
        blob += fw.astype(np.float32).tobytes()
        blob += fb.astype(np.float32).tobytes()
    for fc in ["fc1", "fc2"]:
        w = state[f"{fc}.weight"]; b = state[f"{fc}.bias"]
        blob += w.astype(np.float32).tobytes()
        blob += b.astype(np.float32).tobytes()
    OUTPUT_BLOB.parent.mkdir(parents=True, exist_ok=True)
    with open(OUTPUT_BLOB, "wb") as f:
        f.write(blob)
    print(f"  Total: {len(blob):,} bytes, {len(blob)//4:,} f32s")


# ── 8. Main ─────────────────────────────────────────────────
def main():
    print(f"Device: {DEVICE}")

    print("Loading templates...")
    templates = load_templates()
    print(f"  {len(templates)} templates")

    print("Loading annotations...")
    annotated = load_annotations()
    print(f"  {len(annotated)} labeled frames")

    # Split 80/20
    fnames = list(annotated.keys())
    random.shuffle(fnames)
    n_val = max(20, len(fnames) // 5)
    val_keys = set(fnames[:n_val])
    train_anno = {k: v for k, v in annotated.items() if k not in val_keys}
    val_anno = {k: v for k, v in annotated.items() if k in val_keys}
    print(f"Train: {len(train_anno)}, Val: {len(val_anno)}")

    # Generate datasets (NCC-based patch extraction)
    print("Generating training patches (NCC-localized)...")
    X_train, y_train = generate_dataset(train_anno, templates, augment=True)
    print(f"Train: {X_train.shape[0]} patches, class dist: {dict(sorted(Counter(y_train.tolist()).items()))}")

    print("Generating validation patches...")
    X_val, y_val = generate_dataset(val_anno, templates, augment=False)
    print(f"Val:   {X_val.shape[0]} patches")

    train_ds = torch.utils.data.TensorDataset(X_train, y_train)
    val_ds = torch.utils.data.TensorDataset(X_val, y_val)
    train_loader = torch.utils.data.DataLoader(train_ds, BATCH_SIZE, shuffle=True)
    val_loader = torch.utils.data.DataLoader(val_ds, BATCH_SIZE, shuffle=False)

    model = DigitCNN().to(DEVICE)
    n_params = sum(p.numel() for p in model.parameters())
    print(f"Params: {n_params:,} ({n_params * 4 / 1024:.0f} KB)")

    optimizer = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(optimizer, mode="min", factor=0.5, patience=5)

    best_val_acc = 0.0
    patience = 0

    for epoch in range(1, MAX_EPOCHS + 1):
        model.train()
        train_loss = 0.0
        for X, y in train_loader:
            X, y = X.to(DEVICE), y.to(DEVICE)
            optimizer.zero_grad()
            loss = F.cross_entropy(model(X), y)
            loss.backward()
            optimizer.step()
            train_loss += loss.item() * X.size(0)
        train_loss /= len(train_ds)

        model.eval()
        val_loss = 0.0
        correct = 0
        with torch.no_grad():
            for X, y in val_loader:
                X, y = X.to(DEVICE), y.to(DEVICE)
                outputs = model(X)
                val_loss += F.cross_entropy(outputs, y).item() * X.size(0)
                correct += (torch.argmax(outputs, dim=1) == y).sum().item()
        val_loss /= len(val_ds)
        val_acc = correct / len(val_ds)

        scheduler.step(val_loss)

        if val_acc > best_val_acc:
            best_val_acc = val_acc
            patience = 0
            torch.save(model.state_dict(), BEST_PTH)
            flag = " <- best"
        else:
            patience += 1
            flag = ""

        if epoch <= 3 or epoch % 5 == 0 or flag:
            print(f"  E{epoch:2d} T_loss={train_loss:.3f} V_loss={val_loss:.3f} V_acc={val_acc*100:.1f}%{flag}")

        if patience >= ES_PATIENCE:
            print(f"  Early stop at epoch {epoch}")
            break

    print(f"Best val acc: {best_val_acc*100:.1f}%")

    # Export
    model.load_state_dict(torch.load(BEST_PTH))
    export_weights(model)
    print(f"Exported: {OUTPUT_BLOB}")


if __name__ == "__main__":
    main()
