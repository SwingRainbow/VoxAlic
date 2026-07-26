"""End-to-end mission timer CNN training script v2.

Architecture: 4 conv blocks → shared features → 4 digit classification heads.
Supports mixed real + synthetic training with augmentation.
Exports Conv-BN fused weights as binary blob for Rust deployment.

Usage:
  python train_e2e.py                  # train with available data
  python train_e2e.py --real-only      # use only real annotated frames
  python train_e2e.py --synth-only     # use only synthetic frames
"""

import os
import sys
import struct
import argparse
import numpy as np
from pathlib import Path
from collections import Counter
from PIL import Image

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import Dataset, DataLoader, random_split, ConcatDataset
from torch.optim import AdamW
from torch.optim.lr_scheduler import CosineAnnealingWarmRestarts

# ── Config ──────────────────────────────────────────────────
INPUT_W, INPUT_H = 96, 128
BATCH_SIZE = 32
EPOCHS = 80
LR_PEAK = 0.001
WEIGHT_DECAY = 1e-4
DROPOUT = 0.5
LABEL_SMOOTHING = 0.05
EARLY_STOP_PATIENCE = 20
VAL_SPLIT = 0.15
GRAD_CLIP = 1.0

DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")
SCRIPT_DIR = Path(__file__).parent
BEST_PTH = SCRIPT_DIR / "best_e2e.pth"
WEIGHTS_BIN = SCRIPT_DIR / "cnn_e2e_weights.bin"
APPDATA = Path(os.environ.get("APPDATA", "."))
REAL_DIRS = [
    APPDATA / "com.voxalic.app" / "training_frames",
    APPDATA / "com.voxalic.app" / "low_score_frames",
]
SYNTH_DIR = SCRIPT_DIR / "synthetic_e2e"


# ── Model ───────────────────────────────────────────────────
class ConvBlock(nn.Module):
    def __init__(self, in_ch, out_ch):
        super().__init__()
        self.conv = nn.Conv2d(in_ch, out_ch, 3, padding=1, bias=True)
        self.bn = nn.BatchNorm2d(out_ch)

    def forward(self, x):
        return F.max_pool2d(F.relu(self.bn(self.conv(x))), 2, 2)


class DigitHead(nn.Module):
    def __init__(self, in_features, hidden=64, num_classes=10, dropout=0.5):
        super().__init__()
        self.fc1 = nn.Linear(in_features, hidden, bias=True)
        self.dropout = nn.Dropout(dropout)
        self.fc2 = nn.Linear(hidden, num_classes, bias=True)

    def forward(self, x):
        return self.fc2(self.dropout(F.relu(self.fc1(x))))


class E2EModel(nn.Module):
    def __init__(self, dropout=DROPOUT, small=False):
        super().__init__()
        if small:
            # ~108K params — appropriate for 200-500 real frames
            c1, c2, c3, c4 = 8, 16, 24, 32
            fc_hidden = 32
        else:
            c1, c2, c3, c4 = 16, 32, 64, 128
            fc_hidden = 64

        self.conv1 = ConvBlock(1, c1)
        self.conv2 = ConvBlock(c1, c2)
        self.conv3 = ConvBlock(c2, c3)
        self.conv4 = ConvBlock(c3, c4)

        with torch.no_grad():
            dummy = torch.zeros(1, 1, INPUT_H, INPUT_W)
            x = self.conv4(self.conv3(self.conv2(self.conv1(dummy))))
            self.feat_size = x.view(1, -1).shape[1]

        self.head0 = DigitHead(self.feat_size, hidden=fc_hidden, num_classes=6, dropout=dropout)   # 分十 0-5
        self.head1 = DigitHead(self.feat_size, hidden=fc_hidden, num_classes=10, dropout=dropout)  # 分个 0-9
        self.head2 = DigitHead(self.feat_size, hidden=fc_hidden, num_classes=6, dropout=dropout)   # 秒十 0-5
        self.head3 = DigitHead(self.feat_size, hidden=fc_hidden, num_classes=10, dropout=dropout)  # 秒个 0-9

    def forward(self, x):
        x = self.conv1(x)
        x = self.conv2(x)
        x = self.conv3(x)
        x = self.conv4(x)
        x = x.view(x.size(0), -1)
        return self.head0(x), self.head1(x), self.head2(x), self.head3(x)


# ── Dataset ─────────────────────────────────────────────────
class FrameDataset(Dataset):
    """Load grayscale ROI frames from a directory with CSV labels.

    CSV: filename,label  (label = 4-digit MMSS, e.g. "0352")
    Also auto-detects 4-digit labels from filenames ending in _MMSS.png.
    """

    def __init__(self, img_dir, csv_path=None, img_size=(INPUT_W, INPUT_H),
                 augment=False):
        self.img_dir = Path(img_dir)
        self.img_w, self.img_h = img_size
        self.augment = augment
        self.samples = []

        if csv_path and os.path.exists(csv_path):
            with open(csv_path, "rb") as f:
                raw = f.read()
            # Handle literal \n in CSV
            if b"\n" not in raw:
                raw = raw.replace(bytes([0x5c, 0x6e]), b"\n")
            text = raw.decode("utf-8")
            for line in text.strip().split("\n"):
                line = line.strip()
                if not line or line.startswith("filename"):
                    continue
                parts = line.split(",")
                if len(parts) >= 2:
                    fname, label = parts[0].strip(), parts[1].strip()
                    if len(label) == 4 and label.isdigit():
                        self.samples.append((fname, label))

        if not self.samples:
            # Auto-detect: scan for *_XXXX.png pattern
            for fname in os.listdir(self.img_dir):
                if not fname.endswith(".png"):
                    continue
                stem = fname.rsplit(".", 1)[0]
                parts = stem.split("_")
                for p in reversed(parts):
                    if len(p) == 4 and p.isdigit():
                        self.samples.append((fname, p))
                        break

    def __len__(self):
        return len(self.samples)

    def __getitem__(self, idx):
        fname, label = self.samples[idx]
        img_path = self.img_dir / fname

        try:
            img = Image.open(img_path).convert("L")
        except Exception:
            return self.__getitem__((idx + 1) % max(1, len(self)))

        if self.augment:
            # Random affine: slight rotation + translation
            angle = np.random.uniform(-3, 3)
            translate = (np.random.uniform(-0.05, 0.05), np.random.uniform(-0.05, 0.05))
            # Also slight scale variation
            scale = np.random.uniform(0.92, 1.08)
            img = img.transform(
                img.size, Image.AFFINE,
                (scale, 0, translate[0] * img.size[0],
                 0, scale, translate[1] * img.size[1]),
                Image.BILINEAR,
            )

        # Resize
        img = img.resize((self.img_w, self.img_h), Image.BILINEAR)

        arr = np.array(img, dtype=np.float32) / 255.0

        if self.augment:
            # Brightness/contrast jitter
            arr = np.clip(arr * np.random.uniform(0.85, 1.15) + np.random.uniform(-0.08, 0.08), 0, 1)

        tensor = torch.from_numpy(arr).unsqueeze(0)
        targets = torch.tensor([int(c) for c in label], dtype=torch.long)
        return tensor, targets


# ── Loss ────────────────────────────────────────────────────
def smooth_labels(targets, num_classes=10, eps=LABEL_SMOOTHING):
    """Convert hard targets to soft targets with label smoothing."""
    bsz = targets.size(0)
    smooth = torch.full((bsz, num_classes), eps / (num_classes - 1), device=targets.device)
    smooth.scatter_(1, targets.unsqueeze(1), 1.0 - eps)
    return smooth


HEAD_WEIGHTS = [1.0, 5.0, 1.0, 5.0]  # 分十, 分个, 秒十, 秒个 — hard heads get 5x

def compute_loss(outputs, targets):
    """Sum of 4 cross-entropy losses with label smoothing and per-head weighting."""
    loss = 0.0
    for i, logits in enumerate(outputs):
        soft_target = smooth_labels(targets[:, i], num_classes=logits.size(1))
        ce = torch.sum(-soft_target * F.log_softmax(logits, dim=1), dim=1).mean()
        loss += HEAD_WEIGHTS[i] * ce
    return loss


def compute_accuracy(outputs, targets):
    preds = torch.stack([torch.argmax(o, dim=1) for o in outputs], dim=1)
    exact = (preds == targets).all(dim=1).float().mean()
    per_digit = (preds == targets).float().mean()
    return exact.item(), per_digit.item()


# ── Training ────────────────────────────────────────────────
def train_epoch(model, loader, optimizer, scheduler=None):
    model.train()
    total_loss = 0.0
    total_exact = 0.0
    n = 0

    for imgs, targets in loader:
        imgs, targets = imgs.to(DEVICE), targets.to(DEVICE)
        optimizer.zero_grad()
        outputs = model(imgs)
        loss = compute_loss(outputs, targets)
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), GRAD_CLIP)
        optimizer.step()
        if scheduler:
            scheduler.step()

        exact, _ = compute_accuracy(outputs, targets)
        bs = imgs.size(0)
        total_loss += loss.item() * bs
        total_exact += exact * bs
        n += bs

    return total_loss / n, total_exact / n


def validate(model, loader):
    model.eval()
    total_loss = 0.0
    total_exact = 0.0
    total_digit = 0.0
    n = 0
    head_correct = [0, 0, 0, 0]
    head_total = 0
    # Confusion tracking for worst head
    confusion = torch.zeros(10, 10)

    with torch.no_grad():
        for imgs, targets in loader:
            imgs, targets = imgs.to(DEVICE), targets.to(DEVICE)
            outputs = model(imgs)
            loss = compute_loss(outputs, targets)
            exact, digit = compute_accuracy(outputs, targets)
            bs = imgs.size(0)
            total_loss += loss.item() * bs
            total_exact += exact * bs
            total_digit += digit * bs
            n += bs
            for i in range(4):
                head_correct[i] += (torch.argmax(outputs[i], dim=1) == targets[:, i]).sum().item()
            head_total += bs
            # Track confusion for head1
            preds_1 = torch.argmax(outputs[1], dim=1)
            for t, p in zip(targets[:, 1].cpu(), preds_1.cpu()):
                confusion[t, p] += 1

    head_accs = [c / head_total for c in head_correct]

    # Show top 3 confusions for the worst head (usually head1)
    worst_head = np.argmin(head_accs)
    if worst_head == 1:  # 分个
        conf_str = ""
        for row in range(10):
            top3 = confusion[row].topk(min(3, 10))
            for val, col in zip(top3.values, top3.indices):
                if int(col) != row and val > 0:
                    conf_str += f" {row}→{int(col)}:{int(val)}"
                    if len(conf_str) > 60:
                        break
            if len(conf_str) > 60:
                break
        if conf_str:
            print(f"      top confusions (head1):{conf_str}")

    return total_loss / n, total_exact / n, total_digit / n, head_accs


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--real-only", action="store_true")
    parser.add_argument("--synth-only", action="store_true")
    parser.add_argument("--small", action="store_true", help="Smaller model (~108K params) for limited data")
    parser.add_argument("--epochs", type=int, default=EPOCHS)
    args = parser.parse_args()

    print(f"Device: {DEVICE}")
    print(f"Input: {INPUT_W}x{INPUT_H}, LR peak: {LR_PEAK}, BS: {BATCH_SIZE}")

    # ── Load synthetic data ──
    synth_csv = SYNTH_DIR / "labels.csv"
    if not synth_csv.exists() and not args.real_only:
        print("Synthetic data not found. Run synthesize_e2e.py first.")
        sys.exit(1)

    synth_ds = None
    if not args.real_only and synth_csv.exists():
        synth_ds = FrameDataset(SYNTH_DIR, synth_csv, augment=True)
        print(f"Synthetic: {len(synth_ds)}")

    # ── Load real annotated data ──
    real_ds_list = []
    for csv_path in sorted(SCRIPT_DIR.glob("*annotations*.csv")):
        for real_dir in REAL_DIRS:
            if real_dir.exists():
                ds = FrameDataset(real_dir, csv_path, augment=True)
                if len(ds) > 0:
                    real_ds_list.append(ds)
                    print(f"Real: {len(ds)} ({csv_path.name} @ {real_dir.name})")

    # Merge real datasets
    if real_ds_list:
        all_real_samples = []
        seen = set()
        for ds in real_ds_list:
            for fname, label in ds.samples:
                if fname not in seen:
                    seen.add(fname)
                    all_real_samples.append((fname, label))

        # Build merged real dataset — only keep samples whose files exist
        existing_samples = []
        missing_count = 0
        for fname, label in all_real_samples:
            found = False
            for d in REAL_DIRS:
                if (d / fname).exists():
                    existing_samples.append((fname, label))
                    found = True
                    break
            if not found:
                missing_count += 1
        if missing_count:
            print(f"  (skipped {missing_count} missing files)")

        if not existing_samples:
            print("WARNING: No real frame files found. Training with synthetic only.")
            real_ds = None
        else:
            class MergedRealDS(Dataset):
                def __init__(self, samples, dirs, img_size, augment):
                    self.samples = samples
                    self.dirs = [Path(d) for d in dirs]
                    self.img_w, self.img_h = img_size
                    self.augment = augment
                    # Pre-resolve paths for fast lookup
                    self._paths = {}
                    for fname, _ in samples:
                        for d in self.dirs:
                            p = d / fname
                            if p.exists():
                                self._paths[fname] = p
                                break

                def __len__(self):
                    return len(self.samples)

                def __getitem__(self, idx):
                    fname, label = self.samples[idx]
                    p = self._paths.get(fname)
                    if p is None:
                        # Shouldn't happen after pre-filter, but handle gracefully
                        return torch.zeros(1, self.img_h, self.img_w), torch.zeros(4, dtype=torch.long)

                    img = Image.open(p).convert("L")
                    if self.augment:
                        scale = np.random.uniform(0.94, 1.06)
                        img = img.transform(
                            img.size, Image.AFFINE,
                            (scale, 0, 0, 0, scale, 0), Image.BILINEAR,
                        )
                    img = img.resize((self.img_w, self.img_h), Image.BILINEAR)
                    arr = np.array(img, dtype=np.float32) / 255.0
                    if self.augment:
                        arr = np.clip(
                            arr * np.random.uniform(0.90, 1.10) + np.random.uniform(-0.05, 0.05),
                            0, 1,
                        )
                    tensor = torch.from_numpy(arr).unsqueeze(0)
                    targets = torch.tensor([int(c) for c in label], dtype=torch.long)
                    return tensor, targets

            real_ds = MergedRealDS(existing_samples, REAL_DIRS, (INPUT_W, INPUT_H), augment=True)
            print(f"Real merged: {len(real_ds)} frames (skipped {missing_count} missing)")
    else:
        real_ds = None

    # ── Build train/val split ──
    if real_ds and len(real_ds) >= 20 and not args.synth_only:
        # Real data available: use it for validation
        n_val = max(5, int(len(real_ds) * VAL_SPLIT))
        n_train_real = len(real_ds) - n_val
        real_train, real_val = random_split(real_ds, [n_train_real, n_val])

        # Validation dataset WITHOUT augmentation
        val_samples = [real_ds.samples[i] for i in real_val.indices]
        val_ds = MergedRealDS(val_samples, REAL_DIRS, (INPUT_W, INPUT_H), augment=False)

        # Training: real + synth
        parts = [real_train]
        if synth_ds:
            parts.append(synth_ds)
        train_ds = ConcatDataset(parts)
        print(f"Train: {len(train_ds)} ({n_train_real} real + {len(synth_ds) if synth_ds else 0} synth)")
        print(f"Val:   {len(val_ds)} (real only)")
    elif synth_ds:
        # Synthetic only
        n_val = max(50, int(len(synth_ds) * VAL_SPLIT))
        n_train = len(synth_ds) - n_val
        synth_train, synth_val = random_split(synth_ds, [n_train, n_val])
        train_ds = synth_train
        # Val without augmentation
        val_indices = synth_val.indices
        val_ds = FrameDataset(SYNTH_DIR, synth_csv, augment=False)
        val_ds.samples = [synth_ds.samples[i] for i in val_indices]
        print(f"Train: {len(train_ds)} synth")
        print(f"Val:   {len(val_ds)} synth")
    else:
        print("ERROR: No training data available.")
        sys.exit(1)

    train_loader = DataLoader(train_ds, BATCH_SIZE, shuffle=True, num_workers=0,
                              drop_last=True)
    val_loader = DataLoader(val_ds, BATCH_SIZE, shuffle=False, num_workers=0)

    # ── Model ──
    model = E2EModel(dropout=0.7 if args.small else DROPOUT, small=args.small).to(DEVICE)
    n_params = sum(p.numel() for p in model.parameters())
    print(f"Params: {n_params:,} ({n_params * 4 / 1024:.0f} KB f32)")
    print(f"Conv4 feature size: {model.feat_size}")

    # ── Optimizer & Scheduler ──
    optimizer = AdamW(model.parameters(), lr=LR_PEAK, weight_decay=WEIGHT_DECAY)
    # Cosine warm restarts: cycle length doubles each restart
    scheduler = CosineAnnealingWarmRestarts(
        optimizer, T_0=10, T_mult=2, eta_min=LR_PEAK * 0.01
    )

    # ── Train ──
    best_val_exact = 0.0
    patience_counter = 0
    HEAD_NAMES = ["分十", "分个", "秒十", "秒个"]

    for epoch in range(1, args.epochs + 1):
        train_loss, train_exact = train_epoch(model, train_loader, optimizer, scheduler)
        val_loss, val_exact, val_digit, head_accs = validate(model, val_loader)

        lr = optimizer.param_groups[0]["lr"]
        head_str = " | ".join(
            f"{HEAD_NAMES[i]}:{head_accs[i]*100:.1f}%" for i in range(4)
        )

        print(
            f"E{epoch:3d} lr={lr:.2e} | "
            f"T loss={train_loss:.3f} ex={train_exact*100:.1f}% | "
            f"V loss={val_loss:.3f} ex={val_exact*100:.1f}% dg={val_digit*100:.1f}%"
        )

        if val_exact > best_val_exact:
            best_val_exact = val_exact
            patience_counter = 0
            torch.save(model.state_dict(), BEST_PTH)
            print(f"      <- best | {head_str}")
        else:
            patience_counter += 1
            print(f"      {head_str}")

        if patience_counter >= EARLY_STOP_PATIENCE:
            print(f"Early stop at epoch {epoch}")
            break

    print(f"\nBest val exact={best_val_exact*100:.1f}% digit={val_digit*100:.1f}%")

    # ── Export ──
    if best_val_exact > 0:
        model.load_state_dict(torch.load(BEST_PTH))
        export_fused_weights(model)
        print(f"Exported: {WEIGHTS_BIN}")
    else:
        print("No improvement, skipping export.")


# ── Export ──────────────────────────────────────────────────
def fuse_conv_bn(conv_w, conv_b, bn_w, bn_b, bn_mean, bn_var, eps):
    """Conv-BN fusion: w' = w * gamma/std, b' = gamma*(b-mean)/std + beta."""
    std = np.sqrt(bn_var + eps)
    gamma_div_std = bn_w / std
    fused_w = conv_w * gamma_div_std[:, np.newaxis, np.newaxis, np.newaxis]
    fused_b = gamma_div_std * (conv_b - bn_mean) + bn_b
    return fused_w, fused_b


def export_fused_weights(model):
    """Export Conv-BN fused weights as binary blob for Rust."""
    state = {k: v.cpu().numpy() for k, v in model.state_dict().items()}
    blob = b""
    offsets = []

    eps = model.conv1.bn.eps

    for name in ["conv1", "conv2", "conv3", "conv4"]:
        cw = state[f"{name}.conv.weight"]
        cb = state[f"{name}.conv.bias"]
        bw = state[f"{name}.bn.weight"]
        bb = state[f"{name}.bn.bias"]
        bm = state[f"{name}.bn.running_mean"]
        bv = state[f"{name}.bn.running_var"]

        fw, fb = fuse_conv_bn(cw, cb, bw, bb, bm, bv, eps)
        offsets.append(len(blob))
        blob += fw.astype(np.float32).tobytes()
        blob += fb.astype(np.float32).tobytes()
        print(f"  {name}: fused w{fw.shape} b{fb.shape}")

    for i in range(4):
        for suffix in ["fc1", "fc2"]:
            key = f"head{i}.{suffix}"
            w = state[f"{key}.weight"]
            b = state[f"{key}.bias"]
            offsets.append(len(blob))
            blob += w.astype(np.float32).tobytes()
            blob += b.astype(np.float32).tobytes()
            print(f"  {key}: w{w.shape} b{b.shape}")

    with open(WEIGHTS_BIN, "wb") as f:
        f.write(blob)

    total = len(blob)
    print(f"Total: {total:,} bytes ({total/1024/1024:.2f} MB, {total//4:,} f32s)")

    # Write offset reference for Rust
    offset_path = SCRIPT_DIR / "cnn_e2e_offsets.txt"
    with open(offset_path, "w") as f:
        f.write(f"// Auto-generated weight offsets for cnn_e2e.rs\n")
        f.write(f"// Total: {total} bytes, {total//4} f32s\n")
        layers = ["conv1", "conv2", "conv3", "conv4",
                   "head0.fc1", "head0.fc2",
                   "head1.fc1", "head1.fc2",
                   "head2.fc1", "head2.fc2",
                   "head3.fc1", "head3.fc2"]
        for layer, off in zip(layers, offsets):
            f.write(f"{layer} = {off}\n")
        # End offset
        f.write(f"END = {total}\n")
    print(f"Offsets: {offset_path}")


if __name__ == "__main__":
    main()
