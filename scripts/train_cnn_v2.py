"""Train single-digit CNN on 609 manually labeled frames.

Extracts digit patches from labeled ROI frames using estimated positions,
augments with jitter/brightness/scale/rotation, trains the same 3-conv
architecture used by Rust cnn.rs, exports Conv-BN fused cnn_weights.bin.

Usage:  python scripts/train_cnn_v2.py
Output: best_cnn_v2.pth, src-tauri/resources/cnn_weights.bin
"""

import re
import random
import struct
import numpy as np
from pathlib import Path
from collections import Counter

import torch
import torch.nn as nn
import torch.nn.functional as F
from PIL import Image

# ── Paths ───────────────────────────────────────────────────
SCRIPT_DIR = Path(__file__).parent
APPDATA = Path.home() / "AppData" / "Roaming" / "com.voxalic.app"
TRAINING_DIR = APPDATA / "training_frames"
LOW_SCORE_DIR = APPDATA / "low_score_frames"
BEST_PTH = SCRIPT_DIR / "best_cnn_v2.pth"
OUTPUT_BLOB = Path("src-tauri/resources/cnn_weights.bin")

INPUT_SIZE = 24
NUM_CLASSES = 11  # 0-9 + non-digit
BATCH_SIZE = 64
MAX_EPOCHS = 50
LR = 1e-3
WEIGHT_DECAY = 1e-4
ES_PATIENCE = 12

torch.manual_seed(42)
random.seed(42)
np.random.seed(42)
DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")


# ── 1. Data loading ─────────────────────────────────────────
def load_annotations():
    """Load all annotation CSVs, return {filename: label} for frames that exist."""
    merged = {}
    csv_files = sorted(SCRIPT_DIR.glob("annotations_v*.csv"))

    for csv_path in csv_files:
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
            if not re.match(r"^\d{4}$", label):
                continue
            merged[fname] = label

    # Filter to existing files
    result = {}
    for fname, label in merged.items():
        for d in [TRAINING_DIR, LOW_SCORE_DIR]:
            if (d / fname).exists():
                result[fname] = (label, d / fname)
                break
    return result


def load_gray(path: Path) -> np.ndarray:
    return np.array(Image.open(path).convert("L"), dtype=np.float32)


# ── 2. Patch extraction ─────────────────────────────────────
def digit_positions(w: int, h: int, n_digits: int = 4):
    """Estimate (cx, cy) for each digit. Timer is centred in the ROI."""
    mid_y = h // 2
    spacing = 35
    total_w = n_digits * spacing
    start_x = (w - total_w) // 2 + spacing // 2
    return [(start_x + i * spacing, mid_y) for i in range(n_digits)]


def crop_patch(gray: np.ndarray, cx: int, cy: int, size: int = 40) -> np.ndarray:
    """Crop size×size centred at (cx, cy), resize to 24×24."""
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


def augment_patch(patch: np.ndarray) -> np.ndarray:
    """Random brightness ±20%, scale 0.85-1.15x, ±3° rotation."""
    # Brightness
    patch = patch + random.uniform(-0.20, 0.20)

    # Scale
    scale = random.uniform(0.85, 1.15)
    if abs(scale - 1.0) > 0.02:
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        new_sz = max(8, int(INPUT_SIZE * scale))
        pil = pil.resize((new_sz, new_sz), Image.BILINEAR)
        src = np.array(pil, dtype=np.float32) / 255.0
        patch = np.zeros((INPUT_SIZE, INPUT_SIZE), dtype=np.float32)
        if new_sz <= INPUT_SIZE:
            ox = (INPUT_SIZE - new_sz) // 2
            oy = (INPUT_SIZE - new_sz) // 2
            patch[oy:oy + new_sz, ox:ox + new_sz] = src
        else:
            ox = (new_sz - INPUT_SIZE) // 2
            oy = (new_sz - INPUT_SIZE) // 2
            patch = src[oy:oy + INPUT_SIZE, ox:ox + INPUT_SIZE]

    # Rotation ±3°
    if random.random() < 0.3:
        angle = random.uniform(-3.0, 3.0)
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        pil = pil.rotate(angle, resample=Image.BILINEAR, fillcolor=0)
        patch = np.array(pil, dtype=np.float32) / 255.0

    # Gaussian noise (mild)
    if random.random() < 0.2:
        noise = np.random.normal(0, 0.02, patch.shape)
        patch = patch + noise

    return patch.clip(0, 1)


def generate_dataset(annotated, augment=True):
    """Extract digit patches from all labeled frames."""
    X_list, y_list = [], []
    jitters = [(0, 0), (-4, 0), (4, 0), (0, -3), (0, 3), (-3, -2), (3, 2)]
    if not augment:
        jitters = [(0, 0)]

    for fname, (label, path) in annotated.items():
        gray = load_gray(path)
        h, w = gray.shape
        positions = digit_positions(w, h, 4)

        for i, (cx, cy) in enumerate(positions):
            digit = int(label[i])
            for dx, dy in jitters:
                patch = crop_patch(gray, cx + dx, cy + dy)
                if augment:
                    patch = augment_patch(patch)
                X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                y_list.append(digit)

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
        X_list.append(torch.from_numpy(patch).unsqueeze(0))
        y_list.append(10)

    X = torch.stack(X_list)
    y = torch.tensor(y_list, dtype=torch.long)

    # Shuffle
    perm = torch.randperm(len(X))
    return X[perm], y[perm]


# ── 3. Model (3 conv, same as cnn.rs) ───────────────────────
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
        # After 3 pools: 3×3×32 = 288
        self.fc1 = nn.Linear(3 * 3 * 32, 48)
        self.fc2 = nn.Linear(48, NUM_CLASSES)
        self.dropout = nn.Dropout(0.5)

    def forward(self, x):
        x = self.pool(F.relu(self.bn1(self.conv1(x))))
        x = self.pool(F.relu(self.bn2(self.conv2(x))))
        x = self.pool(F.relu(self.bn3(self.conv3(x))))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        x = self.dropout(x)
        return self.fc2(x)


# ── 4. Training ─────────────────────────────────────────────
def main():
    print(f"Device: {DEVICE}")

    # Load data
    annotated = load_annotations()
    print(f"Labeled frames available: {len(annotated)}")

    # Split 80/20
    fnames = list(annotated.keys())
    random.shuffle(fnames)
    n_val = max(20, len(fnames) // 5)
    val_keys = set(fnames[:n_val])
    train_anno = {k: v for k, v in annotated.items() if k not in val_keys}
    val_anno = {k: v for k, v in annotated.items() if k in val_keys}

    print(f"Train: {len(train_anno)}, Val: {len(val_anno)}")

    # Generate datasets
    print("Generating training patches...")
    X_train, y_train = generate_dataset(train_anno, augment=True)
    print(f"Train: {X_train.shape[0]} patches, class dist: {dict(sorted(Counter(y_train.tolist()).items()))}")

    print("Generating validation patches...")
    X_val, y_val = generate_dataset(val_anno, augment=False)
    print(f"Val:   {X_val.shape[0]} patches, class dist: {dict(sorted(Counter(y_val.tolist()).items()))}")

    # Dataloaders
    train_ds = torch.utils.data.TensorDataset(X_train, y_train)
    val_ds = torch.utils.data.TensorDataset(X_val, y_val)
    train_loader = torch.utils.data.DataLoader(train_ds, BATCH_SIZE, shuffle=True)
    val_loader = torch.utils.data.DataLoader(val_ds, BATCH_SIZE, shuffle=False)

    # Model
    model = DigitCNN().to(DEVICE)
    n_params = sum(p.numel() for p in model.parameters())
    print(f"Params: {n_params:,} ({n_params * 4 / 1024:.0f} KB)")

    optimizer = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
        optimizer, mode="min", factor=0.5, patience=5
    )

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

    # ── Export fused weights ──
    model.load_state_dict(torch.load(BEST_PTH))
    export_weights(model)
    print(f"Exported: {OUTPUT_BLOB}")


# ── 5. Export (Conv-BN fuse → cnn_weights.bin) ─────────────
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
        cw = state[f"conv{ci}.weight"]
        cb = state[f"conv{ci}.bias"]
        bw = state[f"bn{ci}.weight"]
        bb = state[f"bn{ci}.bias"]
        bm = state[f"bn{ci}.running_mean"]
        bv = state[f"bn{ci}.running_var"]
        fw, fb = fuse_conv_bn(cw, cb, bw, bb, bm, bv)
        blob += fw.astype(np.float32).tobytes()
        blob += fb.astype(np.float32).tobytes()
        print(f"  conv{ci}: w{fw.shape} b{fb.shape}")

    for fc in ["fc1", "fc2"]:
        w = state[f"{fc}.weight"]
        b = state[f"{fc}.bias"]
        blob += w.astype(np.float32).tobytes()
        blob += b.astype(np.float32).tobytes()
        print(f"  {fc}: w{w.shape} b{b.shape}")

    OUTPUT_BLOB.parent.mkdir(parents=True, exist_ok=True)
    with open(OUTPUT_BLOB, "wb") as f:
        f.write(blob)

    n_f32 = len(blob) // 4
    print(f"  Total: {len(blob):,} bytes, {n_f32:,} f32s")


if __name__ == "__main__":
    main()
