"""
Phase 1: CNN digit classifier training.

Trains a 3-layer CNN on augmented templates, evaluates, and exports
Conv-BN-fused weights as a binary blob for Rust deployment (Phase 2).

Usage:  python scripts/train_cnn.py
Output: src-tauri/resources/cnn_weights.bin  (binary weight blob)
        best.pth                              (PyTorch checkpoint)
"""

import random
import struct
import numpy as np
from pathlib import Path
from PIL import Image, ImageFilter, ImageOps

import torch
import torch.nn as nn
import torch.nn.functional as F

# ── Config ──────────────────────────────────────────────────────────────
TEMPLATE_DIR = Path("src-tauri/resources/digit_templates")
OUTPUT_BLOB = Path("src-tauri/resources/cnn_weights.bin")
CHECKPOINT  = Path("best.pth")

INPUT_SIZE   = 24
NUM_CLASSES  = 11          # 0-9 + non-digit
TRAIN_PER    = 300         # variants per class
VAL_PER      = 50          # validation per class
BATCH_SIZE   = 64
MAX_EPOCHS   = 50
LR           = 0.001
WEIGHT_DECAY = 1e-4
LR_PATIENCE  = 4           # ReduceLROnPlateau
ES_PATIENCE  = 8           # early stopping
DROPOUT      = 0.3

torch.manual_seed(42)
random.seed(42)
np.random.seed(42)


# ══════════════════════════════════════════════════════════════════════════
# 1. Load templates
# ══════════════════════════════════════════════════════════════════════════

def load_templates() -> dict[int, Image.Image]:
    """Load 10 PNG digit templates as grayscale PIL Images.
    Templates are dark-on-light; we invert so digits = 1.0, bg = 0.0
    to match the OCR binarization convention.
    """
    templates: dict[int, Image.Image] = {}
    for digit in range(10):
        path = TEMPLATE_DIR / f"{digit}.png"
        img = Image.open(path).convert("L")          # grayscale
        img = ImageOps.invert(img)                   # digits→bright, bg→dark
        templates[digit] = img
    return templates


# ══════════════════════════════════════════════════════════════════════════
# 2. Augmentation primitives
# ══════════════════════════════════════════════════════════════════════════

def _to_tensor(img: Image.Image) -> torch.Tensor:
    arr = np.array(img, dtype=np.float32) / 255.0
    return torch.from_numpy(arr).unsqueeze(0)  # (1, H, W)

def _from_tensor(t: torch.Tensor) -> Image.Image:
    arr = (t.squeeze(0).numpy() * 255.0).clip(0, 255).astype(np.uint8)
    return Image.fromarray(arr, mode="L")


def augment(img: Image.Image) -> torch.Tensor:
    """Apply random augmentation pipeline. Returns (1, 24, 24) tensor."""
    w, h = img.size

    # Scale 0.7–1.3x (always applied — covers HUD 50-150%)
    scale = random.uniform(0.7, 1.3)
    new_w = max(8, int(w * scale))
    new_h = max(8, int(h * scale))
    img = img.resize((new_w, new_h), Image.BILINEAR)

    # Rotation ±3° — simulates ROI alignment jitter
    if random.random() < 0.4:
        angle = random.uniform(-3.0, 3.0)
        img = img.rotate(angle, resample=Image.BILINEAR, expand=False,
                         fillcolor=0)

    # Elastic deformation — simulates heat-haze / water refraction
    if random.random() < 0.10:
        img = _elastic_deform(img, alpha=random.uniform(1.0, 3.0),
                              sigma=random.uniform(0.05, 0.1))

    # Pad or crop to INPUT_SIZE × INPUT_SIZE, then convert to tensor
    img = _resize_pad_to(img, INPUT_SIZE)
    t = _to_tensor(img)  # (1, 24, 24), values 0-1

    # Brightness ±50% (always applied)
    b = random.uniform(-0.50, 0.50)
    t = t + b

    # Contrast ±30% (always applied)
    c = random.uniform(0.70, 1.30)
    mean = t.mean()
    t = (t - mean) * c + mean

    # Gaussian noise σ=0.05–0.15
    if random.random() < 0.60:
        sigma = random.uniform(0.05, 0.15)
        t = t + torch.randn_like(t) * sigma

    # Salt & pepper noise — simulates screen tearing
    if random.random() < 0.20:
        p = random.uniform(0.01, 0.03)
        mask = torch.rand_like(t) < p
        t[mask] = torch.rand(1).item()

    # Gaussian blur σ=0.3–1.5
    if random.random() < 0.30:
        sigma = random.uniform(0.3, 1.5)
        t = _gaussian_blur(t, sigma)

    # Motion blur kernel 3–7 — simulates fast turning
    if random.random() < 0.20:
        k = random.choice([3, 5, 7])
        t = _motion_blur(t, k)

    # Random occlusion 0–15% area
    if random.random() < 0.20:
        frac = random.uniform(0.02, 0.15)
        kind = random.choice(["rect", "ellipse"])
        t = _occlude(t, frac, kind)

    t = t.clamp(0.0, 1.0)
    return t


def _resize_pad_to(img: Image.Image, size: int) -> Image.Image:
    """Resize so the longer side ≤ size, then center-pad to size×size."""
    w, h = img.size
    scale = size / max(w, h)
    if scale < 1.0:
        new_w, new_h = int(w * scale), int(h * scale)
        img = img.resize((max(1, new_w), max(1, new_h)), Image.BILINEAR)

    canvas = Image.new("L", (size, size), 0)
    w, h = img.size
    ox = (size - w) // 2
    oy = (size - h) // 2
    canvas.paste(img, (ox, oy))
    return canvas


def _elastic_deform(img: Image.Image, alpha: float, sigma: float) -> Image.Image:
    """Elastic deformation using random displacement field."""
    arr = np.array(img, dtype=np.float32)
    h, w = arr.shape

    dx = np.random.randn(h, w).astype(np.float32) * sigma
    dy = np.random.randn(h, w).astype(np.float32) * sigma
    from scipy.ndimage import gaussian_filter
    dx = gaussian_filter(dx, sigma * w) * alpha
    dy = gaussian_filter(dy, sigma * h) * alpha

    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    x_new = x + dx
    y_new = y + dy

    from scipy.ndimage import map_coordinates
    deformed = map_coordinates(arr, [y_new.ravel(), x_new.ravel()],
                               order=1, mode='constant', cval=0)
    deformed = deformed.reshape(h, w).clip(0, 255).astype(np.uint8)
    return Image.fromarray(deformed, mode="L")


def _gaussian_blur(t: torch.Tensor, sigma: float) -> torch.Tensor:
    """Apply Gaussian blur to a (1,H,W) tensor via PIL round-trip."""
    img = _from_tensor(t.clamp(0, 1))
    r = max(1, int(sigma * 3))
    img = img.filter(ImageFilter.GaussianBlur(radius=r))
    return _to_tensor(img)


def _motion_blur(t: torch.Tensor, kernel: int) -> torch.Tensor:
    """Directional motion blur with random angle."""
    img = _from_tensor(t.clamp(0, 1))
    angle = random.uniform(0, 360)
    # Build a line kernel in PIL by scaling down + up
    tmp = Image.new("L", (kernel * 4, kernel * 4), 0)
    kimg = Image.new("L", (kernel * 4, kernel), 255)
    kimg = kimg.rotate(angle, expand=False, fillcolor=0)
    tmp.paste(kimg, (0, (kernel * 4 - kernel) // 2))
    # Use the tmp as a crude blur kernel via resize trick
    blurred = img.filter(ImageFilter.GaussianBlur(radius=kernel // 2))
    # Blend with original for directional feel
    result = Image.blend(blurred, img, 0.5)
    return _to_tensor(result)


def _occlude(t: torch.Tensor, frac: float, kind: str) -> torch.Tensor:
    """Randomly occlude a rectangle or ellipse covering ~frac of the image."""
    h, w = t.shape[1], t.shape[2]
    area = w * h * frac
    if kind == "rect":
        rw = int(np.sqrt(area * random.uniform(0.5, 2.0)))
        rh = int(area / rw) if rw > 0 else 2
    else:
        r = int(np.sqrt(area / np.pi))
        rw = rh = max(2, r * 2)

    x0 = random.randint(0, max(0, w - rw))
    y0 = random.randint(0, max(0, h - rh))

    if kind == "rect":
        t[:, y0:y0+rh, x0:x0+rw] = 0.0
    else:
        yy, xx = torch.meshgrid(
            torch.arange(h, dtype=torch.float32),
            torch.arange(w, dtype=torch.float32), indexing='ij')
        cx, cy = x0 + rw/2, y0 + rh/2
        d2 = ((xx - cx) / (rw/2))**2 + ((yy - cy) / (rh/2))**2
        t[:, d2 <= 1.0] = 0.0
    return t


# ══════════════════════════════════════════════════════════════════════════
# 3. Negative (non-digit) samples
# ══════════════════════════════════════════════════════════════════════════

def gen_negative(templates: dict[int, Image.Image]) -> torch.Tensor:
    """Generate a single negative sample (class 10 = non-digit)."""
    kind = random.choices(
        ["noise", "struct", "half", "overlap"],
        weights=[0.30, 0.25, 0.25, 0.20])[0]

    if kind == "noise":
        # Pure uniform or Gaussian noise
        if random.random() < 0.5:
            t = torch.rand(1, INPUT_SIZE, INPUT_SIZE)
        else:
            t = torch.randn(1, INPUT_SIZE, INPUT_SIZE) * 0.3 + 0.5
        return t.clamp(0, 1)

    if kind == "struct":
        # Striped/grid noise to simulate HUD background texture
        t = torch.zeros(1, INPUT_SIZE, INPUT_SIZE)
        stripe_w = random.randint(1, 4)
        for i in range(0, INPUT_SIZE, stripe_w * 2):
            t[:, i:i+stripe_w, :] = random.uniform(0.2, 0.6)
        # Add some horizontal stripes occasionally
        if random.random() < 0.5:
            for i in range(0, INPUT_SIZE, random.randint(2, 6)):
                t[:, :, i] = random.uniform(0.1, 0.4)
        return t.clamp(0, 1)

    if kind == "half":
        # Half of a digit (simulates edge-of-ROI clipping)
        d = random.randint(0, 9)
        img = templates[d].copy()
        w, h = img.size
        clip_side = random.choice(["left", "right"])
        if clip_side == "left":
            img = img.crop((0, 0, w * random.uniform(0.3, 0.5), h))
        else:
            img = img.crop((w * random.uniform(0.5, 0.7), 0, w, h))
        img = _resize_pad_to(img, INPUT_SIZE)
        t = _to_tensor(img)
        # Add noise to make it less clearly a digit
        t = t + torch.randn_like(t) * 0.05
        return t.clamp(0, 1)

    # kind == "overlap"
    d1, d2 = random.sample(range(10), 2)
    t1 = augment(templates[d1])
    t2 = augment(templates[d2])
    # Blend with random offset
    ox = random.randint(-6, 6)
    oy = random.randint(-4, 4)
    canvas = torch.zeros(1, INPUT_SIZE, INPUT_SIZE)
    # Paste t1 at center, t2 offset
    x1 = max(0, (INPUT_SIZE - t1.shape[2]) // 2)
    y1 = max(0, (INPUT_SIZE - t1.shape[1]) // 2)
    x2 = x1 + ox
    y2 = y1 + oy
    _paste_tensor(canvas, t1, x1, y1)
    _paste_tensor(canvas, t2, x2, y2, alpha=random.uniform(0.4, 0.7))
    return canvas.clamp(0, 1)


def _paste_tensor(canvas: torch.Tensor, patch: torch.Tensor,
                  x: int, y: int, alpha: float = 1.0):
    """Paste patch onto canvas at (x,y), clamping to bounds."""
    _, ph, pw = patch.shape
    _, ch, cw = canvas.shape
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(cw, x + pw), min(ch, y + ph)
    if x1 <= x0 or y1 <= y0:
        return
    px0 = x0 - x
    py0 = y0 - y
    px1 = px0 + (x1 - x0)
    py1 = py0 + (y1 - y0)
    if alpha >= 1.0:
        canvas[:, y0:y1, x0:x1] = patch[:, py0:py1, px0:px1]
    else:
        canvas[:, y0:y1, x0:x1] = torch.max(
            canvas[:, y0:y1, x0:x1],
            patch[:, py0:py1, px0:px1] * alpha)


# ══════════════════════════════════════════════════════════════════════════
# 4. Dataset generation
# ══════════════════════════════════════════════════════════════════════════

def generate_dataset(templates: dict[int, Image.Image],
                     n_train: int, n_val: int):
    """Generate train and validation tensors.

    Returns (X_train, y_train, X_val, y_val) where X is (N,1,24,24), y is (N,).
    Class 0-9 = digits, class 10 = non-digit.
    """
    X_train_list, y_train_list = [], []
    X_val_list, y_val_list = [], []

    for cls in range(NUM_CLASSES):
        for i in range(n_train + n_val):
            if cls < 10:
                img = templates[cls].copy()
                t = augment(img)
            else:
                t = gen_negative(templates)

            # Validation samples: weaker augmentation (only scale + brightness)
            if i >= n_train:
                if cls < 10:
                    # Lighter augmentation for validation
                    t_val = _val_augment(templates[cls])
                    X_val_list.append(t_val)
                else:
                    X_val_list.append(t)
                y_val_list.append(cls)
            else:
                X_train_list.append(t)
                y_train_list.append(cls)

    X_train = torch.stack(X_train_list)
    y_train = torch.tensor(y_train_list, dtype=torch.long)
    X_val = torch.stack(X_val_list)
    y_val = torch.tensor(y_val_list, dtype=torch.long)

    return X_train, y_train, X_val, y_val


def _val_augment(img: Image.Image) -> torch.Tensor:
    """Light augmentation for validation: only scale + mild brightness."""
    w, h = img.size
    scale = random.uniform(0.7, 1.3)
    new_w = max(8, int(w * scale))
    new_h = max(8, int(h * scale))
    img = img.resize((new_w, new_h), Image.BILINEAR)
    img = _resize_pad_to(img, INPUT_SIZE)
    t = _to_tensor(img)
    t = t + random.uniform(-0.15, 0.15)  # mild brightness
    return t.clamp(0, 1)


# ══════════════════════════════════════════════════════════════════════════
# 5. Model
# ══════════════════════════════════════════════════════════════════════════

class DigitCNN(nn.Module):
    """3-layer CNN, 15.5K params, designed for hand-written Rust forward pass."""

    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.bn1   = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.bn2   = nn.BatchNorm2d(16)
        self.conv3 = nn.Conv2d(16, 32, 3, padding=1)
        self.bn3   = nn.BatchNorm2d(32)
        self.fc1   = nn.Linear(32 * 3 * 3, 48)
        self.fc2   = nn.Linear(48, NUM_CLASSES)
        self.drop   = nn.Dropout(DROPOUT)

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
# 6. Training
# ══════════════════════════════════════════════════════════════════════════

def train(model: DigitCNN, X_train, y_train, X_val, y_val):
    ds = torch.utils.data.TensorDataset(X_train, y_train)
    dl = torch.utils.data.DataLoader(ds, batch_size=BATCH_SIZE, shuffle=True)

    opt = torch.optim.AdamW(model.parameters(), lr=LR,
                            weight_decay=WEIGHT_DECAY)
    sched = torch.optim.lr_scheduler.ReduceLROnPlateau(
        opt, mode='min', patience=LR_PATIENCE, factor=0.5)
    loss_fn = nn.CrossEntropyLoss()

    best_loss = float('inf')
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
            torch.save(model.state_dict(), CHECKPOINT)
        else:
            wait += 1
            if wait >= ES_PATIENCE:
                print(f"Early stop at epoch {epoch} (best was {best_epoch})")
                break

    model.load_state_dict(torch.load(CHECKPOINT, weights_only=True))
    return model


# ══════════════════════════════════════════════════════════════════════════
# 7. Evaluation
# ══════════════════════════════════════════════════════════════════════════

def evaluate(model: DigitCNN, X_val, y_val) -> np.ndarray:
    """Print per-class metrics and return confusion matrix."""
    model.eval()
    with torch.no_grad():
        logits = model(X_val)
        probs = F.softmax(logits, dim=1)
        preds = logits.argmax(dim=1)
        confs = probs.max(dim=1).values

    # Confusion matrix
    cm = np.zeros((NUM_CLASSES, NUM_CLASSES), dtype=int)
    for p, t in zip(preds.tolist(), y_val.tolist()):
        cm[t][p] += 1

    print("\n── Confusion Matrix (rows=true, cols=pred) ──")
    header = "     " + "".join(f"{i:>4}" for i in range(NUM_CLASSES))
    print(header)
    for i in range(NUM_CLASSES):
        label = f"  {i}: " if i < 10 else "  N: "
        print(label + "".join(f"{cm[i][j]:4}" if cm[i][j] else "    "
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
        print(f"  {names[i]:>3s}  prec={prec:.3f}  rec={rec:.3f}  f1={f1:.3f}  "
              f"({cm[i][i]}/{cm[i].sum()})")

    total_correct = cm.diagonal().sum()
    total = cm.sum()
    print(f"\n  overall accuracy: {total_correct}/{total} = {total_correct/total:.4f}")

    # Confidence calibration
    correct_confs = confs[preds == y_val]
    wrong_confs   = confs[preds != y_val]
    print(f"\n── Confidence ──")
    print(f"  correct: mean={correct_confs.mean():.3f}  median={correct_confs.median():.3f}  "
          f"<0.6={((correct_confs < 0.60).float().mean()):.3f}")
    print(f"  wrong:   mean={wrong_confs.mean():.3f}  median={wrong_confs.median():.3f}  "
          f">0.88={((wrong_confs > 0.88).float().mean()):.3f}")

    # High-confidence error rate
    high_conf_wrong = ((confs > 0.88) & (preds != y_val)).float().mean()
    print(f"  high-conf (>0.88) error rate: {high_conf_wrong:.4f}")

    return cm


# ══════════════════════════════════════════════════════════════════════════
# 8. Conv-BN fusion + binary export
# ══════════════════════════════════════════════════════════════════════════

def fuse_conv_bn(conv: nn.Conv2d, bn: nn.BatchNorm2d) -> tuple[np.ndarray, np.ndarray]:
    """Fuse Conv2d + BatchNorm2d into a single Conv2d.

    Returns (weight, bias) as float32 numpy arrays.
    training:  y = γ·(Wx + b_c - μ)/σ + β
    inference: y = (γ·W/σ)x + (γ·(b_c - μ)/σ + β)
    """
    W = conv.weight.data.numpy().astype(np.float32)
    b_c = conv.bias.data.numpy().astype(np.float32) if conv.bias is not None \
        else np.zeros(W.shape[0], dtype=np.float32)

    gamma = bn.weight.data.numpy().astype(np.float32)
    beta  = bn.bias.data.numpy().astype(np.float32)
    mean  = bn.running_mean.data.numpy().astype(np.float32)
    var   = bn.running_var.data.numpy().astype(np.float32)
    eps   = bn.eps

    std = np.sqrt(var + eps)
    a = gamma / std             # scale factor per channel
    b = a * (b_c - mean) + beta  # fused bias

    # a reshaped for broadcasting: (C_out, 1, 1, 1)
    W_fused = W * a.reshape(-1, 1, 1, 1)
    return W_fused, b


def export_blob(model: DigitCNN, path: Path):
    """Export Conv-BN fused weights as binary blob for Rust include_bytes!."""
    model.eval()

    # Fuse Conv+BN layers
    c1w, c1b = fuse_conv_bn(model.conv1, model.bn1)
    c2w, c2b = fuse_conv_bn(model.conv2, model.bn2)
    c3w, c3b = fuse_conv_bn(model.conv3, model.bn3)

    # FC layers (no BN to fuse)
    fc1w = model.fc1.weight.data.numpy().astype(np.float32)
    fc1b = model.fc1.bias.data.numpy().astype(np.float32)
    fc2w = model.fc2.weight.data.numpy().astype(np.float32)
    fc2b = model.fc2.bias.data.numpy().astype(np.float32)

    parts = [c1w, c1b, c2w, c2b, c3w, c3b, fc1w, fc1b, fc2w, fc2b]
    total = sum(p.nbytes for p in parts)

    with open(path, "wb") as f:
        for arr in parts:
            f.write(arr.tobytes())

    print(f"\nExported {total} bytes ({total//4} f32s) to {path}")

    # Print sizes for Rust struct definition
    names = ["conv1_w", "conv1_b", "conv2_w", "conv2_b", "conv3_w", "conv3_b",
             "fc1_w", "fc1_b", "fc2_w", "fc2_b"]
    print("  Layer sizes:")
    for name, arr in zip(names, parts):
        shape = "×".join(str(d) for d in arr.shape)
        print(f"    {name}: {shape}  ({arr.nbytes} bytes)")


# ══════════════════════════════════════════════════════════════════════════
# Main
# ══════════════════════════════════════════════════════════════════════════

def main():
    print("Loading templates...")
    templates = load_templates()
    for d in range(10):
        print(f"  {d}: {templates[d].size[0]}×{templates[d].size[1]}")

    print(f"\nGenerating dataset ({TRAIN_PER}+{VAL_PER}×{NUM_CLASSES} classes)...")
    X_train, y_train, X_val, y_val = generate_dataset(
        templates, TRAIN_PER, VAL_PER)
    print(f"  train: {X_train.shape}  {y_train.shape}")
    print(f"  val:   {X_val.shape}  {y_val.shape}")

    model = DigitCNN()
    print(f"\nModel: {model.param_count():,} parameters")

    print("\nTraining...")
    model = train(model, X_train, y_train, X_val, y_val)

    print("\nEvaluating...")
    cm = evaluate(model, X_val, y_val)

    print("\nExporting...")
    export_blob(model, OUTPUT_BLOB)

    # Quick sanity: run a few predictions and show softmax for each digit
    print("\n── Sample predictions ──")
    model.eval()
    with torch.no_grad():
        for d in range(10):
            img = templates[d].copy()
            t = _val_augment(img).unsqueeze(0)  # (1,1,24,24)
            logits = model(t)
            probs = F.softmax(logits, dim=1)[0]
            top3 = probs.topk(3)
            pred = probs.argmax().item()
            print(f"  true={d}  pred={pred}  "
                  + "  ".join(f"p({int(c)})={p:.3f}"
                              for c, p in zip(top3.indices, top3.values)))


if __name__ == "__main__":
    main()
