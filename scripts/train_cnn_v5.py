"""Train single-digit CNN with COLON-ANCHORED patch extraction (v5).

Uses colon ':' as spatial anchor. Digit offsets proportional to ROI width.
Architecture unchanged: 3-conv CNN, 11 classes.
Exports cnn_weights.bin.
"""

import re, random
import numpy as np
from pathlib import Path
from collections import Counter
from PIL import Image

import torch
import torch.nn as nn
import torch.nn.functional as F

SCRIPT_DIR = Path(__file__).parent
APPDATA = Path.home() / "AppData" / "Roaming" / "com.voxalic.app"
TRAINING_DIR = APPDATA / "training_frames"
LOW_SCORE_DIR = APPDATA / "low_score_frames"
BEST_PTH = SCRIPT_DIR / "best_cnn_v5.pth"
OUTPUT_BLOB = Path("src-tauri/resources/cnn_weights.bin")

INPUT_SIZE = 24
NUM_CLASSES = 11
BATCH_SIZE = 64
MAX_EPOCHS = 50
LR = 1e-3
WEIGHT_DECAY = 1e-4
ES_PATIENCE = 12

# Digit X-offsets from colon, as fraction of ROI width
DIGIT_RATIOS = [-0.305, -0.153, 0.115, 0.267]

torch.manual_seed(42)
random.seed(42)
np.random.seed(42)
DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")


def find_colon(gray, est_cx, est_cy):
    """Find colon ':' — two bright dots vertically aligned with dark gap."""
    h, w = gray.shape
    x0, x1 = max(5, est_cx - 20), min(w - 5, est_cx + 20)
    y0, y1 = max(5, est_cy - 20), min(h - 5, est_cy + 20)
    best_score, best_cx, best_cy = -1.0, est_cx, est_cy
    for cy in range(y0, y1, 2):
        for cx in range(x0, x1, 2):
            if cy - 10 < 0 or cy + 10 >= h or cx - 2 < 0 or cx + 2 >= w:
                continue
            upper = gray[cy - 8:cy - 3, cx - 2:cx + 3].mean()
            mid = gray[cy - 2:cy + 3, cx - 2:cx + 3].mean()
            lower = gray[cy + 3:cy + 8, cx - 2:cx + 3].mean()
            score = upper + lower - 2 * mid
            if score > best_score:
                best_score, best_cx, best_cy = score, cx, cy
    return best_cx, best_cy


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


def crop_cnn_patch(gray, cx, cy, size=40):
    h, w = gray.shape
    half = size // 2
    patch = np.zeros((size, size), dtype=np.float32)
    ox0, oy0 = max(0, cx - half), max(0, cy - half)
    ox1, oy1 = min(w, cx + half), min(h, cy + half)
    px0, py0 = ox0 - (cx - half), oy0 - (cy - half)
    patch[py0:py0 + (oy1 - oy0), px0:px0 + (ox1 - ox0)] = gray[oy0:oy1, ox0:ox1]
    img = Image.fromarray(patch.clip(0, 255).astype(np.uint8))
    img = img.resize((INPUT_SIZE, INPUT_SIZE), Image.BILINEAR)
    return np.array(img, dtype=np.float32) / 255.0


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


def generate_dataset(annotated, augment=True):
    X_list, y_list = [], []
    jitters = [(0, 0), (-3, 0), (3, 0), (0, -2), (0, 2), (-2, -1), (2, 1)]
    if not augment:
        jitters = [(0, 0)]

    colon_found = 0
    colon_missed = 0

    for fname, (label_str, path) in annotated.items():
        gray = load_gray(path)
        h, w = gray.shape
        label = [int(c) for c in label_str]
        mid_y = h // 2
        est_colon_x = w // 2
        colon_cx, colon_cy = find_colon(gray, est_colon_x, mid_y)

        if abs(colon_cx - est_colon_x) > 30 or abs(colon_cy - mid_y) > 30:
            colon_missed += 1
            colon_cx, colon_cy = est_colon_x, mid_y
        else:
            colon_found += 1

        # Digit positions proportional to ROI width
        for i, ratio in enumerate(DIGIT_RATIOS):
            cx = int(colon_cx + ratio * w)
            cy = colon_cy
            for dx, dy in jitters:
                patch = crop_cnn_patch(gray, cx + dx, cy + dy)
                if augment:
                    patch = augment_patch(patch)
                X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                y_list.append(label[i])

    # Non-digit patches (~10%)
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
    print(f"  Colon found: {colon_found}/{colon_found + colon_missed} ({colon_found/(colon_found+colon_missed)*100:.0f}%)")
    return X[perm], y[perm]


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


def main():
    print(f"Device: {DEVICE}")
    print("Loading annotations...")
    annotated = load_annotations()
    print(f"  {len(annotated)} labeled frames")

    fnames = list(annotated.keys())
    random.shuffle(fnames)
    n_val = max(20, len(fnames) // 5)
    val_keys = set(fnames[:n_val])
    train_anno = {k: v for k, v in annotated.items() if k not in val_keys}
    val_anno = {k: v for k, v in annotated.items() if k in val_keys}
    print(f"Train: {len(train_anno)}, Val: {len(val_anno)}")

    print("Generating training patches (colon-anchored, ROI-proportional)...")
    X_train, y_train = generate_dataset(train_anno, augment=True)
    print(f"Train: {X_train.shape[0]} patches, dist: {dict(sorted(Counter(y_train.tolist()).items()))}")

    print("Generating validation patches...")
    X_val, y_val = generate_dataset(val_anno, augment=False)
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

    best_val_acc, patience = 0.0, 0
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
        val_loss, correct = 0.0, 0
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
            best_val_acc, patience = val_acc, 0
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
    model.load_state_dict(torch.load(BEST_PTH, weights_only=True))
    export_weights(model)
    print(f"Exported: {OUTPUT_BLOB}")


if __name__ == "__main__":
    main()
