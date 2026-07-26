"""Generate self-contained HTML annotation page for end-to-end timer CNN.

Reads ROI frames from training_frames/, embeds as base64, produces a
keyboard-driven annotation UI.

Usage:  python scripts/build_annotator_v2.py [N] [output.html]
        N = number of frames to include (default 300)
"""

import sys
import base64
import json
import random
from pathlib import Path
from datetime import datetime

APPDATA = Path.home() / "AppData" / "Roaming" / "com.voxalic.app"
TRAINING_DIR = APPDATA / "training_frames"
LOW_SCORE_DIR = APPDATA / "low_score_frames"

N_FRAMES = int(sys.argv[1]) if len(sys.argv) > 1 else 300
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("scripts/annotate_e2e.html")


def main():
    # Collect PNGs from both directories
    all_pngs = []
    for d in [TRAINING_DIR, LOW_SCORE_DIR]:
        if d.exists():
            all_pngs.extend(sorted(d.glob("*.png")))

    if not all_pngs:
        print(f"No PNGs found in {TRAINING_DIR} or {LOW_SCORE_DIR}")
        return

    # Prioritize high-score frames (from training_frames) for clearer images
    high_score = [p for p in all_pngs if p.parent.name == "training_frames"]
    low_score = [p for p in all_pngs if p.parent.name == "low_score_frames"]

    # Shuffle within each group so we get time diversity
    random.seed(42)
    random.shuffle(high_score)
    random.shuffle(low_score)

    # Take roughly 80% from high-score, 20% from low-score
    n_high = min(len(high_score), int(N_FRAMES * 0.8))
    n_low = min(len(low_score), N_FRAMES - n_high)
    n_high += N_FRAMES - n_high - n_low  # Top up with high-score if low_score short

    selected = high_score[:n_high] + low_score[:n_low]
    random.shuffle(selected)  # Mix them up

    print(f"Selected {len(selected)} frames ({n_high} high-score + {n_low} low-score)")

    # Encode as base64
    images = []
    for i, p in enumerate(selected):
        b64 = base64.b64encode(p.read_bytes()).decode()
        images.append({"name": p.name, "b64": b64})
        if (i + 1) % 50 == 0:
            print(f"  encoded {i + 1}/{len(selected)}")

    imgs_json = json.dumps(images)
    total_kb = sum(len(img["b64"]) for img in images) * 3 // 4 // 1024
    print(f"  {len(images)} images, ~{total_kb} KB embedded")

    now = datetime.now().strftime("%Y-%m-%d")

    html = f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>标注工具 — 端到端计时器 CNN — {len(images)} 帧</title>
<style>
* {{ box-sizing:border-box; margin:0; padding:0; }}
body {{ font-family:system-ui,sans-serif; background:#1a1a1a; color:#e0e0e0;
       display:flex; flex-direction:column; align-items:center; min-height:100vh; }}
#topbar {{ width:100%; padding:10px 20px; background:#222; display:flex;
          align-items:center; gap:16px; border-bottom:1px solid #333; flex-wrap:wrap; }}
#topbar span {{ font-size:13px; }}
#progress {{ flex:1; min-width:100px; height:6px; background:#333; border-radius:3px; overflow:hidden; }}
#progress div {{ height:100%; background:#C01E25; transition:width 0.15s; }}
#main {{ flex:1; display:flex; flex-direction:column; align-items:center;
        justify-content:center; padding:20px; max-width:900px; width:100%; }}
#img-wrap {{ margin-bottom:12px; position:relative; }}
#img-wrap img {{ max-width:100%; max-height:52vh; object-fit:contain;
                  border:2px solid #444; border-radius:6px; image-rendering:pixelated; }}
#fname {{ font-size:11px; color:#888; margin-bottom:6px; max-width:400px;
          overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
#hint {{ font-size:12px; color:#666; margin-bottom:8px; }}
#input-row {{ display:flex; gap:8px; align-items:center; }}
#time-input {{ font-size:32px; padding:6px 14px; width:150px; text-align:center;
              background:#2a2a2a; color:#FFFFF0; border:2px solid #555; border-radius:5px;
              font-family:monospace; letter-spacing:5px; caret-color:#C01E25; }}
#time-input:focus {{ outline:none; border-color:#C01E25; box-shadow:0 0 8px rgba(192,30,37,0.3); }}
#time-input.valid {{ border-color:#2a8; }}
#time-input.invalid {{ border-color:#C01E25; }}
button {{ font-size:18px; padding:8px 20px; border:none; border-radius:5px; cursor:pointer; font-weight:600; }}
#btn-confirm {{ background:#C01E25; color:#FFFFF0; }}
#btn-confirm:hover {{ background:#d42; }}
#btn-skip {{ background:#444; color:#ccc; font-size:14px; padding:6px 14px; }}
#btn-skip:hover {{ background:#555; }}
#btn-export {{ background:#2a6; color:#fff; }}
#btn-export:hover {{ background:#3b7; }}
#btn-export:disabled {{ background:#333; color:#666; cursor:default; }}
#status {{ font-size:13px; color:#999; margin-top:8px; min-height:20px; }}
#stats-row {{ display:flex; gap:20px; font-size:13px; color:#aaa; }}
#export-area {{ margin-top:10px; width:100%; max-width:500px; }}
#export-area textarea {{ width:100%; height:80px; background:#2a2a2a; color:#ccc;
    border:1px solid #555; border-radius:4px; font-family:monospace; font-size:12px; resize:vertical; }}
</style>
</head>
<body>
<div id="topbar">
  <span><b>端到端标注</b> — 输入4位数字 (MMSS)</span>
  <span id="counter">1/{len(images)}</span>
  <div id="progress"><div style="width:0%"></div></div>
  <span id="stats-row"><span>已标:<b id="n-labeled">0</b></span><span>跳过:<b id="n-skipped">0</b></span></span>
  <button id="btn-export" onclick="exportCSV()">导出 CSV</button>
</div>
<div id="main">
  <div id="fname"></div>
  <div id="img-wrap"><img id="img" src="" alt="ROI"></div>
  <div id="hint">输入 <b>MMSS</b> 格式，如 03:52 → <b>0352</b>。回车确认，Tab 跳过。</div>
  <div id="input-row">
    <input id="time-input" type="text" maxlength="4" placeholder="0352"
           autocomplete="off" inputmode="numeric" pattern="[0-9]{{4}}">
    <button id="btn-confirm" onclick="confirm()">确认 (Enter)</button>
    <button id="btn-skip" onclick="skip()">跳过 (Tab)</button>
  </div>
  <div id="status"></div>
  <div id="export-area" style="display:none;">
    <p style="font-size:13px;color:#aaa;">CSV (复制到 scripts/annotations_v2.csv):</p>
    <textarea id="csv-output" readonly></textarea>
  </div>
</div>

<script>
const IMAGES = {imgs_json};
const TOTAL = IMAGES.length;

// Restore state from localStorage
const STORAGE_KEY = 'annotate_e2e_v2';
let state = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{{}}');
if (!state.labels) state.labels = {{}};
if (!state.skipped) state.skipped = {{}};
if (!state.idx || state.idx >= TOTAL) state.idx = 0;

let idx = state.idx;
let input = document.getElementById('time-input');
let img = document.getElementById('img');
let fname = document.getElementById('fname');
let status = document.getElementById('status');
let counter = document.getElementById('counter');
let progressBar = document.querySelector('#progress div');
let nLabeled = document.getElementById('n-labeled');
let nSkipped = document.getElementById('n-skipped');

function updateUI() {{
    let entry = IMAGES[idx];
    img.src = 'data:image/png;base64,' + entry.b64;
    fname.textContent = entry.name;
    counter.textContent = (idx + 1) + '/' + TOTAL;

    let pct = ((getLabeledCount() + Object.keys(state.skipped).length) / TOTAL * 100).toFixed(0);
    progressBar.style.width = pct + '%';
    nLabeled.textContent = getLabeledCount();
    nSkipped.textContent = Object.keys(state.skipped).length;

    // Pre-fill if already labeled
    if (state.labels[entry.name]) {{
        input.value = state.labels[entry.name];
        input.className = 'valid';
    }} else if (state.skipped[entry.name]) {{
        input.value = '';
        input.className = '';
    }} else {{
        input.value = '';
        input.className = '';
    }}

    input.focus();
    status.textContent = '';
}}

function getLabeledCount() {{
    return Object.keys(state.labels).length;
}}

function saveState() {{
    state.idx = idx;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    updateUI();
}}

function confirm() {{
    let val = input.value.trim();
    if (val.length !== 4 || !/^\\d{{4}}$/.test(val)) {{
        status.textContent = '请输入4位数字';
        input.className = 'invalid';
        input.focus();
        return;
    }}
    // Validate seconds < 60
    let secs = parseInt(val.slice(2));
    if (secs >= 60) {{
        status.textContent = '秒数不能 >= 60';
        input.className = 'invalid';
        input.focus();
        return;
    }}

    let entry = IMAGES[idx];
    state.labels[entry.name] = val;
    delete state.skipped[entry.name];

    // Move to next unlabeled
    moveNext();
}}

function skip() {{
    let entry = IMAGES[idx];
    state.skipped[entry.name] = true;
    delete state.labels[entry.name];
    moveNext();
}}

function moveNext() {{
    // Find next unlabeled + unskipped
    for (let i = 1; i <= TOTAL; i++) {{
        let next = (idx + i) % TOTAL;
        let name = IMAGES[next].name;
        if (!state.labels[name] && !state.skipped[name]) {{
            idx = next;
            saveState();
            return;
        }}
    }}
    // All done!
    idx = 0;
    saveState();
    status.textContent = '全部标注完成！点击导出 CSV';
}}

function exportCSV() {{
    let rows = ['filename,label,split'];
    let labeled = Object.entries(state.labels);
    // Shuffle for random train/val split
    let shuffled = labeled.map(([n, l]) => ({{n, l, r: Math.random()}}));
    shuffled.sort((a, b) => a.r - b.r);
    let nVal = Math.max(3, Math.floor(shuffled.length * 0.15));

    for (let i = 0; i < shuffled.length; i++) {{
        let split = i < nVal ? 'val' : 'train';
        rows.push(shuffled[i].n + ',' + shuffled[i].l + ',' + split);
    }}

    let csv = rows.join('\\n');
    document.getElementById('csv-output').value = csv;
    document.getElementById('export-area').style.display = 'block';

    // Also download as file
    let blob = new Blob([csv], {{type:'text/csv'}});
    let url = URL.createObjectURL(blob);
    let a = document.createElement('a');
    a.href = url;
    a.download = 'annotations_v2.csv';
    a.click();
    URL.revokeObjectURL(url);

    status.textContent = '已导出 ' + labeled.length + ' 条标注 (train:' +
        (shuffled.length - nVal) + ' val:' + nVal + ')';
}}

function undo() {{
    // Undo last action: clear current frame's label/skip
    let entry = IMAGES[idx];
    delete state.labels[entry.name];
    delete state.skipped[entry.name];
    saveState();
    status.textContent = '已撤销: ' + entry.name;
}}

// Keyboard shortcuts
document.addEventListener('keydown', function(e) {{
    if (e.key === 'Enter') {{
        e.preventDefault();
        confirm();
    }} else if (e.key === 'Tab') {{
        e.preventDefault();
        skip();
    }} else if (e.key === 'u' && e.ctrlKey) {{
        e.preventDefault();
        undo();
    }} else if (e.key === 'ArrowLeft' && e.ctrlKey) {{
        e.preventDefault();
        idx = (idx - 1 + TOTAL) % TOTAL;
        saveState();
    }} else if (e.key === 'ArrowRight' && e.ctrlKey) {{
        e.preventDefault();
        idx = (idx + 1) % TOTAL;
        saveState();
    }}
}});

// Start
updateUI();
input.focus();
</script>
</body>
</html>"""

    OUTPUT.write_text(html, encoding="utf-8")
    print(f"Written: {OUTPUT} ({OUTPUT.stat().st_size // 1024} KB)")
    print(f"Open in browser to start annotating.")


if __name__ == "__main__":
    main()
