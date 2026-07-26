"""Run colon-anchored CNN on all labeled frames, output predictions vs labels."""
import sys
import numpy as np
from pathlib import Path
from PIL import Image
import torch

sys.path.insert(0, str(Path(__file__).parent))
from train_cnn_v5 import load_annotations, find_colon, crop_cnn_patch, DigitCNN, load_gray, DIGIT_RATIOS

model = DigitCNN()
model.load_state_dict(torch.load(Path(__file__).parent / "best_cnn_v5.pth", map_location="cpu", weights_only=True))
model.eval()

annotated = load_annotations()
HEAD_NAMES = ["分十", "分个", "秒十", "秒个"]
per_pos_correct = [0, 0, 0, 0]
per_pos_total = [0, 0, 0, 0]
exact_match = 0
total = 0

print(f"{'filename':<45} {'label':>5} {'pred':>5} {'match':>6}")
print("-" * 70)

for fname, (label_str, path) in sorted(annotated.items()):
    gray = load_gray(path)
    h, w = gray.shape
    label = [int(c) for c in label_str]
    mid_y = h // 2
    colon_cx, colon_cy = find_colon(gray, w // 2, mid_y)
    if abs(colon_cx - w // 2) > 30 or abs(colon_cy - mid_y) > 30:
        colon_cx, colon_cy = w // 2, mid_y

    offsets_x = [int(colon_cx + r * w) for r in DIGIT_RATIOS]
    preds = []
    for i, cx in enumerate(offsets_x):
        cy = colon_cy
        patch = crop_cnn_patch(gray, cx, cy)
        tensor = torch.from_numpy(patch.copy()).float().unsqueeze(0).unsqueeze(0)
        with torch.no_grad():
            logits = model(tensor)
            pred = torch.argmax(logits, dim=1).item()
        preds.append("X" if pred >= 10 else str(pred))
        per_pos_total[i] += 1
        if pred < 10 and pred == label[i]:
            per_pos_correct[i] += 1

    pred_str = "".join(preds)
    match = "OK" if pred_str == label_str else "ERR"
    if match == "OK":
        exact_match += 1
    total += 1
    print(f"{fname:<45} {label_str:>5} {pred_str:>5} {match:>6}")

print(f"\n{'='*70}")
print(f"Exact match: {exact_match}/{total} ({exact_match/total*100:.1f}%)")
for i in range(4):
    print(f"  {HEAD_NAMES[i]}: {per_pos_correct[i]}/{per_pos_total[i]} ({per_pos_correct[i]/per_pos_total[i]*100:.1f}%)")
