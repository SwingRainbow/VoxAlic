"""
Generate a self-contained annotation HTML page from low_score_frames.

Usage:  python scripts/build_annotator.py [output.html]
        Then open output.html in a browser.
"""

import sys
import base64
import json
from pathlib import Path
from datetime import datetime

FRAMES_DIR = Path.home() / "AppData" / "Roaming" / "com.voxalic.app" / "low_score_frames"
OUTPUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("annotate_219.html")

def main():
    pngs = sorted(FRAMES_DIR.glob("*.png"))
    if not pngs:
        print(f"No PNGs found in {FRAMES_DIR}")
        return

    # Build image list: base64 data URIs
    images = []
    for i, p in enumerate(pngs):
        b64 = base64.b64encode(p.read_bytes()).decode()
        images.append({"name": p.name, "b64": b64})
        if (i + 1) % 50 == 0:
            print(f"  encoded {i + 1}/{len(pngs)}")

    imgs_json = json.dumps(images)
    total_kb = sum(len(img["b64"]) for img in images) * 3 // 4 // 1024
    print(f"  {len(images)} images, ~{total_kb} KB embedded")

    html = f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>标注工具 — {len(images)} 帧</title>
<style>
* {{ box-sizing:border-box; margin:0; padding:0; }}
body {{ font-family:system-ui,sans-serif; background:#1a1a1a; color:#e0e0e0;
       display:flex; flex-direction:column; align-items:center; min-height:100vh; }}
#topbar {{ width:100%; padding:10px 20px; background:#222; display:flex;
          align-items:center; gap:16px; border-bottom:1px solid #333; }}
#topbar span {{ font-size:14px; }}
#progress {{ flex:1; height:6px; background:#333; border-radius:3px; overflow:hidden; }}
#progress div {{ height:100%; background:#C01E25; transition:width 0.2s; }}
#main {{ flex:1; display:flex; flex-direction:column; align-items:center;
        justify-content:center; padding:20px; max-width:900px; width:100%; }}
#img-wrap {{ max-height:55vh; margin-bottom:16px; }}
#img-wrap img {{ max-width:100%; max-height:55vh; object-fit:contain;
                  border:1px solid #444; border-radius:4px; image-rendering:pixelated; }}
#input-row {{ display:flex; gap:10px; align-items:center; }}
#time-input {{ font-size:28px; padding:8px 16px; width:160px; text-align:center;
              background:#2a2a2a; color:#FFFFF0; border:2px solid #555; border-radius:4px;
              font-family:monospace; letter-spacing:4px; }}
#time-input:focus {{ outline:none; border-color:#C01E25; }}
#btn-confirm {{ font-size:20px; padding:8px 24px; background:#C01E25; color:#FFFFF0;
               border:none; border-radius:4px; cursor:pointer; }}
#btn-confirm:hover {{ background:#d42a2f; }}
#btn-skip {{ font-size:16px; padding:8px 16px; background:#444; color:#ccc;
            border:none; border-radius:4px; cursor:pointer; }}
#btn-skip:hover {{ background:#555; }}
#hint {{ color:#888; font-size:13px; margin-top:8px; }}
#hint kbd {{ background:#333; padding:1px 6px; border-radius:3px; font-family:monospace; }}
.label-badge {{ display:inline-block; padding:2px 8px; border-radius:3px; font-size:13px; }}
.label-done {{ background:#1a4a1a; color:#6f6; }}
.label-skip {{ background:#444; color:#aaa; }}
.label-pending {{ background:#333; color:#888; }}
</style>
</head>
<body>
<div id="topbar">
  <span id="counter">0 / {len(images)}</span>
  <div id="progress"><div style="width:0%"></div></div>
  <span id="done-badge" class="label-badge label-pending">未开始</span>
  <button id="btn-export" style="padding:6px 14px;background:#2a5a2a;color:#cfc;border:none;border-radius:4px;cursor:pointer;font-size:14px;">导出 CSV</button>
</div>
<div id="main">
  <div id="img-wrap"><img id="frame-img" src="" alt=""></div>
  <div id="input-row">
    <input type="text" id="time-input" placeholder="M:SS" maxlength="5" autofocus>
    <button id="btn-confirm">确认 ↵</button>
    <button id="btn-skip">跳过 S</button>
  </div>
  <div id="hint"><kbd>Enter</kbd> 确认  <kbd>S</kbd> 跳过  <kbd>B</kbd> 回退  <kbd>←→</kbd> 导航</div>
</div>

<script>
const IMAGES = {imgs_json};
const TOTAL = IMAGES.length;

// State persisted in localStorage
const STORE_KEY = 'annotator_219_state';
let state = JSON.parse(localStorage.getItem(STORE_KEY) || '{{"idx":0,"labels":{{}}}}');
// labels: {{ "filename.png": "3:27" }}  or "SKIP" for skipped
let labels = state.labels || {{}};
let idx = state.idx || 0;

// Ensure key exists for all old state formats
if (typeof labels !== 'object' || Array.isArray(labels)) labels = {{}};

function saveState() {{
    localStorage.setItem(STORE_KEY, JSON.stringify({{ idx, labels }}));
    updateUI();
}}

function updateUI() {{
    let done = 0, skipped = 0;
    for (const k of Object.keys(labels)) {{
        if (labels[k] === 'SKIP') skipped++;
        else done++;
    }}
    document.getElementById('counter').textContent = `${{idx + 1}} / ${{TOTAL}}`;
    document.getElementById('progress').firstElementChild.style.width = `${{(idx + 1) / TOTAL * 100}}%`;

    const badge = document.getElementById('done-badge');
    if (done + skipped === TOTAL) {{
        badge.textContent = `${{done}} 已标 · ${{skipped}} 跳过`;
        badge.className = 'label-badge label-done';
    }} else if (done + skipped > 0) {{
        badge.textContent = `${{done}} 已标 · ${{skipped}} 跳过 · ${{TOTAL - done - skipped}} 待标`;
        badge.className = 'label-badge label-pending';
    }}

    // Show current image
    if (idx >= 0 && idx < TOTAL) {{
        document.getElementById('frame-img').src = 'data:image/png;base64,' + IMAGES[idx].b64;
    }}

    // Show existing label if any
    const name = IMAGES[idx].name;
    const existing = labels[name];
    const inp = document.getElementById('time-input');
    if (existing && existing !== 'SKIP') {{
        inp.value = existing;
    }} else {{
        inp.value = '';
    }}
    inp.focus();
}}

function confirm() {{
    const name = IMAGES[idx].name;
    const val = document.getElementById('time-input').value.trim();
    if (val) {{
        labels[name] = val;
    }} else {{
        labels[name] = 'SKIP';
    }}
    if (idx < TOTAL - 1) {{
        idx++;
    }}
    saveState();
}}

function skip() {{
    const name = IMAGES[idx].name;
    labels[name] = 'SKIP';
    if (idx < TOTAL - 1) idx++;
    saveState();
}}

function goBack() {{
    if (idx > 0) idx--;
    updateUI();
}}

function goTo(i) {{
    if (i >= 0 && i < TOTAL) {{ idx = i; updateUI(); }}
}}

function exportCSV() {{
    let csv = 'filename,label\\n';
    for (const img of IMAGES) {{
        const lbl = labels[img.name] || '';
        csv += `${{img.name}},${{lbl}}\\n`;
    }}
    const blob = new Blob([csv], {{type:'text/csv'}});
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = 'annotations_${{new Date().toISOString().slice(0,10)}}.csv';
    a.click();
    URL.revokeObjectURL(url);
}}

// Keyboard shortcuts
document.addEventListener('keydown', e => {{
    if (e.key === 'Enter') confirm();
    else if (e.key === 's' || e.key === 'S') skip();
    else if (e.key === 'b' || e.key === 'B') goBack();
    else if (e.key === 'ArrowLeft') goBack();
    else if (e.key === 'ArrowRight') {{ if (idx < TOTAL - 1) {{ idx++; updateUI(); }} }}
}});

document.getElementById('btn-confirm').addEventListener('click', confirm);
document.getElementById('btn-skip').addEventListener('click', skip);
document.getElementById('btn-export').addEventListener('click', exportCSV);

updateUI();
</script>
</body>
</html>"""

    OUTPUT.write_text(html, encoding="utf-8")
    size_kb = OUTPUT.stat().st_size // 1024
    print(f"  written: {OUTPUT} ({size_kb} KB)")
    print(f"\n  Open {OUTPUT} in browser to start annotating.")
    print(f"  Progress auto-saves to localStorage.")

if __name__ == "__main__":
    main()
