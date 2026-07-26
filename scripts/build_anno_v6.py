"""Generate v6 annotation HTML for CNN training — 400 frames."""
import sys, os, base64, json, random
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from train_cnn_v5 import load_annotations

labeled = load_annotations()
appdata = Path(os.environ['APPDATA'])
tf = appdata / 'com.voxalic.app' / 'training_frames'
lf = appdata / 'com.voxalic.app' / 'low_score_frames'

ut = [p for p in tf.glob('*.png') if p.name not in labeled]
ul = [p for p in lf.glob('*.png') if p.name not in labeled]
print(f'Unlabeled: {len(ut)} high + {len(ul)} low')
random.seed(123)
random.shuffle(ut); random.shuffle(ul)
sel = ut[:25] + ul[:375]; random.shuffle(sel)
print(f'Selected {len(sel)}')

images = []
for i, p in enumerate(sel):
    b64 = base64.b64encode(p.read_bytes()).decode()
    images.append({'name': p.name, 'b64': b64})
    if (i + 1) % 100 == 0:
        print(f'  {i + 1}/{len(sel)}')

N = len(images)
imgs_json = json.dumps(images)
kb = sum(len(i['b64']) for i in images) * 3 // 4 // 1024
print(f'{N} frames, ~{kb} KB')

html = f'''<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>标注 CNN v6 — {N} 帧</title>
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
#fname {{ font-size:11px; color:#888; margin-bottom:4px; }}
#hint {{ font-size:12px; color:#666; margin-bottom:8px; }}
#input-row {{ display:flex; gap:6px; align-items:center; }}
#time-input {{ font-size:28px; padding:6px 12px; width:140px; text-align:center;
              background:#2a2a2a; color:#FFFFF0; border:2px solid #555; border-radius:5px;
              font-family:monospace; letter-spacing:4px; caret-color:#C01E25; }}
#time-input:focus {{ outline:none; border-color:#C01E25; }}
button {{ font-size:16px; padding:6px 16px; border:none; border-radius:5px; cursor:pointer; font-weight:600; }}
#btn-confirm {{ background:#C01E25; color:#FFFFF0; }}
#btn-skip {{ background:#444; color:#ccc; font-size:13px; }}
#btn-export {{ background:#2a6; color:#fff; }}
#status {{ font-size:13px; color:#999; margin-top:8px; min-height:20px; }}
#stats-row {{ display:flex; gap:16px; font-size:12px; color:#aaa; }}
#export-area {{ margin-top:10px; width:100%; max-width:500px; }}
#export-area textarea {{ width:100%; height:80px; background:#2a2a2a; color:#ccc;
    border:1px solid #555; border-radius:4px; font-family:monospace; font-size:12px; resize:vertical; }}
</style>
</head>
<body>
<div id="topbar">
  <span><b>CNN v6 标注</b> — 4位MMSS (978已标)</span>
  <span id="counter">1/{N}</span>
  <div id="progress"><div style="width:0%"></div></div>
  <span id="stats-row"><span>已标:<b id="n-labeled">0</b></span><span>跳过:<b id="n-skipped">0</b></span></span>
  <button id="btn-export" onclick="exportCSV()">导出 CSV</button>
</div>
<div id="main">
  <div id="fname"></div>
  <div id="img-wrap"><img id="img" src="" alt="ROI"></div>
  <div id="hint">输入 <b>MMSS</b> 如03:52→0352。回车确认，Tab跳过，Ctrl+←→导航</div>
  <div id="input-row">
    <input id="time-input" type="text" maxlength="4" placeholder="0352" autocomplete="off" inputmode="numeric" pattern="[0-9]{{4}}">
    <button id="btn-confirm" onclick="confirmLabel()">确认</button>
    <button id="btn-skip" onclick="skipLabel()">跳过</button>
  </div>
  <div id="status"></div>
  <div id="export-area" style="display:none;">
    <p style="font-size:13px;color:#aaa;">CSV (存为 annotations_v6.csv):</p>
    <textarea id="csv-output" readonly></textarea>
  </div>
</div>
<script>
const IMAGES = {imgs_json};
const TOTAL = IMAGES.length;
const STORAGE_KEY = 'annocnn_v6';
let state = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{{}}');
if (!state.labels) state.labels = {{}};
if (!state.skipped) state.skipped = {{}};
if (!state.idx || state.idx >= TOTAL) state.idx = 0;
let idx = state.idx;

function updateUI() {{
    let e = IMAGES[idx];
    document.getElementById('img').src = 'data:image/png;base64,' + e.b64;
    document.getElementById('fname').textContent = e.name;
    document.getElementById('counter').textContent = (idx+1)+'/'+TOTAL;
    let pct = ((Object.keys(state.labels).length + Object.keys(state.skipped).length)/TOTAL*100).toFixed(0);
    document.querySelector('#progress div').style.width = pct+'%';
    document.getElementById('n-labeled').textContent = Object.keys(state.labels).length;
    document.getElementById('n-skipped').textContent = Object.keys(state.skipped).length;
    let inp = document.getElementById('time-input');
    inp.value = state.labels[e.name] || '';
    inp.focus();
}}

function saveState() {{ state.idx = idx; localStorage.setItem(STORAGE_KEY, JSON.stringify(state)); updateUI(); }}

function confirmLabel() {{
    let val = document.getElementById('time-input').value.trim();
    if (!/^\\d{{4}}$/.test(val)) {{ document.getElementById('status').textContent = '需要4位数字'; return; }}
    if (parseInt(val.slice(2)) >= 60) {{ document.getElementById('status').textContent = '秒数>=60'; return; }}
    state.labels[IMAGES[idx].name] = val;
    delete state.skipped[IMAGES[idx].name];
    moveNext();
}}

function skipLabel() {{ state.skipped[IMAGES[idx].name] = true; delete state.labels[IMAGES[idx].name]; moveNext(); }}

function moveNext() {{
    for (let i=1; i<=TOTAL; i++) {{
        let n = (idx+i)%TOTAL;
        if (!state.labels[IMAGES[n].name] && !state.skipped[IMAGES[n].name]) {{ idx = n; saveState(); return; }}
    }}
    idx = 0; saveState();
    document.getElementById('status').textContent = '全部完成！点导出CSV';
}}

function exportCSV() {{
    let rows = ['filename,label,split'];
    let labeled = Object.entries(state.labels);
    let shuffled = labeled.map(e => ({{n:e[0],l:e[1],r:Math.random()}}));
    shuffled.sort((a,b)=>a.r-b.r);
    let nVal = Math.max(5, Math.floor(shuffled.length*0.15));
    for (let i=0; i<shuffled.length; i++) {{
        rows.push(shuffled[i].n+','+shuffled[i].l+','+(i<nVal?'val':'train'));
    }}
    let csv = rows.join('\\n');
    document.getElementById('csv-output').value = csv;
    document.getElementById('export-area').style.display = 'block';
    let blob = new Blob([csv], {{type:'text/csv'}});
    let url = URL.createObjectURL(blob);
    let a = document.createElement('a');
    a.href = url; a.download = 'annotations_v6.csv'; a.click();
    URL.revokeObjectURL(url);
    document.getElementById('status').textContent = '已导出 '+labeled.length+' 条';
}}

document.addEventListener('keydown', function(e) {{
    if (e.key === 'Enter') {{ e.preventDefault(); confirmLabel(); }}
    else if (e.key === 'Tab') {{ e.preventDefault(); skipLabel(); }}
    else if (e.key === 'ArrowLeft' && e.ctrlKey) {{ e.preventDefault(); idx=(idx-1+TOTAL)%TOTAL; saveState(); }}
    else if (e.key === 'ArrowRight' && e.ctrlKey) {{ e.preventDefault(); idx=(idx+1)%TOTAL; saveState(); }}
}});

updateUI();
</script>
</body>
</html>'''

out = Path(__file__).parent / 'annotate_cnn_v6.html'
out.write_text(html, encoding='utf-8')
print(f'Written: {out} ({out.stat().st_size//1024} KB)')
