"""Fix train_cnn_v4.py generate_dataset to use ROI-proportional offsets."""
import re

path = "scripts/train_cnn_v4.py"
with open(path, "r", encoding="utf-8") as f:
    content = f.read()

# Replace the broken section after colon_found
old = '''            colon_found += 1

# Digit X-offsets from colon, as fraction of ROI width
DIGIT_RATIOS = [-0.305, -0.153, 0.115, 0.267]  # 分十, 分个, 秒十, 秒个
            cy = colon_cy
            for dx, dy in jitters:
                patch = crop_cnn_patch(gray, cx + dx, cy + dy)
                if augment:
                    patch = augment_patch(patch)
                X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                y_list.append(label[i])'''

new = '''            colon_found += 1

        # Digit offsets proportional to ROI width (DIGIT_RATIOS defined at top)
        offsets_x = [int(colon_cx + r * w) for r in DIGIT_RATIOS]
        for i, cx in enumerate(offsets_x):
            cy = colon_cy
            for dx, dy in jitters:
                patch = crop_cnn_patch(gray, cx + dx, cy + dy)
                if augment:
                    patch = augment_patch(patch)
                X_list.append(torch.from_numpy(patch.copy()).float().unsqueeze(0))
                y_list.append(label[i])'''

if old in content:
    content = content.replace(old, new)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    print("Fixed.")
else:
    # Try reading with different encoding
    print("Pattern not found, content around DIGIT_RATIOS:")
    idx = content.find("DIGIT_RATIOS")
    if idx >= 0:
        print(repr(content[idx-50:idx+200]))
    else:
        print("DIGIT_RATIOS not found")
