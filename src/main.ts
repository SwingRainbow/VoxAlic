import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

interface AppConfig {
  close_to_tray: boolean;
  mission_timer: {
    mode: string;
    ocr_interval_secs: number;
    checkpoint_auto_focus: boolean;
    hp_alert_enabled: boolean;
    selected_hwnd: number;
    window_title: string;
    strip_frame: boolean;
    normal_roi: { x: number; y: number; w: number; h: number };
    fissure_roi: { x: number; y: number; w: number; h: number };
    life_support_roi: { x: number; y: number; w: number; h: number };
    fissure_hp_roi: { x: number; y: number; w: number; h: number };
  };
}

// ── Type definitions matching Rust AppStatePayload ──
interface Fissure {
  node_key: string;
  node_name: string;
  planet: string;
  mission_type: string;
  tier_key: string;
  tier_label: string;
  expiry_ms: number;
  is_hard: boolean;
  is_storm: boolean;
  remain_ms: number;
  remain_str: string;
  is_expiring: boolean;
}

interface CycleInfo {
  name: string;
  state: string;
  state_icon: string;
  remain_ms: number;
  is_day: boolean;
}

interface MissionTimerPayload {
  elapsed_secs: number;
  elapsed_str: string;
  state: string;
  mode: string;
  life_support_pct: number;
  life_support_level: string;
  status_text: string;
  detection_rate: number;
}

interface WindowInfo {
  title: string;
  hwnd: number;
}

interface AppStatePayload {
  normal_fissures: Fissure[];
  hard_fissures: Fissure[];
  storm_fissures: Fissure[];
  cycles: CycleInfo[];
  last_update: string;
  countdown_secs: number;
  mission_timer: MissionTimerPayload;
}

// ── Tier colors (match Python original) ──
const TIER_BG: Record<string, string> = {
  VoidT1: '#564b43', VoidT2: '#3e4140', VoidT3: '#383839',
  VoidT4: '#56523f', VoidT5: '#443037', VoidT6: '#384757',
};
const TIER_FG = '#ddd5c5';

let currentData: AppStatePayload | null = null;
let currentSubTab = 'normal';
let currentConfig: AppConfig | null = null;
function updateTimerConfig(partial: Record<string, any>) {
  if (!currentConfig) return;
  const newCfg = {
    ...currentConfig,
    mission_timer: { ...currentConfig.mission_timer, ...partial }
  };
  invoke('set_config', { config: newCfg });
}

// ── Format remaining time ──
function fmtRemain(ms: number): string {
  if (ms <= 0) return '切换中';
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m ${String(sec).padStart(2, '0')}s`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, '0')}s`;
  return `${sec}s`;
}

// ── Render functions ──
function renderCycles(cycles: CycleInfo[]) {
  const container = document.getElementById('cycle-cards')!;
  container.innerHTML = cycles.map(c => `
    <div class="cycle-card ${c.is_day ? 'day' : 'night'}">
      <div class="card-name">${c.name}</div>
      <div class="card-state">${c.state_icon} ${c.state}</div>
      <div class="card-time">剩余 ${fmtRemain(c.remain_ms)}</div>
    </div>
  `).join('');
}

function getFilteredFissures(): Fissure[] {
  if (!currentData) return [];
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  let list: Fissure[];
  if (currentSubTab === 'normal') list = currentData.normal_fissures;
  else if (currentSubTab === 'hard') list = currentData.hard_fissures;
  else list = currentData.storm_fissures;
  return list.filter(f => {
    if (f.remain_ms <= 0) return false;
    if (tier && f.tier_label !== tier) return false;
    if (type && f.mission_type !== type) return false;
    return true;
  });
}

function renderTimer(t: MissionTimerPayload) {
  // Sync time
  document.getElementById('timer-digits')!.textContent = t.elapsed_str;

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

  // Life support
  const lsBar = document.getElementById('ls-bar-fill')!;
  lsBar.style.width = `${t.life_support_pct}%`;
  lsBar.className = 'ls-bar-fill ' + t.life_support_level;
  document.getElementById('ls-pct')!.textContent =
    t.life_support_pct > 0 ? `${t.life_support_pct.toFixed(0)}%` : '--';

  // Mode radio
  document.querySelectorAll<HTMLInputElement>('input[name="timer-mode"]').forEach(r => {
    r.checked = r.value === t.mode;
  });

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

function renderFissures() {
  if (!currentData) return;
  const filtered = getFilteredFissures();
  const tbody = document.querySelector('#fissure-table tbody')!;
  tbody.innerHTML = filtered.map(f => `
    <tr class="${f.is_expiring ? 'expiring' : ''}" style="background:${TIER_BG[f.tier_key] || '#252525'};color:${TIER_FG};">
      <td><img src="/relics/${f.tier_key}.png" class="relic-icon" alt=""> ${f.tier_label}</td>
      <td>${f.node_name}</td>
      <td>${f.planet}</td>
      <td>${f.mission_type}</td>
      <td>${f.remain_str}</td>
    </tr>
  `).join('');

  // Update counts with current filters applied
  const tier = (document.getElementById('tier-filter') as HTMLSelectElement).value;
  const type = (document.getElementById('type-filter') as HTMLSelectElement).value;
  const countFiltered = (list: Fissure[]) => list.filter(f => {
    if (f.remain_ms <= 0) return false;
    if (tier && f.tier_label !== tier) return false;
    if (type && f.mission_type !== type) return false;
    return true;
  }).length;
  document.getElementById('count-normal')!.textContent = String(countFiltered(currentData.normal_fissures));
  document.getElementById('count-hard')!.textContent = String(countFiltered(currentData.hard_fissures));
  document.getElementById('count-storm')!.textContent = String(countFiltered(currentData.storm_fissures));
}

function updateFilters() {
  if (!currentData) return;
  const allFissures = [...currentData.normal_fissures, ...currentData.hard_fissures];
  const tiers = [...new Set(allFissures.map(f => f.tier_label))];
  const types = [...new Set(allFissures.map(f => f.mission_type).filter(t => t && t !== '--'))].sort();

  const tierSelect = document.getElementById('tier-filter') as HTMLSelectElement;
  const typeSelect = document.getElementById('type-filter') as HTMLSelectElement;
  const currentTier = tierSelect.value;
  const currentType = typeSelect.value;

  tierSelect.innerHTML = '<option value="">全部</option>' + tiers.map(t => `<option>${t}</option>`).join('');
  typeSelect.innerHTML = '<option value="">全部</option>' + types.map(t => `<option>${t}</option>`).join('');
  tierSelect.value = currentTier;
  typeSelect.value = currentType;
}

function handleUpdate(payload: AppStatePayload) {
  currentData = payload;
  document.getElementById('status-text')!.textContent =
    `更新于 ${payload.last_update}  下次刷新 ${payload.countdown_secs}s`;
  renderCycles(payload.cycles);
  updateFilters();
  renderFissures();
  renderTimer(payload.mission_timer);
}

// ── Event listeners ──
window.addEventListener('DOMContentLoaded', () => {
  // Tab switching
  document.querySelectorAll('.tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      const tab = (btn as HTMLElement).dataset.tab;
      document.querySelectorAll('.tab-btn, .tab-content').forEach(e => e.classList.remove('active'));
      btn.classList.add('active');
      document.getElementById(`tab-${tab}`)!.classList.add('active');
      if (tab === 'timer') {
        document.getElementById('btn-refresh-windows')!.click();
      }
    });
  });

  // Sub-tab switching
  document.querySelectorAll('.sub-tab-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.sub-tab-btn').forEach(e => e.classList.remove('active'));
      btn.classList.add('active');
      currentSubTab = (btn as HTMLElement).dataset.sub!;
      renderFissures();
    });
  });

  // Refresh button
  document.getElementById('btn-refresh')!.addEventListener('click', () => {
    invoke('refresh_now');
  });

  // Filters
  document.getElementById('tier-filter')!.addEventListener('change', renderFissures);
  document.getElementById('type-filter')!.addEventListener('change', renderFissures);

  // Settings: load config
  const closeToggle = document.getElementById('setting-close-to-tray') as HTMLInputElement;
  invoke<AppConfig>('get_config').then(cfg => {
    currentConfig = cfg;
    closeToggle.checked = cfg.close_to_tray;
    // Init timer settings
    const mt = cfg.mission_timer;
    document.getElementById('ocr-interval')!.setAttribute('value', String(mt.ocr_interval_secs));
    (document.getElementById('toggle-checkpoint-focus') as HTMLInputElement).checked = mt.checkpoint_auto_focus;
    (document.getElementById('toggle-hp-alert') as HTMLInputElement).checked = mt.hp_alert_enabled;
  });

  // Settings: save on change
  closeToggle.addEventListener('change', () => {
    invoke('set_config', { config: { close_to_tray: closeToggle.checked } });
  });

  // Timer: start/stop/reset buttons
  document.getElementById('btn-timer-start')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'start' });
  });
  document.getElementById('btn-timer-stop')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'stop' });
  });
  document.getElementById('btn-timer-reset')!.addEventListener('click', () => {
    invoke('timer_command', { command: 'reset' });
  });

  // Timer: mode radio
  document.querySelectorAll('input[name="timer-mode"]').forEach(radio => {
    radio.addEventListener('change', () => {
      if ((radio as HTMLInputElement).checked) {
        invoke('timer_command', { command: 'set_mode', mode: (radio as HTMLInputElement).value });
      }
    });
  });

  // Window selection
  const windowSelect = document.getElementById('window-select') as HTMLSelectElement;
  document.getElementById('btn-refresh-windows')!.addEventListener('click', async () => {
    const windows: WindowInfo[] = await invoke('list_windows');
    windowSelect.innerHTML = windows.length === 0
      ? '<option value="0">未找到窗口</option>'
      : windows.map(w => `<option value="${w.hwnd}">${w.title}</option>`).join('');
    document.getElementById('window-count')!.textContent = `${windows.length}个窗口`;
    if (windows.length > 0) {
      windowSelect.value = String(windows[0].hwnd);
      invoke('select_window', { hwnd: windows[0].hwnd });
    }
  });
  windowSelect.addEventListener('change', () => {
    invoke('select_window', { hwnd: parseInt(windowSelect.value) });
  });

  // Single capture button
  document.getElementById('btn-single-capture')!.addEventListener('click', () => {
    invoke('single_capture');
  });

  // OCR interval
  const ocrInterval = document.getElementById('ocr-interval') as HTMLInputElement;
  ocrInterval.addEventListener('change', () => {
    const val = Math.max(1, Math.min(30, parseInt(ocrInterval.value) || 2));
    ocrInterval.value = String(val);
    invoke('set_config', { config: {
      ...currentConfig,
      mission_timer: { ...currentConfig?.mission_timer, ocr_interval_secs: val }
    } });
  });

  // Toggle: checkpoint auto-focus
  document.getElementById('toggle-checkpoint-focus')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ checkpoint_auto_focus: this.checked });
  });
  // Toggle: HP alert
  document.getElementById('toggle-hp-alert')!.addEventListener('change', function(this: HTMLInputElement) {
    updateTimerConfig({ hp_alert_enabled: this.checked });
  });

  // Log listener
  let logLines: string[] = [];
  const MAX_LOG = 200;
  const logContent = document.getElementById('log-content')!;
  listen<string>('timer-log', (event) => {
    logLines.push(event.payload);
    if (logLines.length > MAX_LOG) logLines = logLines.slice(-MAX_LOG);
    logContent.innerHTML = logLines.map(line => {
      let cls = 'log-info';
      if (line.includes('⚠')) cls = 'log-warn';
      else if (line.includes('同步') || line.includes('OCR')) cls = 'log-ok';
      return `<div class="${cls}">${line}</div>`;
    }).join('');
    logContent.scrollTop = logContent.scrollHeight;
  });

  document.getElementById('btn-clear-log')!.addEventListener('click', () => {
    logLines = [];
    logContent.innerHTML = '';
  });

  // Tauri events
  listen<AppStatePayload>('worldstate-update', (event) => {
    handleUpdate(event.payload);
  });

  listen<AppStatePayload>('tick-update', (event) => {
    handleUpdate(event.payload);
  });
});
