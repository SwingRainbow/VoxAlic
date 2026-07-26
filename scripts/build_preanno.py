"""Pre-annotation HTML generator for end-to-end timer CNN.

Loads trained single-digit models, predicts on unlabeled frames,
generates HTML with pre-filled labels. xuziyu only fixes errors.

分十 model is reliable (~90%), pre-filled in green.
Other 3 positions are weak — shown as dim suggestions, need verification.
"""

import sys
import base64
import json
import random
import numpy as np
from pathlib import Path
from datetime import datetime
from collections import Counter
from PIL import Image

import torch
import torch.nn as nn
import torch.nn.functional as F

INPUT_W, INPUT_H = 96, 128
APPDATA = Path.home() / "AppData" / "Roaming" / "com.voxalic.app"
TRAINING_DIR = APPDATA / "training_frames"
LOW_SCORE_DIR = APPDATA / "low_score_frames"
SCRIPT_DIR = Path(__file__).parent

# Load labeled filenames to exclude
LABELED = set()
for csv_path in SCRIPT_DIR.glob("annotations_v2.csv"):
    with open(csv_path) as f:
        for line in f:
            if "," in line:
                LABELED.add(line.split(",")[0].strip())

N_FRAMES = int(sys.argv[1]) if len(sys.argv) > 1 else 500
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else SCRIPT_DIR / "annotate_prefilled.html"

HEAD_CONFIG = [
    {"name": "分十", "classes": 6, "idx": 0, "reliable": True},
    {"name": "分个", "classes": 10, "idx": 1, "reliable": False},
    {"name": "秒十", "classes": 6, "idx": 2, "reliable": False},
    {"name": "秒个", "classes": 10, "idx": 3, "reliable": False},
]


class SingleDigitCNN(nn.Module):
    def __init__(self, num_classes):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.bn1 = nn.BatchNorm2d(8)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.bn2 = nn.BatchNorm2d(16)
        self.pool = nn.MaxPool2d(2, 2)
        self.feat_size = 24 * 32 * 16
        self.fc1 = nn.Linear(self.feat_size, 32)
        self.dropout = nn.Dropout(0.5)
        self.fc2 = nn.Linear(32, num_classes)

    def forward(self, x):
        x = self.pool(F.relu(self.bn1(self.conv1(x))))
        x = self.pool(F.relu(self.bn2(self.conv2(x))))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        return self.fc2(x)


def load_model(digit_idx):
    """Load a trained single-digit model."""
    cfg = HEAD_CONFIG[digit_idx]
    path = SCRIPT_DIR / f"best_digit{digit_idx}.pth"
    if not path.exists():
        return None

    model = SingleDigitCNN(cfg["classes"])
    model.load_state_dict(torch.load(path, map_location="cpu"))
    model.eval()
    return model


def predict_digit(model, img_array):
    """Predict a single digit from a normalized image array."""
    tensor = torch.from_numpy(img_array).unsqueeze(0).unsqueeze(0)  # (1, 1, H, W)
    with torch.no_grad():
        logits = model(tensor)
        probs = F.softmax(logits, dim=1)
        pred = torch.argmax(logits, dim=1).item()
        conf = probs[0, pred].item()
    return pred, conf


def main():
    # Collect unlabeled frames
    all_pngs = []
    for d in [TRAINING_DIR, LOW_SCORE_DIR]:
        if d.exists():
            for p in d.glob("*.png"):
                if p.name not in LABELED:
                    all_pngs.append(p)

    print(f"Unlabeled frames: {len(all_pngs)}")

    # Prioritize high-score frames for clearer images
    high_score = [p for p in all_pngs if p.parent.name == "training_frames"]
    low_score = [p for p in all_pngs if p.parent.name == "low_score_frames"]
    random.seed(42)
    random.shuffle(high_score)
    random.shuffle(low_score)

    n_high = min(len(high_score), int(N_FRAMES * 0.7))
    n_low = min(len(low_score), N_FRAMES - n_high)
    n_high += N_FRAMES - n_high - n_low
    selected = high_score[:n_high] + low_score[:n_low]
    random.shuffle(selected)
    print(f"Selected {len(selected)} ({n_high} high + {n_low} low)")

    # Load models
    models = []
    for i in range(4):
        m = load_model(i)
        models.append(m)
        if m:
            print(f"  digit{i} ({HEAD_CONFIG[i]['name']}): loaded")
        else:
            print(f"  digit{i} ({HEAD_CONFIG[i]['name']}): MISSING — will leave blank")

    # Predict on all selected frames
    predictions = {}
    for i, p in enumerate(selected):
        try:
            img = Image.open(p).convert("L")
            img = img.resize((INPUT_W, INPUT_H), Image.BILINEAR)
            arr = np.array(img, dtype=np.float32) / 255.0
        except Exception:
            continue

        preds = []
        confs = []
        for j, model in enumerate(models):
            if model is not None:
                pred, conf = predict_digit(model, arr)
                preds.append(str(pred))
                confs.append(conf)
            else:
                preds.append("?")
                confs.append(0.0)

        predictions[p.name] = {"preds": preds, "confs": confs}

        if (i + 1) % 100 == 0:
            print(f"  predicted {i + 1}/{len(selected)}")

    # Encode images as base64
    images = []
    for p in selected:
        b64 = base64.b64encode(p.read_bytes()).decode()
        name = p.name
        pred = predictions.get(name, {"preds": ["?", "?", "?", "?"], "confs": [0, 0, 0, 0]})
        images.append({
            "name": name,
            "b64": b64,
            "pred0": pred["preds"][0],
            "pred1": pred["preds"][1],
            "pred2": pred["preds"][2],
            "pred3": pred["preds"][3],
            "conf0": round(pred["confs"][0], 2),
            "conf1": round(pred["confs"][1], 2),
            "conf2": round(pred["confs"][2], 2),
            "conf3": round(pred["confs"][3], 2),
        })

    imgs_json = json.dumps(images)
    total_kb = sum(len(img["b64"]) for img in images) * 3 // 4 // 1024
    print(f"  {len(images)} images, ~{total_kb} KB")

    now = datetime.now().strftime("%Y-%m-%d")

    html = f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>预标注工具 — {len(images)} 帧 (分十预填)</title>
<style>
* {{ box-sizing:border-box; margin:0; padding:0; }}
body {{ font-family:system-ui,sans-serif; background:#1a1a1a; color:#e0e0e0;
       display:flex; flex-direction:column; align-items:center; min-height:100vh; }}
#topbar {{ width:100%; padding:10px 20px; background:#222; display:flex;
          align-items:center; gap:14px; border-bottom:1px solid #333; flex-wrap:wrap; }}
#topbar span {{ font-size:12px; }}
#progress {{ flex:1; min-width:80px; height:6px; background:#333; border-radius:3px; overflow:hidden; }}
#progress div {{ height:100%; background:#C01E25; transition:width 0.15s; }}
#main {{ flex:1; display:flex; flex-direction:column; align-items:center;
        justify-content:center; padding:20px; max-width:900px; width:100%; }}
#img-wrap {{ margin-bottom:8px; }}
#img-wrap img {{ max-width:100%; max-height:48vh; object-fit:contain;
                  border:2px solid #444; border-radius:6px; image-rendering:pixelated; }}
#fname {{ font-size:11px; color:#888; margin-bottom:4px; max-width:400px;
          overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
#legend {{ font-size:11px; color:#666; margin-bottom:8px; }}
#legend b {{ padding:1px 4px; border-radius:3px; }}
#legend b.green {{ background:#1a4; color:#fff; }}
#legend b.dim {{ background:#444; color:#999; }}
#input-row {{ display:flex; gap:8px; align-items:center; }}
.digit-box {{ font-size:32px; padding:6px 0; width:52px; text-align:center;
             background:#2a2a2a; border:2px solid #555; border-radius:5px;
             font-family:monospace; caret-color:#C01E25; }}
.digit-box:focus {{ outline:none; }}
.digit-box.prefill {{ color:#0f8; border-color:#1a4; background:#1a2a1a; }}
.digit-box.suggest {{ color:#888; border-color:#555; }}
.digit-box.manual {{ color:#FFFFF0; border-color:#C01E25; }}
.sep {{ font-size:28px; color:#666; font-weight:bold; user-select:none; }}
button {{ font-size:18px; padding:8px 20px; border:none; border-radius:5px; cursor:pointer; font-weight:600; }}
#btn-confirm {{ background:#C01E25; color:#FFFFF0; }}
#btn-confirm:hover {{ background:#d42; }}
#btn-skip {{ background:#444; color:#ccc; font-size:14px; padding:6px 14px; }}
#btn-export {{ background:#2a6; color:#fff; }}
#btn-export:hover {{ background:#3b7; }}
#status {{ font-size:13px; color:#999; margin-top:8px; min-height:20px; }}
#stats-row {{ display:flex; gap:16px; font-size:12px; color:#aaa; }}
#export-area {{ margin-top:10px; width:100%; max-width:500px; }}
#export-area textarea {{ width:100%; height:80px; background:#2a2a2a; color:#ccc;
    border:1px solid #555; border-radius:4px; font-family:monospace; font-size:12px; resize:vertical; }}
</style>
</head>
<body>
<div id="topbar">
  <span><b>预标注</b> — 分十(绿)已预填~90%准确，其余三位置需确认</span>
  <span id="counter">1/{len(images)}</span>
  <div id="progress"><div style="width:0%"></div></div>
  <span id="stats-row"><span>已标:<b id="n-labeled">0</b></span><span>跳过:<b id="n-skipped">0</b></span></span>
  <button id="btn-export" onclick="exportCSV()">导出 CSV</button>
</div>
<div id="main">
  <div id="fname"></div>
  <div id="img-wrap"><img id="img" src="" alt="ROI"></div>
  <div id="legend">
    <b class="green">分十 (预填·可靠)</b>
    <b class="dim">分个 (猜测)</b> :
    <b class="dim">秒十 (猜测)</b>
    <b class="dim">秒个 (猜测)</b>
    &nbsp; 回车确认，Tab跳过，Ctrl+Z撤销
  </div>
  <div id="input-row">
    <input id="d0" class="digit-box prefill" type="text" maxlength="1" inputmode="numeric" pattern="[0-9]">
    <input id="d1" class="digit-box suggest" type="text" maxlength="1" inputmode="numeric" pattern="[0-9]">
    <span class="sep">:</span>
    <input id="d2" class="digit-box suggest" type="text" maxlength="1" inputmode="numeric" pattern="[0-9]">
    <input id="d3" class="digit-box suggest" type="text" maxlength="1" inputmode="numeric" pattern="[0-9]">
    <button id="btn-confirm" onclick="confirm()">确认</button>
    <button id="btn-skip" onclick="skip()">跳过</button>
  </div>
  <div id="status"></div>
  <div id="export-area" style="display:none;">
    <p style="font-size:13px;color:#aaa;">CSV:</p>
    <textarea id="csv-output" readonly></textarea>
  </div>
</div>

<script>
const IMAGES = {imgs_json};
const TOTAL = IMAGES.length;
const STORAGE_KEY = 'preanno_e2e_v1';

let state = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{{}}');
if (!state.labels) state.labels = {{}};
if (!state.skipped) state.skipped = {{}};
if (!state.idx || state.idx >= TOTAL) state.idx = 0;

let idx = state.idx;

function getLabeledCount() {{ return Object.keys(state.labels).length; }}

function updateUI() {{
    let e = IMAGES[idx];
    document.getElementById('img').src = 'data:image/png;base64,' + e.b64;
    document.getElementById('fname').textContent = e.name;
    document.getElementById('counter').textContent = (idx + 1) + '/' + TOTAL;

    let pct = ((getLabeledCount() + Object.keys(state.skipped).length) / TOTAL * 100).toFixed(0);
    document.querySelector('#progress div').style.width = pct + '%';
    document.getElementById('n-labeled').textContent = getLabeledCount();
    document.getElementById('n-skipped').textContent = Object.keys(state.skipped).length;

    // Pre-fill from saved labels or model predictions
    let saved = state.labels[e.name];
    for (let i = 0; i < 4; i++) {{
        let box = document.getElementById('d' + i);
        let predKey = 'pred' + i;
        let confKey = 'conf' + i;

        if (saved) {{
            box.value = saved[i];
            box.className = 'digit-box manual';
        }} else {{
            // Pre-fill: digit0 from reliable model, others as dim suggestions
            if (i === 0 && e[predKey] !== '?') {{
                box.value = e[predKey];
                box.className = 'digit-box prefill';
            }} else if (e[predKey] !== '?') {{
                box.value = e[predKey];
                box.className = 'digit-box suggest';
            }} else {{
                box.value = '';
                box.className = 'digit-box';
            }}
        }}
    }}

    document.getElementById('d0').focus();
    document.getElementById('status').textContent = '';
}}

function saveState() {{
    state.idx = idx;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    updateUI();
}}

function getDigits() {{
    let d = [];
    for (let i = 0; i < 4; i++) {{
        let v = document.getElementById('d' + i).value.trim();
        if (v.length !== 1 || !/^\\d$/.test(v)) return null;
        d.push(v);
    }}
    // Validate seconds < 60
    let secs = parseInt(d[2] + d[3]);
    if (secs >= 60) {{
        document.getElementById('status').textContent = '秒数 ' + secs + ' >= 60，请修正秒位';
        document.getElementById('d2').focus();
        return null;
    }}
    return d.join('');
}}

function confirm() {{
    let label = getDigits();
    if (!label) return;

    state.labels[IMAGES[idx].name] = label;
    delete state.skipped[IMAGES[idx].name];
    moveNext();
}}

function skip() {{
    state.skipped[IMAGES[idx].name] = true;
    delete state.labels[IMAGES[idx].name];
    moveNext();
}}

function moveNext() {{
    for (let i = 1; i <= TOTAL; i++) {{
        let next = (idx + i) % TOTAL;
        if (!state.labels[IMAGES[next].name] && !state.skipped[IMAGES[next].name]) {{
            idx = next;
            saveState();
            return;
        }}
    }}
    idx = 0;
    saveState();
    document.getElementById('status').textContent = '全部完成！点击导出 CSV';
}}

function undo() {{
    let entry = IMAGES[idx];
    delete state.labels[entry.name];
    delete state.skipped[entry.name];
    saveState();
    document.getElementById('status').textContent = '已撤销: ' + entry.name;
}}

function exportCSV() {{
    let rows = ['filename,label,split'];
    let labeled = Object.entries(state.labels);
    let shuffled = labeled.map(function(e) {{ return {{n: e[0], l: e[1], r: Math.random()}}; }});
    shuffled.sort(function(a, b) {{ return a.r - b.r; }});
    let nVal = Math.max(5, Math.floor(shuffled.length * 0.15));

    for (let i = 0; i < shuffled.length; i++) {{
        let split = i < nVal ? 'val' : 'train';
        rows.push(shuffled[i].n + ',' + shuffled[i].l + ',' + split);
    }}

    let csv = rows.join('\\n');
    document.getElementById('csv-output').value = csv;
    document.getElementById('export-area').style.display = 'block';

    let blob = new Blob([csv], {{type:'text/csv'}});
    let url = URL.createObjectURL(blob);
    let a = document.createElement('a');
    a.href = url;
    a.download = 'annotations_v3.csv';
    a.click();
    URL.revokeObjectURL(url);

    document.getElementById('status').textContent = '已导出 ' + labeled.length + ' 条 (train:' +
        (shuffled.length - nVal) + ' val:' + nVal + ')';
}}

// Keyboard
document.addEventListener('keydown', function(e) {{
    if (e.key === 'Enter') {{ e.preventDefault(); confirm(); }}
    else if (e.key === 'Tab') {{ e.preventDefault(); skip(); }}
    else if (e.key === 'u' && e.ctrlKey) {{ e.preventDefault(); undo(); }}
}});

// Auto-advance between digit boxes
for (let i = 0; i < 4; i++) {{
    document.getElementById('d' + i).addEventListener('input', function(e) {{
        let v = this.value.replace(/[^0-9]/g, '');
        this.value = v.slice(0, 1);
        this.className = this.className.replace('prefill', 'manual').replace('suggest', 'manual');
        if (v.length === 1 && i < 3) {{
            document.getElementById('d' + (i + 1)).focus();
            document.getElementById('d' + (i + 1)).select();
        }}
    }});
}}

updateUI();
</script>
</body>
</html>"""

    OUTPUT.write_text(html, encoding="utf-8")
    print(f"Written: {OUTPUT} ({OUTPUT.stat().st_size // 1024} KB)")
    print(f"\nLegend:")
    print(f"  绿色 = 分十预填 (90%准确，大概率对)")
    print(f"  灰色 = 其他三位置模型猜测 (不可靠，请逐位确认)")
    print(f"  红色边框 = 已手动修改")


if __name__ == "__main__":
    main()
