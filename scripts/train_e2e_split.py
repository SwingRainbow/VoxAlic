"""Train 4 independent single-digit CNN models for end-to-end timer OCR.

Each model specializes in one digit position:
  Model 0: 分十 — 6 classes (0-5)
  Model 1: 分个 — 10 classes (0-9)
  Model 2: 秒十 — 6 classes (0-5)
  Model 3: 秒个 — 10 classes (0-9)

Architecture: Small 2-conv CNN per model (~200K params each, ~800K total).
Exports 4 separate weight blobs + one combined blob for Rust.
"""

import os
import sys
import struct
import argparse
import numpy as np
from pathlib import Path
from PIL import Image

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import Dataset, DataLoader
from torch.optim import AdamW
from torch.optim.lr_scheduler import ReduceLROnPlateau

# ── Config ──────────────────────────────────────────────────
INPUT_W, INPUT_H = 96, 128
BATCH_SIZE = 32
EPOCHS = 60
LR = 0.001
WEIGHT_DECAY = 1e-4
DROPOUT = 0.5
EARLY_STOP = 15

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")
SCRIPT_DIR = Path(__file__).parent
APPDATA = Path(os.environ.get("APPDATA", "."))
REAL_DIRS = [
    APPDATA / "com.voxalic.app" / "training_frames",
    APPDATA / "com.voxalic.app" / "low_score_frames",
]
SYNTH_DIR = SCRIPT_DIR / "synthetic_e2e"

HEAD_CONFIG = [
    {"name": "分十", "classes": 6,  "idx": 0},
    {"name": "分个", "classes": 10, "idx": 1},
    {"name": "秒十", "classes": 6,  "idx": 2},
    {"name": "秒个", "classes": 10, "idx": 3},
]


# ── Model ───────────────────────────────────────────────────
class SingleDigitCNN(nn.Module):
    """Small CNN for one digit position."""
    def __init__(self, num_classes, dropout=DROPOUT):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.bn1 = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.bn2 = nn.BatchNorm2d(16)
        self.pool = nn.MaxPool2d(2, 2)
        # After 2 pools: 24 × 32 × 16
        self.feat_size = 24 * 32 * 16
        self.fc1 = nn.Linear(self.feat_size, 32)
        self.dropout = nn.Dropout(dropout)
        self.fc2 = nn.Linear(32, num_classes)

    def forward(self, x):
        x = self.pool(F.relu(self.bn1(self.conv1(x))))
        x = self.pool(F.relu(self.bn2(self.conv2(x))))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        x = self.dropout(x)
        return self.fc2(x)


# ── Dataset ─────────────────────────────────────────────────
class SingleDigitDS(Dataset):
    """Load ROI frames and extract one digit position from the label."""
    def __init__(self, samples, img_size=(INPUT_W, INPUT_H), augment=False):
        self.samples = samples  # list of (Path, digit_label)
        self.img_w, self.img_h = img_size
        self.augment = augment

    def __len__(self):
        return len(self.samples)

    def __getitem__(self, idx):
        path, label = self.samples[idx]
        try:
            img = Image.open(path).convert("L")
        except Exception:
            return self.__getitem__((idx + 1) % len(self))

        if self.augment:
            # Random horizontal translation (±15px scaled to image width)
            tx = np.random.uniform(-0.12, 0.12)  # fraction of image width
            ty = np.random.uniform(-0.04, 0.04)
            img = img.transform(img.size, Image.AFFINE,
                                (1, 0, tx * img.size[0],
                                 0, 1, ty * img.size[1]), Image.BILINEAR)
            scale = np.random.uniform(0.94, 1.06)
            img = img.transform(img.size, Image.AFFINE,
                                (scale, 0, 0, 0, scale, 0), Image.BILINEAR)

        img = img.resize((self.img_w, self.img_h), Image.BILINEAR)
        arr = np.array(img, dtype=np.float32) / 255.0

        if self.augment:
            arr = np.clip(arr * np.random.uniform(0.90, 1.10) +
                          np.random.uniform(-0.05, 0.05), 0, 1)

        return torch.from_numpy(arr).unsqueeze(0), torch.tensor(label, dtype=torch.long)


# ── Data Loading ────────────────────────────────────────────
def load_annotations():
    """Load all annotated real frames, returning {filename: (label, dir_path)}."""
    merged = {}
    for csv_path in sorted(SCRIPT_DIR.glob("*annotations*.csv")):
        # Parse CSV
        with open(csv_path, "rb") as f:
            raw = f.read()
        if b"\n" not in raw:
            raw = raw.replace(bytes([0x5c, 0x6e]), b"\n")
        text = raw.decode("utf-8")
        for line in text.strip().split("\n"):
            line = line.strip()
            if not line or line.startswith("filename") or "," not in line:
                continue
            parts = line.split(",")
            if len(parts) < 2:
                continue
            fname, label = parts[0].strip(), parts[1].strip()
            if len(label) != 4 or not label.isdigit():
                continue
            for d in REAL_DIRS:
                p = d / fname
                if p.exists():
                    merged[fname] = (label, p)
                    break
    return merged


def build_samples(merged, digit_idx):
    """Extract single-digit samples from merged annotations."""
    samples = []
    for fname, (label, path) in merged.items():
        d = int(label[digit_idx])
        samples.append((path, d))
    return samples


# ── Training ────────────────────────────────────────────────
def train_one(model, train_loader, val_loader, epochs, name, digit_idx):
    optimizer = AdamW(model.parameters(), lr=LR, weight_decay=WEIGHT_DECAY)
    scheduler = ReduceLROnPlateau(optimizer, mode="min", factor=0.5, patience=5)

    best_val_acc = 0.0
    patience = 0

    for epoch in range(1, epochs + 1):
        model.train()
        train_loss = 0.0
        for imgs, targets in train_loader:
            imgs, targets = imgs.to(DEVICE), targets.to(DEVICE)
            optimizer.zero_grad()
            loss = F.cross_entropy(model(imgs), targets)
            loss.backward()
            optimizer.step()
            train_loss += loss.item() * imgs.size(0)
        train_loss /= len(train_loader.dataset)

        model.eval()
        val_loss = 0.0
        correct = 0
        n = 0
        with torch.no_grad():
            for imgs, targets in val_loader:
                imgs, targets = imgs.to(DEVICE), targets.to(DEVICE)
                outputs = model(imgs)
                val_loss += F.cross_entropy(outputs, targets).item() * imgs.size(0)
                correct += (torch.argmax(outputs, dim=1) == targets).sum().item()
                n += imgs.size(0)
        val_loss /= n
        val_acc = correct / n

        scheduler.step(val_loss)

        if val_acc > best_val_acc:
            best_val_acc = val_acc
            patience = 0
            flag = " <- best"
            torch.save(model.state_dict(), SCRIPT_DIR / f"best_digit{digit_idx}.pth")
        else:
            patience += 1
            flag = ""

        if epoch <= 5 or epoch % 5 == 0 or flag:
            print(f"  E{epoch:2d} T_loss={train_loss:.3f} V_loss={val_loss:.3f} V_acc={val_acc*100:.1f}%{flag}")

        if patience >= EARLY_STOP:
            print(f"  Early stop at epoch {epoch}")
            break

    return best_val_acc


# ── Export ──────────────────────────────────────────────────
def fuse_conv_bn(conv_w, conv_b, bn_w, bn_b, bn_mean, bn_var, eps):
    std = np.sqrt(bn_var + eps)
    gamma_div_std = bn_w / std
    fused_w = conv_w * gamma_div_std[:, np.newaxis, np.newaxis, np.newaxis]
    fused_b = gamma_div_std * (conv_b - bn_mean) + bn_b
    return fused_w, fused_b


def export_model(model, path):
    """Export a single model's Conv-BN fused weights as binary blob."""
    state = {k: v.cpu().numpy() for k, v in model.state_dict().items()}
    blob = b""
    eps = 1e-5  # PyTorch default BN eps

    for layer_idx in [1, 2]:
        cw = state[f"conv{layer_idx}.weight"]
        cb = state[f"conv{layer_idx}.bias"]
        bw = state[f"bn{layer_idx}.weight"]
        bb = state[f"bn{layer_idx}.bias"]
        bm = state[f"bn{layer_idx}.running_mean"]
        bv = state[f"bn{layer_idx}.running_var"]
        fw, fb = fuse_conv_bn(cw, cb, bw, bb, bm, bv, eps)
        blob += fw.astype(np.float32).tobytes()
        blob += fb.astype(np.float32).tobytes()

    for fc in ["fc1", "fc2"]:
        w = state[f"{fc}.weight"]
        b = state[f"{fc}.bias"]
        blob += w.astype(np.float32).tobytes()
        blob += b.astype(np.float32).tobytes()

    with open(path, "wb") as f:
        f.write(blob)
    return len(blob)


# ── Main ────────────────────────────────────────────────────
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--real-only", action="store_true")
    args = parser.parse_args()

    print(f"Device: {DEVICE}")
    print(f"Input: {INPUT_W}x{INPUT_H}")

    # Load data
    merged = load_annotations()
    print(f"Real labeled frames: {len(merged)}")

    if len(merged) == 0:
        print("ERROR: No labeled frames found.")
        sys.exit(1)

    # Train one model per digit
    models = []
    results = []

    for cfg in HEAD_CONFIG:
        digit_idx = cfg["idx"]
        num_classes = cfg["classes"]
        name = cfg["name"]

        print(f"\n{'='*50}")
        print(f"Training {name} ({num_classes} classes, digit idx={digit_idx})")
        print(f"{'='*50}")

        samples = build_samples(merged, digit_idx)
        print(f"Samples: {len(samples)}")

        # Class distribution
        from collections import Counter
        dist = Counter(s[1] for s in samples)
        print(f"Distribution: {dict(sorted(dist.items()))}")

        # Split
        n_val = max(4, int(len(samples) * 0.15))
        n_train = len(samples) - n_val
        indices = list(range(len(samples)))
        np.random.seed(42)
        np.random.shuffle(indices)
        train_idx, val_idx = indices[n_val:], indices[:n_val]

        train_samples = [samples[i] for i in train_idx]
        val_samples = [samples[i] for i in val_idx]

        train_ds = SingleDigitDS(train_samples, augment=True)
        val_ds = SingleDigitDS(val_samples, augment=False)

        train_loader = DataLoader(train_ds, BATCH_SIZE, shuffle=True, drop_last=True)
        val_loader = DataLoader(val_ds, BATCH_SIZE, shuffle=False)

        print(f"Train: {len(train_ds)}, Val: {len(val_ds)}")

        # Model
        model = SingleDigitCNN(num_classes).to(DEVICE)
        n_params = sum(p.numel() for p in model.parameters())
        print(f"Params: {n_params:,} ({n_params * 4 / 1024:.0f} KB)")

        # Train
        best_acc = train_one(model, train_loader, val_loader, EPOCHS, name, digit_idx)
        baseline = 100.0 / num_classes
        print(f"Best {name}: {best_acc*100:.1f}% (baseline {baseline:.1f}%)")

        models.append(model)
        results.append((name, best_acc, num_classes))

    # Summary
    print(f"\n{'='*50}")
    print("SUMMARY")
    print(f"{'='*50}")
    for name, acc, nc in results:
        baseline = 100.0 / nc
        status = "OK" if acc > baseline * 2 else "WEAK" if acc > baseline * 1.3 else "FAIL"
        print(f"  {name}: {acc*100:.1f}% ({nc}cls, baseline={baseline:.1f}%) [{status}]")

    # Export all 4 models
    print(f"\n{'='*50}")
    print("EXPORT")
    print(f"{'='*50}")
    total_bytes = 0
    for i, (model, cfg) in enumerate(zip(models, HEAD_CONFIG)):
        path = SCRIPT_DIR / f"cnn_digit{i}_{cfg['name']}.bin"
        sz = export_model(model, path)
        total_bytes += sz
        print(f"  digit{i} ({cfg['name']}): {path.name} ({sz:,} bytes)")

    # Combined blob with header
    combined = SCRIPT_DIR / "cnn_e2e_weights.bin"
    with open(combined, "wb") as f:
        # Header: 4 x u32 offsets to each model's weights
        offsets = [0, 0, 0, 0]
        header = struct.pack("<4I", 0, 0, 0, 0)  # placeholder
        f.write(header)
        for i, (model, cfg) in enumerate(zip(models, HEAD_CONFIG)):
            offsets[i] = f.tell()
            path = SCRIPT_DIR / f"cnn_digit{i}_{cfg['name']}.bin"
            f.write(path.read_bytes())
        # Write header with actual offsets
        f.seek(0)
        f.write(struct.pack("<4I", *offsets))

    print(f"\nCombined: {combined.name} ({combined.stat().st_size:,} bytes)")
    print(f"Format: [u32×4 offsets] [model0] [model1] [model2] [model3]")


if __name__ == "__main__":
    main()
