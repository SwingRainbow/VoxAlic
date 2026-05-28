import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

interface AppConfig {
  close_to_tray: boolean;
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
  document.getElementById('timer-digits')!.textContent = t.elapsed_str;
  const statusEl = document.getElementById('timer-status')!;
  statusEl.textContent = t.status_text;
  statusEl.className = 'timer-status';
  if (t.state === 'checkpoint') {
    statusEl.classList.add('checkpoint');
  }

  const lsBar = document.getElementById('ls-bar-fill')!;
  lsBar.style.width = `${t.life_support_pct}%`;
  lsBar.className = 'ls-bar-fill ' + t.life_support_level;
  document.getElementById('ls-pct')!.textContent =
    t.life_support_pct > 0 ? `${t.life_support_pct.toFixed(0)}%` : '--';

  const modeRadios = document.getElementsByName('timer-mode') as NodeListOf<HTMLInputElement>;
  modeRadios.forEach(r => { r.checked = r.value === t.mode; });
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
    closeToggle.checked = cfg.close_to_tray;
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

  // Tauri events
  listen<AppStatePayload>('worldstate-update', (event) => {
    handleUpdate(event.payload);
  });

  listen<AppStatePayload>('tick-update', (event) => {
    handleUpdate(event.payload);
  });
});
