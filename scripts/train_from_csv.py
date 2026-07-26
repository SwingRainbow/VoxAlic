"""
Step 3: CNN fine-tune from annotated real-game frames.

1. Parse CSV annotations (219 frames, 876 digit labels)
2. For each frame: crop digit patches at estimated positions, jitter ±4px
3. Fine-tune the template-trained CNN (best.pth) on real patches
4. Export new cnn_weights.bin

Usage:  python scripts/train_from_csv.py
Output: best_real.pth, src-tauri/resources/cnn_weights.bin
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

# ── Paths ────────────────────────────────────────────────────────────────
CSV_PATH = Path("scripts/_annotations.csv")
FRAMES_DIR = Path.home() / "AppData" / "Roaming" / "com.voxalic.app" / "low_score_frames"
OLD_CKPT = Path("best.pth")
NEW_CKPT = Path("best_real.pth")
OUTPUT_BLOB = Path("src-tauri/resources/cnn_weights.bin")

INPUT_SIZE  = 24
NUM_CLASSES = 11  # 0-9 + non-digit
BATCH_SIZE  = 64
MAX_EPOCHS  = 30
LR          = 1e-3  # train all layers from template init
WEIGHT_DECAY = 1e-4
ES_PATIENCE = 10

torch.manual_seed(42)
random.seed(42)
np.random.seed(42)


# ══════════════════════════════════════════════════════════════════════════
# 1. Parse CSV + load frames
# ══════════════════════════════════════════════════════════════════════════

def parse_csv() -> list[tuple[str, str]]:
    """Parse CSV annotations, return [(filename, time_label), ...].
    time_label is always 4 digits like '0359' meaning '03:59'."""
    with open(CSV_PATH, "rb") as f:
        raw = f.read()
    # CSV has literal \n — replace with actual newlines
    raw = raw.replace(b"\x5c\x6e", b"\x0a")
    text = raw.decode("utf-8")

    entries = []
    for line in text.split("\n"):
        line = line.strip()
        if not line or "filename" in line:
            continue
        parts = line.split(",")
        if len(parts) < 2:
            continue
        fname, label = parts[0], parts[1]
        if label == "SKIP":
            continue
        if re.match(r"^\d{3}$", label):
            label = "0" + label  # pad '359' → '0359'
        if re.match(r"^\d{4}$", label):
            entries.append((fname, label))

    return entries


def load_frame(filename: str) -> np.ndarray:
    """Load a grayscale ROI PNG, return (H, W) float32 array 0-255."""
    path = FRAMES_DIR / filename
    if not path.exists():
        raise FileNotFoundError(f"Missing frame: {path}")
    img = Image.open(path).convert("L")
    return np.array(img, dtype=np.float32)


# ══════════════════════════════════════════════════════════════════════════
# 2. Digit position estimation
# ══════════════════════════════════════════════════════════════════════════

def digit_positions(roi_w: int, roi_h: int, label: str) -> list[tuple[int, int]]:
    """Estimate (cx, cy) pixel positions for each digit in `label` (4 digits).

    The timer text is roughly centred horizontally in the ROI and sits near
    vertical centre.  Digits are ~30 px wide at 1080p, spaced ~35 px apart.
    For MM:SS the colon sits between digit 2 and 3.
    """
    mid_y = roi_h // 2
    n = len(label)  # 4 for MM:SS

    # The timer is centred; estimate total width ≈ n * 35 px
    total_w = n * 35
    start_x = (roi_w - total_w) // 2 + 17  # +17 = half digit width

    positions = []
    for i in range(n):
        cx = start_x + i * 35
        positions.append((cx, mid_y))
    return positions


def crop_patch(gray: np.ndarray, cx: int, cy: int, size: int = 36) -> np.ndarray:
    """Crop a size×size patch centred at (cx, cy), then resize to 24×24.
    Larger crop + resize gives tolerance for HUD jitter (±6 px)."""
    from PIL import Image as PILImage
    h, w = gray.shape
    half = size // 2
    x0 = cx - half
    y0 = cy - half

    patch = np.zeros((size, size), dtype=np.float32)
    ox0 = max(0, x0)
    oy0 = max(0, y0)
    ox1 = min(w, x0 + size)
    oy1 = min(h, y0 + size)
    px0 = ox0 - x0
    py0 = oy0 - y0
    px1 = px0 + (ox1 - ox0)
    py1 = py0 + (oy1 - oy0)
    patch[py0:py1, px0:px1] = gray[oy0:oy1, ox0:ox1]

    # Resize to INPUT_SIZE
    img = PILImage.fromarray((patch.clip(0, 255)).astype(np.uint8))
    img = img.resize((INPUT_SIZE, INPUT_SIZE), PILImage.BILINEAR)
    return np.array(img, dtype=np.float32) / 255.0


# ══════════════════════════════════════════════════════════════════════════
# 3. Dataset generation from real frames
# ══════════════════════════════════════════════════════════════════════════

def generate_real_dataset(
    entries: list[tuple[str, str]],
    augment: bool = True,
) -> tuple[torch.Tensor, torch.Tensor]:
    """Extract digit patches from annotated frames, optionally augment.

    Each entry produces 3 jittered crops per digit (offset ±4 px) for a
    total of ~12 patches per frame.  With 219 frames × ~12 = ~2600 patches.
    Light augmentation (only brightness + scale) is applied during training.
    """
    X_list, y_list = [], []
    jitters = [(0, 0), (-3, 0), (3, 0), (0, -2), (0, 2)] if augment else [(0, 0)]

    for fname, label in entries:
        try:
            gray = load_frame(fname)
        except FileNotFoundError:
            continue

        h, w = gray.shape
        positions = digit_positions(w, h, label)

        for i, (cx, cy) in enumerate(positions):
            digit = int(label[i])
            for dx, dy in jitters:
                patch = crop_patch(gray, cx + dx, cy + dy)

                if augment:
                    patch = light_augment(patch)

                X_list.append(torch.from_numpy(patch).unsqueeze(0))  # (1, 24, 24)
                y_list.append(digit)

    # Add non-digit patches from random regions of random frames
    non_digit_count = len(X_list) // 10  # ~10% non-digit
    rng = random.Random(42)
    for _ in range(non_digit_count):
        fname = rng.choice(entries)[0]
        try:
            gray = load_frame(fname)
        except FileNotFoundError:
            continue
        h, w = gray.shape
        # Random crop from edge regions (where timer is unlikely to be)
        cx = rng.randint(0, w - INPUT_SIZE)
        cy = rng.randint(0, h - INPUT_SIZE)
        patch = gray[cy:cy + INPUT_SIZE, cx:cx + INPUT_SIZE] / 255.0
        X_list.append(torch.from_numpy(patch).unsqueeze(0))
        y_list.append(10)  # non-digit

    X = torch.stack(X_list)
    y = torch.tensor(y_list, dtype=torch.long)
    return X, y


def light_augment(patch: np.ndarray) -> np.ndarray:
    """Light augmentation: brightness ±20%, scale 0.85-1.15x, ±2° rotation."""
    # Brightness
    b = random.uniform(-0.20, 0.20)
    patch = patch + b

    # Scale
    scale = random.uniform(0.85, 1.15)
    if abs(scale - 1.0) > 0.01:
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        new_sz = max(8, int(INPUT_SIZE * scale))
        pil = pil.resize((new_sz, new_sz), Image.BILINEAR)
        src = np.array(pil, dtype=np.float32) / 255.0
        result = np.zeros((INPUT_SIZE, INPUT_SIZE), dtype=np.float32)
        if new_sz <= INPUT_SIZE:
            ox = (INPUT_SIZE - new_sz) // 2
            oy = (INPUT_SIZE - new_sz) // 2
            result[oy:oy + new_sz, ox:ox + new_sz] = src
        else:
            # Center-crop back to INPUT_SIZE
            ox = (new_sz - INPUT_SIZE) // 2
            oy = (new_sz - INPUT_SIZE) // 2
            result = src[oy:oy + INPUT_SIZE, ox:ox + INPUT_SIZE]
        patch = result

    # Rotation ±2°
    if random.random() < 0.3:
        angle = random.uniform(-2.0, 2.0)
        pil = Image.fromarray((patch.clip(0, 1) * 255).astype(np.uint8))
        pil = pil.rotate(angle, resample=Image.BILINEAR, fillcolor=0)
        patch = np.array(pil, dtype=np.float32) / 255.0

    return patch.clip(0, 1)


# ══════════════════════════════════════════════════════════════════════════
# 4. Model (same architecture as Phase 1)
# ══════════════════════════════════════════════════════════════════════════

class DigitCNN(nn.Module):
    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.bn1 = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.bn2 = nn.BatchNorm2d(16)
        self.conv3 = nn.Conv2d(16, 32, 3, padding=1)
        self.bn3 = nn.BatchNorm2d(32)
        self.fc1 = nn.Linear(32 * 3 * 3, 48)
        self.fc2 = nn.Linear(48, NUM_CLASSES)
        self.drop = nn.Dropout(0.3)

    def forward(self, x):
        x = F.max_pool2d(F.relu(self.bn1(self.conv1(x))), 2)
        x = F.max_pool2d(F.relu(self.bn2(self.conv2(x))), 2)
        x = F.max_pool2d(F.relu(self.bn3(self.conv3(x))), 2)
        x = torch.flatten(x, 1)
        x = F.relu(self.fc1(x))
        x = self.drop(x)
        x = self.fc2(x)
        return x

    def param_count(self) -> int:
        return sum(p.numel() for p in self.parameters())


# ══════════════════════════════════════════════════════════════════════════
# 5. Training (fine-tune from template weights)
# ══════════════════════════════════════════════════════════════════════════

def train(model: DigitCNN, X_train, y_train, X_val, y_val):
    ds = torch.utils.data.TensorDataset(X_train, y_train)
    dl = torch.utils.data.DataLoader(ds, batch_size=BATCH_SIZE, shuffle=True)

    # Fine-tune all layers (real grayscale frames differ from template statistics)
    opt = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    sched = torch.optim.lr_scheduler.ReduceLROnPlateau(
        opt, mode="min", patience=4, factor=0.5,
    )
    loss_fn = nn.CrossEntropyLoss()

    best_loss = float("inf")
    best_epoch = 0
    wait = 0

    for epoch in range(1, MAX_EPOCHS + 1):
        model.train()
        train_loss = 0.0
        for xb, yb in dl:
            opt.zero_grad()
            loss = loss_fn(model(xb), yb)
            loss.backward()
            opt.step()
            train_loss += loss.item() * xb.size(0)
        train_loss /= len(ds)

        model.eval()
        with torch.no_grad():
            val_out = model(X_val)
            val_loss = loss_fn(val_out, y_val).item()
            val_preds = val_out.argmax(dim=1)
            val_acc = (val_preds == y_val).float().mean().item()

        sched.step(val_loss)

        if epoch == 1 or epoch % 5 == 0 or val_loss < best_loss:
            print(f"epoch {epoch:2d}  train_loss={train_loss:.4f}  "
                  f"val_loss={val_loss:.4f}  val_acc={val_acc:.3f}  "
                  f"lr={opt.param_groups[0]['lr']:.2e}"
                  + (" *" if val_loss < best_loss else ""))

        if val_loss < best_loss:
            best_loss = val_loss
            best_epoch = epoch
            wait = 0
            torch.save(model.state_dict(), NEW_CKPT)
        else:
            wait += 1
            if wait >= ES_PATIENCE:
                print(f"Early stop at epoch {epoch} (best was {best_epoch})")
                break

    model.load_state_dict(torch.load(NEW_CKPT, weights_only=True))
    return model


# ══════════════════════════════════════════════════════════════════════════
# 6. Evaluation + export
# ══════════════════════════════════════════════════════════════════════════

def evaluate(model: DigitCNN, X_val, y_val) -> np.ndarray:
    model.eval()
    with torch.no_grad():
        logits = model(X_val)
        probs = F.softmax(logits, dim=1)
        preds = logits.argmax(dim=1)
        confs = probs.max(dim=1).values

    cm = np.zeros((NUM_CLASSES, NUM_CLASSES), dtype=int)
    for p, t in zip(preds.tolist(), y_val.tolist()):
        cm[t][p] += 1

    print("\n── Confusion Matrix (rows=true, cols=pred) ──")
    header = "     " + "".join(f"{i:>4}" for i in range(NUM_CLASSES))
    print(header)
    for i in range(NUM_CLASSES):
        label = f"  {i}: " if i < 10 else "  N: "
        print(label + "".join(
            f"{cm[i][j]:4}" if cm[i][j] else "    "
            for j in range(NUM_CLASSES)))

    print("\n── Per-Class ──")
    names = [str(i) for i in range(10)] + ["NON"]
    for i in range(NUM_CLASSES):
        tp = cm[i][i]
        fp = cm[:, i].sum() - tp
        fn = cm[i, :].sum() - tp
        prec = tp / (tp + fp) if (tp + fp) else 0.0
        rec  = tp / (tp + fn) if (tp + fn) else 0.0
        f1 = 2 * prec * rec / (prec + rec) if (prec + rec) else 0.0
        count = cm[i].sum()
        print(f"  {names[i]:>3s}  prec={prec:.3f}  rec={rec:.3f}  f1={f1:.3f}  "
              f"({cm[i][i]}/{count})")

    total_correct = cm.diagonal().sum()
    total = cm.sum()
    print(f"\n  overall accuracy: {total_correct}/{total} = {total_correct / total:.4f}")

    correct_confs = confs[preds == y_val]
    wrong_confs = confs[preds != y_val]
    if len(wrong_confs) > 0:
        print(f"\n── Confidence ──")
        print(f"  correct: mean={correct_confs.mean():.3f}  median={correct_confs.median():.3f}")
        print(f"  wrong:   mean={wrong_confs.mean():.3f}  median={wrong_confs.median():.3f}")
        high_conf_err = ((confs > 0.88) & (preds != y_val)).float().mean()
        print(f"  high-conf (>0.88) error rate: {high_conf_err:.4f}")

    return cm


def fuse_conv_bn(conv: nn.Conv2d, bn: nn.BatchNorm2d):
    W = conv.weight.data.numpy().astype(np.float32)
    b_c = conv.bias.data.numpy().astype(np.float32) if conv.bias is not None \
        else np.zeros(W.shape[0], dtype=np.float32)
    gamma = bn.weight.data.numpy().astype(np.float32)
    beta  = bn.bias.data.numpy().astype(np.float32)
    mean  = bn.running_mean.data.numpy().astype(np.float32)
    var   = bn.running_var.data.numpy().astype(np.float32)
    eps   = bn.eps
    std = np.sqrt(var + eps)
    a = gamma / std
    b = a * (b_c - mean) + beta
    W_fused = W * a.reshape(-1, 1, 1, 1)
    return W_fused, b


def export_blob(model: DigitCNN, path: Path):
    model.eval()
    c1w, c1b = fuse_conv_bn(model.conv1, model.bn1)
    c2w, c2b = fuse_conv_bn(model.conv2, model.bn2)
    c3w, c3b = fuse_conv_bn(model.conv3, model.bn3)
    fc1w = model.fc1.weight.data.numpy().astype(np.float32)
    fc1b = model.fc1.bias.data.numpy().astype(np.float32)
    fc2w = model.fc2.weight.data.numpy().astype(np.float32)
    fc2b = model.fc2.bias.data.numpy().astype(np.float32)

    parts = [c1w, c1b, c2w, c2b, c3w, c3b, fc1w, fc1b, fc2w, fc2b]
    total = sum(p.nbytes for p in parts)

    with open(path, "wb") as f:
        for arr in parts:
            f.write(arr.tobytes())

    print(f"\nExported {total} bytes ({total // 4} f32s) to {path}")


# ══════════════════════════════════════════════════════════════════════════
# Main
# ══════════════════════════════════════════════════════════════════════════

def main():
    print("Parsing CSV...")
    entries = parse_csv()
    print(f"  {len(entries)} annotated frames")

    print("Generating dataset from real frames...")
    X, y = generate_real_dataset(entries, augment=True)
    print(f"  {X.shape[0]} patches, {len(torch.unique(y))} classes")
    dc = Counter(y.tolist())
    for d in range(NUM_CLASSES):
        print(f"    class {d if d < 10 else 'N'}: {dc.get(d, 0)}")

    # Split: 85% train, 15% val (stratified-ish: just random)
    n = X.shape[0]
    idx = torch.randperm(n)
    split = int(n * 0.85)
    X_train, y_train = X[idx[:split]], y[idx[:split]]
    X_val, y_val = X[idx[split:]], y[idx[split:]]
    print(f"  train: {X_train.shape[0]}  val: {X_val.shape[0]}")

    # Load template-trained weights as starting point
    model = DigitCNN()
    if OLD_CKPT.exists():
        print(f"\nLoading template-trained weights from {OLD_CKPT}...")
        state = torch.load(OLD_CKPT, weights_only=True)
        model.load_state_dict(state)
        print(f"  {model.param_count():,} parameters")
    else:
        print(f"\nWARNING: {OLD_CKPT} not found, training from scratch")
        print(f"  {model.param_count():,} parameters")

    print("\nFine-tuning on real frames (Conv1+Conv2 frozen)...")
    model = train(model, X_train, y_train, X_val, y_val)

    print("\nEvaluating...")
    cm = evaluate(model, X_val, y_val)

    print("\nExporting...")
    export_blob(model, OUTPUT_BLOB)

    # Sanity: predict on a few training samples
    print("\n── Sample predictions (training set) ──")
    model.eval()
    with torch.no_grad():
        for d in range(10):
            # Find a training sample of this digit
            mask = y_train == d
            if mask.any():
                idx_d = torch.where(mask)[0][0]
                t = X_train[idx_d].unsqueeze(0)
                logits = model(t)
                probs = F.softmax(logits, dim=1)[0]
                top3 = probs.topk(3)
                pred = probs.argmax().item()
                print(f"  true={d}  pred={pred}  "
                      + "  ".join(f"p({int(c)})={p:.3f}"
                                  for c, p in zip(top3.indices, top3.values)))


if __name__ == "__main__":
    main()
