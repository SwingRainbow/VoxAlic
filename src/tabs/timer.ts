import {
  type MissionTimerPayload, type Box,
  S,
} from '../state';
import { invoke } from '@tauri-apps/api/core';

export function renderTimer(t: MissionTimerPayload) {
  // Sync time
  document.getElementById('timer-digits')!.textContent = t.elapsed_str;

  // OCR raw digits (from backend, if available)
  const ocrEl = document.getElementById('timer-ocr-digits');
  if (ocrEl && t.ocr_raw) {
    ocrEl.textContent = t.ocr_raw;
  } else if (ocrEl && t.state === 'idle') {
    ocrEl.textContent = '--:--';
  }

  // Status
  const statusEl = document.getElementById('timer-status')!;
  statusEl.textContent = t.status_text;
  statusEl.className = 'timer-status';
  if (t.state === 'checkpoint') statusEl.classList.add('checkpoint');

  // Detection rate
  const rateEl = document.getElementById('timer-rate')!;
  if (t.state === 'running') {
    rateEl.textContent = `识别率 ${t.detection_rate.toFixed(0)}%`;
  } else if (t.state === 'idle') {
    rateEl.textContent = '';
  }

  // Window status
  const wsEl = document.getElementById('window-status');
  if (wsEl && t.window_status) {
    wsEl.textContent = t.window_status;
    wsEl.className = 'window-status ' + (t.window_status.includes('检测到') ? 'found' : 'not-found');
  }

  // Life support indicator
  const lsDot = document.getElementById('ls-dot')!;
  const lsStatus = document.getElementById('ls-status')!;
  if (t.state === 'idle') {
    lsDot.className = 'ls-dot';
    lsStatus.className = 'ls-status';
    lsStatus.textContent = '--';
  } else if (t.life_support_level === 'danger') {
    lsDot.className = 'ls-dot danger';
    lsStatus.className = 'ls-status danger';
    lsStatus.textContent = '≤20%';
  } else {
    lsDot.className = 'ls-dot normal';
    lsStatus.className = 'ls-status';
    lsStatus.textContent = '正常';
  }

  // Next checkpoint countdown
  if (t.state === 'running') {
    const elapsed = t.elapsed_secs;
    const next5 = Math.ceil(elapsed / 300) * 300;
    const remain = next5 - elapsed;
    const m5 = Math.floor(next5 / 60);
    const s5 = next5 % 60;
    document.getElementById('cp-target')!.textContent = `${m5}:${String(s5).padStart(2, '0')}`;
    const rm5 = Math.floor(remain / 60);
    const rs5 = remain % 60;
    document.getElementById('cp-remain')!.textContent = `${rm5}:${String(rs5).padStart(2, '0')}`;
    document.getElementById('next-checkpoint')!.style.display = '';
  } else {
    document.getElementById('next-checkpoint')!.style.display = 'none';
  }
}

export function updateTimerConfig(partial: Record<string, any>) {
  if (!S.currentConfig) return;
  const newCfg = {
    ...S.currentConfig,
    mission_timer: { ...S.currentConfig.mission_timer, ...partial }
  };
  S.currentConfig = newCfg;
  invoke('set_config', { config: newCfg });
}

export function getTimerMode(): 'normal' | 'fissure' {
  const r = document.querySelector<HTMLInputElement>('input[name="timer-mode"]:checked');
  return (r?.value === 'fissure') ? 'fissure' : 'normal';
}

export function setupCalibration() {
  const canvas = document.getElementById('calib-canvas') as HTMLCanvasElement;
  const ctx = canvas.getContext('2d')!;
  const resultEl = document.getElementById('calib-result')!;
  const btnTime = document.getElementById('btn-calib-time')!;
  const btnLs = document.getElementById('btn-calib-ls')!;

  let img: HTMLImageElement | null = null;
  const boxes: { time: Box | null; ls: Box | null } = { time: null, ls: null };
  let activeTool: 'time' | 'ls' | null = null;
  let dragging = false;
  let startX = 0, startY = 0;

  function roiToBox(roi: { x: number; y: number; w: number; h: number }): Box {
    return { x: roi.x * canvas.width, y: roi.y * canvas.height, w: roi.w * canvas.width, h: roi.h * canvas.height };
  }

  function redraw() {
    if (!img) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    const draw = (b: Box | null, color: string, label: string) => {
      if (!b) return;
      ctx.lineWidth = 2;
      ctx.strokeStyle = color;
      ctx.strokeRect(b.x, b.y, b.w, b.h);
      ctx.fillStyle = color;
      ctx.font = '14px sans-serif';
      ctx.fillText(label, b.x + 2, Math.max(14, b.y - 4));
    };
    draw(boxes.time, '#4CAF50', '时间');
    draw(boxes.ls, '#FF9800', '维生');
  }

  // Map a pointer event to canvas backing-store pixel coordinates.
  function toCanvasXY(e: MouseEvent): [number, number] {
    const rect = canvas.getBoundingClientRect();
    const sx = canvas.width / rect.width;
    const sy = canvas.height / rect.height;
    return [(e.clientX - rect.left) * sx, (e.clientY - rect.top) * sy];
  }

  function setTool(tool: 'time' | 'ls' | null) {
    activeTool = tool;
    btnTime.classList.toggle('active', tool === 'time');
    btnLs.classList.toggle('active', tool === 'ls');
  }

  btnTime.addEventListener('click', () => setTool(activeTool === 'time' ? null : 'time'));
  btnLs.addEventListener('click', () => setTool(activeTool === 'ls' ? null : 'ls'));

  canvas.addEventListener('mousedown', (e) => {
    if (!img || !activeTool) return;
    dragging = true;
    [startX, startY] = toCanvasXY(e);
  });
  canvas.addEventListener('mousemove', (e) => {
    if (!dragging || !activeTool) return;
    const [cx, cy] = toCanvasXY(e);
    boxes[activeTool] = {
      x: Math.min(startX, cx), y: Math.min(startY, cy),
      w: Math.abs(cx - startX), h: Math.abs(cy - startY),
    };
    redraw();
  });
  const endDrag = () => { dragging = false; };
  canvas.addEventListener('mouseup', endDrag);
  canvas.addEventListener('mouseleave', endDrag);

  // Capture current game frame into the canvas.
  document.getElementById('btn-calib-capture')!.addEventListener('click', () => {
    resultEl.textContent = '截取中…';
    invoke<string>('capture_preview').then(dataUrl => {
      const im = new Image();
      im.onload = () => {
        img = im;
        canvas.width = im.naturalWidth;
        canvas.height = im.naturalHeight;
        // Seed boxes from current config so existing ROIs show as starting frames.
        if (S.currentConfig) {
          const mt = S.currentConfig.mission_timer;
          const mode = getTimerMode();
          boxes.time = roiToBox(mode === 'fissure' ? mt.fissure_roi : mt.normal_roi);
          boxes.ls = roiToBox(mode === 'fissure' ? mt.fissure_hp_roi : mt.life_support_roi);
        }
        redraw();
        resultEl.textContent = '已截图，拖框校准';
      };
      im.src = dataUrl;
    }).catch(err => { resultEl.textContent = String(err); });
  });

  // Test OCR on the current time box.
  document.getElementById('btn-calib-test')!.addEventListener('click', () => {
    if (!img || !boxes.time) { resultEl.textContent = '请先框选时间框'; return; }
    const b = boxes.time;
    const args = { x: b.x / canvas.width, y: b.y / canvas.height, w: b.w / canvas.width, h: b.h / canvas.height };
    resultEl.textContent = '识别中…';
    invoke<string>('test_recognize', args)
      .then(r => { resultEl.textContent = `识别: ${r}`; })
      .catch(err => { resultEl.textContent = String(err); });
  });

  // Save both boxes into the current mode's ROIs.
  document.getElementById('btn-calib-save')!.addEventListener('click', () => {
    if (!img || !boxes.time || !boxes.ls) { resultEl.textContent = '请先截图并框选两个区域'; return; }
    const frac = (b: Box) => ({ x: b.x / canvas.width, y: b.y / canvas.height, w: b.w / canvas.width, h: b.h / canvas.height });
    const mode = getTimerMode();
    if (mode === 'fissure') {
      updateTimerConfig({ fissure_roi: frac(boxes.time), fissure_hp_roi: frac(boxes.ls) });
    } else {
      updateTimerConfig({ normal_roi: frac(boxes.time), life_support_roi: frac(boxes.ls) });
    }
    resultEl.textContent = `已保存（${mode === 'fissure' ? '裂缝' : '普通'}模式）`;
  });
}
